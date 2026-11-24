// The floating pill's Live Words wheel: words wrap into lines, the line being
// spoken sits in front and finished lines roll up and back over a cylinder,
// like a picker wheel. These are the approved settings from the mockup
// (curvature 27°, older line blur 1.1px, "medium" roll speed).

/** Height of one line of words (px). The cylinder's radius follows from it. */
export const WHEEL_LINE_H = 20;
/** The front line sits this far below the wheel's centre, so the previous
 * line has room above it. */
export const WHEEL_FRONT_SHIFT = 6;
/** Rotation between neighbouring lines on the cylinder. */
export const WHEEL_STEP_DEG = 27;
/** Blur added per line of distance from the front line (px). */
export const WHEEL_BLUR_PER_LINE = 1.1;
/** Spring stiffness of the roll (1/s²), critically damped. */
export const WHEEL_SPRING_K = 90;
/** Lines kept in the DOM: the front line, two rolled back and one spare. */
export const WHEEL_RENDERED_LINES = 4;
/** Lines fade out between these distances from the front and are hidden
 * beyond: at rest only the front line and the one above it are drawn. */
const WHEEL_FADE_FROM = 1.5;
const WHEEL_VISIBLE_RANGE = 2;
/** Slack kept at the right edge of a line (px). */
const LINE_SLACK = 4;

/** The words of one line, in order. */
export type WheelLine = string[];

/** Greedy word wrap. It is prefix-stable: appending words never moves a word
 * that is already placed, so a new line only ever arrives at the bottom. A
 * word wider than a whole line (one very long word, unspaced CJK text) is cut
 * between code points, never inside a surrogate pair. */
export function wrapWheelLines(
  text: string,
  measure: (s: string) => number,
  maxWidth: number,
): WheelLine[] {
  const words = text.replace(/\s+/g, ' ').trim().split(' ').filter(Boolean);
  const lines: WheelLine[] = [];
  if (!words.length) return lines;
  const max = Math.max(1, maxWidth - LINE_SLACK);
  const space = measure(' ');
  let line: WheelLine = [];
  let width = 0;
  const newLine = () => { if (line.length) lines.push(line); line = []; width = 0; };
  for (const word of words) {
    const w = measure(word);
    if (w <= max) {
      if (line.length && width + space + w > max) newLine();
      width += (line.length ? space : 0) + w;
      line.push(word);
      continue;
    }
    // Too wide for any line: it starts its own line and fills whole lines.
    newLine();
    let piece = '';
    for (const point of Array.from(word)) {
      if (piece && measure(piece + point) > max) {
        line.push(piece);
        newLine();
        piece = '';
      }
      piece += point;
    }
    line.push(piece);
    width = measure(piece);
  }
  newLine();
  return lines;
}

/** The pose of the line `offset` lines from the wheel's position (negative:
 * older, rolled up and back; positive: incoming from below). */
export interface WheelPose { visible: boolean; transform: string; opacity: number; blur: number }

export function wheelPose(offset: number): WheelPose {
  const distance = Math.abs(offset);
  if (distance >= WHEEL_VISIBLE_RANGE) return { visible: false, transform: 'none', opacity: 0, blur: 0 };
  const step = WHEEL_STEP_DEG * Math.PI / 180;
  const radius = WHEEL_LINE_H / step;
  const angle = -offset * step;
  const c = Math.max(0, Math.cos(Math.min(Math.abs(angle), Math.PI / 2)));
  const edge = Math.min(1, (WHEEL_VISIBLE_RANGE - distance) / (WHEEL_VISIBLE_RANGE - WHEEL_FADE_FROM));
  return {
    visible: true,
    transform: `translateY(${WHEEL_FRONT_SHIFT}px) translateZ(${-radius.toFixed(3)}px) `
      + `rotateX(${angle.toFixed(4)}rad) translateZ(${radius.toFixed(3)}px)`,
    opacity: (offset < 0 ? Math.pow(c, 5) * 0.9 : Math.pow(c, 3)) * edge,
    blur: distance * WHEEL_BLUR_PER_LINE,
  };
}

/** One step of the critically damped spring toward `target`. */
export function stepWheelSpring(pos: number, vel: number, target: number, dt: number): [number, number] {
  const k = WHEEL_SPRING_K;
  const nextVel = vel + (k * (target - pos) - 2 * Math.sqrt(k) * vel) * dt;
  return [pos + nextVel * dt, nextVel];
}
