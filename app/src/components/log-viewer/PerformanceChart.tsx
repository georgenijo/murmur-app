import type { KeyboardEvent } from 'react';
import type { MeasurementV1, ResourceSampleV1 } from '../../lib/performance';

export interface ResourceChartSeries {
  key: string;
  label: string;
  color: string;
  measurement: (sample: ResourceSampleV1) => MeasurementV1<number>;
}

interface PerformanceChartProps {
  title: string;
  scope: string;
  unit: string;
  samples: ResourceSampleV1[];
  series: ResourceChartSeries[];
  selectedIndex: number;
  onSelectedIndexChange: (index: number) => void;
  formatValue: (value: number) => string;
}

const WIDTH = 720;
const HEIGHT = 190;
const PADDING = { top: 18, right: 18, bottom: 30, left: 52 };

function niceMaximum(maximum: number): number {
  if (maximum <= 0) return 1;
  const magnitude = 10 ** Math.floor(Math.log10(maximum));
  return Math.ceil((maximum * 1.08) / magnitude) * magnitude;
}

function measuredSegments(
  samples: ResourceSampleV1[],
  series: ResourceChartSeries,
): Array<Array<{ index: number; value: number }>> {
  const segments: Array<Array<{ index: number; value: number }>> = [];
  let current: Array<{ index: number; value: number }> = [];
  samples.forEach((sample, index) => {
    const measurement = series.measurement(sample);
    if (measurement.status === 'measured') {
      current.push({ index, value: measurement.value });
    } else if (current.length > 0) {
      segments.push(current);
      current = [];
    }
  });
  if (current.length > 0) segments.push(current);
  return segments;
}

export function PerformanceChart({
  title,
  scope,
  unit,
  samples,
  series,
  selectedIndex,
  onSelectedIndexChange,
  formatValue,
}: PerformanceChartProps) {
  const measured = series.flatMap(item =>
    samples.flatMap(sample => {
      const measurement = item.measurement(sample);
      return measurement.status === 'measured' ? [measurement.value] : [];
    }));
  const maximum = niceMaximum(Math.max(...measured, 0));
  const innerWidth = WIDTH - PADDING.left - PADDING.right;
  const innerHeight = HEIGHT - PADDING.top - PADDING.bottom;
  const denominator = Math.max(samples.length - 1, 1);
  const toX = (index: number) => PADDING.left + (index / denominator) * innerWidth;
  const toY = (value: number) =>
    PADDING.top + innerHeight - (Math.max(0, value) / maximum) * innerHeight;
  const cursorIndex = Math.min(Math.max(selectedIndex, 0), Math.max(samples.length - 1, 0));

  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (samples.length === 0) return;
    if (event.key === 'ArrowLeft') {
      event.preventDefault();
      onSelectedIndexChange(Math.max(0, cursorIndex - 1));
    } else if (event.key === 'ArrowRight') {
      event.preventDefault();
      onSelectedIndexChange(Math.min(samples.length - 1, cursorIndex + 1));
    } else if (event.key === 'Home') {
      event.preventDefault();
      onSelectedIndexChange(0);
    } else if (event.key === 'End') {
      event.preventDefault();
      onSelectedIndexChange(samples.length - 1);
    }
  };

  return (
    <section
      tabIndex={0}
      onKeyDown={handleKeyDown}
      aria-label={`${title}. ${scope}. Use Left and Right Arrow keys to move the shared timeline cursor.`}
      className="rounded-2xl border border-outline-variant/15 bg-surface-container-lowest p-4 shadow-sm outline-none focus-visible:ring-2 focus-visible:ring-primary"
    >
      <div className="mb-2 flex flex-wrap items-start justify-between gap-2">
        <div>
          <h3 className="text-sm font-semibold text-on-surface">{title}</h3>
          <p className="mt-0.5 text-[11px] text-on-surface-variant">{scope}</p>
        </div>
        <div className="flex flex-wrap justify-end gap-x-3 gap-y-1">
          {series.map(item => (
            <span key={item.key} className="inline-flex items-center gap-1.5 text-[11px] text-on-surface-variant">
              <span className="h-0.5 w-3 rounded-full" style={{ background: item.color }} />
              {item.label}
            </span>
          ))}
        </div>
      </div>

      {measured.length === 0 ? (
        <div className="flex h-36 items-center justify-center rounded-xl bg-surface-container-low text-sm text-on-surface-variant">
          No measured samples in this window
        </div>
      ) : (
        <svg
          viewBox={`0 0 ${WIDTH} ${HEIGHT}`}
          className="w-full"
          role="img"
          aria-label={`${title} over ${samples.length} typed samples, measured in ${unit}`}
        >
          <title>{title}</title>
          <desc>{scope}. Missing samples create gaps rather than zero values.</desc>
          {[0, maximum / 2, maximum].map(tick => (
            <g key={tick}>
              <line
                x1={PADDING.left}
                y1={toY(tick)}
                x2={WIDTH - PADDING.right}
                y2={toY(tick)}
                stroke="currentColor"
                strokeOpacity={0.08}
              />
              <text
                x={PADDING.left - 8}
                y={toY(tick) + 3}
                textAnchor="end"
                className="fill-on-surface-variant"
                fontSize="9"
              >
                {tick < 10 ? tick.toFixed(1) : Math.round(tick)}
              </text>
            </g>
          ))}
          {series.flatMap(item =>
            measuredSegments(samples, item).map((segment, segmentIndex) => (
              <g key={`${item.key}-${segmentIndex}`}>
                {segment.length === 1 ? (
                  <circle
                    cx={toX(segment[0].index)}
                    cy={toY(segment[0].value)}
                    r={2.5}
                    fill={item.color}
                  />
                ) : (
                  <polyline
                    points={segment.map(point => `${toX(point.index)},${toY(point.value)}`).join(' ')}
                    fill="none"
                    stroke={item.color}
                    strokeWidth={1.8}
                    strokeLinejoin="round"
                    strokeLinecap="round"
                  />
                )}
              </g>
            )))}
          {samples.length > 0 && (
            <line
              x1={toX(cursorIndex)}
              y1={PADDING.top}
              x2={toX(cursorIndex)}
              y2={PADDING.top + innerHeight}
              stroke="var(--murmur-primary)"
              strokeWidth={1}
              strokeDasharray="3 3"
              opacity={0.65}
            />
          )}
          <text x={PADDING.left} y={HEIGHT - 7} className="fill-on-surface-variant" fontSize="9">
            {new Date(samples[0].observedAtMs).toLocaleTimeString([], {
              hour: 'numeric',
              minute: '2-digit',
            })}
          </text>
          <text
            x={WIDTH - PADDING.right}
            y={HEIGHT - 7}
            textAnchor="end"
            className="fill-on-surface-variant"
            fontSize="9"
          >
            {new Date(samples[samples.length - 1].observedAtMs).toLocaleTimeString([], {
              hour: 'numeric',
              minute: '2-digit',
            })}
          </text>
        </svg>
      )}

      {samples[cursorIndex] && (
        <div className="mt-1 flex flex-wrap gap-x-4 gap-y-1 border-t border-outline-variant/10 pt-2">
          {series.map(item => {
            const measurement = item.measurement(samples[cursorIndex]);
            return (
              <span key={item.key} className="text-[11px] text-on-surface-variant">
                <span className="font-medium text-on-surface">{item.label}:</span>{' '}
                {measurement.status === 'measured'
                  ? formatValue(measurement.value)
                  : measurement.status === 'notApplicable' ? 'Not applicable' : 'Unavailable'}
              </span>
            );
          })}
        </div>
      )}
    </section>
  );
}
