import { useState, useEffect } from 'react';
import { loadMetrics, clearMetrics } from '../lib/metrics';
import type { TranscriptionMetric } from '../lib/metrics';

interface PerformanceMetricsProps {
  metricsVersion: number;
}

function formatTime(timestamp: number): string {
  return new Date(timestamp).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}

function computeRollingAverage(metrics: TranscriptionMetric[]): number {
  if (metrics.length === 0) return 0;
  const sum = metrics.reduce((acc, m) => acc + m.inferenceMs, 0);
  return sum / metrics.length;
}

export function PerformanceMetrics({ metricsVersion }: PerformanceMetricsProps) {
  const [metrics, setMetrics] = useState<TranscriptionMetric[]>(() => loadMetrics());
  const [confirmClear, setConfirmClear] = useState(false);

  useEffect(() => { setMetrics(loadMetrics()); }, [metricsVersion]);

  const handleClear = () => {
    if (confirmClear) {
      clearMetrics();
      setMetrics([]);
      setConfirmClear(false);
    } else {
      setConfirmClear(true);
      setTimeout(() => setConfirmClear(false), 3000);
    }
  };

  // Show last 20, newest first
  const recent = [...metrics].reverse().slice(0, 20);
  const rollingAvg = computeRollingAverage(metrics);

  if (recent.length === 0) {
    return (
      <div className="text-xs text-stone-500 dark:text-stone-400">
        No performance data yet. Metrics will appear after your first transcription.
      </div>
    );
  }

  return (
    <div className="space-y-2">
      {/* Summary */}
      <div className="flex gap-2 text-xs">
        <div className="flex-1 px-2 py-1.5 rounded bg-stone-100 dark:bg-stone-700 text-center">
          <div className="font-semibold text-stone-800 dark:text-stone-100 tabular-nums">
            {Math.round(rollingAvg)}ms
          </div>
          <div className="text-[10px] text-stone-500 dark:text-stone-400 leading-none mt-0.5">
            Avg Inference
          </div>
        </div>
        <div className="flex-1 px-2 py-1.5 rounded bg-stone-100 dark:bg-stone-700 text-center">
          <div className="font-semibold text-stone-800 dark:text-stone-100 tabular-nums">
            {metrics.length}
          </div>
          <div className="text-[10px] text-stone-500 dark:text-stone-400 leading-none mt-0.5">
            Total Runs
          </div>
        </div>
      </div>

      {/* Table */}
      <div className="overflow-x-auto">
        <table className="w-full text-xs">
          <thead>
            <tr className="text-stone-500 dark:text-stone-400 border-b border-stone-200 dark:border-stone-600">
              <th className="text-left py-1 pr-2 font-medium">Time</th>
              <th className="text-right py-1 px-2 font-medium">Rec</th>
              <th className="text-right py-1 px-2 font-medium">Inference</th>
              <th className="text-right py-1 px-2 font-medium">Words</th>
              <th className="text-left py-1 pl-2 font-medium">Model</th>
            </tr>
          </thead>
          <tbody>
            {recent.map((m) => {
              const isOutlier = rollingAvg > 0 && m.inferenceMs > rollingAvg * 2;
              return (
                <tr
                  key={m.id}
                  className={`border-b border-stone-100 dark:border-stone-700 ${
                    isOutlier ? 'bg-amber-50 dark:bg-amber-900/20' : ''
                  }`}
                >
                  <td className="py-1 pr-2 text-stone-600 dark:text-stone-300 tabular-nums">
                    {formatTime(m.timestamp)}
                  </td>
                  <td className="py-1 px-2 text-right text-stone-600 dark:text-stone-300 tabular-nums">
                    {m.recordingDurationSecs}s
                  </td>
                  <td className={`py-1 px-2 text-right tabular-nums ${
                    isOutlier
                      ? 'text-amber-700 dark:text-amber-400 font-semibold'
                      : 'text-stone-600 dark:text-stone-300'
                  }`}>
                    {m.inferenceMs}ms
                  </td>
                  <td className="py-1 px-2 text-right text-stone-600 dark:text-stone-300 tabular-nums">
                    {m.wordCount}
                  </td>
                  <td className="py-1 pl-2 text-stone-500 dark:text-stone-400 truncate max-w-[80px]">
                    {m.model}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>

      {/* Clear button */}
      <button
        onClick={handleClear}
        className={`w-full px-3 py-2 rounded-lg text-xs font-medium border transition-colors ${
          confirmClear
            ? 'border-red-400 dark:border-red-600 bg-red-50 dark:bg-red-900/20 text-red-700 dark:text-red-400 hover:bg-red-100 dark:hover:bg-red-900/40'
            : 'border-stone-300 dark:border-stone-600 bg-white dark:bg-stone-700 text-stone-700 dark:text-stone-300 hover:bg-stone-50 dark:hover:bg-stone-600'
        }`}
      >
        {confirmClear ? 'Confirm Clear' : 'Clear Metrics'}
      </button>
    </div>
  );
}
