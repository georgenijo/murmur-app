import { describe, expect, it } from 'vitest';
import {
  WHEEL_BLUR_PER_LINE, WHEEL_COMMIT_MS, advanceWheel, newWheelCommitState, stepWheelSpring,
  wheelLineWidth, wheelPose, wheelTokens,
} from './liveWordsWheel';

// 10px per code point keeps the arithmetic readable.
const measure = (s: string) => Array.from(s).length * 10;
// 84px usable on a line (88 - 4 slack): "aaa bbb" is 70, adding " ccc" makes 110.
const MAX = 88;
const lay = (state: ReturnType<typeof newWheelCommitState>, text: string, now: number) =>
  advanceWheel(state, wheelTokens(text, measure, MAX), now, measure, MAX);

describe('wheelTokens', () => {
  it('splits words and collapses whitespace', () => {
    expect(wheelTokens('  aaa \n bbb   ccc ', measure, MAX)).toEqual(['aaa', 'bbb', 'ccc']);
    expect(wheelTokens('   ', measure, MAX)).toEqual([]);
  });

  it('cuts a word wider than a line, and unspaced CJK, between code points', () => {
    expect(wheelTokens(`hi ${'x'.repeat(25)}`, measure, 104)).toEqual(['hi', 'x'.repeat(10), 'x'.repeat(10), 'xxxxx']);
    const cjk = '今日は天気がとても良いので公園まで散歩に行きたい';
    const cut = wheelTokens(cjk, measure, 64);
    expect(cut.join('')).toBe(cjk);
    for (const piece of cut) expect(measure(piece)).toBeLessThanOrEqual(60);
    const emoji = '😀'.repeat(9);
    const pieces = wheelTokens(emoji, measure, 34);
    expect(pieces.join('')).toBe(emoji);
    expect(pieces.every((p) => !/[\uD800-\uDBFF]$/.test(p))).toBe(true);
  });
});

describe('advanceWheel', () => {
  it('holds a word that does not fit until it has existed for the commit time', () => {
    const state = newWheelCommitState();
    expect(lay(state, 'aaa bbb', 0).lines).toEqual([['aaa', 'bbb']]);
    const held = lay(state, 'aaa bbb ccc', 100);
    expect(held).toMatchObject({ lines: [['aaa', 'bbb']], held: 1, recheckInMs: WHEEL_COMMIT_MS });
    expect(lay(state, 'aaa bbb ccc', 100 + WHEEL_COMMIT_MS - 1).lines).toEqual([['aaa', 'bbb']]);
    const rolled = lay(state, 'aaa bbb ccc', 100 + WHEEL_COMMIT_MS);
    expect(rolled).toMatchObject({ lines: [['aaa', 'bbb'], ['ccc']], held: 0, recheckInMs: null });
  });

  it('keeps a word revised in place at its age', () => {
    const state = newWheelCommitState();
    lay(state, 'aaa bbb ccc', 0);
    // The tail is re-guessed (same index), so it rolls on its first age.
    expect(lay(state, 'aaa bbb cee', WHEEL_COMMIT_MS).lines).toEqual([['aaa', 'bbb'], ['cee']]);
  });

  it('a partial re-guess around a line break never moves the wheel backwards', () => {
    const state = newWheelCommitState();
    // The model alternately adds and drops a tail word across the edge while
    // the speaker pauses, faster than the commit time: nothing rolls.
    let now = 0;
    for (let i = 0; i < 12; i++, now += 520) {
      const view = lay(state, i % 2 ? 'aaa bbb you' : 'aaa bbb', now);
      expect(view.lines).toEqual([['aaa', 'bbb']]);
    }
    // Once committed, dropping the word that started the line, or every
    // word, keeps the line count and the finished line.
    let count = lay(state, 'aaa bbb ccc', now).lines.length;
    now += WHEEL_COMMIT_MS;
    const sequence = ['aaa bbb ccc', 'aaa bbb', 'aaa bbb cc', 'aaa', '', 'aaa bbb ccc ddd eee fff', 'aaa bbb ccc'];
    for (const text of sequence) {
      const view = lay(state, text, now);
      expect(view.lines.length).toBeGreaterThanOrEqual(count);
      expect(view.lines[0]).toEqual(text.startsWith('aaa bbb') ? ['aaa', 'bbb'] : view.lines[0]);
      count = view.lines.length;
      now += 300;
    }
  });

  it('lets a word already on the front line run a few px past the edge', () => {
    // 'aaa bbbb' is 80 of the 84 usable. Re-guessed 7px wider (87) it is
    // past the edge but inside the 4px hysteresis, so it keeps its place; a
    // cold layout of the same words holds the wider word back.
    const wide = (s: string) => (s === 'bbbb+' ? 43 + 4 : measure(s));
    const warm = newWheelCommitState();
    advanceWheel(warm, ['aaa', 'bbbb'], 0, wide, MAX);
    expect(advanceWheel(warm, ['aaa', 'bbbb+'], 10, wide, MAX)).toMatchObject({ lines: [['aaa', 'bbbb+']], held: 0 });
    expect(advanceWheel(newWheelCommitState(), ['aaa', 'bbbb+'], 10, wide, MAX).held).toBe(1);
  });

  it('measures a line with one space between words', () => {
    expect(wheelLineWidth(['aaa', 'bb'], measure)).toBe(60);
    expect(wheelLineWidth([], measure)).toBe(0);
  });
});

describe('wheelPose', () => {
  it('shows the front line flat and centred', () => {
    const front = wheelPose(0);
    expect(front).toMatchObject({ visible: true, opacity: 1, blur: 0 });
    expect(front.transform).toContain('rotateX(0.0000rad)');
    expect(front.transform).not.toContain('translateY');
  });

  it('rolls an outgoing line up and back, gone one line behind the front', () => {
    const half = wheelPose(-0.5);
    expect(half.transform).toContain('rotateX(0.2356rad)'); // 13.5°, half of 27°
    expect(half.opacity).toBeCloseTo(Math.pow(Math.cos(13.5 * Math.PI / 180), 5) * 0.5, 5);
    expect(half.blur).toBeCloseTo(WHEEL_BLUR_PER_LINE / 2);
    // It fades continuously from the front line's full opacity to nothing.
    expect(wheelPose(-0.001).opacity).toBeGreaterThan(0.99);
    expect(wheelPose(-0.99).opacity).toBeLessThan(0.01);
    // At rest the previous line is not drawn at all.
    expect(wheelPose(-1).visible).toBe(false);
  });

  it('fades the next line in at the front, without rolling it up from below', () => {
    expect(wheelPose(0.25)).toMatchObject({ visible: true, transform: 'none', opacity: 0.5, blur: 0 });
    expect(wheelPose(0.5).visible).toBe(false);
    expect(wheelPose(1).visible).toBe(false);
  });
});

describe('stepWheelSpring', () => {
  it('settles on the target without overshooting (critically damped)', () => {
    let pos = 0, vel = 0, max = 0;
    for (let i = 0; i < 120; i++) {
      [pos, vel] = stepWheelSpring(pos, vel, 1, 1 / 60);
      max = Math.max(max, pos);
    }
    expect(pos).toBeCloseTo(1, 3);
    expect(max).toBeLessThanOrEqual(1.0001);
  });
});
