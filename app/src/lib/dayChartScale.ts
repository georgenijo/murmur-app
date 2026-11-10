import type { DaySummary } from './stats';

export interface ChartScale {
  min: number;
  max: number;
  ticks: readonly number[];
}

export function hasSpeed(day: DaySummary): boolean {
  return day.recordings > 0 && Number.isFinite(day.wpm) && day.wpm > 0;
}

/** Words always start at zero; speed starts strictly below its lowest observation. */
export function dayChartScale(days: readonly DaySummary[], metric: 'words' | 'recordings' | 'wpm'): ChartScale {
  const values = days.filter(day => metric !== 'wpm' || hasSpeed(day))
    .map(day => day[metric]).filter(value => Number.isFinite(value) && value >= 0);
  const step = metric === 'wpm' ? 40 : metric === 'words' ? 200 : 2;
  const max = Math.ceil(Math.max(1, ...values) / step) * step;
  const min = metric !== 'wpm' || values.length === 0
    ? 0 : Math.max(0, (Math.ceil(Math.min(...values) / 20) - 1) * 20);
  return { min, max, ticks: [max, (min + max) / 2, min] };
}

export function speedPoints(days: readonly DaySummary[], scale: ChartScale) {
  return days.map((day, index) => ({
    day,
    x: (index + 0.5) / Math.max(1, days.length) * 100,
    y: hasSpeed(day) ? 100 - (day.wpm - scale.min) / (scale.max - scale.min) * 100 : null,
  }));
}

/** A missing observation ends a segment without removing its weekday slot. */
export function speedSegments(points: ReturnType<typeof speedPoints>) {
  const segments: typeof points[] = [];
  let segment: typeof points = [];
  for (const point of points) {
    if (point.y === null) {
      if (segment.length) segments.push(segment);
      segment = [];
    } else segment.push(point);
  }
  if (segment.length) segments.push(segment);
  return segments.filter(segment => segment.length > 1);
}

export function heatmapIntensity(value: number, maximum: number): 0 | 1 | 2 | 3 | 4 {
  if (value <= 0) return 0;
  const ratio = value / Math.max(1, maximum);
  if (ratio > 0.75) return 4;
  if (ratio > 0.5) return 3;
  if (ratio > 0.25) return 2;
  return 1;
}
