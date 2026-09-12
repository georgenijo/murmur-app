import { visit } from 'jsonc-parser';
import {
  MAX_VOCABULARY_ALIASES,
  MAX_VOCABULARY_ENTRIES,
  MAX_VOCABULARY_VALUE_CHARS,
  type Settings,
  type VocabularyEntry,
  type VocabularyScope,
} from './settings';
import { validateVocabularyEntries } from './vocabulary';

export const MAX_VOCABULARY_FILE_BYTES = 256 * 1024;
export type VocabularyExchangeSettings = Pick<Settings, 'vocabularyEntries' | 'voiceCommands'>;

export interface VocabularyFile {
  format: 'murmur-vocabulary';
  version: 1;
  entries: VocabularyEntry[];
}

function object(value: unknown, keys: readonly string[]): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)
    || Object.keys(value).length !== keys.length
    || !keys.every((key) => Object.prototype.hasOwnProperty.call(value, key))) {
    throw new Error('The Vocabulary file has missing or unknown fields.');
  }
  return value as Record<string, unknown>;
}

function text(value: unknown, label: string, maxChars: number): string {
  if (typeof value !== 'string' || !value || Array.from(value).length > maxChars
    || /[\u0000-\u001f\u007f\uD800-\uDFFF]/u.test(value)) {
    throw new Error(`The Vocabulary file contains an invalid ${label}.`);
  }
  return value;
}

function boolean(value: unknown): boolean {
  if (typeof value !== 'boolean') throw new Error('The Vocabulary file contains an invalid enabled flag.');
  return value;
}

function array<T>(value: unknown, limit: number, parse: (entry: unknown) => T): T[] {
  if (!Array.isArray(value) || value.length > limit) {
    throw new Error(`The Vocabulary file exceeds a list limit of ${limit} or contains an invalid list.`);
  }
  return value.map(parse);
}

