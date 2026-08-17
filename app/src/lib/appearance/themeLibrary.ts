import { mirrorDurableBlob, THEME_LIBRARY_STORE } from '../durableUserData';
import { DEFAULT_THEME } from './palettes';
import { resolveTheme } from './resolve';
import { sanitizeRevision, sanitizeTheme } from './sanitize';
import {
  MAX_APPEARANCE_REVISION,
  type ResolvedAppearance,
  type AppearanceDocumentV1,
  type AppearanceSelectionV1,
  type ThemeConfigV1,
  type ThemeLibraryCollectionV1,
  type ThemeLibraryDocumentV1,
  type ThemeLibraryEntryV1,
  type ThemeLibrarySourceV1,
  type ThemeImportPreview,
} from './types';

export const THEME_LIBRARY_STORAGE_KEY = 'murmur-theme-library';
export const THEME_LIBRARY_VERSION = 1 as const;
export const MAX_THEME_LIBRARY_BYTES = 1024 * 1024;
export const MAX_THEME_LIBRARY_ENTRIES = 128;

export interface ThemeLibraryStorageLike {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

export type ThemeLibraryLoadResult =
  | { status: 'ready'; document: ThemeLibraryDocumentV1; needsRepair: boolean }
  | { status: 'unavailable'; document: ThemeLibraryDocumentV1; error: string };

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function byteLength(value: string): number {
  return typeof TextEncoder === 'undefined' ? value.length : new TextEncoder().encode(value).byteLength;
}

function isThemeId(value: unknown): value is string {
  return typeof value === 'string' && /^[a-z0-9](?:[a-z0-9-]{0,63})$/.test(value);
}

function isLabel(value: unknown): value is string {
  return typeof value === 'string' && value.trim().length > 0 && value.trim().length <= 64;
}

function parseModes(value: unknown): ResolvedAppearance[] | null {
  if (!Array.isArray(value)) return null;
  const modes = (['light', 'dark'] as const).filter((mode) => value.includes(mode));
  return modes.length > 0 ? modes : null;
}

function parseCollection(value: unknown): ThemeLibraryCollectionV1 | undefined | null {
  if (value === undefined) return undefined;
  if (!isRecord(value) || !isLabel(value.label)) return null;
  if (typeof value.id !== 'string' || !/^[a-z0-9][a-z0-9.:-]{0,127}$/.test(value.id)) {
    return null;
  }
  return { id: value.id, label: value.label.trim() };
}

function parseSource(value: unknown): ThemeLibrarySourceV1 | null {
  if (!isRecord(value)) return null;
  if (value.kind === 'local') return { kind: 'local' };
  if (
    value.kind !== 'open-vsx'
    || typeof value.extensionId !== 'string'
    || !/^[a-z0-9][a-z0-9._-]{0,127}$/i.test(value.extensionId)
    || typeof value.version !== 'string'
    || value.version.length === 0
    || value.version.length > 64
    || typeof value.license !== 'string'
    || value.license.length === 0
    || value.license.length > 64
  ) {
    return null;
  }
  let sourceUrl: string | undefined;
  if (value.sourceUrl !== undefined) {
    if (typeof value.sourceUrl !== 'string' || value.sourceUrl.length > 2048) return null;
    try {
      const parsed = new URL(value.sourceUrl);
      if (parsed.protocol !== 'https:' || parsed.username || parsed.password) return null;
      sourceUrl = parsed.toString();
    } catch {
      return null;
    }
  }
  return {
    kind: 'open-vsx',
    extensionId: value.extensionId,
    version: value.version,
    license: value.license,
    ...(sourceUrl ? { sourceUrl } : {}),
  };
}

export function parseThemeLibraryEntry(value: unknown): ThemeLibraryEntryV1 | null {
  if (!isRecord(value) || value.version !== THEME_LIBRARY_VERSION) return null;
  if (!isThemeId(value.id) || !isLabel(value.label)) return null;
  const modes = parseModes(value.modes);
  const source = parseSource(value.source);
  const collection = parseCollection(value.collection);
  if (!modes || !source || collection === null || !isRecord(value.theme) || value.theme.version !== 1) {
    return null;
  }
  if (value.theme.presetId !== 'sonic' && value.theme.presetId !== 'custom') return null;
  return {
    version: 1,
    id: value.id,
    label: value.label.trim(),
    modes,
    theme: sanitizeTheme(value.theme),
    source,
    ...(collection ? { collection } : {}),
  };
}

export function emptyThemeLibrary(): ThemeLibraryDocumentV1 {
  return { version: 1, revision: 0, themes: [] };
}

export function loadThemeLibrary(
  storage: ThemeLibraryStorageLike = localStorage,
): ThemeLibraryLoadResult {
  let raw: string | null;
  try {
    raw = storage.getItem(THEME_LIBRARY_STORAGE_KEY);
  } catch {
    return {
      status: 'unavailable',
      document: emptyThemeLibrary(),
      error: 'Theme library storage is unavailable.',
    };
  }
  if (raw === null) return { status: 'ready', document: emptyThemeLibrary(), needsRepair: false };
  if (byteLength(raw) > MAX_THEME_LIBRARY_BYTES) {
    return {
      status: 'unavailable',
      document: emptyThemeLibrary(),
      error: 'Stored theme library exceeds the 1 MiB limit.',
    };
  }
  let value: unknown;
  try {
    value = JSON.parse(raw);
  } catch {
    return {
      status: 'unavailable',
      document: emptyThemeLibrary(),
      error: 'Stored theme library is not valid JSON.',
    };
  }
  if (
    !isRecord(value)
    || value.version !== THEME_LIBRARY_VERSION
    || !Array.isArray(value.themes)
    || value.themes.length > MAX_THEME_LIBRARY_ENTRIES
  ) {
    return {
      status: 'unavailable',
      document: emptyThemeLibrary(),
      error: 'Stored theme library has an unsupported or invalid shape.',
    };
  }

  const themes: ThemeLibraryEntryV1[] = [];
  const ids = new Set<string>();
  let needsRepair = false;
  for (const candidate of value.themes) {
    const theme = parseThemeLibraryEntry(candidate);
    if (!theme || ids.has(theme.id)) {
      needsRepair = true;
      continue;
    }
    ids.add(theme.id);
    themes.push(theme);
  }
  const document: ThemeLibraryDocumentV1 = {
    version: 1,
    revision: sanitizeRevision(value.revision),
    themes,
  };
  if (JSON.stringify(document) !== JSON.stringify(value)) needsRepair = true;
  return { status: 'ready', document, needsRepair };
}

function nextLibraryRevision(revision: number): number {
  return revision >= MAX_APPEARANCE_REVISION - 2 ? 1 : revision + 1;
}

export function writeThemeLibrary(
  document: ThemeLibraryDocumentV1,
  storage: ThemeLibraryStorageLike = localStorage,
): void {
  if (document.themes.length > MAX_THEME_LIBRARY_ENTRIES) {
    throw new Error(`A theme library may contain at most ${MAX_THEME_LIBRARY_ENTRIES} themes.`);
  }
  const canonicalThemes = document.themes.map((theme) => {
    const parsed = parseThemeLibraryEntry(theme);
    if (!parsed) throw new Error(`The theme library entry "${theme.label}" is invalid.`);
    return parsed;
  });
  if (new Set(canonicalThemes.map((theme) => theme.id)).size !== canonicalThemes.length) {
    throw new Error('Theme library IDs must be unique.');
  }
  const canonical: ThemeLibraryDocumentV1 = {
    version: 1,
    revision: sanitizeRevision(document.revision),
    themes: canonicalThemes,
  };
  const serialized = JSON.stringify(canonical);
  if (byteLength(serialized) > MAX_THEME_LIBRARY_BYTES) {
    throw new Error('Theme library exceeds the 1 MiB limit.');
  }
  storage.setItem(THEME_LIBRARY_STORAGE_KEY, serialized);
  if (typeof localStorage !== 'undefined' && storage === localStorage) {
    mirrorDurableBlob(THEME_LIBRARY_STORE, serialized);
  }
}

function requireCurrentLibrary(
  expectedRevision: number,
  storage: ThemeLibraryStorageLike,
): ThemeLibraryDocumentV1 {
  const loaded = loadThemeLibrary(storage);
  if (loaded.status === 'unavailable') throw new Error(loaded.error);
  if (loaded.document.revision !== expectedRevision) {
    throw new Error('The theme library changed while this operation was running. Try again.');
  }
  return loaded.document;
}

function publishMutation(
  current: ThemeLibraryDocumentV1,
  themes: readonly ThemeLibraryEntryV1[],
  storage: ThemeLibraryStorageLike,
): ThemeLibraryDocumentV1 {
  const next: ThemeLibraryDocumentV1 = {
    version: 1,
    revision: nextLibraryRevision(current.revision),
    themes: [...themes],
  };
  writeThemeLibrary(next, storage);
  return next;
}

export function installThemeLibraryEntries(
  expectedRevision: number,
  entries: readonly ThemeLibraryEntryV1[],
  storage: ThemeLibraryStorageLike = localStorage,
): ThemeLibraryDocumentV1 {
  if (entries.length === 0) throw new Error('Choose at least one theme to install.');
  const current = requireCurrentLibrary(expectedRevision, storage);
  const occupied = new Set(current.themes.map((theme) => theme.id));
  for (const entry of entries) {
    if (!parseThemeLibraryEntry(entry)) throw new Error(`The theme "${entry.label}" is invalid.`);
    if (occupied.has(entry.id)) throw new Error(`A theme named "${entry.label}" is already installed.`);
    occupied.add(entry.id);
  }
  return publishMutation(current, [...current.themes, ...entries], storage);
}

export function replaceThemeLibraryCollection(
  expectedRevision: number,
  collectionId: string,
  entries: readonly ThemeLibraryEntryV1[],
  expectedCollection: readonly ThemeLibraryEntryV1[],
  storage: ThemeLibraryStorageLike = localStorage,
): ThemeLibraryDocumentV1 {
  if (entries.length === 0 || entries.some((entry) => entry.collection?.id !== collectionId)) {
    throw new Error('The replacement theme collection is invalid.');
  }
  const current = requireCurrentLibrary(expectedRevision, storage);
  const currentCollection = current.themes.filter((theme) => theme.collection?.id === collectionId);
  if (JSON.stringify(currentCollection) !== JSON.stringify(expectedCollection)) {
    throw new Error('The installed theme collection changed while its update was downloading. Try again.');
  }
  const occupied = new Set(
    current.themes
      .filter((theme) => theme.collection?.id !== collectionId)
      .map((theme) => theme.id),
  );
  for (const entry of entries) {
    if (!parseThemeLibraryEntry(entry) || occupied.has(entry.id)) {
      throw new Error(`The replacement theme "${entry.label}" conflicts with an installed theme.`);
    }
    occupied.add(entry.id);
  }
  const firstCollectionIndex = current.themes.findIndex((theme) => theme.collection?.id === collectionId);
  const retained = current.themes.filter((theme) => theme.collection?.id !== collectionId);
  const insertionIndex = firstCollectionIndex < 0 ? retained.length : firstCollectionIndex;
  const themes = [...retained];
  themes.splice(insertionIndex, 0, ...entries);
  return publishMutation(current, themes, storage);
}

export function removeThemeLibraryEntries(
  expectedRevision: number,
  themeIds: readonly string[],
  storage: ThemeLibraryStorageLike = localStorage,
): ThemeLibraryDocumentV1 {
  const ids = new Set(themeIds);
  if (ids.size === 0) throw new Error('Choose at least one theme to remove.');
  const current = requireCurrentLibrary(expectedRevision, storage);
  const themes = current.themes.filter((theme) => !ids.has(theme.id));
  if (themes.length === current.themes.length) throw new Error('Those themes are not installed.');
  return publishMutation(current, themes, storage);
}

export function themeIdFromLabel(label: string): string {
  return label
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 64) || 'custom-theme';
}

