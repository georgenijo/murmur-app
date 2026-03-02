import { useState, useCallback } from 'react';
import type { AppEvent } from '../../lib/events';

interface MetricsViewProps {
  events: AppEvent[];
}

interface TranscriptionMetric {
  vad_ms: number;
  inference_ms: number;
  paste_ms: number;
  total_ms: number;
  index: number;
}

function extractMetrics(events: AppEvent[]): TranscriptionMetric[] {
  return events
    .filter(e =>
      e.stream === 'pipeline' &&
      e.summary === 'transcription complete' &&
      typeof e.data.vad_ms === 'number' &&
      typeof e.data.inference_ms === 'number' &&
      typeof e.data.paste_ms === 'number'
    )
    .slice(-20)
    .map((e, i) => ({
      vad_ms: e.data.vad_ms as number,
      inference_ms: e.data.inference_ms as number,
      paste_ms: e.data.paste_ms as number,
      total_ms: typeof e.data.total_ms === 'number'
        ? e.data.total_ms
        : (e.data.vad_ms as number) + (e.data.inference_ms as number) + (e.data.paste_ms as number),
      index: i,
    }));
}

function avg(values: number[]): number {
  if (values.length === 0) return 0;
  return Math.round(values.reduce((a, b) => a + b, 0) / values.length);
}

// --- Stat Card ---

interface StatCardProps {
  label: string;
  value: number;
  average: number;
  color: string;
  unit?: string;
}

function StatCard({ label, value, average, color, unit = 'ms' }: StatCardProps) {
  const diff = value - average;
  const trend = diff > average * 0.1 ? 'up' : diff < -average * 0.1 ? 'down' : 'flat';

  return (
    <div className="flex-1 rounded-lg border border-stone-200 dark:border-stone-700 p-3 min-w-0">
      <div className="flex items-center gap-2 mb-1">
        <span className="w-2 h-2 rounded-full shrink-0" style={{ background: color }} />
        <span className="text-[11px] text-stone-500 dark:text-stone-400 truncate">{label}</span>
      </div>
      <div className="flex items-baseline gap-1.5">
        <span className="text-lg font-semibold tabular-nums text-stone-900 dark:text-stone-100">
          {value}
        </span>
        <span className="text-[11px] text-stone-400 dark:text-stone-500">{unit}</span>
        <span className={`text-[10px] ml-auto ${
          trend === 'up' ? 'text-red-500' : trend === 'down' ? 'text-emerald-500' : 'text-stone-400 dark:text-stone-500'
        }`}>
          {trend === 'up' ? '\u25B2' : trend === 'down' ? '\u25BC' : '\u2014'} avg {average}{unit}
        </span>
      </div>
    </div>
  );
}

// --- Line Chart ---

const SERIES_CONFIG: Record<string, { color: string; label: string }> = {
  total:     { color: '#57534e', label: 'Total' },      // stone-600
  inference: { color: '#f59e0b', label: 'Inference' },   // amber-500
  vad:       { color: '#a8a29e', label: 'VAD' },         // stone-400
  paste:     { color: '#64748b', label: 'Paste' },       // slate-500
};

type SeriesKey = 'total' | 'inference' | 'vad' | 'paste';
const ALL_SERIES: SeriesKey[] = ['total', 'inference', 'vad', 'paste'];

interface Series {
  key: string;
  color: string;
  values: number[];
}

function LineChart({ series, height = 140 }: { series: Series[]; height?: number }) {
  const count = series[0]?.values.length ?? 0;
  if (count === 0) return null;

  const padding = { top: 12, right: 16, bottom: 24, left: 48 };
  const chartW = 700;
  const chartH = height;
  const innerW = chartW - padding.left - padding.right;
  const innerH = chartH - padding.top - padding.bottom;

  // Compute shared max across all series for this chart
  const allValues = series.flatMap(s => s.values);
  const maxVal = Math.max(...allValues, 1);
  // Nice round max for y-axis
  const magnitude = Math.pow(10, Math.floor(Math.log10(maxVal)));
  const niceMax = Math.ceil(maxVal / magnitude) * magnitude;

  const xStep = count > 1 ? innerW / (count - 1) : 0;
  const yScale = innerH / niceMax;

  // Y-axis ticks (0, mid, max)
  const yTicks = [0, Math.round(niceMax / 2), niceMax];

  function toX(i: number) {
    return padding.left + (count > 1 ? i * xStep : innerW / 2);
  }
  function toY(v: number) {
    return padding.top + innerH - v * yScale;
  }

  return (
    <svg viewBox={`0 0 ${chartW} ${chartH}`} className="w-full" preserveAspectRatio="xMidYMid meet">
      {/* Grid lines + y labels */}
      {yTicks.map(tick => (
        <g key={tick}>
          <line
            x1={padding.left} y1={toY(tick)}
            x2={chartW - padding.right} y2={toY(tick)}
            stroke="currentColor" strokeOpacity={0.08}
          />
          <text x={padding.left - 8} y={toY(tick) + 3.5} textAnchor="end" className="fill-stone-400 dark:fill-stone-500" fontSize="9">
            {tick}
          </text>
        </g>
      ))}

      {/* X-axis labels (transcription index) */}
      {Array.from({ length: count }, (_, i) => (
        <text key={i} x={toX(i)} y={chartH - 4} textAnchor="middle" className="fill-stone-300 dark:fill-stone-600" fontSize="8">
          {i + 1}
        </text>
      ))}

      {/* Lines + dots for each series */}
      {series.map(s => {
        // Build polyline path
        const points = s.values.map((v, i) => `${toX(i)},${toY(v)}`).join(' ');
        return (
          <g key={s.key}>
            <polyline
              points={points}
              fill="none"
              stroke={s.color}
              strokeWidth={1.5}
              strokeLinejoin="round"
              strokeLinecap="round"
            />
            {/* Dots */}
            {s.values.map((v, i) => (
              <circle key={i} cx={toX(i)} cy={toY(v)} r={2.5} fill={s.color} />
            ))}
          </g>
        );
      })}
    </svg>
  );
}

