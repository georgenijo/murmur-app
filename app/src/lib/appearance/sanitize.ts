import { isHexColor, normalizeHex } from './color';
import { DEFAULT_THEME } from './palettes';
import {
  APPEARANCE_CACHE_VERSION,
  APPEARANCE_VERSION,
  MAX_APPEARANCE_REVISION,
  MURMUR_TOKEN_NAMES,
  THEME_CONTRAST_DEFAULT,
  THEME_CONTRAST_MAX,
  THEME_CONTRAST_MIN,
  type AppearanceMode,
  type AppearanceSelectionV1,
  type MurmurTokens,
  type ResolvedThemeCacheV1,
  type ThemeConfigV1,
} from './types';

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

export function sanitizeMode(value: unknown): AppearanceMode {
  return value === 'light' || value === 'dark' || value === 'system' ? value : 'system';
}

export function sanitizeAppearanceSelection(value: unknown): AppearanceSelectionV1 | undefined {
  if (!isRecord(value)) return undefined;
  const isOwner = (candidate: unknown): candidate is string =>
    typeof candidate === 'string'
    && /^[a-z0-9](?:[a-z0-9-]{0,63})$/.test(candidate);
  return isOwner(value.light) && isOwner(value.dark)
    ? { light: value.light, dark: value.dark }
    : undefined;
}

function sanitizeOverrides(value: unknown): Partial<MurmurTokens> | undefined {
  if (!isRecord(value)) return undefined;
  const output: Partial<MurmurTokens> = {};
  for (const token of MURMUR_TOKEN_NAMES) {
    const color = value[token];
    if (isHexColor(color)) output[token] = normalizeHex(color);
  }
  return Object.keys(output).length > 0 ? output : undefined;
}

export function sanitizeTheme(value: unknown): ThemeConfigV1 {
  if (!isRecord(value) || value.version !== APPEARANCE_VERSION) {
    return { ...DEFAULT_THEME };
  }
  const presetId = value.presetId === 'custom' || value.presetId === 'sonic'
    ? value.presetId
    : 'sonic';
  const output: ThemeConfigV1 = { version: 1, presetId };
  for (const field of ['accent', 'background', 'foreground'] as const) {
    if (isHexColor(value[field])) output[field] = normalizeHex(value[field]);
  }
  if (typeof value.contrast === 'number' && Number.isFinite(value.contrast)) {
    output.contrast = Math.round(
      Math.min(THEME_CONTRAST_MAX, Math.max(THEME_CONTRAST_MIN, value.contrast)),
    );
    if (output.contrast === THEME_CONTRAST_DEFAULT) delete output.contrast;
  }
  const light = sanitizeOverrides(value.light);
  const dark = sanitizeOverrides(value.dark);
  if (light) output.light = light;
  if (dark) output.dark = dark;
  return output;
}

export function sanitizeRevision(value: unknown): number {
  return typeof value === 'number'
    && Number.isSafeInteger(value)
    && value >= 0
    && value < MAX_APPEARANCE_REVISION - 1
    ? value
    : 0;
}

export function validateCache(value: unknown): ResolvedThemeCacheV1 | null {
  if (!isRecord(value) || value.version !== APPEARANCE_CACHE_VERSION) return null;
  const readTable = (candidate: unknown): MurmurTokens | null => {
    if (!isRecord(candidate)) return null;
    const keys = Object.keys(candidate);
    if (keys.length !== MURMUR_TOKEN_NAMES.length) return null;
    const output = {} as MurmurTokens;
    for (const token of MURMUR_TOKEN_NAMES) {
      const color = candidate[token];
      if (!isHexColor(color)) return null;
      output[token] = normalizeHex(color);
    }
    return output;
  };
  const light = readTable(value.light);
  const dark = readTable(value.dark);
  return light && dark ? { version: 1, light, dark } : null;
}

export interface SanitizedStoredAppearance {
  validDocument: boolean;
  revision: number;
  mode: AppearanceMode;
  theme: ThemeConfigV1;
  cache: ResolvedThemeCacheV1 | null;
  selection?: AppearanceSelectionV1;
}

export function sanitizeStoredAppearance(value: unknown): SanitizedStoredAppearance {
  if (!isRecord(value) || value.version !== APPEARANCE_VERSION) {
    return {
      validDocument: false,
      revision: 0,
      mode: 'system',
      theme: { ...DEFAULT_THEME },
      cache: null,
    };
  }
  return {
    validDocument: true,
    revision: sanitizeRevision(value.revision),
    mode: sanitizeMode(value.mode),
    theme: sanitizeTheme(value.theme),
    cache: validateCache(value.cache),
    selection: sanitizeAppearanceSelection(value.selection),
  };
}
