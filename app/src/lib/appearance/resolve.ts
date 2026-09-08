import {
  contrastRatio,
  compositeSrgb,
  ensureContrast,
  hexToOklab,
  mixOklab,
  oklabToOklch,
  oklchToHexInGamut,
} from './color';
import { SONIC_DARK, SONIC_LIGHT } from './palettes';
import { sanitizeTheme } from './sanitize';
import {
  THEME_CONTRAST_DEFAULT,
  type HexColor,
  type MurmurTokenName,
  type MurmurTokens,
  type ResolvedAppearance,
  type ResolvedTheme,
  type ThemeAdjustment,
  type ThemeConfigV1,
} from './types';

const SURFACE_TOKENS = [
  'background',
  'surface',
  'surface-container-low',
  'surface-container',
  'surface-container-high',
  'surface-container-lowest',
  'surface-container-highest',
] as const satisfies readonly MurmurTokenName[];

export interface SemanticContrastPair {
  foreground: MurmurTokenName;
  backgrounds: readonly MurmurTokenName[];
  minimum: number;
  foregroundTint?: number;
  backgroundTint?: {
    token: MurmurTokenName;
    opacity: number;
  };
}

export interface NonTextContrastPair {
  token: MurmurTokenName;
  backgrounds: readonly MurmurTokenName[];
  minimum: 3;
  tokenTint?: number;
}

const PRIMARY_TINT_OPACITIES = [0.05, 0.1, 0.15] as const;

function primaryTintTextPairs(
  foreground: 'on-surface' | 'on-surface-variant',
): SemanticContrastPair[] {
  return PRIMARY_TINT_OPACITIES.map((opacity) => ({
    foreground,
    backgrounds: SURFACE_TOKENS,
    minimum: 4.5,
    backgroundTint: { token: 'primary', opacity },
  }));
}

// Mutable themes are repaired against the complete semantic usage matrix.
// Untouched Sonic uses the narrower, explicitly documented fixture matrix
// below so the original 14 shipped tokens remain byte-for-byte stable.
export const SEMANTIC_CONTRAST_MATRIX: readonly SemanticContrastPair[] = [
  { foreground: 'on-surface', backgrounds: SURFACE_TOKENS, minimum: 4.5 },
  ...primaryTintTextPairs('on-surface'),
  { foreground: 'on-surface-variant', backgrounds: SURFACE_TOKENS, minimum: 4.5 },
  { foreground: 'primary', backgrounds: SURFACE_TOKENS, minimum: 4.5 },
  { foreground: 'on-primary', backgrounds: ['primary', 'primary-dim'], minimum: 4.5 },
  { foreground: 'on-primary', backgrounds: ['error'], minimum: 4.5 },
  { foreground: 'error', backgrounds: SURFACE_TOKENS, minimum: 4.5 },
  { foreground: 'error', backgrounds: SURFACE_TOKENS, minimum: 4.5, foregroundTint: 0.1 },
  { foreground: 'success', backgrounds: SURFACE_TOKENS, minimum: 4.5 },
  { foreground: 'success', backgrounds: SURFACE_TOKENS, minimum: 4.5, foregroundTint: 0.1 },
  { foreground: 'warning', backgrounds: SURFACE_TOKENS, minimum: 4.5 },
  { foreground: 'warning', backgrounds: SURFACE_TOKENS, minimum: 4.5, foregroundTint: 0.1 },
] as const;

export const NON_TEXT_CONTRAST_MATRIX: readonly NonTextContrastPair[] = [
  { token: 'primary', backgrounds: SURFACE_TOKENS, minimum: 3 },
  { token: 'error', backgrounds: SURFACE_TOKENS, minimum: 3 },
  { token: 'success', backgrounds: SURFACE_TOKENS, minimum: 3 },
  { token: 'warning', backgrounds: SURFACE_TOKENS, minimum: 3 },
] as const;

const SONIC_ERROR_TEXT_SURFACES = [
  'background',
  'surface',
  'surface-container-low',
  'surface-container',
  'surface-container-high',
  'surface-container-lowest',
] as const satisfies readonly MurmurTokenName[];

const SONIC_ERROR_TINT_SURFACES = [
  'background',
  'surface',
  'surface-container-low',
  'surface-container',
  'surface-container-lowest',
] as const satisfies readonly MurmurTokenName[];

