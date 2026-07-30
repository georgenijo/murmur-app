import { beforeEach, describe, expect, it } from 'vitest';
import {
  MURMUR_TOKEN_NAMES,
  NON_TEXT_CONTRAST_MATRIX,
  SEMANTIC_CONTRAST_MATRIX,
  SONIC_LEGACY_CONTRAST_EXCEPTIONS,
  SONIC_DARK,
  SONIC_LIGHT,
  THEME_CONTRAST_MAX,
  THEME_CONTRAST_MIN,
  applyResolvedTheme,
  compositeSrgb,
  contrastRatio,
  hexToOklab,
  isHexColor,
  oklabToHex,
  oklabToOklch,
  nonTextContrastFailures,
  previewAppearanceImport,
  resolveTheme,
  sanitizeStoredAppearance,
  sanitizeTheme,
  semanticContrastFailures,
  sonicSupportedNonTextContrastFailures,
  sonicSupportedTextContrastFailures,
  validateCache,
  type MurmurTokens,
} from '.';

const TEST_SURFACE_TOKENS = [
  'background',
  'surface',
  'surface-container-low',
  'surface-container',
  'surface-container-high',
  'surface-container-lowest',
  'surface-container-highest',
] as const;

function isEmergencyFallback(
  tokens: MurmurTokens,
  appearance: 'light' | 'dark',
): boolean {
  const surface = appearance === 'light' ? '#ffffff' : '#000000';
  const primary = appearance === 'light' ? '#005a75' : '#92dbfe';
  return TEST_SURFACE_TOKENS.every((token) => tokens[token] === surface)
    && tokens.primary === primary;
}

function hueDistance(left: string, right: string): number {
  const leftHue = oklabToOklch(hexToOklab(left as `#${string}`)).h;
  const rightHue = oklabToOklch(hexToOklab(right as `#${string}`)).h;
  return Math.min(Math.abs(leftHue - rightHue), 360 - Math.abs(leftHue - rightHue));
}