// --- Section Label ---

function SectionLabel({ children }: { children: React.ReactNode }) {
  return (
    <div className="text-[11px] font-medium text-stone-400 dark:text-stone-500 uppercase tracking-wider">
      {children}
    </div>
  );
}

// --- Main Component ---

export function MetricsView({ events }: MetricsViewProps) {
  const metrics = extractMetrics(events);
  const latest = metrics[metrics.length - 1];
  const [visible, setVisible] = useState<Set<SeriesKey>>(() => new Set(ALL_SERIES));

  const toggle = useCallback((key: SeriesKey) => {
    setVisible(prev => {
      const next = new Set(prev);
      // Don't allow hiding everything — keep at least one
      if (next.has(key) && next.size > 1) next.delete(key);
      else next.add(key);
      return next;
    });
  }, []);

  if (metrics.length === 0) {
    return (
      <div className="flex items-center justify-center h-48 text-stone-400 dark:text-stone-500 text-sm">
        No transcription data yet. Complete a recording to see metrics.
      </div>
    );
  }

  const seriesData: Record<SeriesKey, number[]> = {
    total: metrics.map(m => m.total_ms),
    inference: metrics.map(m => m.inference_ms),
    vad: metrics.map(m => m.vad_ms),
    paste: metrics.map(m => m.paste_ms),
  };

  const seriesValues: Record<SeriesKey, number> = latest ? {
    total: latest.total_ms,
    inference: latest.inference_ms,
    vad: latest.vad_ms,
    paste: latest.paste_ms,
  } : { total: 0, inference: 0, vad: 0, paste: 0 };

  // Split visible series into two charts by magnitude
  const upperKeys = (['total', 'inference'] as SeriesKey[]).filter(k => visible.has(k));
  const lowerKeys = (['vad', 'paste'] as SeriesKey[]).filter(k => visible.has(k));

  const toSeries = (keys: SeriesKey[]): Series[] =>
    keys.map(k => ({ key: k, color: SERIES_CONFIG[k].color, values: seriesData[k] }));

  return (
    <div className="flex flex-col gap-5 p-4">
      {/* Legend — click to toggle */}
      <div className="flex gap-2 justify-center">
        {ALL_SERIES.map(key => {
          const { color, label } = SERIES_CONFIG[key];
          const active = visible.has(key);
          return (
            <button
              key={key}
              type="button"
              aria-pressed={active}
              onClick={() => toggle(key)}
              className={`inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full text-xs font-medium transition-all ${
                active
                  ? 'ring-1 ring-stone-300 dark:ring-stone-600 text-stone-700 dark:text-stone-300'
                  : 'text-stone-400 dark:text-stone-600'
              }`}
            >
              <span
                className="w-3 h-0.5 rounded-full transition-opacity"
                style={{ background: color, opacity: active ? 1 : 0.3 }}
              />
              <span className={active ? '' : 'line-through'}>{label}</span>
            </button>
          );
        })}
      </div>

      {/* Stat Cards — only for visible series */}
      {latest && (
        <div className="flex gap-3">
          {ALL_SERIES.filter(k => visible.has(k)).map(key => (
            <StatCard
              key={key}
              label={SERIES_CONFIG[key].label}
              value={seriesValues[key]}
              average={avg(seriesData[key])}
              color={SERIES_CONFIG[key].color}
            />
          ))}
        </div>
      )}

      {/* Upper chart: Total + Inference */}
      {upperKeys.length > 0 && (
        <div>
          <SectionLabel>{upperKeys.map(k => SERIES_CONFIG[k].label).join(' & ')} (ms)</SectionLabel>
          <LineChart height={150} series={toSeries(upperKeys)} />
        </div>
      )}

      {/* Lower chart: VAD + Paste */}
      {lowerKeys.length > 0 && (
        <div>
          <SectionLabel>{lowerKeys.map(k => SERIES_CONFIG[k].label).join(' & ')} (ms)</SectionLabel>
          <LineChart height={120} series={toSeries(lowerKeys)} />
        </div>
      )}
    </div>
  );
}
