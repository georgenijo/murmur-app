import { completionSnapshot, completionMutation } from './completionData';
import { sanitizeActivityStats, type ActivityStats } from './activityStats';

export const STATS_CHANGED_EVENT = 'murmur-stats-changed';
export const STATS_RESET_EVENT = 'murmur-stats-reset';

// Per-day usage bucket, keyed by 'YYYY-MM-DD' (local time) in `dailyBuckets`.
export interface DayBucket {
  words: number;
  recordings: number;
  recordingSeconds: number;
}

export type DailyBuckets = Record<string, DayBucket>;

export interface DictationStats {
  totalWords: number;
  totalRecordings: number;
  totalDurationSeconds: number;
  wpmSamples: number[];
  // Per-day breakdown for the usage dashboard. Absent on stats saved before
  // this map existed — back-filled to {} on load (see loadStats).
  dailyBuckets: DailyBuckets;
  activity: ActivityStats;
}

const DEFAULT_STATS: DictationStats = {
  totalWords: 0,
  totalRecordings: 0,
  totalDurationSeconds: 0,
  wpmSamples: [],
  dailyBuckets: {},
  activity: sanitizeActivityStats(undefined),
};

const EMPTY_BUCKET: DayBucket = { words: 0, recordings: 0, recordingSeconds: 0 };

// Local-time 'YYYY-MM-DD' key for a date (defaults to now). Local — so a day's
// bucket aligns with the user's calendar, not UTC.
export function dayKey(date: Date = new Date()): string {
  const y = date.getFullYear();
  const m = String(date.getMonth() + 1).padStart(2, '0');
  const d = String(date.getDate()).padStart(2, '0');
  return `${y}-${m}-${d}`;
}

export function loadStats(): DictationStats { return completionSnapshot()?.statistics ?? structuredClone(DEFAULT_STATS); }

export async function resetStats(): Promise<void> {
  await completionMutation('clear_completion_destination', { destination: 'statistics' });
  window.dispatchEvent(new Event(STATS_RESET_EVENT));
}

export function getWPM(stats: DictationStats): number {
  if (stats.wpmSamples.length === 0) return 0;
  const sum = stats.wpmSamples.reduce((a, b) => a + b, 0);
  return Math.round(sum / stats.wpmSamples.length);
}

// --- Daily-bucket derivations for the usage dashboard ---

export interface DaySummary extends DayBucket {
  key: string;   // 'YYYY-MM-DD'
  date: Date;    // local midnight of that day
  wpm: number;   // words per minute over the day (0 when no audio)
}

function bucketFor(buckets: DailyBuckets, key: string): DayBucket {
  return buckets[key] ?? EMPTY_BUCKET;
}

function summaryFor(buckets: DailyBuckets, date: Date): DaySummary {
  const key = dayKey(date);
  const b = bucketFor(buckets, key);
  const minutes = b.recordingSeconds / 60;
  return {
    key,
    date: new Date(date.getFullYear(), date.getMonth(), date.getDate()),
    words: b.words,
    recordings: b.recordings,
    recordingSeconds: b.recordingSeconds,
    wpm: minutes > 0 ? Math.round(b.words / minutes) : 0,
  };
}

// Ordered oldest→newest list of the last `days` calendar days ending today.
export function getRecentDays(stats: DictationStats, days: number): DaySummary[] {
  const out: DaySummary[] = [];
  const today = new Date();
  for (let i = days - 1; i >= 0; i--) {
    const d = new Date(today.getFullYear(), today.getMonth(), today.getDate() - i);
    out.push(summaryFor(stats.dailyBuckets, d));
  }
  return out;
}

// Heatmap grid: `weeks` columns of 7 rows (Sun→Sat), aligned so the last column
// ends on today. Cells before the user's first day still render (empty buckets).
export function getHeatmapWeeks(stats: DictationStats, weeks: number): DaySummary[][] {
  const today = new Date();
  const todayMidnight = new Date(today.getFullYear(), today.getMonth(), today.getDate());
  // Walk back to the Sunday that starts the earliest visible week.
  const start = new Date(todayMidnight);
  start.setDate(start.getDate() - today.getDay() - (weeks - 1) * 7);

  const cols: DaySummary[][] = [];
  for (let w = 0; w < weeks; w++) {
    const col: DaySummary[] = [];
    for (let day = 0; day < 7; day++) {
      const d = new Date(start);
      d.setDate(start.getDate() + w * 7 + day);
      // Days in the future (this week, after today) are omitted as empty cells.
      col.push(d > todayMidnight ? summaryFor({}, d) : summaryFor(stats.dailyBuckets, d));
    }
    cols.push(col);
  }
  return cols;
}

// Consecutive days with >=1 recording, counting back from today. If today has
// no recordings yet the streak still counts from yesterday so it isn't lost
// mid-day before the first recording.
export function getCurrentStreak(stats: DictationStats): number {
  const buckets = stats.dailyBuckets;
  const today = new Date();
  const todayMidnight = new Date(today.getFullYear(), today.getMonth(), today.getDate());
  let streak = 0;
  // Start at today, or yesterday if today is still empty.
  let cursor = new Date(todayMidnight);
  if (bucketFor(buckets, dayKey(cursor)).recordings === 0) {
    cursor.setDate(cursor.getDate() - 1);
  }
  while (bucketFor(buckets, dayKey(cursor)).recordings > 0) {
    streak++;
    cursor.setDate(cursor.getDate() - 1);
  }
  return streak;
}