describe('appearance color math and resolver', () => {
  beforeEach(() => {
    document.documentElement.removeAttribute('data-appearance');
    document.documentElement.removeAttribute('style');
  });

  it.each(['#000000', '#ffffff', '#036785', '#92dbfe', '#ff00aa'] as const)(
    'round-trips %s through dependency-free OKLab',
    (color) => {
      expect(oklabToHex(hexToOklab(color))).toBe(color);
    },
  );

  it.each([
    ['light', SONIC_LIGHT],
    ['dark', SONIC_DARK],
  ] as const)('preserves and directly validates the exact Sonic %s fixture', (appearance, fixture) => {
    const resolved = resolveTheme({ version: 1, presetId: 'sonic' }, appearance);
    expect(resolved.tokens).toEqual(fixture);
    expect(resolved.adjustments).toEqual([]);
    expect(sonicSupportedTextContrastFailures(fixture)).toEqual([]);
    expect(sonicSupportedNonTextContrastFailures(fixture)).toEqual([]);
  });

  it('keeps the untouched Sonic exceptions explicit and narrow', () => {
    expect(SONIC_LEGACY_CONTRAST_EXCEPTIONS).toEqual([
      'outline-variant is decorative; meaningful focus and selection use primary',
      'error text is unsupported on surface-container-highest',
      'error text in a 10% error tint is unsupported on surface-container-high and surface-container-highest',
      'primary is unsupported as text on 10% and 15% primary tints; tinted containers use on-surface',
      'on-surface-variant is unsupported on primary tints; tinted containers use on-surface',
    ]);
    expect(contrastRatio(
      SONIC_LIGHT['outline-variant'],
      SONIC_LIGHT.background,
    )).toBeLessThan(3);
    expect(contrastRatio(
      SONIC_DARK.error,
      SONIC_DARK['surface-container-highest'],
    )).toBeLessThan(4.5);
    for (const surface of ['surface-container-high', 'surface-container-highest'] as const) {
      expect(contrastRatio(
        SONIC_DARK.error,
        compositeSrgb(SONIC_DARK.error, SONIC_DARK[surface], 0.1),
      )).toBeLessThan(4.5);
    }
    for (const alpha of [0.1, 0.15]) {
      expect(contrastRatio(
        SONIC_LIGHT.primary,
        compositeSrgb(
          SONIC_LIGHT.primary,
          SONIC_LIGHT['surface-container-highest'],
          alpha,
        ),
      )).toBeLessThan(4.5);
    }
    expect(contrastRatio(
      SONIC_DARK['on-surface-variant'],
      compositeSrgb(
        SONIC_DARK.primary,
        SONIC_DARK['surface-container-highest'],
        0.1,
      ),
    )).toBeLessThan(4.5);
  });

  it('keeps exact Sonic on-surface text valid on every primary tint and adjacent surface', () => {
    const surfaces = [
      'background',
      'surface',
      'surface-container-low',
      'surface-container',
      'surface-container-high',
      'surface-container-lowest',
      'surface-container-highest',
    ] as const;
    for (const fixture of [SONIC_LIGHT, SONIC_DARK]) {
      for (const alpha of [0.05, 0.1, 0.15]) {
        for (const surface of surfaces) {
          expect(contrastRatio(
            fixture['on-surface'],
            compositeSrgb(fixture.primary, fixture[surface], alpha),
          )).toBeGreaterThanOrEqual(4.5);
        }
      }
    }
  });

  it('keeps exact Sonic success and warning text valid on every supported surface and tint', () => {
    const surfaces = [
      'background',
      'surface',
      'surface-container-low',
      'surface-container',
      'surface-container-high',
      'surface-container-lowest',
      'surface-container-highest',
    ] as const;
    for (const fixture of [SONIC_LIGHT, SONIC_DARK]) {
      for (const status of ['success', 'warning'] as const) {
        for (const surface of surfaces) {
          expect(contrastRatio(fixture[status], fixture[surface])).toBeGreaterThanOrEqual(4.5);
          expect(contrastRatio(
            fixture[status],
            compositeSrgb(fixture[status], fixture[surface], 0.1),
          )).toBeGreaterThanOrEqual(4.5);
        }
      }
    }
  });

  it('enforces the complete mutable matrix after any Sonic mutation', () => {
    expect(semanticContrastFailures(SONIC_DARK).length).toBeGreaterThan(0);
    const customized = resolveTheme({
      version: 1,
      presetId: 'sonic',
      contrast: 1,
    }, 'dark');
    expect(semanticContrastFailures(customized.tokens)).toEqual([]);
  });

  it.each([
    '#ff0000',
    '#00ff00',
    '#0000ff',
    '#ff00ff',
    '#00ffff',
    '#ffff00',
    '#7f00ff',
    '#777777',
  ])('derives valid, accessible accent tokens from %s', (accent) => {
    for (const appearance of ['light', 'dark'] as const) {
      const resolved = resolveTheme({ version: 1, presetId: 'custom', accent }, appearance);
      expect(isHexColor(resolved.tokens.primary)).toBe(true);
      expect(isHexColor(resolved.tokens['primary-dim'])).toBe(true);
      expect(contrastRatio(resolved.tokens['on-primary'], resolved.tokens.primary)).toBeGreaterThanOrEqual(4.5);
      expect(contrastRatio(resolved.tokens['on-primary'], resolved.tokens['primary-dim'])).toBeGreaterThanOrEqual(4.5);
    }
  });

  it.each([THEME_CONTRAST_MIN, THEME_CONTRAST_MAX])(
    'keeps the semantic matrix accessible at contrast extreme %s',
    (contrast) => {
      for (const appearance of ['light', 'dark'] as const) {
        const resolved = resolveTheme({
          version: 1,
          presetId: 'custom',
          background: appearance === 'light' ? '#f0d8c0' : '#18283b',
          foreground: appearance === 'light' ? '#c0b0a0' : '#586a7d',
          accent: '#ff5500',
          contrast,
        }, appearance);
        expect(semanticContrastFailures(resolved.tokens)).toEqual([]);
        expect(nonTextContrastFailures(resolved.tokens)).toEqual([]);
      }
    },
  );

  it('keeps adversarial valid custom colors inside both contrast matrices', () => {
    const colors = [
      '#000000',
      '#ffffff',
      '#777777',
      '#036785',
      '#ff0000',
      '#00ff00',
      '#0000ff',
      '#abcdef',
    ];
    for (const appearance of ['light', 'dark'] as const) {
      for (let index = 0; index < colors.length; index += 1) {
        for (const contrast of [THEME_CONTRAST_MIN, 0, THEME_CONTRAST_MAX]) {
          const resolved = resolveTheme({
            version: 1,
            presetId: 'custom',
            background: colors[index],
            foreground: colors[(index + 2) % colors.length],
            accent: colors[(index + 5) % colors.length],
            contrast,
          }, appearance);
          expect(
            semanticContrastFailures(resolved.tokens),
            `${appearance} semantic failures for ${JSON.stringify({ index, contrast })}`,
          ).toEqual([]);
          expect(
            nonTextContrastFailures(resolved.tokens),
            `${appearance} non-text failures for ${JSON.stringify({ index, contrast })}`,
          ).toEqual([]);
        }
      }
    }
  });

  it('keeps the primary pair coherent for the reported adversarial palette', () => {
    const resolved = resolveTheme({
      version: 1,
      presetId: 'custom',
      background: '#7a5f8a',
      foreground: '#30c961',
      accent: '#9ed34c',
      contrast: -84,
    }, 'light');
    expect(semanticContrastFailures(resolved.tokens)).toEqual([]);
    expect(nonTextContrastFailures(resolved.tokens)).toEqual([]);
    expect(contrastRatio(resolved.tokens['on-primary'], resolved.tokens.primary))
      .toBeGreaterThanOrEqual(4.5);
    expect(contrastRatio(resolved.tokens['on-primary'], resolved.tokens['primary-dim']))
      .toBeGreaterThanOrEqual(4.5);
  });

  it('repairs on-surface text on every primary-tint composite for the reported palette', () => {
    const resolved = resolveTheme({
      version: 1,
      presetId: 'custom',
      background: '#88596c',
      foreground: '#8885db',
      accent: '#16017e',
      contrast: -78,
    }, 'light');
    const surfaces = [
      'background',
      'surface',
      'surface-container-low',
      'surface-container',
      'surface-container-high',
      'surface-container-lowest',
      'surface-container-highest',
    ] as const;
    for (const alpha of [0.05, 0.1, 0.15]) {
      for (const surface of surfaces) {
        expect(contrastRatio(
          resolved.tokens['on-surface'],
          compositeSrgb(resolved.tokens.primary, resolved.tokens[surface], alpha),
        )).toBeGreaterThanOrEqual(4.5);
      }
    }
    expect(semanticContrastFailures(resolved.tokens)).toEqual([]);
  });

  it('preserves requested color families for representative themes and both reported repros', () => {
    const representative = [
      { version: 1 as const, presetId: 'custom' as const, accent: '#ff0000' },
      { version: 1 as const, presetId: 'custom' as const, accent: '#ff5500' },
      { version: 1 as const, presetId: 'custom' as const, accent: '#abcdef' },
      { version: 1 as const, presetId: 'custom' as const, accent: '#036785' },
      { version: 1 as const, presetId: 'custom' as const, background: '#46708a' },
      { version: 1 as const, presetId: 'custom' as const, foreground: '#f2d9bd' },
      {
        version: 1 as const,
        presetId: 'custom' as const,
        background: '#46708a',
        foreground: '#f2d9bd',
        accent: '#b83f78',
        contrast: 24,
      },
      {
        version: 1 as const,
        presetId: 'custom' as const,
        background: '#7a5f8a',
        foreground: '#30c961',
        accent: '#9ed34c',
        contrast: -84,
      },
      {
        version: 1 as const,
        presetId: 'custom' as const,
        background: '#88596c',
        foreground: '#8885db',
        accent: '#16017e',
        contrast: -78,
      },
    ];

    for (const theme of representative) {
      for (const appearance of ['light', 'dark'] as const) {
        const resolved = resolveTheme(theme, appearance);
        expect(semanticContrastFailures(resolved.tokens)).toEqual([]);
        expect(nonTextContrastFailures(resolved.tokens)).toEqual([]);
        expect(
          isEmergencyFallback(resolved.tokens, appearance),
          `${appearance} emergency fallback for ${JSON.stringify(theme)}`,
        ).toBe(false);
        expect(resolved.tokens.primary).not.toBe(resolved.tokens['primary-dim']);
        expect(oklabToOklch(hexToOklab(resolved.tokens.primary)).c).toBeGreaterThan(0.025);
        if (theme.accent) {
          expect(
            hueDistance(theme.accent, resolved.tokens.primary),
            `${appearance} accent family for ${JSON.stringify(theme)}`,
          ).toBeLessThan(12);
        }
        if (theme.background) {
          expect(
            hueDistance(theme.background, resolved.tokens.background),
            `${appearance} background family for ${JSON.stringify(theme)}`,
          ).toBeLessThan(12);
        }
        if (theme.foreground) {
          expect(
            hueDistance(theme.foreground, resolved.tokens['on-surface']),
            `${appearance} foreground family for ${JSON.stringify(theme)}`,
          ).toBeLessThan(12);
        }
      }
    }
  }, 30_000);

  it('uses no emergency fallbacks across 2,000 deterministic ordinary UI-field themes', () => {
    let seed = 0x377c0de;
    const next = () => {
      seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
      return seed;
    };
    const color = () => {
      const channel = () => 24 + (next() % 208);
      return `#${[channel(), channel(), channel()]
        .map((value) => value.toString(16).padStart(2, '0'))
        .join('')}`;
    };
    let fallbackCount = 0;
    let collapsedPrimaryCount = 0;
    let extremePrimaryLightnessCount = 0;
    let lowPrimaryChromaCount = 0;
    let highAdjustmentCount = 0;
    for (let index = 0; index < 2_000; index += 1) {
      const accent = color();
      const background = color();
      const foreground = color();
      const fields = index % 4;
      const theme = {
        version: 1 as const,
        presetId: 'custom' as const,
        accent,
        ...(fields >= 1 ? { background } : {}),
        ...(fields >= 2 ? { foreground } : {}),
        ...(fields === 3 ? { contrast: (next() % 161) - 80 } : {}),
      };
      const appearance = index % 2 === 0 ? 'light' as const : 'dark' as const;
      const resolved = resolveTheme(theme, appearance);
      expect(
        semanticContrastFailures(resolved.tokens),
        `semantic ordinary sample ${index}: ${JSON.stringify(theme)}`,
      ).toEqual([]);
      expect(
        nonTextContrastFailures(resolved.tokens),
        `non-text ordinary sample ${index}: ${JSON.stringify(theme)}`,
      ).toEqual([]);
      if (isEmergencyFallback(resolved.tokens, appearance)) fallbackCount += 1;
      if (resolved.tokens.primary === resolved.tokens['primary-dim']) {
        collapsedPrimaryCount += 1;
      }
      const primary = oklabToOklch(hexToOklab(resolved.tokens.primary));
      if (primary.l < 0.08 || primary.l > 0.92) extremePrimaryLightnessCount += 1;
      if (primary.c < 0.015) lowPrimaryChromaCount += 1;
      if (resolved.adjustments.length >= 10) highAdjustmentCount += 1;
    }
    expect(fallbackCount).toBe(0);
    expect(collapsedPrimaryCount).toBe(0);
    expect(extremePrimaryLightnessCount).toBe(0);
    expect(lowPrimaryChromaCount).toBe(0);
    expect(highAdjustmentCount).toBe(0);
  }, 120_000);

  it('passes deterministic seeded adversarial palettes before returning', () => {
    let seed = 0x5eed1234;
    const next = () => {
      seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
      return seed;
    };
    const color = () => `#${(next() & 0xffffff).toString(16).padStart(6, '0')}`;
    for (let index = 0; index < 96; index += 1) {
      const theme = {
        version: 1 as const,
        presetId: 'custom' as const,
        background: color(),
        foreground: color(),
        accent: color(),
        contrast: (next() % 201) - 100,
      };
      const appearance = index % 2 === 0 ? 'light' as const : 'dark' as const;
      const resolved = resolveTheme(theme, appearance);
      expect(
        semanticContrastFailures(resolved.tokens),
        `semantic seed case ${index}: ${JSON.stringify(theme)}`,
      ).toEqual([]);
      expect(
        nonTextContrastFailures(resolved.tokens),
        `non-text seed case ${index}: ${JSON.stringify(theme)}`,
      ).toEqual([]);
    }
  });

  it('locks the exact semantic pair inventory established by the themed DOM audit', () => {
    expect(SEMANTIC_CONTRAST_MATRIX.map((pair) => {
      const container = pair.backgroundTint
        ? `${pair.backgroundTint.token}@${pair.backgroundTint.opacity}`
        : pair.foregroundTint !== undefined
          ? `self@${pair.foregroundTint}`
          : 'raw';
      return `${pair.foreground}:${container}:${pair.backgrounds.join(',')}`;
    })).toEqual([
      `on-surface:raw:${TEST_SURFACE_TOKENS.join(',')}`,
      `on-surface:primary@0.05:${TEST_SURFACE_TOKENS.join(',')}`,
      `on-surface:primary@0.1:${TEST_SURFACE_TOKENS.join(',')}`,
      `on-surface:primary@0.15:${TEST_SURFACE_TOKENS.join(',')}`,
      `on-surface-variant:raw:${TEST_SURFACE_TOKENS.join(',')}`,
      `primary:raw:${TEST_SURFACE_TOKENS.join(',')}`,
      'on-primary:raw:primary,primary-dim',
      `error:raw:${TEST_SURFACE_TOKENS.join(',')}`,
      `error:self@0.1:${TEST_SURFACE_TOKENS.join(',')}`,
      `success:raw:${TEST_SURFACE_TOKENS.join(',')}`,
      `success:self@0.1:${TEST_SURFACE_TOKENS.join(',')}`,
      `warning:raw:${TEST_SURFACE_TOKENS.join(',')}`,
      `warning:self@0.1:${TEST_SURFACE_TOKENS.join(',')}`,
    ]);
    expect(NON_TEXT_CONTRAST_MATRIX.map((pair) =>
      `${pair.token}:${pair.tokenTint ?? 'raw'}:${pair.backgrounds.join(',')}`,
    )).toEqual([
      `primary:raw:${TEST_SURFACE_TOKENS.join(',')}`,
      `error:raw:${TEST_SURFACE_TOKENS.join(',')}`,
      `success:raw:${TEST_SURFACE_TOKENS.join(',')}`,
      `warning:raw:${TEST_SURFACE_TOKENS.join(',')}`,
    ]);
  });

  it('repairs an explicitly split black/white surface ladder', () => {
    for (const appearance of ['light', 'dark'] as const) {
      const resolved = resolveTheme({
        version: 1,
        presetId: 'custom',
        [appearance]: {
          background: '#000000',
          surface: '#ffffff',
          'surface-container-low': '#000000',
          'surface-container': '#ffffff',
          'surface-container-high': '#000000',
          'surface-container-lowest': '#ffffff',
          'surface-container-highest': '#000000',
        },
      }, appearance);
      expect(semanticContrastFailures(resolved.tokens)).toEqual([]);
      expect(nonTextContrastFailures(resolved.tokens)).toEqual([]);
    }
  });

  it('enforces contrast after explicit overrides', () => {
    const resolved = resolveTheme({
      version: 1,
      presetId: 'custom',
      light: {
        'on-surface': '#ffffff',
        'on-surface-variant': '#eeeeee',
        'on-primary': '#036785',
        error: '#f7fafc',
        success: '#f7fafc',
        warning: '#f7fafc',
      },
    }, 'light');
    expect(semanticContrastFailures(resolved.tokens)).toEqual([]);
    expect(resolved.adjustments.some((adjustment) => adjustment.reason === 'contrast')).toBe(true);
  });

  it('returns exactly the allowlisted token record', () => {
    const tokens = resolveTheme({
      version: 1,
      presetId: 'custom',
      light: {
        background: '#123456',
        ...({ '--evil': '#ffffff' } as Record<string, string>),
      } as Partial<MurmurTokens>,
    }, 'light').tokens;
    expect(Object.keys(tokens)).toEqual([...MURMUR_TOKEN_NAMES]);
    expect('--evil' in tokens).toBe(false);
  });

  it('applies concrete state and only allowlisted custom properties', () => {
    const root = document.documentElement;
    root.style.setProperty('--not-murmur', 'keep');
    const resolved = resolveTheme({ version: 1, presetId: 'sonic' }, 'dark');
    applyResolvedTheme(resolved, root);
    expect(root.dataset.appearance).toBe('dark');
    expect(root.style.colorScheme).toBe('dark');
    for (const token of MURMUR_TOKEN_NAMES) {
      expect(root.style.getPropertyValue(`--murmur-${token}`)).toBe(resolved.tokens[token]);
    }
    expect(root.style.getPropertyValue('--not-murmur')).toBe('keep');
    expect(root.style.background).toBe('');
    expect(root.style.backgroundColor).toBe('');
  });
});

