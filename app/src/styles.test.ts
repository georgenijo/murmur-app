import { describe, expect, it } from 'vitest';
// @ts-expect-error Node types are intentionally absent from the browser app.
import { readFileSync } from 'node:fs';

const css = readFileSync('./src/styles.css', 'utf8');

const lightTheme = {
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
} as const;

const darkTheme = {
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
} as const;

function luminance(hex: string): number {
  const channels = hex.slice(1).match(/.{2}/g)!.map((channel) => parseInt(channel, 16) / 255);
  const [red, green, blue] = channels.map((channel) =>
    channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4,
  );
  return 0.2126 * red + 0.7152 * green + 0.0722 * blue;
}

function contrast(foreground: string, background: string): number {
  const foregroundLuminance = luminance(foreground);
  const backgroundLuminance = luminance(background);
  return (
    (Math.max(foregroundLuminance, backgroundLuminance) + 0.05) /
    (Math.min(foregroundLuminance, backgroundLuminance) + 0.05)
  );
}

describe('Sonic Canvas semantic color tokens', () => {
  it('defines the complete light and dark palettes in the Tailwind v4 stylesheet', () => {
    expect(css).toContain('@theme inline');
    expect(css).toContain('@media (prefers-color-scheme: dark)');

    for (const [token, value] of Object.entries(lightTheme)) {
      expect(css).toContain(`--murmur-${token}: ${value};`);
      expect(css).toContain(`--color-${token}: var(--murmur-${token});`);
    }

    for (const [token, value] of Object.entries(darkTheme)) {
      expect(css).toContain(`--murmur-${token}: ${value};`);
    }
  });

  it.each([
    ['light', lightTheme],
    ['dark', darkTheme],
  ] as const)('%s text pairs meet WCAG AA contrast', (_mode, theme) => {
    expect(contrast(theme['on-surface'], theme.background)).toBeGreaterThanOrEqual(4.5);
    expect(contrast(theme['on-surface-variant'], theme.background)).toBeGreaterThanOrEqual(4.5);
    expect(contrast(theme['on-primary'], theme.primary)).toBeGreaterThanOrEqual(4.5);
    expect(contrast(theme.error, theme.background)).toBeGreaterThanOrEqual(4.5);
  });
});
