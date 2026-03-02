import type { AppEvent } from '../../lib/events';

interface MetricsViewProps {
  events: AppEvent[];
}

interface TranscriptionMetric {
  vad_ms: number;
  inference_ms: number;
  paste_ms: number;
  total_ms: number;
  timestamp: string;
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
    .map(e => ({
      vad_ms: e.data.vad_ms as number,
      inference_ms: e.data.inference_ms as number,
      paste_ms: e.data.paste_ms as number,
      total_ms: (e.data.total_ms as number) || ((e.data.vad_ms as number) + (e.data.inference_ms as number) + (e.data.paste_ms as number)),
      timestamp: e.timestamp,
    }));
}

const BAR_COLORS = {
  vad:       { fill: '#a8a29e', label: 'VAD' },         // stone-400
  inference: { fill: '#fbbf24', label: 'Inference' },    // amber-400
  paste:     { fill: '#64748b', label: 'Paste' },        // slate-500
  outlier:   { fill: '#ef4444', label: 'Outlier' },      // red-500
};

function StackedBarChart({ metrics }: { metrics: TranscriptionMetric[] }) {
  if (metrics.length === 0) {
    return (
      <div className="flex items-center justify-center h-48 text-stone-400 dark:text-stone-500 text-sm">
        No transcription data yet. Complete a recording to see metrics.
      </div>
    );
  }

  const padding = { top: 20, right: 20, bottom: 40, left: 50 };
  const chartW = 700;
  const chartH = 250;
  const innerW = chartW - padding.left - padding.right;
  const innerH = chartH - padding.top - padding.bottom;

  const maxTotal = Math.max(...metrics.map(m => m.vad_ms + m.inference_ms + m.paste_ms), 100);
  const barW = Math.min(30, (innerW / metrics.length) * 0.7);
  const gap = (innerW - barW * metrics.length) / Math.max(metrics.length - 1, 1);

  // Rolling average for outlier detection
  const inferenceValues = metrics.map(m => m.inference_ms);
  const rollingAvg = inferenceValues.map((_, i) => {
    const start = Math.max(0, i - 4);
    const slice = inferenceValues.slice(start, i + 1);
    return slice.reduce((a, b) => a + b, 0) / slice.length;
  });

  // Y-axis ticks
  const yTicks = [0, Math.round(maxTotal / 2), maxTotal];

  return (
    <svg viewBox={`0 0 ${chartW} ${chartH}`} className="w-full" preserveAspectRatio="xMidYMid meet">
      {/* Y-axis */}
      {yTicks.map(tick => {
        const y = padding.top + innerH - (tick / maxTotal) * innerH;
        return (
          <g key={tick}>
            <line x1={padding.left} y1={y} x2={chartW - padding.right} y2={y} stroke="currentColor" strokeOpacity={0.1} />
            <text x={padding.left - 8} y={y + 4} textAnchor="end" className="fill-stone-400 dark:fill-stone-500" fontSize="10">
              {tick}ms
            </text>
          </g>
        );
      })}

      {/* Bars */}
      {metrics.map((m, i) => {
        const x = padding.left + i * (barW + gap);
        const total = m.vad_ms + m.inference_ms + m.paste_ms;
        const scale = innerH / maxTotal;

        const pasteH = m.paste_ms * scale;
        const inferenceH = m.inference_ms * scale;
        const vadH = m.vad_ms * scale;

        const isOutlier = i > 0 && m.inference_ms > rollingAvg[i] * 2;

        const baseY = padding.top + innerH;

        return (
          <g key={i}>
            {/* Paste (bottom) */}
            <rect x={x} y={baseY - pasteH} width={barW} height={Math.max(pasteH, 0.5)} fill={BAR_COLORS.paste.fill} rx={1} />
            {/* Inference (middle) */}
            <rect
              x={x}
              y={baseY - pasteH - inferenceH}
              width={barW}
              height={Math.max(inferenceH, 0.5)}
              fill={isOutlier ? BAR_COLORS.outlier.fill : BAR_COLORS.inference.fill}
              rx={1}
            />
            {/* VAD (top) */}
            <rect x={x} y={baseY - pasteH - inferenceH - vadH} width={barW} height={Math.max(vadH, 0.5)} fill={BAR_COLORS.vad.fill} rx={1} />

            {/* Outlier marker */}
            {isOutlier && (
              <text x={x + barW / 2} y={baseY - pasteH - inferenceH - vadH - 4} textAnchor="middle" fill={BAR_COLORS.outlier.fill} fontSize="10" fontWeight="bold">
                !
              </text>
            )}

            {/* Total label */}
            <text x={x + barW / 2} y={baseY + 14} textAnchor="middle" className="fill-stone-400 dark:fill-stone-500" fontSize="9">
              {total}
            </text>
          </g>
        );
      })}
    </svg>
  );
}

export function MetricsView({ events }: MetricsViewProps) {
  const metrics = extractMetrics(events);
  const latest = metrics[metrics.length - 1];

  return (
    <div className="flex flex-col gap-4 p-4">
      {/* Latest summary */}
      {latest && (
        <div className="text-xs text-stone-500 dark:text-stone-400">
          Last: VAD {latest.vad_ms}ms · Inference {latest.inference_ms}ms · Paste {latest.paste_ms}ms
        </div>
      )}

      {/* Legend */}
      <div className="flex gap-4 text-xs">
        <div className="flex items-center gap-1.5">
          <span className="w-3 h-2 rounded-sm" style={{ background: BAR_COLORS.vad.fill }} />
          <span className="text-stone-500 dark:text-stone-400">VAD</span>
        </div>
        <div className="flex items-center gap-1.5">
          <span className="w-3 h-2 rounded-sm" style={{ background: BAR_COLORS.inference.fill }} />
          <span className="text-stone-500 dark:text-stone-400">Inference</span>
        </div>
        <div className="flex items-center gap-1.5">
          <span className="w-3 h-2 rounded-sm" style={{ background: BAR_COLORS.paste.fill }} />
          <span className="text-stone-500 dark:text-stone-400">Paste</span>
        </div>
      </div>

      {/* Chart */}
      <StackedBarChart metrics={metrics} />
    </div>
  );
}
