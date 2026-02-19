import { useState } from 'react';
import { useResourceMonitor, ResourceReading } from '../lib/hooks/useResourceMonitor';

const STORAGE_KEY = 'resource-monitor-collapsed';
const MAX_READINGS = 60;
const CHART_W = MAX_READINGS;
const CHART_H = 40;

function loadCollapsed(): boolean {
  try {
    return localStorage.getItem(STORAGE_KEY) === 'true';
  } catch {
    return false;
  }
}

function toPolylinePoints(
  readings: ResourceReading[],
  getValue: (r: ResourceReading) => number,
  maxVal: number,
): string {
  if (readings.length === 0) return '';
  return readings
    .map((r, i) => {
      const x = (i / (MAX_READINGS - 1)) * CHART_W;
      const y = (1 - getValue(r) / maxVal) * CHART_H;
      return `${x.toFixed(2)},${y.toFixed(2)}`;
    })
    .join(' ');
}

export function ResourceMonitor() {
  const [isCollapsed, setIsCollapsed] = useState(loadCollapsed);

  const readings = useResourceMonitor(true);

  const latest = readings[readings.length - 1];
  const cpuNow = latest ? latest.cpu_percent.toFixed(1) : '—';
  const memNow = latest ? latest.memory_mb.toLocaleString() : '—';

  const maxMem = Math.max(...readings.map(r => r.memory_mb), 1024);

  const cpuPoints = toPolylinePoints(readings, r => r.cpu_percent, 100);
  const memPoints = toPolylinePoints(readings, r => r.memory_mb, maxMem);

  const toggle = () => {
    const next = !isCollapsed;
    setIsCollapsed(next);
    try { localStorage.setItem(STORAGE_KEY, String(next)); } catch { /* ignore */ }
  };

  return (
    <div className="shrink-0 rounded-lg border border-stone-200 dark:border-stone-700 bg-stone-50 dark:bg-stone-800/50 overflow-hidden">
      {/* Header row */}
      <button
        onClick={toggle}
        className="w-full flex items-center justify-between px-3 py-2 text-left hover:bg-stone-100 dark:hover:bg-stone-700/50 transition-colors"
      >
        <span className="text-xs font-medium text-stone-500 dark:text-stone-400 uppercase tracking-wider">
          Resources
        </span>
        <div className="flex items-center gap-3">
          <span className="text-xs text-stone-500 dark:text-stone-400">
            <span className="text-stone-600 dark:text-stone-300 font-medium">CPU</span>
            {' '}{cpuNow}%
          </span>
          <span className="text-xs text-stone-500 dark:text-stone-400">
            <span className="text-amber-600 dark:text-amber-400 font-medium">MEM</span>
            {' '}{memNow} MB
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

      {/* Chart */}
      {!isCollapsed && (
        <div className="px-3 pb-3">
          <svg
            viewBox={`0 0 ${CHART_W} ${CHART_H}`}
            preserveAspectRatio="none"
            className="w-full h-14 rounded"
            style={{ background: 'transparent' }}
          >
            {/* Subtle grid lines at 25%, 50%, 75% */}
            {[0.25, 0.5, 0.75].map(p => (
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
            {cpuPoints && (
              <polyline
                points={cpuPoints}
                fill="none"
                stroke="#57534e"
                strokeWidth="1.2"
                strokeLinejoin="round"
                strokeLinecap="round"
              />
            )}
            {memPoints && (
              <polyline
                points={memPoints}
                fill="none"
                stroke="#f59e0b"
                strokeWidth="1.2"
                strokeLinejoin="round"
                strokeLinecap="round"
              />
            )}
          </svg>
          {/* Legend */}
          <div className="flex gap-3 mt-1">
            <span className="flex items-center gap-1 text-xs text-stone-500 dark:text-stone-400">
              <span className="inline-block w-2.5 h-0.5 bg-stone-600 dark:bg-stone-400 rounded" />
              CPU %
            </span>
            <span className="flex items-center gap-1 text-xs text-stone-500 dark:text-stone-400">
              <span className="inline-block w-2.5 h-0.5 bg-amber-500 rounded" />
              Memory MB
            </span>
          </div>
        </div>
      )}
    </div>
  );
}
