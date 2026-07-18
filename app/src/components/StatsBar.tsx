import { useState, useEffect } from 'react';
import { loadStats, getWPM, getApproxTokens } from '../lib/stats';
import type { DictationStats } from '../lib/stats';

interface StatsBarProps {
  statsVersion: number;
}

export function StatsBar({ statsVersion }: StatsBarProps) {
  const [stats, setStats] = useState<DictationStats>(() => loadStats());
  useEffect(() => { setStats(loadStats()); }, [statsVersion]);
  const wpm = getWPM(stats);
  const tokens = getApproxTokens(stats);

  return (
    <div className="grid shrink-0 grid-cols-4 gap-2 bg-background px-4 py-3">
      <StatCard label="Total Words" value={stats.totalWords.toLocaleString()} icon="words" />
      <StatCard label="Avg WPM" value={wpm > 0 ? wpm.toLocaleString() : '—'} icon="speed" />
      <StatCard label="Recordings" value={stats.totalRecordings.toLocaleString()} icon="recordings" />
      <StatCard label="Approx Tokens" value={tokens > 0 ? tokens.toLocaleString() : '—'} icon="tokens" />
    </div>
  );
}

type StatIcon = 'words' | 'speed' | 'recordings' | 'tokens';

function StatIconGraphic({ icon }: { icon: StatIcon }) {
  if (icon === 'speed') {
    return <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.8} d="M4 15a8 8 0 1116 0M12 15l4-4M7 18h10" />;
  }
  if (icon === 'recordings') {
    return <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.8} d="M12 15a3 3 0 003-3V6a3 3 0 10-6 0v6a3 3 0 003 3zm5-3a5 5 0 01-10 0m5 5v3m-3 0h6" />;
  }
  if (icon === 'tokens') {
    return <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.8} d="M8 7h8M8 12h5M8 17h8M5 4h14a1 1 0 011 1v14a1 1 0 01-1 1H5a1 1 0 01-1-1V5a1 1 0 011-1z" />;
  }
  return <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.8} d="M5 6h14M5 10h14M5 14h9M5 18h11" />;
}

function StatCard({ label, value, icon }: { label: string; value: string; icon: StatIcon }) {
  return (
    <div className="flex min-w-0 items-center gap-2.5 rounded-xl bg-surface-container-low p-2.5 shadow-sm">
      <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-surface-container-lowest text-primary">
        <svg aria-hidden="true" className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <StatIconGraphic icon={icon} />
        </svg>
      </span>
      <span className="min-w-0">
        <span className="block truncate text-[10px] font-semibold uppercase tracking-wider text-on-surface-variant">{label}</span>
        <span className="mt-0.5 block truncate text-lg font-bold leading-none tabular-nums text-on-surface">{value}</span>
      </span>
    </div>
  );
}
