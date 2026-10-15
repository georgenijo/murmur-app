import type { DictationStats } from '../../lib/stats';
import { getUsageOverview } from '../../lib/homeDashboard';
import { DashboardAction } from '../ui/DashboardPrimitives';

interface HomeInsightsRailProps {
  stats: DictationStats;
  onOpenInsights: () => void;
}

/** Compact month summary that shares the recording bar's row instead of taking a column. */
export function HomeInsightsRail({ stats, onOpenInsights }: HomeInsightsRailProps) {
  const usage = getUsageOverview(stats);

  return (
    <aside className="home-month-stat" aria-label="Usage summary">
      <div className="home-month-stat-text">
        <span className="dashboard-eyebrow">This month</span>
        <p className="home-month-stat-value">
          <strong>{usage.wordsThisMonth.toLocaleString()}</strong> words · {usage.recordingsThisMonth.toLocaleString()} {usage.recordingsThisMonth === 1 ? 'dictation' : 'dictations'}
        </p>
      </div>
      <DashboardAction variant="quiet" icon="forward" onActivate={onOpenInsights}>
        View insights
      </DashboardAction>
    </aside>
  );
}
