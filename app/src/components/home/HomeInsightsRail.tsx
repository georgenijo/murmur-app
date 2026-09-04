import type { CSSProperties } from 'react';
import { getRecentDays, type DictationStats } from '../../lib/stats';
import { getUsageOverview, type PersonalizationSummary } from '../../lib/homeDashboard';
import type { HistoryEntry } from '../../lib/history';
import ActivityGraph, { type ActivityGraphDatum } from '../ui/activity-graph/activity-graph';
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
  historyEntries: HistoryEntry[];
  personalization: PersonalizationSummary;
  onOpenInsights: () => void;
  onOpenVocabulary: () => void;
  onOpenStyles: () => void;
}

const ACTIVITY_WINDOW_DAYS = 56;
const MIN_ACTIVITY_DAYS = 14;

interface HistoryActivity {
  data: ActivityGraphDatum[];
  startDate: string;
  endDate: string;
}

interface HomeActivityStyle extends CSSProperties {
  '--activity-graph-empty': string;
  '--activity-graph-level-1': string;
  '--activity-graph-level-2': string;
  '--activity-graph-level-3': string;
  '--activity-graph-level-4': string;
  '--activity-graph-cell-size': string;
  '--activity-graph-cell-gap': string;
  '--activity-graph-cell-radius': string;
}

const HOME_ACTIVITY_STYLE: HomeActivityStyle = {
  '--activity-graph-empty': 'var(--ui-chart-0)',
  '--activity-graph-level-1': 'var(--ui-chart-1)',
  '--activity-graph-level-2': 'var(--ui-chart-2)',
  '--activity-graph-level-3': 'var(--ui-chart-3)',
  '--activity-graph-level-4': 'var(--ui-chart-4)',
  '--activity-graph-cell-size': '0.6875rem',
  '--activity-graph-cell-gap': '0.1875rem',
  '--activity-graph-cell-radius': '0.1875rem',
};

function localDayKey(date: Date): string {
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, '0');
  const day = String(date.getDate()).padStart(2, '0');
  return `${year}-${month}-${day}`;
}

function shiftedLocalDay(date: Date, offset: number): Date {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate() + offset);
}

export function buildHistoryActivity(
  entries: HistoryEntry[],
  referenceDate = new Date(),
): HistoryActivity | null {
  const endDate = localDayKey(referenceDate);
  const startDate = localDayKey(shiftedLocalDay(referenceDate, -(ACTIVITY_WINDOW_DAYS - 1)));
  const counts = new Map<string, number>();

  for (const entry of entries) {
    const key = localDayKey(new Date(entry.timestamp));
    if (key < startDate || key > endDate) continue;
    counts.set(key, (counts.get(key) ?? 0) + 1);
  }

  if (counts.size < MIN_ACTIVITY_DAYS) return null;

  const data = Array.from(counts.entries())
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([date, value]) => ({
      date,
      value,
      label: `${value} ${value === 1 ? 'dictation' : 'dictations'}`,
    }));
  return { data, startDate, endDate };
}

export function HomeInsightsRail({
  stats,
  historyEntries,
  personalization,
  onOpenInsights,
  onOpenVocabulary,
  onOpenStyles,
}: HomeInsightsRailProps) {
  const usage = getUsageOverview(stats);
  const recent = getRecentDays(stats, 7);
  const activity = buildHistoryActivity(historyEntries);

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

      {activity && (
        <DashboardSurface as="section" variant="outlined" padding="standard" ariaLabel="Recent dictation activity">
          <DashboardSectionHeader eyebrow="Last 8 weeks" />
          <ActivityGraph
            className="home-activity-graph"
            data={activity.data}
            startDate={activity.startDate}
            endDate={activity.endDate}
            maxDays={ACTIVITY_WINDOW_DAYS}
            levels={4}
            weekStartsOn={1}
            showValue={false}
            showLegend={false}
            showWeekdayLabels={false}
            showMonthLabels
            showTooltip
            ariaLabel="Dictations per day over the last eight weeks"
            style={HOME_ACTIVITY_STYLE}
          />
        </DashboardSurface>
      )}

      <PersonalizationCard
        summary={personalization}
        onOpenVocabulary={onOpenVocabulary}
        onOpenStyles={onOpenStyles}
      />
    </aside>
  );
}
