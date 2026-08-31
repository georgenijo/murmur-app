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

  // Re-read stats whenever localStorage changes from another window/tab, and
  // when the panel is expanded so it reflects recordings made while collapsed.
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
    try { localStorage.setItem(STORAGE_KEY, String(next)); } catch { /* ignore */ }
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
        <div className={`usage-dashboard-sections px-3 pb-3 flex flex-col gap-4 ${displayMode !== 'inline' ? 'pt-3' : ''}`}>
          <Section title="Voice Query · all time">
            <div className="grid grid-cols-3 gap-2">
              <QueryMetric label="Queries" value={stats.query.queriesRun.toLocaleString()} />
              <QueryMetric label="Tokens in" value={stats.query.inputTokens.toLocaleString()} />
              <QueryMetric label="Tokens out" value={stats.query.outputTokens.toLocaleString()} />
            </div>
            {stats.query.reportedCostUsd > 0 && (
              <p className="mt-1.5 text-[10px] text-on-surface-variant">
                Provider-reported cost · {formatQueryCost(stats.query.reportedCostUsd)}
              </p>
            )}
            {activeQueryProviders.length > 0 && (
              <div className="mt-2 space-y-1 border-t border-outline-variant/20 pt-2">
                {activeQueryProviders.map(provider => {
                  const providerStats = stats.query.byProvider[provider];
                  return (
                    <div key={provider} className="flex items-center justify-between gap-3 text-[10px] text-on-surface-variant">
                      <span className="font-medium text-on-surface">{PROVIDER_LABELS[provider]}</span>
                      <span className="tabular-nums">
                        {providerStats.queriesRun.toLocaleString()} queries · {providerStats.inputTokens.toLocaleString()} in · {providerStats.outputTokens.toLocaleString()} out
                      </span>
                    </div>
                  );
                })}
              </div>
            )}
            {queryFailures.length > 0 && (
              <p className="mt-2 text-[10px] leading-relaxed text-on-surface-variant">
                Failures · {queryFailures.map(([code, count]) => `${failureLabel(code)} ${count}`).join(' · ')}
              </p>
            )}
            <p className="mt-1.5 text-[9px] leading-relaxed text-on-surface-variant/75">
              Content-free counters only; questions and answers are never stored here.
            </p>
          </Section>

          <Section title={`Activity · last ${HEATMAP_WEEKS} weeks`}>
            <DayChart kind="heatmap" metric="words" weeks={weeks} ariaLabel="Words per day heatmap" />
          </Section>

          <Section title="Words per day · last 7 days">
            <DayChart kind="bars" metric="words" days={recent} ariaLabel="Words per day bar chart" />
          </Section>

          <Section title="WPM trend · last 7 days">
            <DayChart kind="line" metric="wpm" days={recent} ariaLabel="Words-per-minute trend line" />
          </Section>
        </div>
      )}
      </div>
    </DashboardSurface>
  );
}

function QueryMetric({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-md bg-surface-container px-2 py-1.5">
      <div className="text-[9px] uppercase tracking-wider text-on-surface-variant">{label}</div>
      <div className="mt-0.5 text-xs font-semibold tabular-nums text-on-surface">{value}</div>
    </div>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div>
      <DashboardSectionHeader eyebrow={title} />
      {children}
    </div>
  );
}