export const SONIC_SUPPORTED_TEXT_CONTRAST_MATRIX: readonly SemanticContrastPair[] = [
  { foreground: 'on-surface', backgrounds: SURFACE_TOKENS, minimum: 4.5 },
  { foreground: 'on-surface-variant', backgrounds: SURFACE_TOKENS, minimum: 4.5 },
  ...primaryTintTextPairs('on-surface'),
  { foreground: 'primary', backgrounds: SURFACE_TOKENS, minimum: 4.5 },
  { foreground: 'on-primary', backgrounds: ['primary', 'primary-dim'], minimum: 4.5 },
  { foreground: 'on-primary', backgrounds: ['error'], minimum: 4.5 },
  { foreground: 'error', backgrounds: SONIC_ERROR_TEXT_SURFACES, minimum: 4.5 },
  { foreground: 'error', backgrounds: SONIC_ERROR_TINT_SURFACES, minimum: 4.5, foregroundTint: 0.1 },
  { foreground: 'success', backgrounds: SURFACE_TOKENS, minimum: 4.5 },
  { foreground: 'success', backgrounds: SURFACE_TOKENS, minimum: 4.5, foregroundTint: 0.1 },
  { foreground: 'warning', backgrounds: SURFACE_TOKENS, minimum: 4.5 },
  { foreground: 'warning', backgrounds: SURFACE_TOKENS, minimum: 4.5, foregroundTint: 0.1 },
] as const;

export const SONIC_SUPPORTED_NON_TEXT_CONTRAST_MATRIX: readonly NonTextContrastPair[] = [
  { token: 'primary', backgrounds: SURFACE_TOKENS, minimum: 3 },
  { token: 'error', backgrounds: SURFACE_TOKENS, minimum: 3 },
  { token: 'success', backgrounds: SURFACE_TOKENS, minimum: 3 },
  { token: 'warning', backgrounds: SURFACE_TOKENS, minimum: 3 },
] as const;

export const SONIC_LEGACY_CONTRAST_EXCEPTIONS = [
  'outline-variant is decorative; meaningful focus and selection use primary',
  'error text is unsupported on surface-container-highest',
  'error text in a 10% error tint is unsupported on surface-container-high and surface-container-highest',
  'primary is unsupported as text on 10% and 15% primary tints; tinted containers use on-surface',
  'on-surface-variant is unsupported on primary tints; tinted containers use on-surface',
] as const;

function cloneTokens(tokens: MurmurTokens): MurmurTokens {
  return { ...tokens };
}

const SURFACE_POLE_MINIMUM = 7;

function contrastPole(background: HexColor): HexColor {
  const black = '#000000' as HexColor;
  const white = '#ffffff' as HexColor;
  return contrastRatio(black, compositeSrgb(black, background, 0.15))
    >= contrastRatio(white, compositeSrgb(white, background, 0.15))
    ? black
    : white;
}

function repairSurfaceForPole(surface: HexColor, foreground: HexColor): HexColor {
  if (
    contrastRatio(foreground, compositeSrgb(foreground, surface, 0.15))
    >= SURFACE_POLE_MINIMUM
  ) {
    return surface;
  }
  const original = oklabToOklch(hexToOklab(surface));
  let best: { color: HexColor; distance: number } | null = null;
  for (let step = 0; step <= 200; step += 1) {
    const lightness = step / 200;
    const candidate = oklchToHexInGamut({ ...original, l: lightness }).color;
    if (
      contrastRatio(foreground, compositeSrgb(foreground, candidate, 0.15))
      < SURFACE_POLE_MINIMUM
    ) {
      continue;
    }
    const distance = Math.abs(lightness - original.l);
    if (!best || distance < best.distance) best = { color: candidate, distance };
  }
  return best?.color ?? (foreground === '#000000' ? '#ffffff' : '#000000');
}

