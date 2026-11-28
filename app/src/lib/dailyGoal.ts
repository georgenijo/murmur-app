import { dayKey, type DictationStats } from './stats';

/**
 * Opt-in daily word goal for Insights. `0` means off (the default). The goal
 * only reads the local per-day word buckets that Insights already shows; it
 * never touches transcription, delivery, or history.
 */
export const DAILY_WORD_GOAL_OFF = 0;
export const DAILY_WORD_GOAL_PRESETS = [250, 500, 1000, 2000, 5000] as const;
export const DAILY_WORD_GOAL_MAX = 100_000;
/** Days the "goal met" summary looks back over, including today. */
export const DAILY_GOAL_WINDOW_DAYS = 7;

/** Any finite positive integer up to the cap is kept; anything else is off. */
export function sanitizeDailyWordGoal(value: unknown): number {
  if (typeof value !== 'number' || !Number.isFinite(value)) return DAILY_WORD_GOAL_OFF;
  const rounded = Math.round(value);
  if (rounded <= 0) return DAILY_WORD_GOAL_OFF;
  return Math.min(rounded, DAILY_WORD_GOAL_MAX);
}

export interface DailyGoalProgress {
  goal: number;
  todayWords: number;
  /** Words still needed today; 0 once the goal is met. */
  remaining: number;
  /** Today's progress as a whole percentage, capped at 100 for the bar. */
  percent: number;
  met: boolean;
  /** Days within the last `DAILY_GOAL_WINDOW_DAYS` (today included) that met the goal. */
  metDaysInWindow: number;
  /**
   * Consecutive days meeting the goal, counting back from today. An unmet
   * today does not break it, so the streak isn't lost before the first
   * dictation of the day (matching the recording streak on Insights).
   */
  goalStreak: number;
}

function wordsOn(stats: DictationStats, date: Date): number {
  const words = stats.dailyBuckets[dayKey(date)]?.words ?? 0;
  return Number.isFinite(words) && words > 0 ? words : 0;
}

function daysBefore(today: Date, days: number): Date {
  return new Date(today.getFullYear(), today.getMonth(), today.getDate() - days);
}

/** Returns `null` when the goal is off so callers render nothing goal-specific. */
export function getDailyGoalProgress(
  stats: DictationStats,
  goalSetting: unknown,
  now: Date = new Date(),
): DailyGoalProgress | null {
  const goal = sanitizeDailyWordGoal(goalSetting);
  if (goal === DAILY_WORD_GOAL_OFF) return null;
  const today = daysBefore(now, 0);
  const todayWords = wordsOn(stats, today);
  const met = todayWords >= goal;

  let metDaysInWindow = 0;
  for (let i = 0; i < DAILY_GOAL_WINDOW_DAYS; i++) {
    if (wordsOn(stats, daysBefore(today, i)) >= goal) metDaysInWindow++;
  }

  // Bounded by the buckets that exist, so a corrupt map cannot spin forever.
  const maxStreak = Object.keys(stats.dailyBuckets).length;
  let goalStreak = 0;
  let offset = met ? 0 : 1;
  while (goalStreak < maxStreak && wordsOn(stats, daysBefore(today, offset)) >= goal) {
    goalStreak++;
    offset++;
  }

  return {
    goal,
    todayWords,
    remaining: Math.max(0, goal - todayWords),
    percent: Math.min(100, Math.floor((todayWords / goal) * 100)),
    met,
    metDaysInWindow,
    goalStreak,
  };
}
