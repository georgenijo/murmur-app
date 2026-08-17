import { normalizeHex } from './color';
import { resolveTheme } from './resolve';
import { sanitizeTheme } from './sanitize';
import type {
  HexColor,
  MurmurTokenName,
  ResolvedAppearance,
  ThemeConfigV1,
} from './types';

type Rgba = { r: number; g: number; b: number; a: number };
type Rgb = { r: number; g: number; b: number };

export interface ConvertedVsCodeTheme {
  label: string;
  appearance: ResolvedAppearance;
  theme: ThemeConfigV1;
  sourceName?: string;
  sourcePath?: string;
}

export interface PairedVsCodeTheme {
  label: string;
  modes: ResolvedAppearance[];
  theme: ThemeConfigV1;
  sourceIdentities: string[];
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function decodeGamma(value: number): number {
  return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
}

function encodeGamma(value: number): number {
  const clamped = Math.max(0, Math.min(1, value));
  return clamped <= 0.0031308 ? clamped * 12.92 : 1.055 * clamped ** (1 / 2.4) - 0.055;
}

function parseColorFunction(value: string): Rgba | null {
  const match = /^color\(\s*(display-p3|srgb)\s+([^)]+)\)$/i.exec(value);
  if (!match) return null;
  const space = match[1]!.toLowerCase();
  const [channelPart, alphaPart] = match[2]!.split('/');
  const channels = channelPart!
    .trim()
    .split(/\s+/)
    .map((part) => (part.endsWith('%') ? Number.parseFloat(part) / 100 : Number.parseFloat(part)));
  if (channels.length !== 3 || channels.some((channel) => !Number.isFinite(channel))) return null;
  const alphaRaw = alphaPart?.trim();
  const alpha = alphaRaw === undefined
    ? 1
    : alphaRaw.endsWith('%')
      ? Number.parseFloat(alphaRaw) / 100
      : Number.parseFloat(alphaRaw);
  if (!Number.isFinite(alpha)) return null;
  const [red, green, blue] = channels as [number, number, number];
  if (space === 'srgb') {
    return { r: red * 255, g: green * 255, b: blue * 255, a: Math.max(0, Math.min(1, alpha)) };
  }
  const [linearRed, linearGreen, linearBlue] = [red, green, blue].map(decodeGamma) as [
    number,
    number,
    number,
  ];
  const srgb = [
    1.2249401762805 * linearRed - 0.2249401762805 * linearGreen,
    -0.042056961239 * linearRed + 1.042056961239 * linearGreen,
    -0.0196375547643 * linearRed - 0.0786360655012 * linearGreen + 1.0982736202656 * linearBlue,
  ].map((channel) => encodeGamma(channel) * 255) as [number, number, number];
  return { r: srgb[0], g: srgb[1], b: srgb[2], a: Math.max(0, Math.min(1, alpha)) };
}

