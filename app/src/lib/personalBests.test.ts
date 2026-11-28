import { describe, expect, it } from 'vitest';
import { getPersonalBests, hasPersonalBests } from './personalBests';
import { loadStats, type DailyBuckets, type DictationStats } from './stats';

function stats(dailyBuckets: DailyBuckets): DictationStats {
  return { ...loadStats(), dailyBuckets };
}

const day = (words: number, recordings = 1) => ({ words, recordings, recordingSeconds: 60 });

describe('getPersonalBests', () => {
  it('returns nothing for empty statistics', () => {
    const bests = getPersonalBests(stats({}));
    expect(bests).toEqual({ mostWordsDay: null, mostDictationsDay: null, longestStreak: null, bestWeek: null });
    expect(hasPersonalBests(bests)).toBe(false);
  });

  it('finds the best day for words and for dictations independently', () => {
    const bests = getPersonalBests(stats({
      '2026-11-02': day(400, 9),
      '2026-11-10': day(1200, 3),
      '2026-11-11': day(50, 1),
    }));
    expect(bests.mostWordsDay).toEqual({ key: '2026-11-10', value: 1200 });
    expect(bests.mostDictationsDay).toEqual({ key: '2026-11-02', value: 9 });
    expect(hasPersonalBests(bests)).toBe(true);
  });

  it('keeps the earliest day on a tie', () => {
    const bests = getPersonalBests(stats({ '2026-11-12': day(500, 2), '2026-11-03': day(500, 2) }));
    expect(bests.mostWordsDay?.key).toBe('2026-11-03');
    expect(bests.mostDictationsDay?.key).toBe('2026-11-03');
  });

  it('measures the longest run of consecutive recording days across month and year ends', () => {
    const bests = getPersonalBests(stats({
      '2025-12-30': day(10),
      '2025-12-31': day(10),
      '2026-01-01': day(10),
      '2026-01-02': day(0, 1), // a recording with no words still counts toward the streak
      '2026-01-04': day(10),
      '2026-01-05': day(10),
      '2026-02-27': day(10),
      '2026-02-28': day(10),
      '2026-03-01': day(10),
    }));
    expect(bests.longestStreak).toEqual({ days: 4, startKey: '2025-12-30', endKey: '2026-01-02' });
  });

  it('breaks a streak on a stored zero-recording day', () => {
    const bests = getPersonalBests(stats({
      '2026-11-01': day(10),
      '2026-11-02': { words: 0, recordings: 0, recordingSeconds: 0 },
      '2026-11-03': day(10),
    }));
    expect(bests.longestStreak).toEqual({ days: 1, startKey: '2026-11-01', endKey: '2026-11-01' });
  });

  it('totals words per Sunday-to-Saturday calendar week', () => {
    // 2026-11-01 is a Sunday; 2026-11-07 the Saturday of the same week.
    const bests = getPersonalBests(stats({
      '2026-11-01': day(300),
      '2026-11-07': day(300),
      '2026-11-08': day(500),
      '2026-11-14': day(50),
    }));
    expect(bests.bestWeek).toEqual({ startKey: '2026-11-01', words: 600 });
  });

  it('ignores impossible or malformed day keys and non-finite counts', () => {
    const bests = getPersonalBests(stats({
      '2026-02-31': day(9000, 40),
      'not-a-day': day(9000, 40),
      '2026-11-05': { words: Number.NaN, recordings: 2, recordingSeconds: 60 },
      '2026-11-06': day(20, 1),
    }));
    expect(bests.mostWordsDay).toEqual({ key: '2026-11-06', value: 20 });
    expect(bests.mostDictationsDay).toEqual({ key: '2026-11-05', value: 2 });
    expect(bests.longestStreak?.days).toBe(2);
  });
});
