import type { DictationStats } from '../../lib/stats';
import { getUsageOverview } from '../../lib/homeDashboard';
import { DashboardAction, DashboardSectionHeader, DashboardSurface } from '../ui/DashboardPrimitives';

interface HomeInsightsRailProps {
  stats: DictationStats;
  onOpenInsights: () => void;
}

export function HomeInsightsRail({ stats, onOpenInsights }: HomeInsightsRailProps) {
  const usage = getUsageOverview(stats);

  return (
    <aside className="home-insights-rail" aria-label="Usage summary">
      <DashboardSurface as="section" variant="outlined" padding="standard">
        <DashboardSectionHeader eyebrow="This month" />
        <p className="home-month-words">{usage.wordsThisMonth.toLocaleString()}</p>
        <p className="home-month-caption">
          words · {usage.recordingsThisMonth.toLocaleString()} {usage.recordingsThisMonth === 1 ? 'dictation' : 'dictations'}
        </p>
        <DashboardAction variant="quiet" icon="forward" onActivate={onOpenInsights}>
          View insights
        </DashboardAction>
      </DashboardSurface>
    </aside>
  );
}
