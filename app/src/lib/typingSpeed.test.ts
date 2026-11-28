import { describe, expect, it } from 'vitest';
import {
  TYPING_CHECK_PASSAGES,
  TYPING_SPEED_MAX,
  TYPING_SPEED_MIN,
  dictationSpeedup,
  effectiveTypingWpm,
  sanitizeTypingSpeedWpm,
  scoreTypingCheck,
} from './typingSpeed';
import { ASSUMED_TYPING_WPM, getActivityInsights } from './activityStats';
import { loadStats } from './stats';

const passage = TYPING_CHECK_PASSAGES[0];

describe('sanitizeTypingSpeedWpm', () => {
  it('treats junk, zero and negatives as not measured', () => {
    for (const value of [undefined, null, '60', NaN, Infinity, -Infinity, 0, -3, 0.4, true, {}]) {
      expect(sanitizeTypingSpeedWpm(value)).toBe(0);
    }
  });

  it('rounds and clamps measured speeds', () => {
    expect(sanitizeTypingSpeedWpm(61.6)).toBe(62);
    expect(sanitizeTypingSpeedWpm(1)).toBe(TYPING_SPEED_MIN);
    expect(sanitizeTypingSpeedWpm(9000)).toBe(TYPING_SPEED_MAX);
  });

  it('falls back to the assumed speed when nothing is measured', () => {
    expect(effectiveTypingWpm(0)).toBe(ASSUMED_TYPING_WPM);
    expect(effectiveTypingWpm(Number.NaN)).toBe(ASSUMED_TYPING_WPM);
    expect(effectiveTypingWpm(72)).toBe(72);
  });
});

describe('scoreTypingCheck', () => {
  it('scores net words per minute from correctly placed characters', () => {
    // The whole passage typed perfectly in 18 s.
    const result = scoreTypingCheck(passage, passage, 18_000);
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.accuracy).toBe(1);
    expect(result.correctChars).toBe(passage.length);
    expect(result.wpm).toBe(Math.round((passage.length / 5) / (18_000 / 60_000)));
  });

  it('counts only characters that match their position', () => {
    const typed = 'The quick brown fax jumps over the lazy dog';
    const result = scoreTypingCheck(passage, typed, 6000);
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.typedChars).toBe(typed.length);
    expect(result.correctChars).toBe(typed.length - 1);
    expect(result.wpm).toBe(Math.round(((typed.length - 1) / 5) / 0.1));
  });

  it('refuses runs that are too short in text or time', () => {
    expect(scoreTypingCheck(passage, 'The quick', 10_000)).toMatchObject({ ok: false, reason: 'too-short' });
    expect(scoreTypingCheck(passage, passage.slice(0, 39), 10_000)).toMatchObject({ ok: false, reason: 'too-short' });
    expect(scoreTypingCheck(passage, passage, 1999)).toMatchObject({ ok: false, reason: 'too-short' });
    expect(scoreTypingCheck(passage, passage, Number.NaN)).toMatchObject({ ok: false, reason: 'too-short' });
    expect(scoreTypingCheck(passage, '', 10_000)).toMatchObject({ ok: false, reason: 'too-short', accuracy: 0 });
  });

  it('refuses runs where most characters are wrong', () => {
    const result = scoreTypingCheck(passage, 'x'.repeat(40), 10_000);
    expect(result).toMatchObject({ ok: false, reason: 'too-many-typos' });
  });

  it('never reports a speed outside the stored range', () => {
    const fast = scoreTypingCheck(passage, passage, 2000);
    expect(fast.ok && fast.wpm).toBe(TYPING_SPEED_MAX);
    const slow = scoreTypingCheck(passage, passage.slice(0, 40), 10 * 60_000);
    expect(slow.ok && slow.wpm).toBe(TYPING_SPEED_MIN);
  });

  it('keeps every practice sentence plain ASCII on one line', () => {
    for (const text of TYPING_CHECK_PASSAGES) {
      expect(text).toMatch(/^[\x20-\x7e]+$/);
      expect(text.length).toBeGreaterThan(60);
    }
  });
});

describe('dictationSpeedup', () => {
  it('compares speeds to one decimal and needs both', () => {
    expect(dictationSpeedup(135, 62)).toBe(2.2);
    expect(dictationSpeedup(0, 62)).toBeNull();
    expect(dictationSpeedup(135, 0)).toBeNull();
  });
});

describe('getActivityInsights typing speed', () => {
  it('uses the given typing speed for time saved and keeps 40 wpm by default', () => {
    const stats = { ...loadStats(), totalWords: 1200, totalDurationSeconds: 300 };
    expect(getActivityInsights(stats).typingMinutesSaved).toBe(1200 / 40 - 5);
    expect(getActivityInsights(stats, new Date(), 60).typingMinutesSaved).toBe(1200 / 60 - 5);
    expect(getActivityInsights(stats, new Date(), 300).typingMinutesSaved).toBe(0);
  });
});