function parseScope(value: unknown): VocabularyScope {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    throw new Error('The Vocabulary file contains an invalid scope.');
  }
  const scope = value as Record<string, unknown>;
  if (scope.kind === 'global') {
    object(scope, ['kind']);
    return { kind: 'global' };
  }
  if (scope.kind === 'app') {
    object(scope, ['kind', 'bundleId']);
    const bundleId = text(scope.bundleId, 'app bundle ID', 255);
    if (!/^[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+$/.test(bundleId)) {
      throw new Error('The Vocabulary file contains an invalid app bundle ID.');
    }
    return { kind: 'app', bundleId };
  }
  if (scope.kind === 'project') {
    object(scope, ['kind', 'bundleId', 'root']);
    const bundleId = text(scope.bundleId, 'project bundle ID', 255);
    if (!/^[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+$/.test(bundleId)) {
      throw new Error('The Vocabulary file contains an invalid project bundle ID.');
    }
    const root = text(scope.root, 'project root', 4096);
    if (new TextEncoder().encode(root).length > 4096) {
      throw new Error('The Vocabulary file contains an invalid project root.');
    }
    return { kind: 'project', bundleId, root };
  }
  throw new Error('The Vocabulary file contains an unsupported scope.');
}

function parseEntry(value: unknown): VocabularyEntry {
  const entry = object(value, ['id', 'written', 'aliases', 'enabled', 'scope']);
  const aliases = array(entry.aliases, MAX_VOCABULARY_ALIASES, (alias) =>
    text(alias, 'spoken alias', MAX_VOCABULARY_VALUE_CHARS));
  const written = text(entry.written, 'written form', MAX_VOCABULARY_VALUE_CHARS);
  return {
    id: text(entry.id, 'entry ID', 256),
    written,
    aliases,
    enabled: boolean(entry.enabled),
    scope: parseScope(entry.scope),
  };
}

export function parseVocabularyFile(contents: string): VocabularyFile {
  if (new TextEncoder().encode(contents).length > MAX_VOCABULARY_FILE_BYTES) {
    throw new Error('Vocabulary files must be 256 KiB or smaller.');
  }
  const keySets: Set<string>[] = [];
  let depth = 0;
  visit(contents, {
    onObjectBegin: () => {
      if (++depth > 8) throw new Error('The Vocabulary file is nested too deeply.');
      keySets.push(new Set());
    },
    onObjectProperty: (key) => {
      const keys = keySets[keySets.length - 1];
      if (keys?.has(key)) throw new Error('Duplicate JSON keys are not allowed.');
      keys?.add(key);
    },
    onObjectEnd: () => { keySets.pop(); depth--; },
    onArrayBegin: () => { if (++depth > 8) throw new Error('The Vocabulary file is nested too deeply.'); },
    onArrayEnd: () => { depth--; },
    onError: () => { throw new Error('Choose a valid JSON Vocabulary file.'); },
  }, { disallowComments: true, allowTrailingComma: false });
  let value: unknown;
  try { value = JSON.parse(contents); } catch { throw new Error('Choose a valid JSON Vocabulary file.'); }
  const file = object(value, ['format', 'version', 'entries']);
  if (file.format !== 'murmur-vocabulary' || file.version !== 1) {
    throw new Error('This Vocabulary file format or version is not supported.');
  }
  const entries = array(file.entries, MAX_VOCABULARY_ENTRIES, parseEntry);
  const validationError = validateVocabularyEntries(entries, []);
  if (validationError) throw new Error(`The Vocabulary file is invalid: ${validationError}`);
  return { format: 'murmur-vocabulary', version: 1, entries };
}

export function exportVocabularyFile(settings: VocabularyExchangeSettings): string {
  const file: VocabularyFile = { format: 'murmur-vocabulary', version: 1, entries: settings.vocabularyEntries };
  const validated = parseVocabularyFile(JSON.stringify(file));
  const pretty = `${JSON.stringify(validated, null, 2)}\n`;
  return new TextEncoder().encode(pretty).length <= MAX_VOCABULARY_FILE_BYTES
    ? pretty : JSON.stringify(validated);
}

function equal(left: VocabularyEntry, right: VocabularyEntry): boolean {
  if (left.id !== right.id || left.written !== right.written || left.enabled !== right.enabled
    || left.aliases.length !== right.aliases.length
    || left.aliases.some((alias, index) => alias !== right.aliases[index])) return false;
  if (left.scope.kind !== right.scope.kind) return false;
  if (left.scope.kind === 'global' || right.scope.kind === 'global') return true;
  if (left.scope.bundleId !== right.scope.bundleId) return false;
  if (left.scope.kind === 'app') return true;
  return right.scope.kind === 'project' && left.scope.root === right.scope.root;
}

function scopesOverlap(left: VocabularyScope, right: VocabularyScope): boolean {
  if (left.kind === 'global' || right.kind === 'global') return true;
  if (left.kind === 'app' && right.kind === 'app') return left.bundleId === right.bundleId;
  if (left.kind === 'project' && right.kind === 'project') {
    return left.bundleId === right.bundleId && left.root === right.root;
  }
  return left.bundleId === right.bundleId;
}

function sameWritten(left: VocabularyEntry, right: VocabularyEntry): boolean {
  return left.written.trim().replace(/\s+/g, ' ').toLowerCase()
    === right.written.trim().replace(/\s+/g, ' ').toLowerCase();
}

export type VocabularyImportPreview = {
  baseline: string;
  file: VocabularyFile;
  counts: { entries: number; duplicates: number };
} & ({ kind: 'ready'; next: VocabularyExchangeSettings } | { kind: 'conflict'; conflicts: string[] });

export function vocabularySettingsSnapshot(settings: VocabularyExchangeSettings): string {
  return JSON.stringify(settings);
}

export function previewVocabularyImport(
  file: VocabularyFile,
  settings: VocabularyExchangeSettings,
): VocabularyImportPreview {
  const entries = [...settings.vocabularyEntries];
  const conflicts: string[] = [];
  const counts = { entries: 0, duplicates: 0 };
  for (const imported of file.entries) {
    const existingById = entries.find((entry) => entry.id === imported.id);
    if (existingById) {
      if (equal(existingById, imported)) counts.duplicates++;
      else conflicts.push(`Entry ID conflict: ${imported.id}`);
      continue;
    }
    const writtenConflict = entries.find((entry) =>
      entry.enabled && imported.enabled && scopesOverlap(entry.scope, imported.scope) && sameWritten(entry, imported));
    if (writtenConflict) {
      conflicts.push(`Written term conflict: ${imported.written.trim()}`);
      continue;
    }
    entries.push(imported);
    counts.entries++;
  }
  if (entries.length > MAX_VOCABULARY_ENTRIES) {
    conflicts.push(`Import would exceed the ${MAX_VOCABULARY_ENTRIES} vocabulary entry limit.`);
  }
  const validationError = validateVocabularyEntries(entries, settings.voiceCommands);
  if (validationError) conflicts.push(validationError);
  const base = { baseline: vocabularySettingsSnapshot(settings), file, counts };
  return conflicts.length ? { ...base, kind: 'conflict', conflicts } : {
    ...base,
    kind: 'ready',
    next: { vocabularyEntries: entries, voiceCommands: settings.voiceCommands },
  };
}

export function commitVocabularyImport(
  preview: VocabularyImportPreview,
  settings: VocabularyExchangeSettings,
): VocabularyExchangeSettings {
  if (preview.baseline !== vocabularySettingsSnapshot(settings)) {
    throw new Error('Saved spellings changed. Choose the file again to review a fresh preview.');
  }
  if (preview.kind === 'conflict') throw new Error('Resolve the listed conflicts before importing. Nothing was changed.');
  return preview.next;
}