const ACCESSIBLE_MUTABLE_FALLBACK: Record<ResolvedAppearance, MurmurTokens> = {
  light: {
    background: '#ffffff',
    surface: '#ffffff',
    'surface-container-low': '#ffffff',
    'surface-container': '#ffffff',
    'surface-container-high': '#ffffff',
    'surface-container-lowest': '#ffffff',
    'surface-container-highest': '#ffffff',
    primary: '#005a75',
    'primary-dim': '#004a60',
    'on-primary': '#ffffff',
    'on-surface': '#111111',
    'on-surface-variant': '#333333',
    'outline-variant': '#595959',
    error: '#8b1a1a',
    success: '#176b3a',
    warning: '#704d00',
  },
  dark: {
    background: '#000000',
    surface: '#000000',
    'surface-container-low': '#000000',
    'surface-container': '#000000',
    'surface-container-high': '#000000',
    'surface-container-lowest': '#000000',
    'surface-container-highest': '#000000',
    primary: '#92dbfe',
    'primary-dim': '#84cdef',
    'on-primary': '#000000',
    'on-surface': '#f5f5f5',
    'on-surface-variant': '#cccccc',
    'outline-variant': '#a0a0a0',
    error: '#ff8a85',
    success: '#66d99a',
    warning: '#f4bd65',
  },
};

function surfaceLadder(
  background: HexColor,
  contrast: number,
): Pick<MurmurTokens, (typeof SURFACE_TOKENS)[number]> {
  const pole = contrastPole(background);
  const repairedBackground = repairSurfaceForPole(background, pole);
  const lowestPole = pole === '#000000' ? '#ffffff' : '#000000';
  const strength = 1 + contrast / 100;
  const rung = (target: HexColor, amount: number) =>
    repairSurfaceForPole(mixOklab(repairedBackground, target, amount), pole);
  return {
    background: repairedBackground,
    surface: repairedBackground,
    'surface-container-low': rung(pole, 0.035 * strength),
    'surface-container': rung(pole, 0.065 * strength),
    'surface-container-high': rung(pole, 0.1 * strength),
    'surface-container-lowest': rung(lowestPole, 0.02 * strength),
    'surface-container-highest': rung(pole, 0.14 * strength),
  };
}

function deriveAccent(
  accent: HexColor,
  appearance: ResolvedAppearance,
): Pick<MurmurTokens, 'primary' | 'primary-dim' | 'on-primary'> & {
  clipped: boolean;
} {
  const input = oklabToOklch(hexToOklab(accent));
  const lightness = appearance === 'light'
    ? Math.min(input.l, 0.52)
    : Math.max(input.l, 0.78);
  const primaryResult = oklchToHexInGamut({
    l: lightness,
    c: Math.min(0.22, Math.max(0.045, input.c)),
    h: input.h,
  });
  const dimResult = oklchToHexInGamut({
    l: appearance === 'light' ? Math.max(0, lightness - 0.055) : Math.max(0, lightness - 0.045),
    c: Math.min(0.22, Math.max(0.04, input.c)),
    h: input.h,
  });
  const preferredOnPrimary = appearance === 'light'
    ? '#ffffff' as HexColor
    : '#001216' as HexColor;
  const onPrimary = ensureContrast(
    preferredOnPrimary,
    [primaryResult.color, dimResult.color],
    4.5,
  );
  return {
    primary: primaryResult.color,
    'primary-dim': dimResult.color,
    'on-primary': onPrimary,
    clipped: primaryResult.clipped || dimResult.clipped,
  };
}

function recordChange(
  tokens: MurmurTokens,
  adjustments: ThemeAdjustment[],
  appearance: ResolvedAppearance,
  token: MurmurTokenName,
  value: HexColor,
  reason: ThemeAdjustment['reason'],
): void {
  const previous = tokens[token];
  if (previous === value) return;
  tokens[token] = value;
  adjustments.push({ appearance, token, reason, from: previous, to: value });
}

function ensureCoherentSurfaceRange(
  tokens: MurmurTokens,
  adjustments: ThemeAdjustment[],
  appearance: ResolvedAppearance,
): void {
  // Leave enough headroom beyond AA for a chromatic foreground on the chosen
  // pole. Repairing surfaces only to the 4.5 boundary forces saturated accents
  // to collapse to pure black/white once 15% tint composites are considered.
  const foreground = contrastPole(tokens.background);

  for (const token of SURFACE_TOKENS) {
    recordChange(
      tokens,
      adjustments,
      appearance,
      token,
      repairSurfaceForPole(tokens[token], foreground),
      'contrast',
    );
  }
}

