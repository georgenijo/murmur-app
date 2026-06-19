import { useState, useEffect, useMemo } from 'react';
import {
  loadStats,
  getRecentDays,
  getHeatmapWeeks,
  getCurrentStreak,
  type DaySummary,
} from '../lib/stats';

const STORAGE_KEY = 'usage-dashboard-collapsed';
const HEATMAP_WEEKS = 8;
const RECENT_DAYS = 7;

// Heatmap geometry (SVG user units; viewBox scales to container width).
const CELL = 12;
const GAP = 3;
const STEP = CELL + GAP;

// Bar/line chart geometry.
const CHART_W = 100;
const CHART_H = 40;

const DOW = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];

function loadCollapsed(): boolean {
  try {
    return localStorage.getItem(STORAGE_KEY) === 'true';
  } catch {
    return false;
  }
}

// GitHub-style intensity: 0 = empty, 1..4 scaled against the busiest day shown.
function intensity(words: number, max: number): number {
  if (words <= 0) return 0;
  if (max <= 0) return 1;
  const ratio = words / max;
  if (ratio > 0.66) return 4;
  if (ratio > 0.33) return 3;
  return 2;
}

const HEAT_FILL: Record<number, string> = {
  0: 'var(--heat-0)',
  1: 'var(--heat-1)',
  2: 'var(--heat-2)',
  3: 'var(--heat-3)',
  4: 'var(--heat-4)',
};

function shortDay(d: Date): string {
  return DOW[d.getDay()].slice(0, 1);
}

interface UsageDashboardProps {
  // Bumped by App when a recording finishes (or stats reset) — forces a re-read.
  statsVersion: number;
}

