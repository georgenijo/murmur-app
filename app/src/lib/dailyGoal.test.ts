import { describe, expect, it } from 'vitest';
import { getDailyGoalProgress, sanitizeDailyWordGoal } from './dailyGoal';
import { dayKey, type DictationStats } from './stats';
import { sanitizeActivityStats } from './activityStats';

const NOW = new Date(2026, 10, 27, 15, 0, 0);

function statsWith(wordsByDaysAgo: Record<number, number>): DictationStats {
  const dailyBuckets: DictationStats['dailyBuckets'] = {};
  for (const [ago, words] of Object.entries(wordsByDaysAgo)) {
    const date = new Date(NOW.getFullYear(), NOW.getMonth(), NOW.getDate() - Number(ago));
    dailyBuckets[dayKey(date)] = { words, recordings: words > 0 ? 1 : 0, recordingSeconds: 60 };
  }
  return { totalWords: 0, totalRecordings: 0, totalDurationSeconds: 0, wpmSamples: [], dailyBuckets, activity: sanitizeActivityStats(undefined) };
}

describe('sanitizeDailyWordGoal', () => {
  it('treats anything but a finite positive number as off and caps large goals', () => {
    expect(sanitizeDailyWordGoal(undefined)).toBe(0);
    expect(sanitizeDailyWordGoal(Number.NaN)).toBe(0);
    expect(sanitizeDailyWordGoal(Infinity)).toBe(0);
    expect(sanitizeDailyWordGoal(0.4)).toBe(0);
    expect(sanitizeDailyWordGoal(250)).toBe(250);
    expect(sanitizeDailyWordGoal(10_000_000)).toBe(100_000);
  });
});

describe('getDailyGoalProgress', () => {
  it('returns null while the goal is off', () => {
    expect(getDailyGoalProgress(statsWith({ 0: 900 }), 0, NOW)).toBeNull();
    expect(getDailyGoalProgress(statsWith({ 0: 900 }), 'nope', NOW)).toBeNull();
  });

  it('reports partial progress for today with a floored, capped percentage', () => {
    const progress = getDailyGoalProgress(statsWith({ 0: 333, 1: 1200, 2: 1000, 3: 10 }), 1000, NOW)!;
    expect(progress).toMatchObject({ goal: 1000, todayWords: 333, remaining: 667, percent: 33, met: false, metDaysInWindow: 2 });
    // An unmet today does not break the streak carried in from yesterday.
    expect(progress.goalStreak).toBe(2);
  });

  it('counts today in the streak once met and caps the bar at 100%', () => {
    const progress = getDailyGoalProgress(statsWith({ 0: 1500, 1: 600, 2: 100, 3: 900 }), 500, NOW)!;
    expect(progress).toMatchObject({ todayWords: 1500, remaining: 0, percent: 100, met: true, goalStreak: 2, metDaysInWindow: 3 });
  });

  it('only counts the last seven days, today included, and has no streak without data', () => {
    expect(getDailyGoalProgress(statsWith({ 6: 500, 7: 500, 8: 500 }), 500, NOW)!.metDaysInWindow).toBe(1);
    expect(getDailyGoalProgress(statsWith({}), 500, NOW)).toMatchObject({ todayWords: 0, percent: 0, goalStreak: 0, metDaysInWindow: 0 });
  });

  it('ignores malformed negative buckets', () => {
    expect(getDailyGoalProgress(statsWith({ 0: -40 }), 100, NOW)!.todayWords).toBe(0);
  });
});
