import { useMemo } from 'react';
import type { Settings } from '../../lib/settings';
import { loadStats } from '../../lib/stats';
import { derivePersonalization, getUsageOverview } from '../../lib/homeDashboard';
import { UsageDashboard } from '../UsageDashboard';
import { PersonalizationCard } from './PersonalizationCard';

interface InsightsViewProps {
  statsVersion: number;
  settings: Settings;
  onOpenVocabulary: () => void;
  onOpenStyles: () => void;
}

export function InsightsView({ statsVersion, settings, onOpenVocabulary, onOpenStyles }: InsightsViewProps) {
  const stats = useMemo(() => loadStats(), [statsVersion]);
  const usage = useMemo(() => getUsageOverview(stats), [stats]);
  const personalization = useMemo(
    () => derivePersonalization(settings, stats),
    [settings.vocabularyEntries, settings.appProfiles, stats],
  );

  return (
    <div className="insights-view">
      <header className="insights-heading">
        <div>
          <h1>Insights</h1>
          <p>Your usage patterns, computed on this Mac and never uploaded.</p>
        </div>
      </header>

      <section className="insights-stat-grid" aria-label="Usage totals">
        <article><span>Total words</span><strong>{usage.totalWords.toLocaleString()}</strong><small>all time</small></article>
        <article><span>Average speed</span><strong>{usage.averageWpm || '—'}</strong><small>wpm</small></article>
        <article><span>Recordings</span><strong>{usage.totalRecordings.toLocaleString()}</strong><small>{usage.recordingsThisMonth.toLocaleString()} this month</small></article>
        <article><span>Current streak</span><strong>{usage.currentStreak}</strong><small>{usage.currentStreak === 1 ? 'day' : 'days'}</small></article>
      </section>

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
