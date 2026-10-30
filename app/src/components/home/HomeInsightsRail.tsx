import type { DictationStats } from '../../lib/stats';
import { getUsageOverview } from '../../lib/homeDashboard';
import { DashboardAction } from '../ui/DashboardPrimitives';

interface HomeInsightsRailProps {
  stats: DictationStats;
  onOpenInsights: () => void;
}

/** Month summary rendered in the recording composer's footer strip. */
export function HomeInsightsRail({ stats, onOpenInsights }: HomeInsightsRailProps) {
  const usage = getUsageOverview(stats);

  return (
    <aside className="home-month-stat" aria-label="Usage summary">
      <p className="home-month-stat-value">
        <span className="home-month-stat-label">This month</span>
        <strong>{usage.wordsThisMonth.toLocaleString()}</strong> words · <strong>{usage.recordingsThisMonth.toLocaleString()}</strong> {usage.recordingsThisMonth === 1 ? 'dictation' : 'dictations'}
      </p>
      <DashboardAction variant="quiet" icon="forward" onActivate={onOpenInsights}>
        View insights
      </DashboardAction>
    </aside>
  );
}
