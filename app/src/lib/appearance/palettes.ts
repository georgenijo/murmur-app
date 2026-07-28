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
