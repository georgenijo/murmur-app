import { memo, useEffect, useRef, useState } from 'react';
import { getCurrentStreak, getWPM, loadStats, type DictationStats } from '../lib/stats';
import { UsageDashboard } from './UsageDashboard';

interface FooterStatsProps {
  statsVersion: number;
}

export const FooterStats = memo(function FooterStats({ statsVersion }: FooterStatsProps) {
  const [stats, setStats] = useState<DictationStats>(() => loadStats());
  const [open, setOpen] = useState(false);
  const shellRef = useRef<HTMLDivElement>(null);

  useEffect(() => setStats(loadStats()), [statsVersion]);
  useEffect(() => {
    if (!open) return;
    const close = (event: MouseEvent) => {
      if (!shellRef.current?.contains(event.target as Node)) setOpen(false);
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setOpen(false);
    };
    document.addEventListener('mousedown', close);
    document.addEventListener('keydown', escape);
    return () => {
      document.removeEventListener('mousedown', close);
      document.removeEventListener('keydown', escape);
    };
  }, [open]);

  const wpm = getWPM(stats);
  const streak = getCurrentStreak(stats);

  return (
    <footer className="relative flex h-8 shrink-0 items-center gap-1.5 border-t border-outline-variant/15 bg-background/95 px-3.5 text-xs text-on-surface-variant">
      <span><b className="font-bold text-on-surface">{stats.totalWords.toLocaleString()}</b> words</span>
      <span aria-hidden="true" className="text-outline-variant/60">·</span>
      <span><b className="font-bold text-on-surface">{wpm || '—'}</b> wpm</span>
      <span aria-hidden="true" className="text-outline-variant/60">·</span>
      <span><b className="font-bold text-on-surface">{stats.totalRecordings.toLocaleString()}</b> recordings</span>
      <span aria-hidden="true" className="text-outline-variant/60">·</span>
      <span><b className="font-bold text-on-surface">{streak}</b> day streak</span>

      <div ref={shellRef} className="relative ml-auto">
        <button
          type="button"
          onClick={() => setOpen((value) => !value)}
          aria-expanded={open}
          className="inline-flex items-center gap-1 rounded-md px-1.5 py-1 text-xs font-semibold uppercase tracking-[0.12em] text-on-surface-variant transition-colors hover:bg-surface-container-low hover:text-on-surface focus:outline-none focus-visible:ring-2 focus-visible:ring-primary"
        >
          Insights
          <svg className={`h-3 w-3 transition-transform ${open ? '' : 'rotate-180'}`} fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2.5} d="M5 15l7-7 7 7" />
          </svg>
        </button>
        {open && (
          <div className="absolute bottom-9 right-0 z-40 w-[min(420px,calc(100vw-32px))] rounded-2xl border border-outline-variant/25 bg-surface-container-lowest p-1 shadow-2xl">
            <div className="flex items-center justify-between px-3 pt-2">
              <p className="text-xs font-bold uppercase tracking-[0.12em] text-on-surface-variant">Usage insights</p>
              <button type="button" onClick={() => setOpen(false)} aria-label="Close insights" className="rounded-md p-1 text-on-surface-variant hover:bg-surface-container hover:text-on-surface">×</button>
            </div>
            <UsageDashboard statsVersion={statsVersion} displayMode="popover" />
          </div>
        )}
      </div>
    </footer>
  );
});