describe('appearance sanitization', () => {
  it('falls unknown versions, modes, and presets back safely', () => {
    expect(sanitizeStoredAppearance({ version: 2 }).validDocument).toBe(false);
    const stored = sanitizeStoredAppearance({
      version: 1,
      revision: -10,
      mode: 'sepia',
      theme: { version: 1, presetId: 'remote-theme', accent: '#ABCDEF' },
    });
    expect(stored).toMatchObject({
      validDocument: true,
      revision: 0,
      mode: 'system',
      theme: { version: 1, presetId: 'sonic', accent: '#abcdef' },
    });
  });

  it('drops invalid colors and token keys and clamps contrast', () => {
    const sanitized = sanitizeTheme({
      version: 1,
      presetId: 'custom',
      accent: 'red',
      background: '#ABCDEF',
      contrast: 99.6,
      light: {
        primary: '#123456',
        error: '#xyzxyz',
        arbitrary: '#ffffff',
      },
    });
    expect(sanitized).toEqual({
      version: 1,
      presetId: 'custom',
      background: '#abcdef',
      contrast: 100,
      light: { primary: '#123456' },
    });
  });

  it('requires exact cache keys and valid values', () => {
    const valid = { version: 1, light: SONIC_LIGHT, dark: SONIC_DARK };
    expect(validateCache(valid)).toEqual(valid);
    expect(validateCache({
      ...valid,
      light: { ...SONIC_LIGHT, extra: '#ffffff' },
    })).toBeNull();
    const partial = { ...SONIC_DARK } as Partial<MurmurTokens>;
    delete partial.warning;
    expect(validateCache({ ...valid, dark: partial })).toBeNull();
    expect(validateCache({
      ...valid,
      dark: { ...SONIC_DARK, primary: 'red' },
    })).toBeNull();
  });

  it('strips imported cache and sanitizes authoritative configuration', () => {
    const preview = previewAppearanceImport(JSON.stringify({
      version: 1,
      mode: 'dark',
      theme: { version: 1, presetId: 'custom', accent: '#abcdef' },
      cache: {
        version: 1,
        light: Object.fromEntries(MURMUR_TOKEN_NAMES.map((token) => [token, '#000000'])),
        dark: Object.fromEntries(MURMUR_TOKEN_NAMES.map((token) => [token, '#000000'])),
      },
    }));
    expect(preview.mode).toBe('dark');
    expect(preview.light).not.toEqual(
      Object.fromEntries(MURMUR_TOKEN_NAMES.map((token) => [token, '#000000'])),
    );
  });
});
