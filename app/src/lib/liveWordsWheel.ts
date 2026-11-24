// The floating pill's Live Words wheel: words wrap into lines, the line being
// spoken sits in front and each finished line rolls up and back out of the
// top of the slim 34pt pill, like a picker wheel turning; the next line fades
// in where it left. Curvature 27°, older line blur 1.1px and the "medium"
// roll speed are the approved settings. The pill's width hugs the front line
// (see overlayPillFit.ts).

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
/** A word that would start a new line only does so once it has existed this
 * long. While paused the model keeps re-guessing its last word; a guess that
 * crosses the line edge and is dropped again never rolls the wheel. */
export const WHEEL_COMMIT_MS = 700;
/** A word already shown on the front line keeps its place until it is this
 * far past the edge (px), so a re-guess a pixel wider does not flip it. */
export const WHEEL_EDGE_HYSTERESIS = 4;
/** Slack kept at the right edge of a line (px), so a held word's hysteresis
 * never takes the text past the line's width. */
const LINE_SLACK = WHEEL_EDGE_HYSTERESIS;

/** The words of one line, in order. */
export type WheelLine = string[];

/** The partial transcript as wheel tokens: its words, with any word wider
 * than a whole line (one very long word, unspaced CJK text) cut between code
 * points, never inside a surrogate pair. Cutting is prefix-stable: a word
 * that grows keeps its earlier pieces. */
export function wheelTokens(text: string, measure: (s: string) => number, maxWidth: number): string[] {
  const max = Math.max(1, maxWidth - LINE_SLACK);
  const tokens: string[] = [];
  for (const word of text.replace(/\s+/g, ' ').trim().split(' ')) {
    if (!word) continue;
    if (measure(word) <= max) { tokens.push(word); continue; }
    let piece = '';
    for (const point of Array.from(word)) {
      if (piece && measure(piece + point) > max) { tokens.push(piece); piece = ''; }
      piece += point;
    }
    if (piece) tokens.push(piece);
  }
  return tokens;
}

/** What the wheel has committed for one dictation. Kept across partials. */
export interface WheelCommitState {
  /** End token index (exclusive) of each finished line. Only ever grows. */
  done: number[];
  /** When each token index first appeared (ms). A word revised in place keeps
   * its age. */
  born: number[];
  /** Tokens the front line showed last time, for the edge hysteresis. */
  frontCount: number;
}

export function newWheelCommitState(): WheelCommitState {
  return { done: [], born: [], frontCount: 0 };
}

export interface WheelView {
  /** Finished lines, then the front line. Never fewer lines than before. */
  lines: WheelLine[];
  /** Words held back from the front line until they are old enough to start
   * a new line (they do not fit yet). */
  held: number;
  /** Milliseconds until a held word may start a new line, or null. */
  recheckInMs: number | null;
}

/** Lay the tokens out on the wheel. Finished lines are committed and never
 * roll back. A word that does not fit on the front line is held back, and
 * only starts a new line once it has existed for WHEEL_COMMIT_MS, so a pause
 * spent re-guessing the last word never moves the wheel backwards. */
export function advanceWheel(
  state: WheelCommitState,
  tokens: string[],
  now: number,
  measure: (s: string) => number,
  maxWidth: number,
): WheelView {
  state.born.length = Math.min(state.born.length, tokens.length);
  for (let i = state.born.length; i < tokens.length; i++) state.born[i] = now;
  const max = Math.max(1, maxWidth - LINE_SLACK);
  const space = measure(' ');
  for (;;) {
    const start = state.done.length ? state.done[state.done.length - 1] : 0;
    let width = 0;
    let k = 0;
    for (let i = start; i < tokens.length; i++) {
      const w = measure(tokens[i]);
      const edge = k < state.frontCount ? max + WHEEL_EDGE_HYSTERESIS : max;
      if (k > 0 && width + space + w > edge) break;
      width += (k > 0 ? space : 0) + w;
      k++;
    }
    const firstHeld = start + k;
    if (firstHeld < tokens.length && k > 0 && now - state.born[firstHeld] >= WHEEL_COMMIT_MS) {
      state.done.push(firstHeld);
      state.frontCount = 0;
      continue;
    }
    state.frontCount = k;
    const lines: WheelLine[] = [];
    let from = 0;
    for (const end of state.done) { lines.push(tokens.slice(from, end)); from = end; }
    lines.push(tokens.slice(start, firstHeld));
    const held = Math.max(0, tokens.length - firstHeld);
    return {
      lines,
      held,
      recheckInMs: held ? Math.max(0, WHEEL_COMMIT_MS - (now - state.born[firstHeld])) : null,
    };
  }
}

/** The width (px) of one line as the wheel renders it: words with one space
 * between them. */
export function wheelLineWidth(line: WheelLine, measure: (s: string) => number): number {
  if (!line.length) return 0;
  return line.reduce((sum, word) => sum + measure(word), 0) + measure(' ') * (line.length - 1);
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