export function availableThemeId(label: string, occupiedIds: ReadonlySet<string>): string {
  const base = themeIdFromLabel(label);
  if (!occupiedIds.has(base) && base !== 'sonic' && base !== 'custom') return base;
  for (let suffix = 2; suffix < 1000; suffix += 1) {
    const tail = `-${suffix}`;
    const candidate = `${base.slice(0, 64 - tail.length)}${tail}`;
    if (!occupiedIds.has(candidate)) return candidate;
  }
  throw new Error(`Too many themes are named "${label}".`);
}

export function makeLocalThemeEntry(
  id: string,
  label: string,
  theme: ThemeConfigV1,
  modes: readonly ResolvedAppearance[] = ['light', 'dark'],
): ThemeLibraryEntryV1 {
  const entry: ThemeLibraryEntryV1 = {
    version: 1,
    id,
    label,
    modes: [...modes],
    theme: sanitizeTheme(theme),
    source: { kind: 'local' },
  };
  const parsed = parseThemeLibraryEntry(entry);
  if (!parsed) throw new Error('The local theme is invalid.');
  return parsed;
}

export function themeTokensForMode(
  entry: ThemeLibraryEntryV1,
  appearance: ResolvedAppearance,
) {
  if (!entry.modes.includes(appearance)) return null;
  return resolveTheme(entry.theme, appearance).tokens;
}

