import { useState, useEffect, useMemo } from 'react';
import {
  loadStats,
  getRecentDays,
  getHeatmapWeeks,
  getCurrentStreak,
} from '../lib/stats';
import { QUERY_PROVIDER_IDS, formatQueryCost } from '../lib/queryUsage';
import type { QueryProviderId } from '../lib/settings';
import { DashboardSectionHeader, DashboardSurface } from './ui/DashboardPrimitives';
import { DayChart } from './ui/DayChart';

const STORAGE_KEY = 'usage-dashboard-collapsed';
const HEATMAP_WEEKS = 8;
const RECENT_DAYS = 7;

const PROVIDER_LABELS: Record<QueryProviderId, string> = {
  claude: 'Claude',
  codex: 'Codex',
  grok: 'Grok',
  cursor: 'Cursor',
  custom: 'Custom',
};

function loadCollapsed(): boolean {
  try {
    return localStorage.getItem(STORAGE_KEY) === 'true';
  } catch {
    return false;
  }
}

function failureLabel(code: string): string {
  return code.split('_').join(' ');
}

interface UsageDashboardProps {
  // Bumped by App when a recording finishes (or stats reset) — forces a re-read.
  statsVersion: number;
  displayMode?: 'inline' | 'popover' | 'page';
}

export function UsageDashboard({ statsVersion, displayMode = 'inline' }: UsageDashboardProps) {
  const [isCollapsed, setIsCollapsed] = useState(loadCollapsed);
  const [version, setVersion] = useState(0);
  const expanded = displayMode !== 'inline' || !isCollapsed;

  useEffect(() => {
    const onStorage = (e: StorageEvent) => {
      if (e.key === 'dictation-stats') setVersion(v => v + 1);
    };
    window.addEventListener('storage', onStorage);
    return () => window.removeEventListener('storage', onStorage);
  }, []);

  const stats = useMemo(
    () => (expanded ? loadStats() : null),
    [expanded, version, statsVersion],
  );

  const streak = stats ? getCurrentStreak(stats) : 0;
  const weeks = stats ? getHeatmapWeeks(stats, HEATMAP_WEEKS) : [];
  const recent = stats ? getRecentDays(stats, RECENT_DAYS) : [];

  const activeQueryProviders = stats
    ? QUERY_PROVIDER_IDS.filter(provider => stats.query.byProvider[provider].queriesRun > 0)
    : [];
  const queryFailures = stats
    ? Object.entries(stats.query.failuresByErrorCode).filter(([, count]) => (count ?? 0) > 0)
    : [];

  const toggle = () => {
    const next = !isCollapsed;
    setIsCollapsed(next);
    try { localStorage.setItem(STORAGE_KEY, String(next)); } catch {}
  };

  return (
    <DashboardSurface variant={displayMode === 'page' ? 'outlined' : 'flat'}>
      <div className="usage-dashboard-content" data-display-mode={displayMode}>
      {displayMode === 'inline' && (
        <button
          onClick={toggle}
          className="w-full flex items-center justify-between px-3 py-2 text-left hover:bg-surface-container transition-colors"
        >
          <span className="text-xs font-medium text-on-surface-variant uppercase tracking-wider">
            Insights
          </span>
          <div className="flex items-center gap-3">
            <span className="text-xs text-on-surface-variant">
              <span className="text-primary font-medium">Streak</span>
              {' '}{streak} {streak === 1 ? 'day' : 'days'}
            </span>
            <svg
              className={`w-3.5 h-3.5 text-on-surface-variant transition-transform duration-200 ${isCollapsed ? 'rotate-180' : ''}`}
              fill="none"
              viewBox="0 0 24 24"
              stroke="currentColor"
              strokeWidth={2.5}
            >
              <path strokeLinecap="round" strokeLinejoin="round" d="M5 15l7-7 7 7" />
            </svg>
          </div>
        </button>
      )}

      {expanded && stats && (
        <div className="usage-dashboard-sections">
          <Section title="Voice Query · all time" kind="query">
            <dl className="usage-query-metrics" aria-label="Voice Query totals">
              {[
                ['Queries', stats.query.queriesRun.toLocaleString()],
                ['Tokens in', stats.query.inputTokens.toLocaleString()],
                ['Tokens out', stats.query.outputTokens.toLocaleString()],
              ].map(([label, value]) => (
                <div key={label}>
                  <dt>{label}</dt>
                  <dd>{value}</dd>
                </div>
              ))}
            </dl>
            {stats.query.reportedCostUsd > 0 && (
              <p className="usage-query-note" data-query-note="cost">
                Provider-reported cost · {formatQueryCost(stats.query.reportedCostUsd)}
              </p>
            )}
            {activeQueryProviders.length > 0 && (
              <div className="usage-query-providers" role="table" aria-label="Voice Query providers">
                <div className="usage-query-provider-header" role="row">
                  <span role="columnheader">Provider</span>
                  <span role="columnheader">Queries</span>
                  <span role="columnheader">In</span>
                  <span role="columnheader">Out</span>
                </div>
                {activeQueryProviders.map(provider => {
                  const providerStats = stats.query.byProvider[provider];
                  return (
                    <div key={provider} role="row" data-provider={provider}>
                      <span role="cell">{PROVIDER_LABELS[provider]}</span>
                      <span role="cell">{providerStats.queriesRun.toLocaleString()}</span>
                      <span role="cell">{providerStats.inputTokens.toLocaleString()}</span>
                      <span role="cell">{providerStats.outputTokens.toLocaleString()}</span>
                    </div>
                  );
                })}
              </div>
            )}
            {queryFailures.length > 0 && (
              <p className="usage-query-note" data-query-note="failures">
                Failures · {queryFailures.map(([code, count]) => `${failureLabel(code)} ${count}`).join(' · ')}
              </p>
            )}
            <p className="usage-query-note" data-query-note="privacy">
              Content-free counters only; questions and answers are never stored here.
            </p>
          </Section>

          <Section title={`Activity · last ${HEATMAP_WEEKS} weeks`} kind="activity">
            <DayChart kind="heatmap" metric="words" weeks={weeks} ariaLabel="Words per day heatmap" />
          </Section>

          <Section title="Words per day · last 7 days" kind="words">
            <DayChart kind="bars" metric="words" days={recent} ariaLabel="Words per day bar chart" />
          </Section>

          <Section title="WPM trend · last 7 days" kind="wpm">
            <DayChart kind="line" metric="wpm" days={recent} ariaLabel="Words-per-minute trend line" />
          </Section>
        </div>
      )}
      </div>
    </DashboardSurface>
  );
}

function Section({ title, kind, children }: {
  title: string;
  kind: 'query' | 'activity' | 'words' | 'wpm';
  children: React.ReactNode;
}) {
  return (
    <section className="usage-analytics-section" data-analytics={kind}>
      <DashboardSectionHeader eyebrow={title} />
      {children}
    </section>
  );
}
