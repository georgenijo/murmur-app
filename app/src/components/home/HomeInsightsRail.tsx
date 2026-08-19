import { getRecentDays, type DictationStats } from '../../lib/stats';
import { getUsageOverview, recentWeekPeak, type PersonalizationSummary } from '../../lib/homeDashboard';
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
  const peak = recentWeekPeak(stats);

  return (
    <aside className="home-insights-rail" aria-label="Usage summary">
      <section className="dashboard-card usage-summary-card">
        <div className="dashboard-card-heading">
          <p className="dashboard-eyebrow">This month</p>
          <button type="button" onClick={onOpenInsights}>View insights</button>
        </div>
        <dl>
          <div><dt>Words</dt><dd>{usage.wordsThisMonth.toLocaleString()}</dd></div>
          <div><dt>Average WPM</dt><dd>{usage.averageWpm || '—'}</dd></div>
          <div><dt>Recordings</dt><dd>{usage.recordingsThisMonth.toLocaleString()}</dd></div>
          <div><dt>Day streak</dt><dd>{usage.currentStreak}</dd></div>
        </dl>
      </section>

      <section className="dashboard-card weekly-card">
        <p className="dashboard-eyebrow">This week</p>
        <div className="weekly-bars" role="img" aria-label="Words per day for the last seven days">
          {recent.map((day) => (
            <div key={day.key} className="weekly-bar">
              <span style={{ height: `${Math.max(4, Math.round((day.words / peak) * 48))}px` }} title={`${day.words} words`} />
              <small>{day.date.toLocaleDateString(undefined, { weekday: 'narrow' })}</small>
            </div>
          ))}
        </div>
      </section>

      <PersonalizationCard
        summary={personalization}
        onOpenVocabulary={onOpenVocabulary}
        onOpenStyles={onOpenStyles}
      />
    </aside>
  );
}