function enforceTextPair(
  tokens: MurmurTokens,
  pair: SemanticContrastPair,
): HexColor {
  let output = tokens[pair.foreground];
  for (let attempt = 0; attempt < 8; attempt += 1) {
    const backgrounds = pair.backgrounds.map((token) => {
      if (pair.foregroundTint !== undefined) {
        return compositeSrgb(output, tokens[token], pair.foregroundTint);
      }
      if (pair.backgroundTint !== undefined) {
        return compositeSrgb(
          tokens[pair.backgroundTint.token],
          tokens[token],
          pair.backgroundTint.opacity,
        );
      }
      return tokens[token];
    });
    const next = ensureContrast(output, backgrounds, pair.minimum);
    if (next === output) break;
    output = next;
  }
  return output;
}

function enforceNonTextPair(
  tokens: MurmurTokens,
  pair: NonTextContrastPair,
): HexColor {
  let output = tokens[pair.token];
  for (let attempt = 0; attempt < 8; attempt += 1) {
    const backgrounds = pair.backgrounds.map((token) =>
      pair.tokenTint === undefined
        ? tokens[token]
        : compositeSrgb(output, tokens[token], pair.tokenTint),
    );
    const next = ensureContrast(output, backgrounds, pair.minimum);
    if (next === output) break;
    output = next;
  }
  return output;
}

function textPairPassesCandidate(
  tokens: MurmurTokens,
  pair: SemanticContrastPair,
  candidate: HexColor,
): boolean {
  return pair.backgrounds.every((background) => {
    let surface = tokens[background];
    if (pair.foregroundTint !== undefined) {
      surface = compositeSrgb(candidate, tokens[background], pair.foregroundTint);
    } else if (pair.backgroundTint !== undefined) {
      surface = compositeSrgb(
        tokens[pair.backgroundTint.token],
        tokens[background],
        pair.backgroundTint.opacity,
      );
    }
    return contrastRatio(candidate, surface) >= pair.minimum;
  });
}

function nonTextPairPassesCandidate(
  tokens: MurmurTokens,
  pair: NonTextContrastPair,
  candidate: HexColor,
): boolean {
  return pair.backgrounds.every((background) => {
    const adjacent = pair.tokenTint === undefined
      ? tokens[background]
      : compositeSrgb(candidate, tokens[background], pair.tokenTint);
    return contrastRatio(candidate, adjacent) >= pair.minimum;
  });
}

/**
 * Solve every constraint owned by one semantic token at once. Searching a
 * single OKLCH lightness axis preserves the requested hue and as much chroma
 * as the sRGB gamut permits, while avoiding sequential pair repairs that can
 * oscillate between opposite contrast poles.
 */
function solveSemanticToken(
  tokens: MurmurTokens,
  token: MurmurTokenName,
  textPairs: readonly SemanticContrastPair[],
  nonTextPairs: readonly NonTextContrastPair[],
): HexColor {
  const source = tokens[token];
  const passes = (candidate: HexColor) =>
    textPairs.every((pair) => textPairPassesCandidate(tokens, pair, candidate))
    && nonTextPairs.every((pair) => nonTextPairPassesCandidate(tokens, pair, candidate));
  if (passes(source)) return source;

  const original = oklabToOklch(hexToOklab(source));
  let best: { color: HexColor; distance: number } | null = null;
  const visited = new Set<HexColor>();
  for (let step = 0; step <= 200; step += 1) {
    const lightness = step / 200;
    const candidate = oklchToHexInGamut({ ...original, l: lightness }).color;
    if (visited.has(candidate)) continue;
    visited.add(candidate);
    if (!passes(candidate)) continue;
    const distance = Math.abs(lightness - original.l);
    if (!best || distance < best.distance) best = { color: candidate, distance };
  }
  if (best) return best.color;

  // Poles are included explicitly in case extreme OKLCH chroma clipping does
  // not land on an exact neutral endpoint.
  for (const pole of ['#000000', '#ffffff'] as const) {
    if (passes(pole)) return pole;
  }
  return source;
}

