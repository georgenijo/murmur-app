import { useMemo } from 'react';
import { loadStats } from '../../lib/stats';
import { getUsageOverview } from '../../lib/homeDashboard';
import { UsageDashboard } from '../UsageDashboard';
import { DashboardStatGroup, WorkspacePageHeader } from '../ui/DashboardPrimitives';

interface InsightsViewProps {
  statsVersion: number;
  onBackToHome: () => void;
}

export function InsightsView({ statsVersion, onBackToHome }: InsightsViewProps) {
  const stats = useMemo(() => loadStats(), [statsVersion]);
  const usage = useMemo(() => getUsageOverview(stats), [stats]);

  return (
    <div className="insights-view">
      <WorkspacePageHeader
        title="Insights"
        titleId="insights-view-title"
        description="Your usage patterns, computed on this Mac and never uploaded."
        back={{ label: 'Back to Home', onActivate: onBackToHome }}
      />

      <DashboardStatGroup
        kind="tiles"
        ariaLabel="Usage totals"
        items={[
          { id: 'words', label: 'Total words', value: usage.totalWords.toLocaleString(), detail: 'all time' },
          { id: 'wpm', label: 'Average speed', value: usage.averageWpm || '—', detail: 'wpm' },
          { id: 'recordings', label: 'Recordings', value: usage.totalRecordings.toLocaleString(), detail: `${usage.recordingsThisMonth.toLocaleString()} this month` },
          { id: 'streak', label: 'Current streak', value: usage.currentStreak, detail: usage.currentStreak === 1 ? 'day' : 'days' },
        ]}
      />

      <UsageDashboard statsVersion={statsVersion} displayMode="page" />
    </div>
  );
}
