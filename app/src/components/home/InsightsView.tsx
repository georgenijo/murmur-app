import { useMemo } from 'react';
import type { Settings } from '../../lib/settings';
import { loadStats } from '../../lib/stats';
import { derivePersonalization, getUsageOverview } from '../../lib/homeDashboard';
import { UsageDashboard } from '../UsageDashboard';
import { DashboardStatGroup, WorkspacePageHeader } from '../ui/DashboardPrimitives';
import { PersonalizationCard } from './PersonalizationCard';

interface InsightsViewProps {
  statsVersion: number;
  settings: Settings;
  onBackToHome: () => void;
  onOpenVocabulary: () => void;
  onOpenStyles: () => void;
}

export function InsightsView({ statsVersion, settings, onBackToHome, onOpenVocabulary, onOpenStyles }: InsightsViewProps) {
  const stats = useMemo(() => loadStats(), [statsVersion]);
  const usage = useMemo(() => getUsageOverview(stats), [stats]);
  const personalization = useMemo(
    () => derivePersonalization(settings, stats),
    [settings.vocabularyEntries, settings.appProfiles, stats],
  );

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

      <div className="insights-content-grid">
        <UsageDashboard statsVersion={statsVersion} displayMode="page" />
        <PersonalizationCard
          summary={personalization}
          expanded
          onOpenVocabulary={onOpenVocabulary}
          onOpenStyles={onOpenStyles}
        />
      </div>
    </div>
  );
}
