import { useMemo } from 'react';
import { ChevronRight } from 'lucide-react';
import { loadStats } from '../../lib/stats';
import { ASSUMED_TYPING_WPM, getActivityInsights } from '../../lib/activityStats';
import { getUsageOverview } from '../../lib/homeDashboard';
import { UsageDashboard } from '../UsageDashboard';
import { DashboardStatGroup, WorkspacePageHeader, type DashboardStatItem } from '../ui/DashboardPrimitives';
import '../../styles/insights.css';

interface InsightsViewProps {
  statsVersion: number;
  initialMetric?: 'words' | 'wpm';
  defaultMoreOpen?: boolean;
  onBackToHome: () => void;
}

export function InsightsView({ statsVersion, onBackToHome, initialMetric = 'words', defaultMoreOpen = false }: InsightsViewProps) {
  const stats = useMemo(() => loadStats(), [statsVersion]);
  const usage = useMemo(() => getUsageOverview(stats), [stats]);
  const activity = useMemo(() => getActivityInsights(stats), [stats]);
  const minutes = (value: number) => value.toLocaleString(undefined, { maximumFractionDigits: 1 });
  const unit = (value: string | number, suffix: string) => <>{value}<span className="insights-stat-unit">{suffix}</span></>;
  const samples = stats.wpmSamples.length;
  const details: DashboardStatItem[] = [];
  if (Math.round(activity.dictationMinutes * 10) > 0) details.push({ id: 'dictation-time', label: 'Total dictation time', value: `${minutes(activity.dictationMinutes)} min`, detail: '· All time' });
  if (activity.month.meetings > 0) details.push({ id: 'meetings', label: 'Meetings this month', value: activity.month.meetings.toLocaleString(), detail: `· ${minutes(activity.month.meetingMinutes)} minutes` });

  return (
    <div className="insights-view">
      <WorkspacePageHeader
        title="Insights"
        titleId="insights-view-title"
        description="Your usage patterns, computed on this Mac and never uploaded"
        back={{ label: 'Back to Home', onActivate: onBackToHome }}
      />
      <DashboardStatGroup
        kind="headline"
        ariaLabel="Headline usage statistics"
        items={[
          { id: 'words', label: 'Total words', value: usage.totalWords.toLocaleString(), detail: 'All time' },
          { id: 'wpm', label: 'Average speed', value: samples ? unit(usage.averageWpm, 'wpm') : '–', detail: samples === 0 ? usage.totalRecordings > 0 ? 'No speed samples yet' : 'No dictations yet' : samples === 1 ? 'Last dictation' : `Last ${samples} dictations` },
          { id: 'recordings', label: 'Dictations', value: usage.totalRecordings.toLocaleString(), detail: 'All time' },
          { id: 'typing-time', label: 'Estimated typing time saved', value: usage.totalRecordings ? unit(Math.round(activity.typingMinutesSaved).toLocaleString(), 'min') : '–', detail: `All time · vs typing at ${ASSUMED_TYPING_WPM} wpm` },
        ]}
      />
      <UsageDashboard statsVersion={statsVersion} displayMode="page" initialMetric={initialMetric} />
      {details.length > 0 && (
        <details className="insights-more-statistics" open={defaultMoreOpen || undefined}>
          <summary><ChevronRight aria-hidden="true" />More statistics</summary>
          <DashboardStatGroup kind="details" ariaLabel="More usage statistics" items={details} />
        </details>
      )}
    </div>
  );
}