export function UsageDashboard({ statsVersion }: UsageDashboardProps) {
  const [isCollapsed, setIsCollapsed] = useState(loadCollapsed);
  const [version, setVersion] = useState(0);

  // Re-read stats whenever localStorage changes from another window/tab, and
  // when the panel is expanded so it reflects recordings made while collapsed.
  useEffect(() => {
    const onStorage = (e: StorageEvent) => {
      if (e.key === 'dictation-stats') setVersion(v => v + 1);
    };
    window.addEventListener('storage', onStorage);
    return () => window.removeEventListener('storage', onStorage);
  }, []);

  // Re-read on expand, on every storage bump, and when App signals new stats;
  // memoized so the derived charts don't recompute on unrelated re-renders.
  const stats = useMemo(
    () => (isCollapsed ? null : loadStats()),
    [isCollapsed, version, statsVersion],
  );

  const streak = stats ? getCurrentStreak(stats) : 0;
  const weeks = stats ? getHeatmapWeeks(stats, HEATMAP_WEEKS) : [];
  const recent = stats ? getRecentDays(stats, RECENT_DAYS) : [];

  const maxHeat = Math.max(0, ...weeks.flat().map(d => d.words));
  const maxWords = Math.max(1, ...recent.map(d => d.words));
  const maxWpm = Math.max(1, ...recent.map(d => d.wpm));

  const toggle = () => {
    const next = !isCollapsed;
    setIsCollapsed(next);
    if (!next) setVersion(v => v + 1); // refresh on expand
    try { localStorage.setItem(STORAGE_KEY, String(next)); } catch { /* ignore */ }
  };

  const heatW = HEATMAP_WEEKS * STEP - GAP;
  const heatH = 7 * STEP - GAP;

  return (
    // Stone/amber palette. CSS vars drive SVG fills/strokes so they track dark mode.
    <div className="shrink-0 rounded-lg border border-stone-200 dark:border-stone-700 bg-stone-50 dark:bg-stone-800/50 overflow-hidden [--heat-0:#e7e5e4] dark:[--heat-0:#292524] [--heat-1:#fde68a] dark:[--heat-1:#78350f] [--heat-2:#fcd34d] dark:[--heat-2:#b45309] [--heat-3:#f59e0b] dark:[--heat-3:#d97706] [--heat-4:#d97706] dark:[--heat-4:#fbbf24] [--bar-fill:#f59e0b] dark:[--bar-fill:#fbbf24] [--wpm-stroke:#57534e] dark:[--wpm-stroke:#a8a29e]">
      {/* Header row */}
      <button
        onClick={toggle}
        className="w-full flex items-center justify-between px-3 py-2 text-left hover:bg-stone-100 dark:hover:bg-stone-700/50 transition-colors"
      >
        <span className="text-xs font-medium text-stone-500 dark:text-stone-400 uppercase tracking-wider">
          Insights
        </span>
        <div className="flex items-center gap-3">
          <span className="text-xs text-stone-500 dark:text-stone-400">
            <span className="text-amber-600 dark:text-amber-400 font-medium">Streak</span>
            {' '}{streak} {streak === 1 ? 'day' : 'days'}
          </span>
          <svg
            className={`w-3.5 h-3.5 text-stone-400 transition-transform duration-200 ${isCollapsed ? 'rotate-180' : ''}`}
            fill="none"
            viewBox="0 0 24 24"
            stroke="currentColor"
            strokeWidth={2.5}
          >
            <path strokeLinecap="round" strokeLinejoin="round" d="M5 15l7-7 7 7" />
          </svg>
        </div>
      </button>

      {!isCollapsed && stats && (
        <div className="px-3 pb-3 flex flex-col gap-4">
          {/* Heatmap — last ~8 weeks of words/day */}
          <Section title={`Activity · last ${HEATMAP_WEEKS} weeks`}>
            <svg
              viewBox={`0 0 ${heatW} ${heatH}`}
              className="w-full max-w-[260px] h-auto"
              role="img"
              aria-label="Words per day heatmap"
            >
              {weeks.map((col, w) =>
                col.map((day, d) => (
                  <rect
                    key={day.key}
                    x={w * STEP}
                    y={d * STEP}
                    width={CELL}
                    height={CELL}
                    rx={2}
                    fill={HEAT_FILL[intensity(day.words, maxHeat)]}
                  >
                    <title>{`${day.key}: ${day.words} words, ${day.recordings} recordings`}</title>
                  </rect>
                )),
              )}
            </svg>
            <div className="flex items-center gap-1.5 mt-1.5 text-[10px] text-stone-500 dark:text-stone-400">
              <span>Less</span>
              {[0, 1, 2, 3, 4].map(level => (
                <span
                  key={level}
                  className="inline-block w-2.5 h-2.5 rounded-[2px]"
                  style={{ background: HEAT_FILL[level] }}
                />
              ))}
              <span>More</span>
            </div>
          </Section>

          {/* Words/day bar chart — last 7 days */}
          <Section title="Words per day · last 7 days">
            <svg
              viewBox={`0 0 ${CHART_W} ${CHART_H}`}
              preserveAspectRatio="none"
              className="w-full h-16"
              role="img"
              aria-label="Words per day bar chart"
            >
              {recent.map((day, i) => {
                const bw = CHART_W / RECENT_DAYS;
                const inset = bw * 0.18;
                const h = (day.words / maxWords) * CHART_H;
                return (
                  <rect
                    key={day.key}
                    x={i * bw + inset}
                    y={CHART_H - h}
                    width={bw - inset * 2}
                    height={h}
                    rx={1}
                    fill="var(--bar-fill)"
                  >
                    <title>{`${day.key}: ${day.words} words`}</title>
                  </rect>
                );
              })}
            </svg>
            <DayAxis days={recent} />
          </Section>

          {/* WPM trend line — last 7 days */}
          <Section title="WPM trend · last 7 days">
            <svg
              viewBox={`0 0 ${CHART_W} ${CHART_H}`}
              preserveAspectRatio="none"
              className="w-full h-16"
              role="img"
              aria-label="Words-per-minute trend line"
            >
              {[0.5].map(p => (
                <line
                  key={p}
                  x1={0} y1={CHART_H * (1 - p)}
                  x2={CHART_W} y2={CHART_H * (1 - p)}
                  stroke="currentColor"
                  strokeWidth="0.5"
                  className="text-stone-200 dark:text-stone-700"
                  strokeDasharray="2,2"
                />
              ))}
              <polyline
                points={wpmPoints(recent, maxWpm)}
                fill="none"
                stroke="var(--wpm-stroke)"
                strokeWidth="1.2"
                strokeLinejoin="round"
                strokeLinecap="round"
                vectorEffect="non-scaling-stroke"
              />
            </svg>
            <DayAxis days={recent} />
          </Section>
        </div>
      )}
    </div>
  );
}

function wpmPoints(days: DaySummary[], maxWpm: number): string {
  if (days.length === 0) return '';
  return days
    .map((day, i) => {
      const x = (i / (RECENT_DAYS - 1)) * CHART_W;
      const y = (1 - day.wpm / maxWpm) * CHART_H;
      return `${x.toFixed(2)},${y.toFixed(2)}`;
    })
    .join(' ');
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div>
      <div className="text-[10px] font-medium text-stone-400 dark:text-stone-500 uppercase tracking-wider mb-1.5">
        {title}
      </div>
      {children}
    </div>
  );
}

function DayAxis({ days }: { days: DaySummary[] }) {
  return (
    <div className="flex justify-between mt-1 px-0.5">
      {days.map(day => (
        <span key={day.key} className="text-[9px] text-stone-400 dark:text-stone-500 tabular-nums">
          {shortDay(day.date)}
        </span>
      ))}
    </div>
  );
}
