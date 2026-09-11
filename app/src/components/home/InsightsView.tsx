import { useMemo } from 'react';
import { loadStats } from '../../lib/stats';
import { ASSUMED_TYPING_WPM, getActivityInsights } from '../../lib/activityStats';
import { BUILTIN_MODES, type MurmurMode } from '../../lib/settings';
import { getUsageOverview } from '../../lib/homeDashboard';
import { UsageDashboard } from '../UsageDashboard';
import { DashboardStatGroup, WorkspacePageHeader } from '../ui/DashboardPrimitives';

interface InsightsViewProps {
  statsVersion: number;
  modes?: readonly MurmurMode[];
  onBackToHome: () => void;
}

export function InsightsView({ statsVersion, modes = [], onBackToHome }: InsightsViewProps) {
  const stats = useMemo(() => loadStats(), [statsVersion]);
  const usage = useMemo(() => getUsageOverview(stats), [stats]);
  const activity = useMemo(() => getActivityInsights(stats), [stats]);
  const modeName = activity.mostUsedModeId === null ? 'None yet'
    : [...BUILTIN_MODES, ...modes].find((mode) => mode.id === activity.mostUsedModeId)?.name ?? 'Removed Mode';
  const minutes = (value: number) => value.toLocaleString(undefined, { maximumFractionDigits: 1 });

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
          { id: 'dictation-time', label: 'Total dictation time', value: `${minutes(activity.dictationMinutes)} min`, detail: 'all time' },
          { id: 'typing-time', label: 'Estimated typing time saved', value: `${minutes(activity.typingMinutesSaved)} min`, detail: `At ${ASSUMED_TYPING_WPM} typing wpm, minus dictation time` },
          { id: 'mode', label: 'Most-used Mode', value: modeName, detail: `${activity.mostUsedModeRecordings.toLocaleString()} recordings` },
          { id: 'meetings', label: 'Meetings this month', value: activity.month.meetings.toLocaleString(), detail: `${minutes(activity.month.meetingMinutes)} minutes` },
          { id: 'transforms', label: 'Transforms this month', value: activity.month.runs.toLocaleString(), detail: `${activity.approvalRate}% approved` },
          { id: 'most-used-transform', label: 'Most-used transform', value: activity.mostUsedTransformName ?? 'None yet', detail: `${activity.mostUsedTransformRuns.toLocaleString()} runs` },
          { id: 'corrections', label: 'Corrections taught', value: stats.activity.corrections.taught.toLocaleString(), detail: `${stats.activity.corrections.proposed.toLocaleString()} proposed` },
        ]}
      />

      <UsageDashboard statsVersion={statsVersion} displayMode="page" />
    </div>
  );
}
