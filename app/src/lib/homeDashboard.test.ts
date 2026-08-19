import { describe, expect, it } from 'vitest';
import { DEFAULT_SETTINGS } from './settings';
import { derivePersonalization, getUsageOverview } from './homeDashboard';
import { loadStats, type DictationStats } from './stats';

function stats(overrides: Partial<DictationStats> = {}): DictationStats {
  return { ...loadStats(), ...overrides };
}

describe('home dashboard derivations', () => {
  it('separates all-time totals from calendar-month activity', () => {
    const result = getUsageOverview(stats({
      totalWords: 4000,
      totalRecordings: 80,
      wpmSamples: [120, 180],
      dailyBuckets: {
        '2026-08-01': { words: 100, recordings: 2, recordingSeconds: 60 },
        '2026-08-03': { words: 200, recordings: 3, recordingSeconds: 60 },
        '2026-07-31': { words: 900, recordings: 9, recordingSeconds: 60 },
      },
    }), new Date(2026, 7, 18));

    expect(result.totalWords).toBe(4000);
    expect(result.totalRecordings).toBe(80);
    expect(result.averageWpm).toBe(150);
    expect(result.activeDaysThisMonth).toBe(2);
    expect(result.wordsThisMonth).toBe(300);
    expect(result.recordingsThisMonth).toBe(5);
  });

  it('uses explicit local milestones instead of a percentage score', () => {
    const result = derivePersonalization({
      ...DEFAULT_SETTINGS,
      vocabularyEntries: [{
        id: 'term',
        written: 'Tauri',
        aliases: ['Tori'],
        enabled: true,
        scope: { kind: 'global' },
      }],
      appProfiles: [],
    }, stats({
      dailyBuckets: {
        '2026-08-01': { words: 10, recordings: 1, recordingSeconds: 2 },
      },
    }), new Date(2026, 7, 18));

    expect(result.stage).toBe('Developing');
    expect(result.completed).toBe(1);
    expect(result.total).toBe(3);
    expect(result.milestones.map((milestone) => milestone.complete)).toEqual([true, false, false]);
    expect(result.nextAction).toContain('app style');
    expect(JSON.stringify(result)).not.toContain('%');
  });

  it('reaches Personalized only when every visible milestone is complete', () => {
    const dailyBuckets = Object.fromEntries(
      [1, 2, 3, 4, 5].map((day) => [
        `2026-08-0${day}`,
        { words: 10, recordings: 1, recordingSeconds: 2 },
      ]),
    );
    const result = derivePersonalization({
      ...DEFAULT_SETTINGS,
      vocabularyEntries: [{
        id: 'term',
        written: 'Murmur',
        aliases: [],
        enabled: true,
        scope: { kind: 'global' },
      }],
      appProfiles: [{
        bundleId: 'com.apple.TextEdit',
        label: 'TextEdit',
        writingStyle: 'polished',
        autoPasteOverride: null,
        cleanupOverride: null,
        smartFormattingOverride: null,
        cliFormattingOverride: null,
        ideContextEnabled: false,
        ideProjectRoots: [],
        queryContextExcluded: false,
      }],
    }, stats({ dailyBuckets }), new Date(2026, 7, 18));

    expect(result.stage).toBe('Personalized');
    expect(result.completed).toBe(3);
    expect(result.nextAction).toContain('all active');
  });
});
