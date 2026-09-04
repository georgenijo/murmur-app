import { useState, type CSSProperties } from 'react';
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
  getValue: (r: ResourceReading) => number | null,
  maxVal: number,
): string {
  if (readings.length === 0) return '';
  return readings
    .flatMap((r, i) => {
      const value = getValue(r);
      if (value === null) return [];
      const x = (i / (MAX_READINGS - 1)) * CHART_W;
      const y = (1 - value / maxVal) * CHART_H;
      return [`${x.toFixed(2)},${y.toFixed(2)}`];
    })
    .join(' ');
}

export function ResourceMonitor() {
  const [isCollapsed, setIsCollapsed] = useState(loadCollapsed);

  // Only poll when expanded — no background work when the chart is hidden.
  const readings = useResourceMonitor(!isCollapsed);

  const latest = readings[readings.length - 1];
  const cpuNow = latest?.host_cpu_percent == null
    ? '—'
    : latest.host_cpu_percent.toFixed(1);
  const memNow = latest?.rss_mb == null
    ? '—'
    : latest.rss_mb.toLocaleString();

  const maxMem = Math.max(
    ...readings.flatMap(r => r.rss_mb === null ? [] : [r.rss_mb]),
    1024,
  );

  const cpuPoints = toPolylinePoints(readings, r => r.host_cpu_percent, 100);
  const memPoints = toPolylinePoints(readings, r => r.rss_mb, maxMem);

  const toggle = () => {
    const next = !isCollapsed;
    setIsCollapsed(next);
    try { localStorage.setItem(STORAGE_KEY, String(next)); } catch { /* ignore */ }
  };

  return (
    // Semantic CSS vars keep SVG strokes synchronized with every appearance preset.
    <div
      className="dialog-card shrink-0 overflow-hidden"
      style={{
        '--cpu-stroke': 'var(--murmur-on-surface-variant)',
        '--mem-stroke': 'var(--murmur-warning)',
      } as CSSProperties}
    >
      {/* Header row */}
      <button
        onClick={toggle}
        className="w-full flex items-center justify-between px-3 py-2 text-left hover:bg-surface-container transition-colors"
      >
        <span className="dialog-eyebrow text-on-surface-variant">
          Resources
        </span>
        <div className="flex items-center gap-3">
            <span className="text-xs text-on-surface-variant">
              <span className="text-on-surface-variant font-medium">Host CPU</span>
            {' '}{cpuNow}{cpuNow === '—' ? '' : '%'}
          </span>
          <span className="text-xs text-on-surface-variant">
            <span className="text-primary font-medium">Murmur RSS</span>
            {' '}{memNow} MB
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
                className="text-outline-variant/30"
                strokeDasharray="2,2"
              />
            ))}
            {cpuPoints && (
              <polyline
                points={cpuPoints}
                fill="none"
                stroke="var(--cpu-stroke)"
                strokeWidth="1.2"
                strokeLinejoin="round"
                strokeLinecap="round"
              />
            )}
            {memPoints && (
              <polyline
                points={memPoints}
                fill="none"
                stroke="var(--mem-stroke)"
                strokeWidth="1.2"
                strokeLinejoin="round"
                strokeLinecap="round"
              />
            )}
          </svg>
          {/* Legend — swatches use the same CSS vars as the polylines */}
          <div className="flex gap-3 mt-1">
            <span className="flex items-center gap-1 text-xs text-on-surface-variant">
              <span className="inline-block w-2.5 h-0.5 rounded" style={{ background: 'var(--cpu-stroke)' }} />
              Host CPU %
            </span>
            <span className="flex items-center gap-1 text-xs text-on-surface-variant">
              <span className="inline-block w-2.5 h-0.5 rounded" style={{ background: 'var(--mem-stroke)' }} />
              Murmur RSS MB
            </span>
          </div>
        </div>
      )}
    </div>
  );
}
