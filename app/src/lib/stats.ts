import { clearDurableBlob, saveDurableBlob, STATS_STORE } from './durableUserData';
import {
  QUERY_PROVIDER_IDS,
  isQueryProviderId,
  isQueryUsage,
  type QueryUsage,
} from './queryUsage';
import type { QueryProviderId } from './settings';

// Per-day usage bucket, keyed by 'YYYY-MM-DD' (local time) in `dailyBuckets`.
export interface DayBucket {
  words: number;
  recordings: number;
  recordingSeconds: number;
}

export type DailyBuckets = Record<string, DayBucket>;

export const QUERY_FAILURE_CODES = [
  'not_configured',
  'invalid_executable',
  'invalid_arguments',
  'invalid_timeout',
  'busy',
  'audio_start_failed',
  'audio_not_ready',
  'audio_stalled',
  'audio_recovering',
  'audio_recovery_stalled',
  'no_speech',
  'transcription_failed',
  'empty_query',
  'query_too_large',
  'spawn_failed',
  'timed_out',
  'exit_nonzero',
  'provider_not_authenticated',
  'provider_error',
  'empty_answer',
  'clipboard_unavailable',
  'output_too_large',
  'process_failed',
  'termination_unconfirmed',
  'cancelled',
  'unknown',
] as const;

export type QueryFailureCode = typeof QUERY_FAILURE_CODES[number];

export interface QueryProviderStats {
  queriesRun: number;
  successfulQueries: number;
  failedQueries: number;
  inputTokens: number;
  outputTokens: number;
  reportedCostUsd: number;
}

export interface QueryStats extends QueryProviderStats {
  byProvider: Record<QueryProviderId, QueryProviderStats>;
  failuresByErrorCode: Partial<Record<QueryFailureCode, number>>;
}

export interface QueryCompletion {
  provider: QueryProviderId;
  succeeded: boolean;
  errorCode: string | null;
  usage: QueryUsage | null;
}

export interface DictationStats {
  totalWords: number;
  totalRecordings: number;
  totalDurationSeconds: number;
  wpmSamples: number[];
  // Per-day breakdown for the usage dashboard. Absent on stats saved before
  // this map existed — back-filled to {} on load (see loadStats).
  dailyBuckets: DailyBuckets;
  // Voice Query stores content-free counters only. Question, answer, command,
  // stderr, paths, and credentials are not accepted by this schema.
  query: QueryStats;
}

const EMPTY_PROVIDER_STATS: QueryProviderStats = {
  queriesRun: 0,
  successfulQueries: 0,
  failedQueries: 0,
  inputTokens: 0,
  outputTokens: 0,
  reportedCostUsd: 0,
};

function emptyProviderBreakdown(): Record<QueryProviderId, QueryProviderStats> {
  return {
    claude: { ...EMPTY_PROVIDER_STATS },
    codex: { ...EMPTY_PROVIDER_STATS },
    grok: { ...EMPTY_PROVIDER_STATS },
    cursor: { ...EMPTY_PROVIDER_STATS },
    custom: { ...EMPTY_PROVIDER_STATS },
  };
}

function emptyQueryStats(): QueryStats {
  return {
    ...EMPTY_PROVIDER_STATS,
    byProvider: emptyProviderBreakdown(),
    failuresByErrorCode: {},
  };
}

const DEFAULT_STATS: DictationStats = {
  totalWords: 0,
  totalRecordings: 0,
  totalDurationSeconds: 0,
  wpmSamples: [],
  dailyBuckets: {},
  query: emptyQueryStats(),
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

function sanitizeProviderStats(raw: unknown): QueryProviderStats {
  if (!raw || typeof raw !== 'object') return { ...EMPTY_PROVIDER_STATS };
  const stats = raw as Record<string, unknown>;
  return {
    queriesRun: safeCount(stats.queriesRun),
    successfulQueries: safeCount(stats.successfulQueries),
    failedQueries: safeCount(stats.failedQueries),
    inputTokens: safeCount(stats.inputTokens),
    outputTokens: safeCount(stats.outputTokens),
    reportedCostUsd: safeCost(stats.reportedCostUsd),
  };
}

function isQueryFailureCode(value: string): value is QueryFailureCode {
  return (QUERY_FAILURE_CODES as readonly string[]).includes(value);
}

function sanitizeFailures(raw: unknown): Partial<Record<QueryFailureCode, number>> {
  if (!raw || typeof raw !== 'object') return {};
  const failures: Partial<Record<QueryFailureCode, number>> = {};
  for (const [code, count] of Object.entries(raw as Record<string, unknown>)) {
    if (isQueryFailureCode(code)) failures[code] = safeCount(count);
  }
  return failures;
}

function sanitizeQueryStats(raw: unknown): QueryStats {
  const totals = sanitizeProviderStats(raw);
  const value = raw && typeof raw === 'object'
    ? raw as Record<string, unknown>
    : {};
  const rawProviders = value.byProvider && typeof value.byProvider === 'object'
    ? value.byProvider as Record<string, unknown>
    : {};
  const byProvider = emptyProviderBreakdown();
  for (const provider of QUERY_PROVIDER_IDS) {
    byProvider[provider] = sanitizeProviderStats(rawProviders[provider]);
  }
  return {
    ...totals,
    byProvider,
    failuresByErrorCode: sanitizeFailures(value.failuresByErrorCode),
  };
}

const MAX_WPM_SAMPLES = 100;

function sanitizeStats(parsed: Partial<DictationStats>): DictationStats {
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
    query: sanitizeQueryStats(parsed.query),
  };
}

