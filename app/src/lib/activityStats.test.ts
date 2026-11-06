import { beforeEach, describe, expect, it } from 'vitest';
import { addActivityCompletion, getActivityInsights } from './activityStats';
import { loadStats as emptyStats } from './stats';
import type { ActivityCompletion } from './activityStats';
let current = emptyStats();
const loadStats = () => current;
const resetStats = () => { current = emptyStats(); };
const updateActivityStats = (event: ActivityCompletion, month?: string) => { current = { ...current, activity: addActivityCompletion(current.activity, event, month) }; };

beforeEach(resetStats);

describe('local activity statistics', () => {
  it('has no most-used transform when no named transform has run', () => {
    expect(getActivityInsights(loadStats())).toMatchObject({ mostUsedTransformName: null, mostUsedTransformRuns: 0 });
    updateActivityStats({ kind: 'transform', outcome: 'approved', presetName: 'Uncounted' });
    updateActivityStats({ kind: 'transform', outcome: 'runs', presetName: null });
    expect(getActivityInsights(loadStats())).toMatchObject({ mostUsedTransformName: null, mostUsedTransformRuns: 0 });
  });

  it('counts the most-used named transform across months without counting approvals or undos as runs', () => {
    updateActivityStats({ kind: 'transform', outcome: 'runs', presetName: 'Meeting notes' }, '2026-08');
    updateActivityStats({ kind: 'transform', outcome: 'runs', presetName: 'Meeting notes' }, '2026-09');
    updateActivityStats({ kind: 'transform', outcome: 'approved', presetName: 'Meeting notes' }, '2026-09');
    updateActivityStats({ kind: 'transform', outcome: 'undone', presetName: 'Meeting notes' }, '2026-09');
    expect(getActivityInsights(loadStats(), new Date(2026, 8, 1))).toMatchObject({
      mostUsedTransformName: 'Meeting notes', mostUsedTransformRuns: 2, month: { runs: 1 }, approvalRate: 100,
    });
    updateActivityStats({ kind: 'transform', outcome: 'runs', presetName: 'Shorten' }, '2026-09');
    expect(getActivityInsights(loadStats()).mostUsedTransformName).toBe('Meeting notes');
  });

  it('breaks transform run ties by name regardless of insertion order', () => {
    for (const names of [['Shorten', 'Bullets'], ['Bullets', 'Shorten']]) {
      resetStats();
      for (const presetName of names) updateActivityStats({ kind: 'transform', outcome: 'runs', presetName });
      expect(getActivityInsights(loadStats())).toMatchObject({ mostUsedTransformName: 'Bullets', mostUsedTransformRuns: 1 });
    }
  });

  it('states a fixed typing-rate assumption and never reports negative time saved', () => {
    const stats = { ...loadStats(), totalWords: 120, totalDurationSeconds: 60 };
    expect(getActivityInsights(stats).dictationMinutes).toBe(1);
    expect(getActivityInsights(stats).typingMinutesSaved).toBe(2);
    expect(getActivityInsights({ ...stats, totalDurationSeconds: 600 }).typingMinutesSaved).toBe(0);
  });

});
