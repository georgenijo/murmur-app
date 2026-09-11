import { beforeEach, describe, expect, it, vi } from 'vitest';
import { getActivityInsights } from './activityStats';
import { loadStats, resetStats, saveStats, updateActivityStats, updateQueryStats, updateStats } from './stats';
import { STATS_STORE } from './durableUserData';

beforeEach(() => localStorage.clear());

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

  it('back-fills every new counter to zero for legacy stats without changing old usage', () => {
    localStorage.setItem(STATS_STORE.storageKey, JSON.stringify({ totalWords: 80, totalRecordings: 2, totalDurationSeconds: 10 }));
    const stats = loadStats();
    expect(stats.totalWords).toBe(80);
    expect(stats.activity).toEqual({
      meetings: { count: 0, totalMinutes: 0, summariesGenerated: 0 },
      transforms: { runs: 0, approved: 0, undone: 0, byPresetName: {} },
      corrections: { proposed: 0, taught: 0, byScope: { global: 0, app: 0, project: 0 } },
      recordingsByMode: {}, pasteLastUses: 0, monthlyBuckets: {},
    });
  });

  it('preserves counters across interleaved dictation, query, and activity writes and repeated loads', () => {
    updateActivityStats({ kind: 'meeting', durationMs: 90_000 }, '2026-08');
    updateActivityStats({ kind: 'meeting_summary' });
    updateActivityStats({ kind: 'transform', outcome: 'runs', presetName: 'Exact saved name' }, '2026-08');
    updateActivityStats({ kind: 'transform', outcome: 'approved', presetName: 'Exact saved name' }, '2026-08');
    updateActivityStats({ kind: 'correction_proposed' });
    updateActivityStats({ kind: 'correction_taught', scope: 'project' });
    updateActivityStats({ kind: 'paste_last' });
    updateStats('one two three four', 3, 'mode-id');
    updateQueryStats({ provider: 'claude', succeeded: true, errorCode: null, usage: null });
    const stats = loadStats();
    saveStats(stats);
    expect(loadStats()).toEqual(stats);
    expect(stats.totalWords).toBe(4);
    expect(stats.query.successfulQueries).toBe(1);
    expect(stats.activity).toMatchObject({
      meetings: { count: 1, totalMinutes: 1.5, summariesGenerated: 1 },
      transforms: { runs: 1, approved: 1, byPresetName: { 'Exact saved name': { runs: 1, approved: 1 } } },
      corrections: { proposed: 1, taught: 1, byScope: { project: 1 } },
      recordingsByMode: { 'mode-id': 1 }, pasteLastUses: 1,
    });
    expect(getActivityInsights(stats, new Date(2026, 7, 1)).month.meetingMinutes).toBe(1.5);
    expect(getActivityInsights(stats, new Date(2026, 8, 1)).month.meetings).toBe(0);
  });

  it('accepts prototype-like private labels without inheriting or corrupting counters', () => {
    for (const name of ['__proto__', 'constructor', 'toString']) {
      updateStats('word', 1, name);
      updateStats('word', 1, name);
      updateActivityStats({ kind: 'transform', outcome: 'runs', presetName: name });
      updateActivityStats({ kind: 'transform', outcome: 'approved', presetName: name });
    }
    const stats = loadStats();
    for (const name of ['__proto__', 'constructor', 'toString']) {
      expect(stats.activity.recordingsByMode[name]).toBe(2);
      expect(stats.activity.transforms.byPresetName[name]).toEqual({ runs: 1, approved: 1, undone: 0 });
    }
    expect(Object.getPrototypeOf(stats.activity.recordingsByMode)).toBe(Object.prototype);
  });

  it('strips private content and invalid counters on both read and write', () => {
    const unsafe = {
      ...loadStats(),
      instruction: 'PRIVATE_INSTRUCTION',
      activity: {
        meetings: { count: -1, totalMinutes: Infinity, summariesGenerated: 2, summary: 'PRIVATE_SUMMARY' },
        transforms: { runs: 3, approved: 1, undone: 0, instruction: 'PRIVATE_INSTRUCTION', byPresetName: { SafeLabel: { runs: 3, approved: 1, undone: 0, body: 'PRIVATE_BODY' } } },
        corrections: { proposed: 2, taught: 1, byScope: { global: 1, app: 0, project: 0, root: 'PRIVATE_ROOT' }, text: 'PRIVATE_CORRECTION' },
        recordingsByMode: { SafeId: 1, Bad: -1 }, pasteLastUses: NaN,
        monthlyBuckets: { '2026-08': { meetings: 2, meetingMinutes: 3, runs: 0, approved: 0, undone: 0, text: 'PRIVATE_TEXT' }, 'PRIVATE_DATE': { meetings: 1, meetingMinutes: 0, runs: 0, approved: 0, undone: 0 } },
      },
    };
    localStorage.setItem(STATS_STORE.storageKey, JSON.stringify(unsafe));
    expect(JSON.stringify(loadStats())).not.toContain('PRIVATE');
    saveStats(unsafe);
    const serialized = localStorage.getItem(STATS_STORE.storageKey);
    expect(serialized).not.toContain('PRIVATE');
    expect(loadStats().activity.meetings).toEqual({ count: 0, totalMinutes: 0, summariesGenerated: 2 });
    expect(loadStats().activity.pasteLastUses).toBe(0);
  });

  it('states a fixed typing-rate assumption and never reports negative time saved', () => {
    const stats = { ...loadStats(), totalWords: 120, totalDurationSeconds: 60 };
    expect(getActivityInsights(stats).dictationMinutes).toBe(1);
    expect(getActivityInsights(stats).typingMinutesSaved).toBe(2);
    expect(getActivityInsights({ ...stats, totalDurationSeconds: 600 }).typingMinutesSaved).toBe(0);
  });

  it('removes every counter on reset and never logs malformed private labels', () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {});
    localStorage.setItem(STATS_STORE.storageKey, '{ PRIVATE_LABEL');
    expect(loadStats().activity.transforms.runs).toBe(0);
    expect(JSON.stringify(error.mock.calls)).not.toContain('PRIVATE_LABEL');
    updateStats('words', 1, 'LocalMode');
    updateActivityStats({ kind: 'paste_last' });
    resetStats();
    expect(localStorage.getItem(STATS_STORE.storageKey)).toBeNull();
    expect(loadStats().activity.pasteLastUses).toBe(0);
    expect(loadStats().activity.recordingsByMode).toEqual({});
    error.mockRestore();
  });
});
