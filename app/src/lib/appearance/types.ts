export type AppearanceMode = 'system' | 'light' | 'dark';
export type ResolvedAppearance = 'light' | 'dark';
export type AppearanceChangeReason = 'user' | 'repair' | 'reset' | 'import' | 'library';

export const MURMUR_TOKEN_NAMES = [
  'background',
  'surface',
  'surface-container-low',
  'surface-container',
  'surface-container-high',
  'surface-container-lowest',
  'surface-container-highest',
  'primary',
  'primary-dim',
  'on-primary',
  'on-surface',
  'on-surface-variant',
  'outline-variant',
  'error',
  'success',
  'warning',
] as const;

export type MurmurTokenName = (typeof MURMUR_TOKEN_NAMES)[number];
export type HexColor = `#${string}`;
export type MurmurTokens = Record<MurmurTokenName, HexColor>;

export const BUILTIN_PRESET_IDS = ['sonic'] as const;
export type BuiltinPresetId = (typeof BUILTIN_PRESET_IDS)[number];

export const APPEARANCE_VERSION = 1 as const;
export const APPEARANCE_CACHE_VERSION = 1 as const;
// Reserved as a saturation sentinel; canonical stored revisions are strictly lower.
export const MAX_APPEARANCE_REVISION = 2_147_483_647;
export const THEME_CONTRAST_MIN = -100;
export const THEME_CONTRAST_MAX = 100;
export const THEME_CONTRAST_DEFAULT = 0;

export interface ThemeConfigV1 {
  version: 1;
  presetId: BuiltinPresetId | 'custom';
  accent?: string;
  background?: string;
  foreground?: string;
  contrast?: number;
  light?: Partial<MurmurTokens>;
  dark?: Partial<MurmurTokens>;
}

export interface AppearanceSelectionV1 {
  light: string;
  dark: string;
}

export type ThemeLibrarySourceV1 =
  | { kind: 'local' }
  | {
      kind: 'open-vsx';
      extensionId: string;
      version: string;
      license: string;
      sourceUrl?: string;
    };

export interface ThemeLibraryCollectionV1 {
  id: string;
  label: string;
}

export interface ThemeLibraryEntryV1 {
  version: 1;
  id: string;
  label: string;
  modes: ResolvedAppearance[];
  theme: ThemeConfigV1;
  source: ThemeLibrarySourceV1;
  collection?: ThemeLibraryCollectionV1;
}

export interface ThemeLibraryDocumentV1 {
  version: 1;
  revision: number;
  themes: ThemeLibraryEntryV1[];
}

export interface ResolvedThemeCacheV1 {
  version: 1;
  light: MurmurTokens;
  dark: MurmurTokens;
}

export interface AppearanceDocumentV1 {
  version: 1;
  revision: number;
  mode: AppearanceMode;
  theme: ThemeConfigV1;
  cache: ResolvedThemeCacheV1;
  /** Source ownership for each compiled half. Missing on pre-library documents. */
  selection?: AppearanceSelectionV1;
}

export interface AppearanceChangedEvent {
  revision: number;
  reason: AppearanceChangeReason;
}

export type ThemeAdjustmentReason = 'contrast' | 'gamut';

export interface ThemeAdjustment {
  appearance: ResolvedAppearance;
  token: MurmurTokenName;
  reason: ThemeAdjustmentReason;
  from: HexColor;
  to: HexColor;
}

export interface ResolvedTheme {
  appearance: ResolvedAppearance;
  colorScheme: ResolvedAppearance;
  tokens: MurmurTokens;
  adjustments: ThemeAdjustment[];
}

export interface ThemeImportPreview {
  mode: AppearanceMode;
  theme: ThemeConfigV1;
  light: MurmurTokens;
  dark: MurmurTokens;
  adjustments: ThemeAdjustment[];
  selection?: AppearanceSelectionV1;
  label?: string;
  modes?: ResolvedAppearance[];
}

export interface ThemeLibraryController {
  document: ThemeLibraryDocumentV1;
  error: string | null;
  saveCurrent: (label: string) => Promise<ThemeLibraryEntryV1>;
  savePreview: (label: string, preview: ThemeImportPreview) => Promise<ThemeLibraryEntryV1>;
  install: (entries: readonly ThemeLibraryEntryV1[]) => Promise<void>;
  replaceCollection: (
    collectionId: string,
    entries: readonly ThemeLibraryEntryV1[],
    expectedCollection: readonly ThemeLibraryEntryV1[],
  ) => Promise<void>;
  remove: (themeIds: readonly string[]) => Promise<void>;
  previewSelection: (
    themeId: string,
    appearance?: ResolvedAppearance,
  ) => ThemeImportPreview;
  exportEntryToPath: (entry: ThemeLibraryEntryV1, path: string) => Promise<void>;
  clearError: () => void;
}

export interface AppearanceController {
  document: AppearanceDocumentV1;
  resolvedAppearance: ResolvedAppearance;
  adjustments: ThemeAdjustment[];
  busy: boolean;
  error: string | null;
  setMode: (mode: AppearanceMode) => Promise<void>;
  updateTheme: (updates: Partial<ThemeConfigV1>) => Promise<void>;
  reset: () => Promise<void>;
  previewImport: (text: string) => ThemeImportPreview;
  importFromPath: (path: string) => Promise<ThemeImportPreview>;
  commitImport: (preview: ThemeImportPreview) => Promise<void>;
  exportText: () => string;
  exportToPath: (path: string) => Promise<void>;
  library: ThemeLibraryController;
  clearError: () => void;
}