function solvePrimaryDim(tokens: MurmurTokens): HexColor {
  const primary = oklabToOklch(hexToOklab(tokens.primary));
  const requestedDim = oklabToOklch(hexToOklab(tokens['primary-dim']));
  const black = '#000000' as HexColor;
  const white = '#ffffff' as HexColor;
  const onPrimaryPole = contrastRatio(black, tokens.primary)
    >= contrastRatio(white, tokens.primary)
    ? black
    : white;
  const target = oklchToHexInGamut({
    ...requestedDim,
    l: Math.max(0, Math.min(1, primary.l - 0.045)),
  }).color;
  const repaired = ensureContrast(target, [onPrimaryPole], 4.5);
  if (repaired !== tokens.primary) return repaired;

  // A tiny extra move toward the primary pair's pole keeps the hover/gradient
  // role visibly distinct without changing hue.
  return oklchToHexInGamut({
    ...requestedDim,
    l: Math.max(
      0,
      Math.min(1, primary.l + (onPrimaryPole === black ? 0.02 : -0.02)),
    ),
  }).color;
}

function solveMutablePalette(
  tokens: MurmurTokens,
  adjustments: ThemeAdjustment[],
  appearance: ResolvedAppearance,
): void {
  // Accent tokens establish the primary-tint backgrounds first. Surface and
  // status foregrounds are then solved against those stable composites, and
  // on-primary is derived last against the final primary pair.
  const primary = solveSemanticToken(
    tokens,
    'primary',
    SEMANTIC_CONTRAST_MATRIX.filter((pair) => pair.foreground === 'primary'),
    NON_TEXT_CONTRAST_MATRIX.filter((pair) => pair.token === 'primary'),
  );
  recordChange(tokens, adjustments, appearance, 'primary', primary, 'contrast');
  // primary-dim is a derived hover/gradient role, not a user-selected token.
  // Aligning it with the final primary is normal derivation, not an automatic
  // accessibility adjustment to report.
  tokens['primary-dim'] = solvePrimaryDim(tokens);

  const order = [
    'error',
    'success',
    'warning',
    'on-surface',
    'on-surface-variant',
    'on-primary',
  ] as const satisfies readonly MurmurTokenName[];

  for (const token of order) {
    const after = solveSemanticToken(
      tokens,
      token,
      SEMANTIC_CONTRAST_MATRIX.filter((pair) => pair.foreground === token),
      NON_TEXT_CONTRAST_MATRIX.filter((pair) => pair.token === token),
    );
    recordChange(tokens, adjustments, appearance, token, after, 'contrast');
  }
}

