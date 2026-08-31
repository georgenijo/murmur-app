import { getRecentDays, type DictationStats } from '../../lib/stats';
import { getUsageOverview, type PersonalizationSummary } from '../../lib/homeDashboard';
import {
  DashboardAction,
  DashboardSectionHeader,
  DashboardStatGroup,
  DashboardSurface,
} from '../ui/DashboardPrimitives';
import { DayChart } from '../ui/DayChart';
import { PersonalizationCard } from './PersonalizationCard';

interface HomeInsightsRailProps {
  stats: DictationStats;
  personalization: PersonalizationSummary;
  onOpenInsights: () => void;
  onOpenVocabulary: () => void;
  onOpenStyles: () => void;
}

export function HomeInsightsRail({
  stats,
  personalization,
  onOpenInsights,
  onOpenVocabulary,
  onOpenStyles,
}: HomeInsightsRailProps) {
  const usage = getUsageOverview(stats);
  const recent = getRecentDays(stats, 7);

  return (
    <aside className="home-insights-rail" aria-label="Usage summary">
      <DashboardSurface as="section" variant="outlined" padding="standard">
        <DashboardSectionHeader
          eyebrow="This month"
          action={(
            <DashboardAction variant="quiet" icon="forward" onActivate={onOpenInsights}>
              View insights
            </DashboardAction>
          )}
        />
        <DashboardStatGroup
          kind="rows"
          ariaLabel="Usage this month"
          items={[
            { id: 'words', label: 'Words', value: usage.wordsThisMonth.toLocaleString() },
            { id: 'wpm', label: 'Average WPM', value: usage.averageWpm || '—' },
            { id: 'recordings', label: 'Recordings', value: usage.recordingsThisMonth.toLocaleString() },
            { id: 'streak', label: 'Day streak', value: usage.currentStreak },
          ]}
        />
      </DashboardSurface>

      <DashboardSurface as="section" variant="outlined" padding="standard">
        <DashboardSectionHeader eyebrow="This week" />
        <DayChart
          kind="bars"
          metric="words"
          days={recent}
          density="compact"
          highlightLast
          ariaLabel="Words per day for the last seven days"
        />
      </DashboardSurface>

      <PersonalizationCard
        summary={personalization}
        onOpenVocabulary={onOpenVocabulary}
        onOpenStyles={onOpenStyles}
      />
    </aside>
  );
}
