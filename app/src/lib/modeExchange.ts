import { visit } from 'jsonc-parser';
import {
  AVAILABLE_MODEL_OPTIONS, BUILTIN_MODES, LANGUAGE_OPTIONS, SITE_MODE_BROWSERS,
  WRITING_STYLE_VALUES, normalizeSiteHost,
  type AppProfile, type BrowserSiteRule, type MurmurMode, type Settings,
} from './settings';

export const MAX_MODE_FILE_BYTES = 256 * 1024;
export type ModeExchangeSettings = Pick<Settings, 'modes' | 'appProfiles' | 'browserSiteRules' | 'siteModeLookupEnabled'>;
type AppBinding = { bundleId: AppProfile['bundleId']; modeId: MurmurMode['id'] };
export interface ModeFile {
  format: 'murmur-modes';
  version: 1;
  modes: MurmurMode[];
  appBindings: AppBinding[];
  browserSiteRules: BrowserSiteRule[];
}

function object(value: unknown, keys: readonly string[]): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)
    || Object.keys(value).length !== keys.length || !keys.every((key) => Object.prototype.hasOwnProperty.call(value, key))) {
    throw new Error('The Mode file has missing or unknown fields.');
  }
  return value as Record<string, unknown>;
}
function text(value: unknown, max = 128): string {
  if (typeof value !== 'string' || !value || value.trim() !== value || new TextEncoder().encode(value).length > max
    || /[\u0000-\u001f\u007f\uD800-\uDFFF]/u.test(value)) throw new Error('The Mode file contains an invalid string.');
  return value;
}
function boolean(value: unknown): boolean {
  if (typeof value !== 'boolean') throw new Error('The Mode file contains an invalid boolean.');
  return value;
}
function nullableBoolean(value: unknown): boolean | null {
  return value === null ? null : boolean(value);
}
function choice<T extends string>(value: unknown, options: readonly T[]): T {
  const found = options.find((option) => option === value);
  if (found === undefined) throw new Error('The Mode file contains an unsupported policy, model, language, or browser.');
  return found;
}
function array<T>(value: unknown, limit: number, parse: (entry: unknown) => T): T[] {
  if (!Array.isArray(value) || value.length > limit) throw new Error(`The Mode file exceeds a list limit of ${limit} or contains an invalid list.`);
  return value.map(parse);
}
function parseMode(value: unknown): MurmurMode {
  const mode = object(value, ['id', 'name', 'builtIn', 'enabled', 'writingStyle', 'cleanupEnabled',
    'smartFormattingEnabled', 'cliFormattingEnabled', 'vocabularyPolicy', 'contextPolicy', 'modelId', 'language', 'autoPaste']);
  const id = text(mode.id);
  if (mode.builtIn !== false || id.startsWith('builtin.') || BUILTIN_MODES.some((builtin) => builtin.id === id)) {
    throw new Error('Built-in Modes cannot be imported or replaced.');
  }
  return {
    id, name: text(mode.name), builtIn: false, enabled: boolean(mode.enabled),
    writingStyle: mode.writingStyle === null ? null : choice(mode.writingStyle, WRITING_STYLE_VALUES),
    cleanupEnabled: nullableBoolean(mode.cleanupEnabled),
    smartFormattingEnabled: nullableBoolean(mode.smartFormattingEnabled),
    cliFormattingEnabled: nullableBoolean(mode.cliFormattingEnabled),
    vocabularyPolicy: choice(mode.vocabularyPolicy, ['inherit', 'general', 'technical']),
    contextPolicy: choice(mode.contextPolicy, ['none', 'project']),
    modelId: mode.modelId === null ? null : choice(mode.modelId, AVAILABLE_MODEL_OPTIONS.map((option) => option.value)),
    language: mode.language === null ? null : choice(mode.language, LANGUAGE_OPTIONS.map((option) => option.value)),
    autoPaste: nullableBoolean(mode.autoPaste),
  };
}

