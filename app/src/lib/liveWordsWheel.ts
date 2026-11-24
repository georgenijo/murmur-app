// The floating pill's Live Words wheel: words wrap into lines, the line being
// spoken sits in front and each finished line rolls up and back out of the
// top of the slim 34pt pill, like a picker wheel turning; the next line fades
// in where it left. Curvature 27°,
// older line blur 1.1px and the "medium" roll speed are the approved settings.

/** Height of one line of words (px). The cylinder's radius follows from it. */
export const WHEEL_LINE_H = 16;
/** Rotation between neighbouring lines on the cylinder. */
export const WHEEL_STEP_DEG = 27;
/** Blur added per line of distance from the front line (px). */
export const WHEEL_BLUR_PER_LINE = 1.1;
/** Spring stiffness of the roll (1/s²), critically damped. */
export const WHEEL_SPRING_K = 90;
/** Lines kept in the DOM: the front line, the one rolling away and a spare
 * for fast speech. */
export const WHEEL_RENDERED_LINES = 3;
/** An outgoing line is fully gone one line behind the front: at rest only the
 * front line is drawn, and the previous one shows only while it rolls away. */
const WHEEL_OUTGOING_RANGE = 1;
/** A new line does not roll in from below (two lines would crowd the 34pt
 * pill): it waits in the front position and fades in once the outgoing line
 * is half way out of the top. */
const WHEEL_INCOMING_RANGE = 0.5;
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
 * older, rolling up and back; positive: the next line, waiting in front). */
export interface WheelPose { visible: boolean; transform: string; opacity: number; blur: number }

export function wheelPose(offset: number): WheelPose {
  const distance = Math.abs(offset);
  if (offset > 0) {
    if (distance >= WHEEL_INCOMING_RANGE) return { visible: false, transform: 'none', opacity: 0, blur: 0 };
    return { visible: true, transform: 'none', opacity: 1 - distance / WHEEL_INCOMING_RANGE, blur: 0 };
  }
  if (distance >= WHEEL_OUTGOING_RANGE) return { visible: false, transform: 'none', opacity: 0, blur: 0 };
  const step = WHEEL_STEP_DEG * Math.PI / 180;
  const radius = WHEEL_LINE_H / step;
  const angle = -offset * step;
  // The outgoing line dims with its angle and fades linearly to nothing one
  // line back, so it is only glimpsed while it rolls away.
  return {
    visible: true,
    transform: `translateZ(${-radius.toFixed(3)}px) rotateX(${angle.toFixed(4)}rad) translateZ(${radius.toFixed(3)}px)`,
    opacity: Math.pow(Math.cos(angle), 5) * (1 - distance / WHEEL_OUTGOING_RANGE),
    blur: distance * WHEEL_BLUR_PER_LINE,
  };
}

/** One step of the critically damped spring toward `target`. */
export function stepWheelSpring(pos: number, vel: number, target: number, dt: number): [number, number] {
  const k = WHEEL_SPRING_K;
  const nextVel = vel + (k * (target - pos) - 2 * Math.sqrt(k) * vel) * dt;
  return [pos + nextVel * dt, nextVel];
}
