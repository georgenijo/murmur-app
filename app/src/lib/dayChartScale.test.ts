import { describe, expect, it } from 'vitest';
import { dayChartScale, hasSpeed, heatmapIntensity, speedPoints, speedSegments } from './dayChartScale';
import type { DaySummary } from './stats';

const day = (wpm: number, recordings = 1): DaySummary => ({ key: String(wpm), date: new Date(2026, 8, 4), words: wpm, recordings, recordingSeconds: 60, wpm });

describe('daily chart scales', () => {
  it('rounds word counts upwards and keeps zero values on the baseline', () => {
    const scale = dayChartScale([day(0), day(900)], 'words');
    expect(scale).toEqual({ min: 0, max: 1000, ticks: [1000, 500, 0] });
  });

  it('starts speed strictly below the smallest observation, including exact multiples of 20', () => {
    expect(dayChartScale([140, 170, 116, 180, 153, 120, 175].map(value => day(value)), 'wpm'))
      .toEqual({ min: 100, max: 200, ticks: [200, 150, 100] });
    expect(dayChartScale([day(120)], 'wpm').min).toBe(100);
    expect(dayChartScale([day(19)], 'wpm').min).toBe(0);
  });

  it('ignores absent and invalid speeds and produces a usable empty range', () => {
    const absent = [day(0), day(200, 0), day(NaN), day(Infinity)];
    expect(absent.every(value => !hasSpeed(value))).toBe(true);
    expect(dayChartScale(absent, 'wpm')).toEqual({ min: 0, max: 40, ticks: [40, 20, 0] });
    expect(dayChartScale([...absent, day(116), day(180)], 'wpm').ticks).toEqual([200, 150, 100]);
  });

  it('preserves seven slots, breaks the line at absent days, and positions observed points against the range', () => {
    const days = [140, 170, 116, 180, 153, 120, 175].map((value, index) => day(value, index === 3 ? 0 : 1));
    const points = speedPoints(days, dayChartScale(days, 'wpm'));
    expect(points).toHaveLength(7);
    expect(points[0].y).toBe(60);
    expect(points[3].y).toBeNull();
    expect(speedSegments(points).map(segment => segment.length)).toEqual([3, 3]);
    expect(speedSegments(speedPoints([day(140), day(0), day(160)], dayChartScale(days, 'wpm')))).toEqual([]);
  });

  it('uses all five heatmap levels with empty days distinct from activity', () => {
    expect([0, 1, 25, 26, 50, 51, 75, 76, 100].map(value => heatmapIntensity(value, 100)))
      .toEqual([0, 1, 1, 2, 2, 3, 3, 4, 4]);
    expect(heatmapIntensity(0, 0)).toBe(0);
  });
});