export function parseModeFile(contents: string): ModeFile {
  if (new TextEncoder().encode(contents).length > MAX_MODE_FILE_BYTES) throw new Error('Mode files must be 256 KiB or smaller.');
  const keySets: Set<string>[] = [];
  let depth = 0;
  visit(contents, {
    onObjectBegin: () => { if (++depth > 8) throw new Error('The Mode file is nested too deeply.'); keySets.push(new Set()); },
    onObjectProperty: (key) => {
      const keys = keySets[keySets.length - 1];
      if (keys?.has(key)) throw new Error('Duplicate JSON keys are not allowed.');
      keys?.add(key);
    },
    onObjectEnd: () => { keySets.pop(); depth--; },
    onArrayBegin: () => { if (++depth > 8) throw new Error('The Mode file is nested too deeply.'); },
    onArrayEnd: () => { depth--; },
    onError: () => { throw new Error('Choose a valid JSON Mode file.'); },
  }, { disallowComments: true, allowTrailingComma: false });
  let value: unknown;
  try { value = JSON.parse(contents); } catch { throw new Error('Choose a valid JSON Mode file.'); }
  const file = object(value, ['format', 'version', 'modes', 'appBindings', 'browserSiteRules']);
  if (file.format !== 'murmur-modes' || file.version !== 1) throw new Error('This Mode file format or version is not supported.');
  const modes = array(file.modes, 100, parseMode);
  const ids = new Set(modes.map((mode) => mode.id));
  const reference = (value: unknown) => {
    const id = text(value);
    if (!ids.has(id)) throw new Error('Every binding must reference a custom Mode included in the file.');
    return id;
  };
  const appBindings = array(file.appBindings, 100, (value): AppBinding => {
    const binding = object(value, ['bundleId', 'modeId']);
    const bundleId = text(binding.bundleId, 255);
    if (!/^[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+$/.test(bundleId)) throw new Error('An app binding has an invalid bundle ID.');
    return { bundleId, modeId: reference(binding.modeId) };
  });
  const browserSiteRules = array(file.browserSiteRules, 128, (value): BrowserSiteRule => {
    const rule = object(value, ['id', 'browserBundleId', 'host', 'modeId', 'enabled']);
    const host = text(rule.host, 253);
    if (normalizeSiteHost(host) !== host) throw new Error('Site rules require a lowercase exact host without a URL or trailing dot.');
    return { id: text(rule.id), browserBundleId: choice(rule.browserBundleId, SITE_MODE_BROWSERS.map((browser) => browser.bundleId)),
      host, modeId: reference(rule.modeId), enabled: boolean(rule.enabled) };
  });
  return { format: 'murmur-modes', version: 1, modes, appBindings, browserSiteRules };
}

export function exportModeFile(settings: ModeExchangeSettings): string {
  const modes = settings.modes.filter((mode) => !mode.builtIn && !mode.id.startsWith('builtin.'));
  const ids = new Set(modes.map((mode) => mode.id));
  const file: ModeFile = {
    format: 'murmur-modes', version: 1, modes,
    appBindings: settings.appProfiles.flatMap((profile) => profile.modeId && ids.has(profile.modeId)
      ? [{ bundleId: profile.bundleId, modeId: profile.modeId }] : []),
    browserSiteRules: settings.browserSiteRules.filter((rule) => ids.has(rule.modeId)),
  };
  return JSON.stringify(parseModeFile(JSON.stringify(file)), null, 2) + '\n';
}

function equal<T extends object>(left: T, right: T): boolean {
  return Object.keys(left).length === Object.keys(right).length
    && Object.entries(left).every(([key, value]) => Object.prototype.hasOwnProperty.call(right, key) && Reflect.get(right, key) === value);
}
export type ModeImportPreview = {
  baseline: string;
  file: ModeFile;
  counts: { modes: number; bindings: number; sites: number; duplicates: number; missingApps: number };
} & ({ kind: 'ready'; next: ModeExchangeSettings } | { kind: 'conflict'; conflicts: string[] });

export function modeSettingsSnapshot(settings: ModeExchangeSettings): string {
  return JSON.stringify(settings);
}

export function previewModeImport(file: ModeFile, settings: ModeExchangeSettings): ModeImportPreview {
  const modes = [...settings.modes];
  const appProfiles = settings.appProfiles.map((profile) => ({ ...profile }));
  const browserSiteRules = [...settings.browserSiteRules];
  const conflicts: string[] = [];
  const counts = { modes: 0, bindings: 0, sites: 0, duplicates: 0, missingApps: 0 };
  for (const mode of file.modes) {
    const existing = modes.find((item) => item.id === mode.id);
    if (!existing) { modes.push(mode); counts.modes++; }
    else if (equal(existing, mode)) counts.duplicates++;
    else conflicts.push(`Mode ID conflict: ${mode.id}`);
  }
  const importedBindings = new Map<string, string>();
  for (const binding of file.appBindings) {
    const previous = importedBindings.get(binding.bundleId);
    if (previous && previous !== binding.modeId) { conflicts.push(`App binding conflict: ${binding.bundleId}`); continue; }
    importedBindings.set(binding.bundleId, binding.modeId);
    const matches = appProfiles.filter((profile) => profile.bundleId === binding.bundleId);
    if (matches.length === 0) { counts.missingApps++; continue; }
    if (matches.length > 1) { conflicts.push(`Ambiguous app profile: ${binding.bundleId}`); continue; }
    for (const profile of matches) {
      if (profile.modeId === binding.modeId) counts.duplicates++;
      else if (profile.modeId) conflicts.push(`App binding conflict: ${binding.bundleId}`);
      else { profile.modeId = binding.modeId; counts.bindings++; }
    }
  }
  for (const rule of file.browserSiteRules) {
    const existing = browserSiteRules.find((item) => item.id === rule.id
      || (item.browserBundleId === rule.browserBundleId && item.host === rule.host));
    if (!existing) { browserSiteRules.push(rule); counts.sites++; }
    else if (equal(existing, rule)) counts.duplicates++;
    else conflicts.push(`Site rule conflict: ${rule.host}`);
  }
  if (modes.length > 100) conflicts.push('Import would exceed the 100 custom Mode limit.');
  if (browserSiteRules.length > 128) conflicts.push('Import would exceed the 128 site rule limit.');
  const base = { baseline: modeSettingsSnapshot(settings), file, counts };
  return conflicts.length ? { ...base, kind: 'conflict', conflicts } : {
    ...base, kind: 'ready', next: { modes, appProfiles, browserSiteRules, siteModeLookupEnabled: settings.siteModeLookupEnabled },
  };
}

export function commitModeImport(preview: ModeImportPreview, settings: ModeExchangeSettings): ModeExchangeSettings {
  if (preview.baseline !== modeSettingsSnapshot(settings)) throw new Error('Modes or bindings changed. Choose the file again to review a fresh preview.');
  if (preview.kind === 'conflict') throw new Error('Resolve the listed conflicts before importing. Nothing was changed.');
  return preview.next;
}