export function resolveTheme(
  theme: ThemeConfigV1,
  appearance: ResolvedAppearance,
): ResolvedTheme {
  const safeTheme = sanitizeTheme(theme);
  // Unknown preset IDs are sanitized before this point; custom deliberately
  // starts from Sonic as its stable base.
  const tokens = cloneTokens(appearance === 'light' ? SONIC_LIGHT : SONIC_DARK);
  const adjustments: ThemeAdjustment[] = [];
  const contrast = safeTheme.contrast ?? THEME_CONTRAST_DEFAULT;

  if (safeTheme.background || safeTheme.contrast !== undefined) {
    const background = (safeTheme.background ?? tokens.background) as HexColor;
    const ladder = surfaceLadder(background, contrast);
    Object.assign(tokens, ladder);
    if (ladder.background !== background) {
      adjustments.push({
        appearance,
        token: 'background',
        reason: 'contrast',
        from: background,
        to: ladder.background,
      });
    }
  }

  if (safeTheme.accent) {
    const derived = deriveAccent(safeTheme.accent as HexColor, appearance);
    // These are the normal outputs of the accent control. The grouped solver
    // below records only a further accessibility repair, avoiding duplicate
    // Sonic→derived→solved adjustment chains.
    tokens.primary = derived.primary;
    tokens['primary-dim'] = derived['primary-dim'];
    tokens['on-primary'] = derived['on-primary'];
  }

  if (safeTheme.foreground) {
    const foreground = safeTheme.foreground as HexColor;
    tokens['on-surface'] = foreground;
    tokens['on-surface-variant'] = mixOklab(foreground, tokens.background, 0.22);
  }

  Object.assign(tokens, appearance === 'light' ? safeTheme.light : safeTheme.dark);

  // Untouched Sonic preserves the exact shipped first-paint/reset fixtures,
  // but still validates every pair supported by the post-migration UI. Only
  // the explicit SONIC_LEGACY_CONTRAST_EXCEPTIONS are excluded. Every mutable
  // path uses the complete matrix below.
  const untouchedSonic = safeTheme.presetId === 'sonic'
    && safeTheme.accent === undefined
    && safeTheme.background === undefined
    && safeTheme.foreground === undefined
    && safeTheme.contrast === undefined
    && safeTheme.light === undefined
    && safeTheme.dark === undefined;
  const contrastMatrix = untouchedSonic
    ? SONIC_SUPPORTED_TEXT_CONTRAST_MATRIX
    : SEMANTIC_CONTRAST_MATRIX;

  if (!untouchedSonic) ensureCoherentSurfaceRange(tokens, adjustments, appearance);

  if (untouchedSonic) {
    for (const pair of contrastMatrix) {
      const after = enforceTextPair(tokens, pair);
      recordChange(tokens, adjustments, appearance, pair.foreground, after, 'contrast');
    }
    for (const pair of SONIC_SUPPORTED_NON_TEXT_CONTRAST_MATRIX) {
      const after = enforceNonTextPair(tokens, pair);
      recordChange(tokens, adjustments, appearance, pair.token, after, 'contrast');
    }
  } else {
    solveMutablePalette(tokens, adjustments, appearance);

    // This postcondition is deliberately fail-closed. Sanitized custom input
    // must never escape with a known text or non-text failure, even if a future
    // resolver change introduces an unexpected interaction between matrices.
    if (
      semanticContrastFailures(tokens).length > 0
      || nonTextContrastFailures(tokens).length > 0
    ) {
      const fallback = ACCESSIBLE_MUTABLE_FALLBACK[appearance];
      for (const token of Object.keys(fallback) as MurmurTokenName[]) {
        recordChange(tokens, adjustments, appearance, token, fallback[token], 'contrast');
      }
    }
  }

  return {
    appearance,
    colorScheme: appearance,
    tokens,
    adjustments,
  };
}

export function nonTextContrastFailures(tokens: MurmurTokens): NonTextContrastPair[] {
  return contrastFailuresForNonTextMatrix(tokens, NON_TEXT_CONTRAST_MATRIX);
}

function contrastFailuresForNonTextMatrix(
  tokens: MurmurTokens,
  matrix: readonly NonTextContrastPair[],
): NonTextContrastPair[] {
  return matrix.filter((pair) =>
    pair.backgrounds.some((background) => {
      const adjacent = pair.tokenTint === undefined
        ? tokens[background]
        : compositeSrgb(tokens[pair.token], tokens[background], pair.tokenTint);
      return contrastRatio(tokens[pair.token], adjacent) < pair.minimum;
    }),
  );
}

export const semanticNonTextContrastFailures = nonTextContrastFailures;

export function semanticContrastFailures(tokens: MurmurTokens): SemanticContrastPair[] {
  return contrastFailuresForTextMatrix(tokens, SEMANTIC_CONTRAST_MATRIX);
}

function contrastFailuresForTextMatrix(
  tokens: MurmurTokens,
  matrix: readonly SemanticContrastPair[],
): SemanticContrastPair[] {
  return matrix.filter((pair) =>
    pair.backgrounds.some((background) => {
      let surface = tokens[background];
      if (pair.foregroundTint !== undefined) {
        surface = compositeSrgb(
          tokens[pair.foreground],
          tokens[background],
          pair.foregroundTint,
        );
      } else if (pair.backgroundTint !== undefined) {
        surface = compositeSrgb(
          tokens[pair.backgroundTint.token],
          tokens[background],
          pair.backgroundTint.opacity,
        );
      }
      return contrastRatio(tokens[pair.foreground], surface) < pair.minimum;
    }),
  );
}

export function sonicSupportedTextContrastFailures(tokens: MurmurTokens): SemanticContrastPair[] {
  return contrastFailuresForTextMatrix(tokens, SONIC_SUPPORTED_TEXT_CONTRAST_MATRIX);
}

export function sonicSupportedNonTextContrastFailures(tokens: MurmurTokens): NonTextContrastPair[] {
  return contrastFailuresForNonTextMatrix(tokens, SONIC_SUPPORTED_NON_TEXT_CONTRAST_MATRIX);
}
