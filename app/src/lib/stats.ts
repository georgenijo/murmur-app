import { clearDurableBlob, saveDurableBlob, STATS_STORE } from './durableUserData';
import { addActivityCompletion, sanitizeActivityStats, type ActivityCompletion, type ActivityStats } from './activityStats';

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

function isValidBucket(b: unknown): b is DayBucket {
  if (!b || typeof b !== 'object') return false;
  const r = b as Record<string, unknown>;
  return (
    typeof r.words === 'number' && isFinite(r.words) &&
    typeof r.recordings === 'number' && isFinite(r.recordings) &&
    typeof r.recordingSeconds === 'number' && isFinite(r.recordingSeconds)
  );
}

function sanitizeBuckets(raw: unknown): DailyBuckets {
  if (!raw || typeof raw !== 'object') return {};
  const out: DailyBuckets = {};
  for (const [key, value] of Object.entries(raw as Record<string, unknown>)) {
    if (/^\d{4}-\d{2}-\d{2}$/.test(key) && isValidBucket(value)) {
      out[key] = {
        words: value.words,
        recordings: value.recordings,
        recordingSeconds: value.recordingSeconds,
      };
    }
  }
  return out;
}

function safeCount(value: unknown): number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0
    ? value
    : 0;
}

function safeCost(value: unknown): number {
  return typeof value === 'number' && Number.isFinite(value) && value >= 0
    ? value
    : 0;
}

const MAX_WPM_SAMPLES = 100;

function sanitizeStats(value: unknown): DictationStats {
  const parsed = value !== null && typeof value === 'object'
    ? value as Partial<DictationStats> : {};
  const wpmSamples = Array.isArray(parsed.wpmSamples)
    ? parsed.wpmSamples.filter((value) => (
      typeof value === 'number' && Number.isFinite(value) && value >= 0
    )).slice(-MAX_WPM_SAMPLES)
    : [];
  return {
    totalWords: safeCount(parsed.totalWords),
    totalRecordings: safeCount(parsed.totalRecordings),
    totalDurationSeconds: safeCost(parsed.totalDurationSeconds),
    wpmSamples,
    dailyBuckets: sanitizeBuckets(parsed.dailyBuckets),
    activity: sanitizeActivityStats(parsed.activity),
  };
}

export function loadStats(): DictationStats {
  try {
    const stored = localStorage.getItem(STATS_STORE.storageKey);
    if (stored) {
      const parsed = JSON.parse(stored) as unknown;
      // Back-compat: stats saved before dailyBuckets existed have no map; the
      // sanitizer turns `undefined` into {} so older installs migrate cleanly.
      return sanitizeStats(parsed);
    }
  } catch {
    console.error('Failed to load stats');
  }
  return sanitizeStats(DEFAULT_STATS);
}

export function saveStats(stats: DictationStats): void {
  try {
    // Canonicalize at the write boundary as well as on read: structural
    // typing or future callers must not smuggle arbitrary content fields into
    // the durable statistics blob.
    const blob = JSON.stringify(sanitizeStats(stats));
    saveDurableBlob(STATS_STORE, blob);
    window.dispatchEvent(new Event(STATS_CHANGED_EVENT));
  } catch {
    console.error('Failed to save stats');
  }
}

export function updateStats(text: string, durationSeconds: number): void {
  try {
    const stats = loadStats();
    const wordCount = text.trim() === '' ? 0 : text.trim().split(/\s+/).length;

    const newSamples = [...stats.wpmSamples];
    if (durationSeconds > 0 && wordCount > 0) {
      newSamples.push((wordCount / durationSeconds) * 60);
      if (newSamples.length > MAX_WPM_SAMPLES) {
        newSamples.splice(0, newSamples.length - MAX_WPM_SAMPLES);
      }
    }

    // Fold this recording into today's bucket (recordings increments even for
    // empty transcriptions, mirroring totalRecordings — drives the streak).
    const key = dayKey();
    const prev = stats.dailyBuckets[key] ?? EMPTY_BUCKET;
    const dailyBuckets: DailyBuckets = {
      ...stats.dailyBuckets,
      [key]: {
        words: prev.words + wordCount,
        recordings: prev.recordings + 1,
        recordingSeconds: prev.recordingSeconds + durationSeconds,
      },
    };

    saveStats({
      ...stats,
      totalWords: stats.totalWords + wordCount,
      totalRecordings: stats.totalRecordings + 1,
      totalDurationSeconds: stats.totalDurationSeconds + durationSeconds,
      wpmSamples: newSamples,
      dailyBuckets,
    });
  } catch {
    console.error('Failed to update stats');
  }
}

export function updateActivityStats(completion: ActivityCompletion, month?: string): void {
  const stats = loadStats();
  saveStats({ ...stats, activity: addActivityCompletion(stats.activity, completion, month) });
}

export function resetStats(): void {
  clearDurableBlob(STATS_STORE);
  window.dispatchEvent(new Event(STATS_RESET_EVENT));
  window.dispatchEvent(new Event(STATS_CHANGED_EVENT));
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
