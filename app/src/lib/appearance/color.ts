import type { HexColor } from './types';

export interface Oklab {
  l: number;
  a: number;
  b: number;
}

export interface Oklch {
  l: number;
  c: number;
  h: number;
}

const HEX_COLOR = /^#[0-9a-fA-F]{6}$/;

export function isHexColor(value: unknown): value is HexColor {
  return typeof value === 'string' && HEX_COLOR.test(value);
}

export function normalizeHex(value: string): HexColor {
  return value.toLowerCase() as HexColor;
}

function srgbToLinear(value: number): number {
  const channel = value / 255;
  return channel <= 0.04045
    ? channel / 12.92
    : ((channel + 0.055) / 1.055) ** 2.4;
}

function linearToSrgb(value: number): number {
  const channel = value <= 0.0031308
    ? 12.92 * value
    : 1.055 * value ** (1 / 2.4) - 0.055;
  return Math.round(Math.min(1, Math.max(0, channel)) * 255);
}

export function hexToOklab(hex: HexColor): Oklab {
  const red = srgbToLinear(parseInt(hex.slice(1, 3), 16));
  const green = srgbToLinear(parseInt(hex.slice(3, 5), 16));
  const blue = srgbToLinear(parseInt(hex.slice(5, 7), 16));

  const l = 0.4122214708 * red + 0.5363325363 * green + 0.0514459929 * blue;
  const m = 0.2119034982 * red + 0.6806995451 * green + 0.1073969566 * blue;
  const s = 0.0883024619 * red + 0.2817188376 * green + 0.6299787005 * blue;
  const lRoot = Math.cbrt(l);
  const mRoot = Math.cbrt(m);
  const sRoot = Math.cbrt(s);

  return {
    l: 0.2104542553 * lRoot + 0.793617785 * mRoot - 0.0040720468 * sRoot,
    a: 1.9779984951 * lRoot - 2.428592205 * mRoot + 0.4505937099 * sRoot,
    b: 0.0259040371 * lRoot + 0.7827717662 * mRoot - 0.808675766 * sRoot,
  };
}

function linearRgb(lab: Oklab): [number, number, number] {
  const lRoot = lab.l + 0.3963377774 * lab.a + 0.2158037573 * lab.b;
  const mRoot = lab.l - 0.1055613458 * lab.a - 0.0638541728 * lab.b;
  const sRoot = lab.l - 0.0894841775 * lab.a - 1.291485548 * lab.b;
  const l = lRoot ** 3;
  const m = mRoot ** 3;
  const s = sRoot ** 3;
  return [
    4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
  ];
}

export function isOklabInGamut(lab: Oklab): boolean {
  return linearRgb(lab).every((channel) => channel >= -0.000001 && channel <= 1.000001);
}

export function oklabToHex(lab: Oklab): HexColor {
  const [red, green, blue] = linearRgb(lab).map(linearToSrgb);
  return `#${red.toString(16).padStart(2, '0')}${green.toString(16).padStart(2, '0')}${blue.toString(16).padStart(2, '0')}` as HexColor;
}

export function oklabToOklch(lab: Oklab): Oklch {
  const chroma = Math.sqrt(lab.a ** 2 + lab.b ** 2);
  let hue = Math.atan2(lab.b, lab.a) * 180 / Math.PI;
  if (hue < 0) hue += 360;
  return { l: lab.l, c: chroma, h: Number.isFinite(hue) ? hue : 0 };
}

export function oklchToOklab(lch: Oklch): Oklab {
  const radians = lch.h * Math.PI / 180;
  return {
    l: lch.l,
    a: lch.c * Math.cos(radians),
    b: lch.c * Math.sin(radians),
  };
}

export function oklchToHexInGamut(lch: Oklch): { color: HexColor; clipped: boolean } {
  let candidate = oklchToOklab(lch);
  if (isOklabInGamut(candidate)) return { color: oklabToHex(candidate), clipped: false };
  let low = 0;
  let high = Math.max(0, lch.c);
  for (let index = 0; index < 24; index += 1) {
    const chroma = (low + high) / 2;
    candidate = oklchToOklab({ ...lch, c: chroma });
    if (isOklabInGamut(candidate)) low = chroma;
    else high = chroma;
  }
  return {
    color: oklabToHex(oklchToOklab({ ...lch, c: low })),
    clipped: true,
  };
}

export function mixOklab(from: HexColor, to: HexColor, amount: number): HexColor {
  const left = hexToOklab(from);
  const right = hexToOklab(to);
  const t = Math.min(1, Math.max(0, amount));
  return oklabToHex({
    l: left.l + (right.l - left.l) * t,
    a: left.a + (right.a - left.a) * t,
    b: left.b + (right.b - left.b) * t,
  });
}

export function relativeLuminance(hex: HexColor): number {
  const red = srgbToLinear(parseInt(hex.slice(1, 3), 16));
  const green = srgbToLinear(parseInt(hex.slice(3, 5), 16));
  const blue = srgbToLinear(parseInt(hex.slice(5, 7), 16));
  return 0.2126 * red + 0.7152 * green + 0.0722 * blue;
}

export function contrastRatio(foreground: HexColor, background: HexColor): number {
  const foregroundLuminance = relativeLuminance(foreground);
  const backgroundLuminance = relativeLuminance(background);
  return (
    (Math.max(foregroundLuminance, backgroundLuminance) + 0.05)
    / (Math.min(foregroundLuminance, backgroundLuminance) + 0.05)
  );
}

export function compositeSrgb(
  foreground: HexColor,
  background: HexColor,
  alpha: number,
): HexColor {
  const opacity = Math.min(1, Math.max(0, alpha));
  const channel = (color: HexColor, start: number) => parseInt(color.slice(start, start + 2), 16);
  const output = [1, 3, 5].map((start) =>
    Math.round(channel(foreground, start) * opacity + channel(background, start) * (1 - opacity)),
  );
  return `#${output.map((value) => value.toString(16).padStart(2, '0')).join('')}` as HexColor;
}

export function ensureContrast(
  source: HexColor,
  backgrounds: readonly HexColor[],
  minimum: number,
): HexColor {
  if (backgrounds.every((background) => contrastRatio(source, background) >= minimum)) {
    return source;
  }

  const original = oklabToOklch(hexToOklab(source));
  let best: { color: HexColor; distance: number } | null = null;
  for (let step = 0; step <= 200; step += 1) {
    const lightness = step / 200;
    const candidate = oklchToHexInGamut({ ...original, l: lightness }).color;
    if (!backgrounds.every((background) => contrastRatio(candidate, background) >= minimum)) {
      continue;
    }
    const distance = Math.abs(lightness - original.l);
    if (!best || distance < best.distance) best = { color: candidate, distance };
  }

  if (best) return best.color;
  const black = '#000000' as HexColor;
  const white = '#ffffff' as HexColor;
  const blackScore = Math.min(...backgrounds.map((background) => contrastRatio(black, background)));
  const whiteScore = Math.min(...backgrounds.map((background) => contrastRatio(white, background)));
  return blackScore >= whiteScore ? black : white;
}