function parseVsCodeColor(value: unknown): Rgba | null {
  if (typeof value !== 'string' || value.length > 128) return null;
  const trimmed = value.trim();
  if (trimmed.startsWith('color(')) return parseColorFunction(trimmed);
  const hex = trimmed.replace(/^#/, '');
  if (!/^(?:[0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})$/i.test(hex)) return null;
  const expand = (part: string) =>
    part.length === 1 ? Number.parseInt(part + part, 16) : Number.parseInt(part, 16);
  if (hex.length <= 4) {
    return {
      r: expand(hex[0]!),
      g: expand(hex[1]!),
      b: expand(hex[2]!),
      a: hex.length === 4 ? expand(hex[3]!) / 255 : 1,
    };
  }
  return {
    r: Number.parseInt(hex.slice(0, 2), 16),
    g: Number.parseInt(hex.slice(2, 4), 16),
    b: Number.parseInt(hex.slice(4, 6), 16),
    a: hex.length === 8 ? Number.parseInt(hex.slice(6, 8), 16) / 255 : 1,
  };
}

function toHex(color: Rgb): HexColor {
  const channel = (value: number) =>
    Math.max(0, Math.min(255, Math.round(value))).toString(16).padStart(2, '0');
  return normalizeHex(`#${channel(color.r)}${channel(color.g)}${channel(color.b)}`) as HexColor;
}

function flattenOver(color: Rgba, base: Rgb): HexColor {
  if (color.a >= 1) return toHex(color);
  return toHex({
    r: color.r * color.a + base.r * (1 - color.a),
    g: color.g * color.a + base.g * (1 - color.a),
    b: color.b * color.a + base.b * (1 - color.a),
  });
}

function relativeLuminance(color: Rgb): number {
  const channel = (value: number) => {
    const ratio = value / 255;
    return ratio <= 0.03928 ? ratio / 12.92 : ((ratio + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(color.r) + 0.7152 * channel(color.g) + 0.0722 * channel(color.b);
}

function contrastRatio(first: Rgb, second: Rgb): number {
  const a = relativeLuminance(first);
  const b = relativeLuminance(second);
  return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
}

function hexToRgb(value: string): Rgb {
  const parsed = parseVsCodeColor(value);
  return parsed ?? { r: 0, g: 0, b: 0 };
}

export function isVsCodeThemeFile(value: unknown): boolean {
  if (!isRecord(value)) return false;
  const hasWorkbenchColors =
    isRecord(value.colors) && Object.keys(value.colors).some((key) => key.includes('.'));
  return hasWorkbenchColors || Array.isArray(value.tokenColors);
}

function resolveAppearance(value: Record<string, unknown>, canvas: Rgb): ResolvedAppearance {
  const type = typeof value.type === 'string' ? value.type.toLowerCase() : null;
  if (type === 'light' || type === 'hc-light') return 'light';
  if (type === 'dark' || type === 'hc-black') return 'dark';
  return relativeLuminance(canvas) < 0.179 ? 'dark' : 'light';
}

export function humanizeThemeName(raw: string): string {
  const trimmed = raw.trim();
  if (/\s/.test(trimmed) || !/[-_.]/.test(trimmed)) return trimmed;
  return trimmed
    .split(/[-_.]+/)
    .filter(Boolean)
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(' ');
}

function resolveName(value: Record<string, unknown>): string {
  for (const candidate of [value.displayName, value.name]) {
    if (typeof candidate !== 'string') continue;
    const humanized = humanizeThemeName(candidate).trim();
    if (humanized) return humanized.slice(0, 64);
  }
  return 'VS Code theme';
}

export function parseVsCodeThemeFile(
  value: unknown,
  metadata: { sourceName?: string; sourcePath?: string } = {},
): ConvertedVsCodeTheme {
  if (!isRecord(value)) throw new Error('Theme files must contain a JSON object.');
  const colors = isRecord(value.colors) ? value.colors : {};
  const pick = (...keys: readonly string[]): Rgba | null => {
    for (const key of keys) {
      const parsed = parseVsCodeColor(colors[key]);
      if (parsed) return parsed;
    }
    return null;
  };
  const solidOver = (base: Rgb, ...keys: readonly string[]): HexColor | null => {
    const parsed = pick(...keys);
    return parsed ? flattenOver(parsed, base) : null;
  };

  const canvasColor = pick('editor.background', 'editorPane.background');
  if (!canvasColor) {
    throw new Error('That VS Code theme has no "editor.background" color.');
  }
  const canvas = { r: canvasColor.r, g: canvasColor.g, b: canvasColor.b };
  const canvasHex = toHex(canvas);
  const appearance = resolveAppearance(value, canvas);
  const accentColor = pick(
    'focusBorder',
    'button.background',
    'textLink.foreground',
    'activityBarBadge.background',
    'progressBar.background',
    'badge.background',
  );
  const accentHex = accentColor ? flattenOver(accentColor, canvas) : canvasHex;
  const foregroundCandidate = solidOver(canvas, 'editor.foreground', 'foreground');
  const foregroundHex = foregroundCandidate
    && contrastRatio(hexToRgb(foregroundCandidate), canvas) >= 4.5
    ? foregroundCandidate
    : relativeLuminance(canvas) < 0.179 ? '#ffffff' : '#000000';
  const seed: ThemeConfigV1 = {
    version: 1,
    presetId: 'custom',
    background: canvasHex,
    foreground: foregroundHex,
    accent: accentHex,
  };
  const derived = resolveTheme(seed, appearance).tokens;
  const readableOn = (
    surface: string,
    fallback: HexColor,
    ...keys: readonly string[]
  ): HexColor => {
    const surfaceRgb = hexToRgb(surface);
    const specified = solidOver(surfaceRgb, ...keys);
    if (specified && contrastRatio(hexToRgb(specified), surfaceRgb) >= 4.5) return specified;
    if (contrastRatio(hexToRgb(fallback), surfaceRgb) >= 4.5) return fallback;
    return relativeLuminance(surfaceRgb) < 0.179 ? '#ffffff' : '#000000';
  };

  const sidebar = solidOver(canvas, 'sideBar.background', 'activityBar.background')
    ?? derived['surface-container-low'];
  const panel = solidOver(canvas, 'panel.background', 'editorWidget.background')
    ?? derived['surface-container'];
  const overlay = solidOver(canvas, 'menu.background', 'quickInput.background', 'dropdown.background')
    ?? derived['surface-container-high'];
  const action = solidOver(canvas, 'button.background') ?? accentHex;
  const overrides: Partial<Record<MurmurTokenName, HexColor>> = {
    ...derived,
    background: canvasHex,
    surface: solidOver(canvas, 'editorPane.background') ?? canvasHex,
    'surface-container-lowest': canvasHex,
    'surface-container-low': sidebar,
    'surface-container': panel,
    'surface-container-high': overlay,
    'surface-container-highest': solidOver(canvas, 'list.activeSelectionBackground')
      ?? derived['surface-container-highest'],
    primary: accentHex,
    'primary-dim': action,
    'on-primary': readableOn(action, derived['on-primary'], 'button.foreground'),
    'on-surface': readableOn(canvasHex, derived['on-surface'], 'editor.foreground', 'foreground'),
    'on-surface-variant': readableOn(
      sidebar,
      derived['on-surface-variant'],
      'sideBar.foreground',
      'descriptionForeground',
      'disabledForeground',
    ),
    'outline-variant': solidOver(canvas, 'panel.border', 'sideBar.border', 'contrastBorder')
      ?? derived['outline-variant'],
    error: readableOn(canvasHex, derived.error, 'editorError.foreground', 'errorForeground'),
    warning: readableOn(canvasHex, derived.warning, 'editorWarning.foreground'),
  };
  const theme = sanitizeTheme({
    ...seed,
    [appearance]: overrides,
  });
  return {
    label: resolveName(value),
    appearance,
    theme,
    ...metadata,
  };
}

function stripAppearance(label: string): string {
  return label.replace(/\b(?:light|dark)\b/gi, ' ').replace(/\s+/g, ' ').trim();
}

export function resolveVsCodeThemeLabelCollisions(
  themes: readonly ConvertedVsCodeTheme[],
): ConvertedVsCodeTheme[] {
  const counts = new Map<string, number>();
  for (const theme of themes) {
    const key = theme.label.toLocaleLowerCase();
    counts.set(key, (counts.get(key) ?? 0) + 1);
  }
  const seen = new Map<string, number>();
  return themes.map((theme) => {
    const key = theme.label.toLocaleLowerCase();
    if ((counts.get(key) ?? 0) < 2) return theme;
    const stem = theme.sourceName?.replace(/\.[^.]+$/, '');
    const fromFile = stem ? humanizeThemeName(stem).slice(0, 64) : theme.label;
    const occurrence = (seen.get(fromFile.toLocaleLowerCase()) ?? 0) + 1;
    seen.set(fromFile.toLocaleLowerCase(), occurrence);
    return {
      ...theme,
      label: occurrence === 1 ? fromFile : `${fromFile.slice(0, 60)} ${occurrence}`,
    };
  });
}

export function pairVsCodeThemes(
  input: readonly ConvertedVsCodeTheme[],
): PairedVsCodeTheme[] {
  const themes = resolveVsCodeThemeLabelCollisions(input);
  type Group = { light: ConvertedVsCodeTheme[]; dark: ConvertedVsCodeTheme[]; order: number };
  const groups = new Map<string, Group>();
  const passthrough: Array<{ theme: ConvertedVsCodeTheme; order: number }> = [];
  themes.forEach((theme, order) => {
    const key = stripAppearance(theme.label);
    if (!key || key === theme.label) {
      passthrough.push({ theme, order });
      return;
    }
    const group = groups.get(key) ?? { light: [], dark: [], order };
    group[theme.appearance].push(theme);
    groups.set(key, group);
  });
  const output: Array<{ theme: PairedVsCodeTheme; order: number }> = passthrough.map(
    ({ theme, order }) => ({
      order,
      theme: {
        label: theme.label,
        modes: [theme.appearance],
        theme: theme.theme,
        sourceIdentities: [theme.sourcePath ?? theme.sourceName ?? `${order}`],
      },
    }),
  );
  for (const [label, group] of groups) {
    if (group.light.length === 1 && group.dark.length === 1) {
      const light = group.light[0]!;
      const dark = group.dark[0]!;
      output.push({
        order: group.order,
        theme: {
          label,
          modes: ['light', 'dark'],
          theme: sanitizeTheme({
            version: 1,
            presetId: 'custom',
            light: resolveTheme(light.theme, 'light').tokens,
            dark: resolveTheme(dark.theme, 'dark').tokens,
          }),
          sourceIdentities: [
            light.sourcePath ?? light.sourceName ?? `${group.order}:light`,
            dark.sourcePath ?? dark.sourceName ?? `${group.order}:dark`,
          ],
        },
      });
    } else {
      for (const theme of [...group.light, ...group.dark]) {
        output.push({
          order: group.order,
          theme: {
            label: theme.label,
            modes: [theme.appearance],
            theme: theme.theme,
            sourceIdentities: [theme.sourcePath ?? theme.sourceName ?? `${group.order}`],
          },
        });
      }
    }
  }
  return output.sort((a, b) => a.order - b.order).map(({ theme }) => theme);
}
