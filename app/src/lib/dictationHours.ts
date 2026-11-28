import { entrySource, type HistoryEntry } from './history';

/**
 * Time-of-day profile for the opt-in Insights "When you dictate" card.
 *
 * Computed only from the timestamps of spoken entries already retained in
 * local History. Imported files are skipped because their timestamp is when
 * the file was transcribed, not when the user spoke. Display only.
 */
export type DayPeriodId = 'morning' | 'afternoon' | 'evening' | 'night';

export interface DayPeriod {
  id: DayPeriodId;
  label: string;
  /** Inclusive start hour and exclusive end hour; night wraps past midnight. */
  startHour: number;
  endHour: number;
  count: number;
}

export interface DictationHours {
  /** Spoken dictations per local hour, index 0 = midnight to 1 AM. */
  counts: number[];
  total: number;
  /** Busiest hour; ties keep the earliest hour. Null when there is no data. */
  peakHour: number | null;
  periods: DayPeriod[];
}

const PERIODS: ReadonlyArray<Omit<DayPeriod, 'count'>> = [
  { id: 'morning', label: 'Morning', startHour: 5, endHour: 12 },
  { id: 'afternoon', label: 'Afternoon', startHour: 12, endHour: 17 },
  { id: 'evening', label: 'Evening', startHour: 17, endHour: 22 },
  { id: 'night', label: 'Night', startHour: 22, endHour: 5 },
];

export function periodForHour(hour: number): DayPeriodId {
  for (const period of PERIODS) {
    const inside = period.startHour < period.endHour
      ? hour >= period.startHour && hour < period.endHour
      : hour >= period.startHour || hour < period.endHour;
    if (inside) return period.id;
  }
  return 'night';
}

export function getDictationHours(entries: readonly HistoryEntry[]): DictationHours {
  const counts = Array.from({ length: 24 }, () => 0);
  for (const entry of entries) {
    if (entrySource(entry) !== 'recording') continue;
    if (typeof entry.timestamp !== 'number' || !Number.isFinite(entry.timestamp)) continue;
    const date = new Date(entry.timestamp);
    const hour = date.getHours();
    if (!Number.isInteger(hour)) continue;
    counts[hour] += 1;
  }
  const total = counts.reduce((sum, value) => sum + value, 0);
  let peakHour: number | null = null;
  for (let hour = 0; hour < 24; hour += 1) {
    if (counts[hour] > 0 && (peakHour === null || counts[hour] > counts[peakHour])) peakHour = hour;
  }
  const periods = PERIODS.map((period) => ({
    ...period,
    count: counts.reduce((sum, value, hour) => (periodForHour(hour) === period.id ? sum + value : sum), 0),
  }));
  return { counts, total, peakHour, periods };
}

/** Whole-number share that never rounds a nonzero count down to 0%. */
export function periodShare(count: number, total: number): number {
  if (total <= 0 || count <= 0) return 0;
  return Math.max(1, Math.round((count / total) * 100));
}

function hourLabel(hour: number): string {
  const date = new Date(2000, 0, 1, hour % 24);
  return date.toLocaleTimeString(undefined, { hour: 'numeric' });
}

/** Local label for one hour slot, e.g. "9 AM – 10 AM". */
export function formatHourRange(hour: number): string {
  return `${hourLabel(hour)} – ${hourLabel(hour + 1)}`;
}

export function formatHour(hour: number): string {
  return hourLabel(hour);
}
