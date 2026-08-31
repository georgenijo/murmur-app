import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { compile } from 'tailwindcss';

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
  success: '#146333',
  warning: '#654500',
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
  success: '#66d99a',
  warning: '#f4bd65',
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
  it('compiles dark utilities against the concrete appearance selector', async () => {
    const compiler = await compile(css, {
      base: process.cwd(),
      loadStylesheet: async (id, base) => {
        const path = id === 'tailwindcss'
          ? resolve(process.cwd(), 'node_modules/tailwindcss/index.css')
          : resolve(base, id);
        return { path, base: dirname(path), content: readFileSync(path, 'utf8') };
      },
    });
    const output = compiler.build(['dark:bg-warning/10']);
    expect(output).toMatch(
      /\.dark\\:bg-warning\\\/10\s*\{\s*&:where\(\[data-appearance="dark"\], \[data-appearance="dark"\] \*\)/,
    );
  });

  it('defines the complete light and dark palettes in the Tailwind v4 stylesheet', () => {
    expect(css).toContain('@theme inline');
    expect(css).toContain('@media (prefers-color-scheme: dark)');
    expect(css).toContain('@custom-variant dark (&:where([data-appearance="dark"], [data-appearance="dark"] *))');

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

describe('Murmur layout contracts', () => {
  it('locks the compact chrome and history geometry to shared tokens', () => {
    expect(css).toContain('--ui-font-caption: 0.75rem;');
    expect(css).toContain('--ui-font-label: 0.8125rem;');
    expect(css).toContain('--ui-font-body: 0.875rem;');
    expect(css).toContain('--ui-window-header-height: 2.625rem;');
    expect(css).toContain('--ui-window-content-offset-y: -0.125rem;');
    expect(css).toContain('--ui-record-width: 4.5rem;');
    expect(css).toContain('--ui-status-min-width: 4.5rem;');
    expect(css).toContain('--ui-history-gap: 0.3125rem;');
    expect(css).toContain('--ui-history-card-y: 0.5rem;');
  });

  it('reserves home-history text space for copy feedback and draws a complete inset outline', () => {
    expect(css).toMatch(
      /\.transcript-copy-feedback\s*\{[^}]*position:\s*absolute;[^}]*right:\s*var\(--ui-space-4\);[^}]*bottom:\s*var\(--ui-space-3\);/s,
    );
    expect(css).toMatch(
      /\.home-history \.transcript-text\s*\{[^}]*grid-column:\s*2;[^}]*padding-right:\s*3\.75rem;/s,
    );
    expect(css).toMatch(
      /\.home-history \.transcript-card\[data-copied="true"\]:not\(\[data-day-end="true"\]\)\s*\{[^}]*box-shadow:\s*inset 0 -1px 0 var\(--murmur-success\);/s,
    );
    expect(css).toMatch(
      /\.home-history \.transcript-copy-feedback\s*\{[^}]*right:\s*var\(--ui-space-5\);[^}]*bottom:\s*auto;[^}]*top:\s*var\(--ui-space-3\);/s,
    );
  });

  it('does not let hover override a selected filter chip', () => {
    expect(css).toContain(
      '.ui-filter-chip:hover:not([aria-pressed="true"]):not([aria-current="page"])',
    );
  });

  it('keeps persistent navigation surfaces isolated on compositor layers', () => {
    expect(css).toMatch(
      /\.ui-persistent-surface\s*\{[^}]*contain:\s*layout style paint;[^}]*will-change:\s*transform, opacity;[^}]*transform:\s*translateZ\(0\);/s,
    );
  });

  it('lets WebKit skip offscreen diagnostics rows', () => {
    expect(css).toMatch(
      /\.diagnostic-event-row\s*\{[^}]*content-visibility:\s*auto;[^}]*contain-intrinsic-size:\s*auto 28px;/s,
    );
  });
});
