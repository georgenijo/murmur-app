import { useState, useEffect } from 'react';
import { loadStats, getWPM } from '../lib/stats';
import type { DictationStats } from '../lib/stats';

interface StatsBarProps {
  statsVersion: number;
}

export function StatsBar({ statsVersion }: StatsBarProps) {
  const [stats, setStats] = useState<DictationStats>(() => loadStats());
  useEffect(() => { setStats(loadStats()); }, [statsVersion]);
  const wpm = getWPM(stats);

  return (
    <div className="shrink-0 flex gap-2 px-4 py-1.5">
      <Chip label="Words" value={stats.totalWords.toLocaleString()} />
      <Chip label="Avg WPM" value={wpm > 0 ? wpm.toLocaleString() : '—'} />
      <Chip label="Recordings" value={stats.totalRecordings.toLocaleString()} />
    </div>
  );
}

function Chip({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex-1 flex items-center justify-center gap-1.5 px-2 py-1 rounded-lg bg-stone-100 dark:bg-stone-800 border border-stone-200 dark:border-stone-700">
      <span className="text-[10px] text-stone-500 dark:text-stone-400 leading-none">{label}</span>
      <span className="text-[11px] font-semibold text-stone-800 dark:text-stone-100 tabular-nums">{value}</span>
    </div>
  );
}
