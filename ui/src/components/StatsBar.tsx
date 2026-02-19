import { useMemo } from 'react';
import { loadStats, getWPM, getApproxTokens } from '../lib/stats';

interface StatsBarProps {
  statsVersion: number;
}

export function StatsBar({ statsVersion }: StatsBarProps) {
  const stats = useMemo(() => loadStats(), [statsVersion]);
  const wpm = getWPM(stats);
  const tokens = getApproxTokens(stats);

  return (
    <div className="shrink-0 flex gap-2 px-4 py-2">
      <Chip label="Total Words" value={stats.totalWords.toLocaleString()} />
      <Chip label="Avg WPM" value={wpm > 0 ? wpm.toString() : '—'} />
      <Chip label="Recordings" value={stats.totalRecordings.toLocaleString()} />
      <Chip label="Approx Tokens" value={tokens > 0 ? tokens.toLocaleString() : '—'} />
    </div>
  );
}

function Chip({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex-1 flex flex-col items-center px-3 py-2 rounded-lg bg-stone-100 dark:bg-stone-800 border border-stone-200 dark:border-stone-700">
      <span className="text-xs font-semibold text-stone-800 dark:text-stone-100 tabular-nums">{value}</span>
      <span className="text-[10px] text-stone-500 dark:text-stone-400 mt-0.5 leading-none">{label}</span>
    </div>
  );
}
