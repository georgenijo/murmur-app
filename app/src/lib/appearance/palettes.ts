import type { MurmurTokens, ThemeConfigV1 } from './types';

export const SONIC_LIGHT: MurmurTokens = {
  background: '#f7fafc',
  surface: '#f7fafc',
  'surface-container-low': '#eff4f8',
  'surface-container': '#e9eff3',
  'surface-container-high': '#e2e9ee',
  'surface-container-lowest': '#ffffff',
  'surface-container-highest': '#dbe4e9',
  primary: '#036785',
  'primary-dim': '#005a75',
  'on-primary': '#f3faff',
  'on-surface': '#2b3438',
  'on-surface-variant': '#586065',
  'outline-variant': '#abb3b9',
  error: '#a83836',
  success: '#146333',
  warning: '#654500',
};

export const SONIC_DARK: MurmurTokens = {
  background: '#0b0f11',
  surface: '#0b0f11',
  'surface-container-low': '#151a1e',
  'surface-container': '#1e2529',
  'surface-container-high': '#283035',
  'surface-container-lowest': '#0f1315',
  'surface-container-highest': '#323b41',
  primary: '#92dbfe',
  'primary-dim': '#84cdef',
  'on-primary': '#00394b',
  'on-surface': '#dbe4e9',
  'on-surface-variant': '#abb3b9',
  'outline-variant': '#586065',
  error: '#fa746f',
  success: '#66d99a',
  warning: '#f4bd65',
};

export const DEFAULT_THEME: ThemeConfigV1 = {
  version: 1,
  presetId: 'sonic',
};

// OpenCode's light/dark neutrals, accent and status colors from Rune's
// `rune-theme-opencode` (anomalyco/rune @ 100b7d93, MIT,
// themes/opencode/assets/theme.json), mapped onto Murmur roles. Translucent
// borders are composited over `v2-background-bg-base`, matching Rune's sRGB
// blend. These are stored inputs: the mutable-palette resolver still repairs
// them against the semantic contrast matrix (docs/features/appearance.md).
export const OPENCODE_LIGHT: MurmurTokens = {
  background: '#f8f8f8',
  surface: '#f8f8f8',
  'surface-container-low': '#fafafa',
  'surface-container': '#f2f2f2',
  'surface-container-high': '#eeeeee',
  'surface-container-lowest': '#ffffff',
  'surface-container-highest': '#dbdbdb',
  primary: '#3b5cf6',
  'primary-dim': '#3250df',
  'on-primary': '#ffffff',
  'on-surface': '#161616',
  'on-surface-variant': '#5c5c5c',
  'outline-variant': '#cccccc',
  error: '#b82d35',
  success: '#198b43',
  warning: '#cb9f34',
};

export const OPENCODE_DARK: MurmurTokens = {
  background: '#121212',
  surface: '#121212',
  'surface-container-low': '#242424',
  'surface-container': '#2e2e2e',
  'surface-container-high': '#3a3a3a',
  'surface-container-lowest': '#161616',
  'surface-container-highest': '#5c5c5c',
  primary: '#a2bcff',
  'primary-dim': '#7698fd',
  'on-primary': '#161616',
  'on-surface': '#fafafa',
  'on-surface-variant': '#aeaeae',
  'outline-variant': '#454545',
  error: '#f17471',
  success: '#6bd586',
  warning: '#f2cf76',
};

export const OPENCODE_THEME: ThemeConfigV1 = {
  version: 1,
  presetId: 'custom',
  light: OPENCODE_LIGHT,
  dark: OPENCODE_DARK,
};