export function loadStats(): DictationStats {
  try {
    const stored = localStorage.getItem(STATS_STORE.storageKey);
    if (stored) {
      const parsed = JSON.parse(stored) as Partial<DictationStats>;
      // Back-compat: stats saved before dailyBuckets existed have no map; the
      // sanitizer turns `undefined` into {} so older installs migrate cleanly.
      return sanitizeStats(parsed);
    }
  } catch (e) {
    console.error('Failed to load stats:', e);
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
  } catch (e) {
    console.error('Failed to save stats:', e);
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
      totalWords: stats.totalWords + wordCount,
      totalRecordings: stats.totalRecordings + 1,
      totalDurationSeconds: stats.totalDurationSeconds + durationSeconds,
      wpmSamples: newSamples,
      dailyBuckets,
      query: stats.query,
    });
  } catch (e) {
    console.error('Failed to update stats:', e);
  }
}

function addCount(current: number, increment: number): number {
  return Math.min(Number.MAX_SAFE_INTEGER, current + increment);
}

function addCost(current: number, increment: number): number {
  const total = current + increment;
  return Number.isFinite(total) && total >= 0 ? total : current;
}

function addCompletion(
  current: QueryProviderStats,
  succeeded: boolean,
  usage: QueryUsage | null,
): QueryProviderStats {
  return {
    queriesRun: addCount(current.queriesRun, 1),
    successfulQueries: addCount(current.successfulQueries, succeeded ? 1 : 0),
    failedQueries: addCount(current.failedQueries, succeeded ? 0 : 1),
    inputTokens: addCount(current.inputTokens, usage?.inputTokens ?? 0),
    outputTokens: addCount(current.outputTokens, usage?.outputTokens ?? 0),
    reportedCostUsd: addCost(current.reportedCostUsd, usage?.costUsd ?? 0),
  };
}

export function updateQueryStats(completion: QueryCompletion): void {
  try {
    if (!isQueryProviderId(completion.provider)) return;
    const usage = isQueryUsage(completion.usage) ? completion.usage : null;
    const stats = loadStats();
    const totals = addCompletion(stats.query, completion.succeeded, usage);
    const byProvider = {
      ...stats.query.byProvider,
      [completion.provider]: addCompletion(
        stats.query.byProvider[completion.provider],
        completion.succeeded,
        usage,
      ),
    };
    const failuresByErrorCode = { ...stats.query.failuresByErrorCode };
    if (!completion.succeeded) {
      const code = completion.errorCode && isQueryFailureCode(completion.errorCode)
        ? completion.errorCode
        : 'unknown';
      failuresByErrorCode[code] = addCount(
        failuresByErrorCode[code] ?? 0,
        1,
      );
    }
    const query: QueryStats = { ...totals, byProvider, failuresByErrorCode };
    saveStats({ ...stats, query });
  } catch (e) {
    console.error('Failed to update query stats:', e);
  }
}

export function resetStats(): void {
  clearDurableBlob(STATS_STORE);
}

export function getWPM(stats: DictationStats): number {
  if (stats.wpmSamples.length === 0) return 0;
  const sum = stats.wpmSamples.reduce((a, b) => a + b, 0);
  return Math.round(sum / stats.wpmSamples.length);
}

export function getApproxTokens(stats: DictationStats): number {
  return Math.round(stats.totalWords * 1.3);
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