export function appearanceSelection(document: AppearanceDocumentV1): AppearanceSelectionV1 {
  if (document.selection) return document.selection;
  const owner = document.theme.presetId === 'sonic' ? 'sonic' : 'custom';
  return { light: owner, dark: owner };
}

function tokensForOwner(
  owner: string,
  appearance: ResolvedAppearance,
  current: AppearanceDocumentV1,
  library: ThemeLibraryDocumentV1,
) {
  if (owner === 'sonic') return resolveTheme(DEFAULT_THEME, appearance).tokens;
  if (owner === 'custom') return resolveTheme(current.theme, appearance).tokens;
  const entry = library.themes.find((theme) => theme.id === owner);
  const tokens = entry ? themeTokensForMode(entry, appearance) : null;
  if (!tokens) throw new Error(`The selected ${appearance} theme is no longer available.`);
  return tokens;
}

export function composeThemeSelection(
  current: AppearanceDocumentV1,
  library: ThemeLibraryDocumentV1,
  selection: AppearanceSelectionV1,
): ThemeConfigV1 {
  if (selection.light === 'sonic' && selection.dark === 'sonic') return { ...DEFAULT_THEME };
  if (selection.light === 'custom' && selection.dark === 'custom') return sanitizeTheme(current.theme);
  if (selection.light === selection.dark) {
    const entry = library.themes.find((theme) => theme.id === selection.light);
    if (entry && entry.modes.includes('light') && entry.modes.includes('dark')) {
      return sanitizeTheme(entry.theme);
    }
  }
  return sanitizeTheme({
    version: 1,
    presetId: 'custom',
    light: tokensForOwner(selection.light, 'light', current, library),
    dark: tokensForOwner(selection.dark, 'dark', current, library),
  });
}

