import type { Settings } from './settings';
import {
  getCurrentStreak,
  getRecentDays,
  getWPM,
  type DictationStats,
} from './stats';

export type MainDestination = 'home' | 'meetings' | 'queries' | 'insights';

export interface UsageOverview {
  totalWords: number;
  totalRecordings: number;
  averageWpm: number;
  currentStreak: number;
  activeDaysThisMonth: number;
  wordsThisMonth: number;
  recordingsThisMonth: number;
}

export interface PersonalizationMilestone {
  id: 'vocabulary' | 'styles' | 'usage';
  label: string;
  detail: string;
  complete: boolean;
}

export interface PersonalizationSummary {
  stage: 'Learning' | 'Developing' | 'Personalized';
  completed: number;
  total: number;
  milestones: PersonalizationMilestone[];
  nextAction: string;
}

function isCurrentMonth(key: string, now: Date): boolean {
  const prefix = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, '0')}-`;
  return key.startsWith(prefix);
}

export function getUsageOverview(
  stats: DictationStats,
  now: Date = new Date(),
): UsageOverview {
  const monthBuckets = Object.entries(stats.dailyBuckets)
    .filter(([key]) => isCurrentMonth(key, now))
    .map(([, bucket]) => bucket);

  return {
    totalWords: stats.totalWords,
    totalRecordings: stats.totalRecordings,
    averageWpm: getWPM(stats),
    currentStreak: getCurrentStreak(stats),
    activeDaysThisMonth: monthBuckets.filter((bucket) => bucket.recordings > 0).length,
    wordsThisMonth: monthBuckets.reduce((sum, bucket) => sum + bucket.words, 0),
    recordingsThisMonth: monthBuckets.reduce((sum, bucket) => sum + bucket.recordings, 0),
  };
}

export function derivePersonalization(
  settings: Pick<Settings, 'vocabularyEntries' | 'appProfiles'>,
  stats: DictationStats,
  now: Date = new Date(),
): PersonalizationSummary {
  const preferredTerms = settings.vocabularyEntries.filter((entry) => entry.enabled).length;
  const styledApps = settings.appProfiles.filter((profile) => profile.writingStyle !== null).length;
  const activeDays = getUsageOverview(stats, now).activeDaysThisMonth;

  const milestones: PersonalizationMilestone[] = [
    {
      id: 'vocabulary',
      label: 'Preferred terms',
      detail: preferredTerms > 0
        ? `${preferredTerms.toLocaleString()} enabled`
        : 'Add a name or spelling',
      complete: preferredTerms > 0,
    },
    {
      id: 'styles',
      label: 'App styles',
      detail: styledApps > 0
        ? `${styledApps.toLocaleString()} configured`
        : 'Choose a style for an app',
      complete: styledApps > 0,
    },
    {
      id: 'usage',
      label: 'Regular use',
      detail: activeDays >= 5
        ? `${activeDays} active days this month`
        : `${activeDays} of 5 active days`,
      complete: activeDays >= 5,
    },
  ];
  const completed = milestones.filter((milestone) => milestone.complete).length;
  const next = milestones.find((milestone) => !milestone.complete);

  return {
    stage: completed === milestones.length
      ? 'Personalized'
      : completed === 0
        ? 'Learning'
        : 'Developing',
    completed,
    total: milestones.length,
    milestones,
    nextAction: next?.id === 'vocabulary'
      ? 'Add a preferred term so names and spellings stay consistent.'
      : next?.id === 'styles'
        ? 'Configure an app style so Murmur formats text for where you work.'
        : next?.id === 'usage'
          ? `Use Murmur on ${Math.max(0, 5 - activeDays)} more ${5 - activeDays === 1 ? 'day' : 'days'} this month to complete setup.`
          : 'Your configured terms, app styles, and usage setup are all active.',
  };
}

export function recentWeekPeak(stats: DictationStats): number {
  return Math.max(1, ...getRecentDays(stats, 7).map((day) => day.words));
}
