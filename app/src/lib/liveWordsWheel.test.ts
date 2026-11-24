import { describe, expect, it } from 'vitest';
import {
  WHEEL_BLUR_PER_LINE, stepWheelSpring, wheelPose, wrapWheelLines,
} from './liveWordsWheel';

// 10px per code point keeps the arithmetic readable.
const measure = (s: string) => Array.from(s).length * 10;

describe('wrapWheelLines', () => {
  it('wraps greedily and collapses whitespace', () => {
    // 84px usable (88 - 4 slack): "aaa bbb" is 70, adding " ccc" makes 110.
    expect(wrapWheelLines('  aaa \n bbb   ccc dd ', measure, 88)).toEqual([['aaa', 'bbb'], ['ccc', 'dd']]);
    expect(wrapWheelLines('   ', measure, 88)).toEqual([]);
  });

  it('is prefix-stable: appending words never moves a placed word', () => {
    const words = 'one two three four five six seven eight nine ten eleven twelve'.split(' ');
    let previous = wrapWheelLines('', measure, 124);
    for (let n = 1; n <= words.length; n++) {
      const next = wrapWheelLines(words.slice(0, n).join(' '), measure, 124);
      previous.slice(0, -1).forEach((line, i) => expect(next[i]).toEqual(line));
      const last = previous.length - 1;
      if (last >= 0) expect(next[last].slice(0, previous[last].length)).toEqual(previous[last]);
      previous = next;
    }
  });

  it('cuts a word wider than a line, and unspaced CJK, between code points', () => {
    const long = 'x'.repeat(25);
    const lines = wrapWheelLines(`hi ${long} yo`, measure, 104);
    expect(lines).toEqual([['hi'], ['x'.repeat(10)], ['x'.repeat(10)], ['xxxxx', 'yo']]);
    const cjk = '今日は天気がとても良いので公園まで散歩に行きたい';
    const cut = wrapWheelLines(cjk, measure, 64);
    expect(cut.flat().join('')).toBe(cjk);
    for (const line of cut) expect(measure(line.join(' '))).toBeLessThanOrEqual(60);
    const emoji = '😀'.repeat(9);
    const pieces = wrapWheelLines(emoji, measure, 34).flat();
    expect(pieces.join('')).toBe(emoji);
    expect(pieces.every((p) => !/[\uD800-\uDBFF]$/.test(p))).toBe(true);
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