export function previewThemeLibrarySelection(
  current: AppearanceDocumentV1,
  library: ThemeLibraryDocumentV1,
  themeId: string,
  appearance?: ResolvedAppearance,
): ThemeImportPreview {
  const entry = library.themes.find((theme) => theme.id === themeId);
  const modes: ResolvedAppearance[] = themeId === 'sonic' ? ['light', 'dark'] : entry?.modes ?? [];
  if (!entry && themeId !== 'sonic') throw new Error('That theme is not installed.');
  const currentSelection = appearanceSelection(current);
  let selection: AppearanceSelectionV1;
  if (appearance) {
    if (!modes.includes(appearance)) {
      throw new Error(`That theme has no ${appearance} variant.`);
    }
    selection = { ...currentSelection, [appearance]: themeId };
  } else if (modes.length === 1) {
    const onlyMode = modes[0]!;
    selection = { ...currentSelection, [onlyMode]: themeId };
  } else {
    selection = { light: themeId, dark: themeId };
  }
  const theme = composeThemeSelection(current, library, selection);
  const light = resolveTheme(theme, 'light');
  const dark = resolveTheme(theme, 'dark');
  return {
    mode: current.mode,
    theme,
    light: light.tokens,
    dark: dark.tokens,
    adjustments: [...light.adjustments, ...dark.adjustments],
    selection,
  };
}
