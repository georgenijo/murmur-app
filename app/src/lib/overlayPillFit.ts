// The floating pill with Live Words on hugs the line you are reading. Its
// native window stays at the `words` size (224×34pt) for the whole dictation,
// so following the words never resizes a window: only the capsule the webview
// draws and the native material behind it (`pill_w` on set_overlay_material)
// change width. Before any words, and in every non-recording state, it rests
// at the compact width.
import type { OverlayPillSize } from './overlayGeometry';

/** The compact pill (dot and the full waveform), `PILL_W` in overlay.rs. */
export const PILL_COMPACT_W = 120;
/** Bars of the pill's waveform: all twelve before words, four alongside them. */
export const PILL_WAVE_BARS_FULL = 12;
export const PILL_WAVE_BARS_MINI = 4;
const BAR_W = 2;
const BAR_GAP = 1.5;
export function waveWidth(bars: number): number {
  return bars * BAR_W + (bars - 1) * BAR_GAP;
}
/** Everything in the fitted pill except the words: the 1px hairline each
 * side, 10px row padding each side, the 12px dot, two 8px gaps and the
 * four-bar waveform (`.bezel-row` in overlay.css). */
export const PILL_FIT_CHROME = 2 + 2 * 10 + 12 + 8 + waveWidth(PILL_WAVE_BARS_MINI) + 8;
/** Rounding slack, so subpixel text never touches the clip. */
const FIT_SLACK = 1;

/** The widest front line the `words` pill can hold, for wrapping. */
export function pillWordsMaxText(pillW: number): number {
  return pillW - PILL_FIT_CHROME - FIT_SLACK;
}

/** The capsule width to draw, or undefined to keep the size's own width
 * (notch, Live Words off, a warning). `frontW` is the front line's text width,
 * null before any words. Whole points, so a sub-pixel re-guess does not move
 * it. */
export function overlayPillFitWidth(
  floating: boolean,
  pillSize: OverlayPillSize | undefined,
  pillW: number,
  recording: boolean,
  frontW: number | null,
): number | undefined {
  if (!floating || pillSize !== 'words') return undefined;
  if (!recording || frontW === null || frontW <= 0) return Math.min(pillW, PILL_COMPACT_W);
  return Math.min(pillW, Math.ceil(PILL_FIT_CHROME + frontW + FIT_SLACK));
}
