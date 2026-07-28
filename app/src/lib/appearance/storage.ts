import { DEFAULT_THEME } from './palettes';
import { resolveTheme } from './resolve';
import { sanitizeMode, sanitizeStoredAppearance, sanitizeTheme } from './sanitize';
import {
  APPEARANCE_VERSION,
  MAX_APPEARANCE_REVISION,
  type AppearanceDocumentV1,
  type AppearanceMode,
  type ResolvedThemeCacheV1,
  type ThemeConfigV1,
  type ThemeImportPreview,
} from './types';

export const APPEARANCE_STORAGE_KEY = 'murmur-appearance';
export const MAX_APPEARANCE_BYTES = 64 * 1024;
export const APPEARANCE_REVISION_ROLLOVER_AT = MAX_APPEARANCE_REVISION - 2;

export interface StorageLike {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

export interface AppearanceLoadResult {
  document: AppearanceDocumentV1;
  needsRepair: boolean;
  error: string | null;
}

function byteLength(value: string): number {
  if (typeof TextEncoder !== 'undefined') return new TextEncoder().encode(value).byteLength;
  return value.length;
}

export function resolvedCache(theme: ThemeConfigV1): ResolvedThemeCacheV1 {
  return {
    version: 1,
    light: resolveTheme(theme, 'light').tokens,
    dark: resolveTheme(theme, 'dark').tokens,
  };
}

export function createAppearanceDocument(
  mode: AppearanceMode = 'system',
  theme: ThemeConfigV1 = DEFAULT_THEME,
  revision = 0,
): AppearanceDocumentV1 {
  const sanitizedTheme = sanitizeTheme(theme);
  return {
    version: 1,
    revision: Number.isSafeInteger(revision)
      && revision >= 0
      && revision < MAX_APPEARANCE_REVISION - 1
      ? revision
      : 0,
    mode: sanitizeMode(mode),
    theme: sanitizedTheme,
    cache: resolvedCache(sanitizedTheme),
  };
}

export function nextAppearanceRevision(current: number): number {
  if (!Number.isInteger(current) || current < 0 || current > APPEARANCE_REVISION_ROLLOVER_AT) {
    throw new Error('Appearance revision is exhausted or invalid.');
  }
  if (current === APPEARANCE_REVISION_ROLLOVER_AT) return 1;
  return current + 1;
}

export function isNewerAppearanceRevision(current: number, candidate: number): boolean {
  if (candidate > current) return true;
  return current === APPEARANCE_REVISION_ROLLOVER_AT
    && candidate > 0
    && candidate < APPEARANCE_REVISION_ROLLOVER_AT;
}

function sameCache(left: ResolvedThemeCacheV1 | null, right: ResolvedThemeCacheV1): boolean {
  return left !== null && JSON.stringify(left) === JSON.stringify(right);
}

export function loadAppearanceDocument(
  storage: StorageLike = localStorage,
): AppearanceLoadResult {
  let raw: string | null;
  try {
    raw = storage.getItem(APPEARANCE_STORAGE_KEY);
  } catch (error) {
    return {
      document: createAppearanceDocument(),
      needsRepair: false,
      error: `Appearance storage is unavailable: ${String(error)}`,
    };
  }
  if (raw === null) {
    return { document: createAppearanceDocument(), needsRepair: false, error: null };
  }
  if (byteLength(raw) > MAX_APPEARANCE_BYTES) {
    return {
      document: createAppearanceDocument(),
      needsRepair: true,
      error: 'Stored appearance exceeds the 64 KiB limit.',
    };
  }

  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return {
      document: createAppearanceDocument(),
      needsRepair: true,
      error: 'Stored appearance is not valid JSON.',
    };
  }

  const sanitized = sanitizeStoredAppearance(parsed);
  if (!sanitized.validDocument) {
    return {
      document: createAppearanceDocument(),
      needsRepair: true,
      error: 'Stored appearance has an unsupported version.',
    };
  }
  const document = createAppearanceDocument(
    sanitized.mode,
    sanitized.theme,
    sanitized.revision,
  );
  const canonical = JSON.stringify(document);
  return {
    document,
    // The byte comparison is intentional: semantically equivalent documents
    // with reordered or unknown keys get one repair write into canonical form.
    needsRepair: !sameCache(sanitized.cache, document.cache) || canonical !== JSON.stringify(parsed),
    error: null,
  };
}

export function writeAppearanceDocument(
  document: AppearanceDocumentV1,
  storage: StorageLike = localStorage,
): void {
  const canonical = createAppearanceDocument(document.mode, document.theme, document.revision);
  const serialized = JSON.stringify(canonical);
  if (byteLength(serialized) > MAX_APPEARANCE_BYTES) {
    throw new Error('Appearance document exceeds the 64 KiB limit.');
  }
  storage.setItem(APPEARANCE_STORAGE_KEY, serialized);
}

interface ThemeFileV1 {
  version: 1;
  mode: AppearanceMode;
  theme: ThemeConfigV1;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

export function exportAppearanceText(document: AppearanceDocumentV1): string {
  const file: ThemeFileV1 = {
    version: 1,
    mode: sanitizeMode(document.mode),
    theme: sanitizeTheme(document.theme),
  };
  return `${JSON.stringify(file, null, 2)}\n`;
}

export function previewAppearanceImport(text: string): ThemeImportPreview {
  if (byteLength(text) > MAX_APPEARANCE_BYTES) {
    throw new Error('Theme file exceeds the 64 KiB limit.');
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    throw new Error('Theme file is not valid JSON.');
  }
  if (!isRecord(parsed) || parsed.version !== APPEARANCE_VERSION) {
    throw new Error('Theme file has an unsupported version.');
  }
  if (parsed.mode !== 'system' && parsed.mode !== 'light' && parsed.mode !== 'dark') {
    throw new Error('Theme file has an invalid appearance mode.');
  }
  if (!isRecord(parsed.theme) || parsed.theme.version !== APPEARANCE_VERSION) {
    throw new Error('Theme file has an unsupported theme version.');
  }
  const theme = sanitizeTheme(parsed.theme);
  const light = resolveTheme(theme, 'light');
  const dark = resolveTheme(theme, 'dark');
  return {
    mode: parsed.mode,
    theme,
    light: light.tokens,
    dark: dark.tokens,
    adjustments: [...light.adjustments, ...dark.adjustments],
  };
}

export async function readAppearancePreview(
  path: string,
  read: (path: string) => Promise<string>,
): Promise<ThemeImportPreview> {
  return previewAppearanceImport(await read(path));
}

export async function writeAppearanceExport(
  path: string,
  document: AppearanceDocumentV1,
  write: (path: string, contents: string) => Promise<void>,
): Promise<void> {
  await write(path, exportAppearanceText(document));
}
