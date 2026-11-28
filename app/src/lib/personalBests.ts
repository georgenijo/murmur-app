import type { DailyBuckets, DictationStats } from './stats';

/**
 * Opt-in "Personal bests" for Insights (`showPersonalBests`, off by default).
 * Every figure is derived from the local per-day buckets Insights already
 * keeps; nothing here touches transcription, delivery, or history.
 *
 * Ties keep the earliest day or week: a best is the first time it was reached.
 */
export interface DayBest {
  key: string;   // 'YYYY-MM-DD'
  value: number;
}

export interface StreakBest {
  days: number;
  startKey: string;
  endKey: string;
}

export interface WeekBest {
  /** Sunday that starts the calendar week, 'YYYY-MM-DD'. */
  startKey: string;
  words: number;
}

export interface PersonalBests {
  mostWordsDay: DayBest | null;
  mostDictationsDay: DayBest | null;
  longestStreak: StreakBest | null;
  bestWeek: WeekBest | null;
}

const DAY_KEY = /^(\d{4})-(\d{2})-(\d{2})$/;

function parseKey(key: string): Date | null {
  const match = DAY_KEY.exec(key);
  if (!match) return null;
  const date = new Date(Number(match[1]), Number(match[2]) - 1, Number(match[3]));
  // Reject impossible dates such as 2026-02-31 instead of rolling them over.
  return date.getMonth() === Number(match[2]) - 1 && date.getDate() === Number(match[3]) ? date : null;
}

function keyOf(date: Date): string {
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}`;
}

function count(value: number): number {
  return Number.isFinite(value) && value > 0 ? value : 0;
}

/** Valid day keys in ascending order (string order equals date order). */
function sortedDays(buckets: DailyBuckets): string[] {
  return Object.keys(buckets).filter((key) => parseKey(key) !== null).sort();
}

function bestBy(buckets: DailyBuckets, days: string[], field: 'words' | 'recordings'): DayBest | null {
  let best: DayBest | null = null;
  for (const key of days) {
    const value = count(buckets[key][field]);
    if (value > 0 && (best === null || value > best.value)) best = { key, value };
  }
  return best;
}

function longestStreak(buckets: DailyBuckets, days: string[]): StreakBest | null {
  let best: StreakBest | null = null;
  let run: StreakBest | null = null;
  let previous: Date | null = null;
  for (const key of days) {
    if (count(buckets[key].recordings) === 0) {
      run = null;
      previous = null;
      continue;
    }
    const date = parseKey(key)!;
    const continues: boolean = run !== null && previous !== null
      && keyOf(new Date(previous.getFullYear(), previous.getMonth(), previous.getDate() + 1)) === key;
    run = run !== null && continues
      ? { days: run.days + 1, startKey: run.startKey, endKey: key }
      : { days: 1, startKey: key, endKey: key };
    previous = date;
    if (best === null || run.days > best.days) best = run;
  }
  return best;
}

function bestWeek(buckets: DailyBuckets, days: string[]): WeekBest | null {
  const weeks = new Map<string, number>();
  for (const key of days) {
    const words = count(buckets[key].words);
    if (words === 0) continue;
    const date = parseKey(key)!;
    const start = keyOf(new Date(date.getFullYear(), date.getMonth(), date.getDate() - date.getDay()));
    weeks.set(start, (weeks.get(start) ?? 0) + words);
  }
  let best: WeekBest | null = null;
  for (const startKey of [...weeks.keys()].sort()) {
    const words = weeks.get(startKey)!;
    if (best === null || words > best.words) best = { startKey, words };
  }
  return best;
}

export function getPersonalBests(stats: DictationStats): PersonalBests {
  const buckets = stats.dailyBuckets;
  const days = sortedDays(buckets);
  return {
    mostWordsDay: bestBy(buckets, days, 'words'),
    mostDictationsDay: bestBy(buckets, days, 'recordings'),
    longestStreak: longestStreak(buckets, days),
    bestWeek: bestWeek(buckets, days),
  };
}

export function hasPersonalBests(bests: PersonalBests): boolean {
  return bests.mostDictationsDay !== null;
}

/** Local calendar date for a day key, for display only. */
export function dateForKey(key: string): Date | null {
  return parseKey(key);
}
