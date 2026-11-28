import { describe, expect, it } from 'vitest';
import { formatHourRange, getDictationHours, periodForHour, periodShare } from './dictationHours';
import type { HistoryEntry } from './history';

function at(hour: number, minute = 0, extra: Partial<HistoryEntry> = {}): HistoryEntry {
  return {
    id: `e-${hour}-${minute}-${Math.random()}`,
    text: 'fixture',
    timestamp: new Date(2026, 10, 20, hour, minute).getTime(),
    duration: 4,
    ...extra,
  } as HistoryEntry;
}

describe('getDictationHours', () => {
  it('returns an empty profile without entries', () => {
    const hours = getDictationHours([]);
    expect(hours.total).toBe(0);
    expect(hours.peakHour).toBeNull();
    expect(hours.counts).toHaveLength(24);
    expect(hours.periods.every((period) => period.count === 0)).toBe(true);
  });

  it('counts spoken entries by local hour and finds the busiest hour', () => {
    const hours = getDictationHours([at(9), at(9, 30), at(9, 59), at(14), at(23, 59), at(0, 1)]);
    expect(hours.total).toBe(6);
    expect(hours.counts[9]).toBe(3);
    expect(hours.counts[14]).toBe(1);
    expect(hours.counts[23]).toBe(1);
    expect(hours.counts[0]).toBe(1);
    expect(hours.peakHour).toBe(9);
  });

  it('keeps the earliest hour on a tie', () => {
    expect(getDictationHours([at(16), at(7), at(16), at(7)]).peakHour).toBe(7);
  });

  it('skips imported files and unusable timestamps', () => {
    const hours = getDictationHours([
      at(10),
      at(11, 0, { source: 'file', sourceName: 'note.wav' }),
      at(12, 0, { source: 'recording' }),
      { ...at(13), timestamp: Number.NaN },
      { ...at(13), timestamp: Number.POSITIVE_INFINITY },
    ]);
    expect(hours.total).toBe(2);
    expect(hours.counts[11]).toBe(0);
    expect(hours.counts[13]).toBe(0);
  });

  it('groups hours into periods with night wrapping midnight', () => {
    const hours = getDictationHours([at(5), at(11), at(12), at(16), at(17), at(21), at(22), at(4)]);
    expect(Object.fromEntries(hours.periods.map((period) => [period.id, period.count]))).toEqual({
      morning: 2, afternoon: 2, evening: 2, night: 2,
    });
    expect([0, 4, 22, 23].map(periodForHour)).toEqual(['night', 'night', 'night', 'night']);
    expect([5, 11].map(periodForHour)).toEqual(['morning', 'morning']);
  });
});

describe('periodShare', () => {
  it('rounds to whole percents but never shows a nonzero period as 0%', () => {
    expect(periodShare(0, 10)).toBe(0);
    expect(periodShare(1, 0)).toBe(0);
    expect(periodShare(1, 400)).toBe(1);
    expect(periodShare(1, 3)).toBe(33);
    expect(periodShare(3, 3)).toBe(100);
  });
});

describe('formatHourRange', () => {
  it('labels an hour slot and wraps 11 PM to midnight', () => {
    expect(formatHourRange(9)).toMatch(/9.*10/);
    expect(formatHourRange(23)).toMatch(/11.*12/);
  });
});
