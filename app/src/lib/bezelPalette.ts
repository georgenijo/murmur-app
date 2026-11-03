// Bezel: the floating pill's macOS HUD material and its monochrome ink.
//
// The material is native (`overlay_material.rs`): a behind-window blur under a
// tint that is 74–76% opaque, so whatever the blur passes through, 11pt labels
// keep at least 7:1. These values mirror `DARK_TINT`/`LIGHT_TINT` there and the
// `.bezel` custom properties in `styles/overlay.css`; the contrast tests lock
// all three. Without the native material (or under Reduce Transparency) the
// same tone is drawn fully opaque.

type Rgb = readonly [number, number, number];

export interface BezelTone {
  /** Material tint, sRGB 0–255, and its opacity over the blur. */
  material: Rgb;
  materialAlpha: number;
  /** Primary ink for labels, glyphs and waveform bars. */
  ink: Rgb;
  inkAlpha: number;
  /** Opacity of not-yet-final Live Words, relative to the ink. */
  tentativeAlpha: number;
}

export const BEZEL_PALETTE: Record<'dark' | 'light', BezelTone> = {
  // Solid white: 92% white falls to about 6.4:1 over a white page.
  dark: { material: [28, 28, 30], materialAlpha: 0.74, ink: [255, 255, 255], inkAlpha: 1, tentativeAlpha: 0.74 },
  light: { material: [246, 246, 248], materialAlpha: 0.76, ink: [0, 0, 0], inkAlpha: 0.87, tentativeAlpha: 0.74 },
};

function over(top: Rgb, alpha: number, bottom: Rgb): Rgb {
  return [0, 1, 2].map(i => top[i] * alpha + bottom[i] * (1 - alpha)) as unknown as Rgb;
}

function luminance([r, g, b]: Rgb): number {
  const lin = (v: number) => {
    const c = v / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
}

export function contrastRatio(a: Rgb, b: Rgb): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

/** Worst-case contrast of ink at `alpha` against the material over a pure
 * white or pure black backdrop (the blur can pass through either). */
export function worstInkContrast(tone: BezelTone, alpha = tone.inkAlpha): number {
  return Math.min(...([[255, 255, 255], [0, 0, 0]] as const).map(backdrop => {
    const material = over(tone.material, tone.materialAlpha, backdrop);
    return contrastRatio(over(tone.ink, alpha, material), material);
  }));
}

/** Contrast of ink at `alpha` against the material over one backdrop. */
export function inkContrastOver(tone: BezelTone, backdrop: Rgb, alpha = tone.inkAlpha): number {
  const material = over(tone.material, tone.materialAlpha, backdrop);
  return contrastRatio(over(tone.ink, alpha, material), material);
}
