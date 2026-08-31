import { useCallback, useEffect, useId, useRef, useState, type CSSProperties, type FocusEvent, type MouseEvent } from 'react';
import type { DaySummary } from '../../lib/stats';

type DayMetric = 'words' | 'recordings' | 'wpm';

interface DayChartBase {
  ariaLabel: string;
  density?: 'compact' | 'standard';
}

type DayChartProps =
  | (DayChartBase & {
      kind: 'bars';
      metric: 'words' | 'recordings';
      days: readonly DaySummary[];
      highlightLast?: boolean;
    })
  | (DayChartBase & {
      kind: 'line';
      metric: 'wpm';
      days: readonly DaySummary[];
    })
  | (DayChartBase & {
      kind: 'heatmap';
      metric: 'words';
      weeks: readonly (readonly DaySummary[])[];
    });

interface ActiveDay {
  key: string;
  day: DaySummary;
  metric: DayMetric;
}

function metricValue(day: DaySummary, metric: DayMetric): number {
  return Math.max(0, day[metric]);
}

function dayLabel(day: DaySummary): string {
  return day.date.toLocaleDateString(undefined, {
    month: 'short',
    day: 'numeric',
    year: 'numeric',
  });
}

function valueLabel(day: DaySummary, metric: DayMetric): string {
  if (metric === 'wpm') return day.wpm > 0 ? `${day.wpm} WPM` : 'No WPM data';
  const value = metricValue(day, metric);
  return `${value.toLocaleString()} ${metric}`;
}

function markLabel(day: DaySummary, metric: DayMetric): string {
  const recordings = `${day.recordings.toLocaleString()} ${day.recordings === 1 ? 'recording' : 'recordings'}`;
  return metric === 'recordings'
    ? `${dayLabel(day)}, ${recordings}`
    : `${dayLabel(day)}, ${valueLabel(day, metric)}, ${recordings}`;
}

function useActiveDay() {
  const [hovered, setHovered] = useState<ActiveDay | null>(null);
  const [focused, setFocused] = useState<ActiveDay | null>(null);
  const [selected, setSelected] = useState<ActiveDay | null>(null);
  const value = (day: DaySummary, metric: DayMetric): ActiveDay => ({ key: day.key, day, metric });
  const dismiss = useCallback(() => {
    setHovered(null);
    setFocused(null);
    setSelected(null);
  }, []);
  const markEvents = (day: DaySummary, metric: DayMetric) => ({
    onMouseEnter: (_event: MouseEvent) => setHovered(value(day, metric)),
    onMouseLeave: () => setHovered((current) => current?.key === day.key ? null : current),
    onFocus: (_event: FocusEvent) => setFocused(value(day, metric)),
    onBlur: () => {
      setFocused((current) => current?.key === day.key ? null : current);
      setSelected((current) => current?.key === day.key ? null : current);
    },
    onClick: () => setSelected(value(day, metric)),
  });
  return { active: hovered ?? focused ?? selected, dismiss, markEvents };
}

function ChartTooltip({ active, id }: { active: ActiveDay | null; id: string }) {
  return (
    <div id={id} className="ui-day-chart-tooltip" role="status" aria-live="polite">
      {active ? (
        <>
          <strong>{dayLabel(active.day)}</strong>
          <span>
            {valueLabel(active.day, active.metric)}
            {active.metric !== 'recordings' && (
              <> · {active.day.recordings.toLocaleString()} {active.day.recordings === 1 ? 'recording' : 'recordings'}</>
            )}
          </span>
        </>
      ) : (
        <span>Focus a day for exact values.</span>
      )}
    </div>
  );
}

function DayAxis({ days }: { days: readonly DaySummary[] }) {
  return (
    <div className="ui-day-chart-axis" aria-hidden="true">
      {days.map((day) => (
        <span key={day.key}>{day.date.toLocaleDateString(undefined, { weekday: 'narrow' })}</span>
      ))}
    </div>
  );
}

function BarsChart({
  days,
  metric,
  activeKey,
  tooltipId,
  highlightLast,
  markEvents,
}: {
  days: readonly DaySummary[];
  metric: 'words' | 'recordings';
  activeKey: string | null;
  tooltipId: string;
  highlightLast: boolean;
  markEvents: ReturnType<typeof useActiveDay>['markEvents'];
}) {
  const peak = Math.max(1, ...days.map((day) => metricValue(day, metric)));
  return (
    <>
      <div className="ui-day-chart-plot" data-plot="bars">
        {days.map((day, index) => {
          const value = metricValue(day, metric);
          return (
            <button
              key={day.key}
              type="button"
              className="ui-day-chart-bar"
              data-active={activeKey === day.key}
              data-highlighted={highlightLast && index === days.length - 1}
              aria-label={markLabel(day, metric)}
              aria-describedby={tooltipId}
              {...markEvents(day, metric)}
            >
              <span style={{ '--day-chart-value': `${Math.max(0.08, value / peak) * 100}%` } as CSSProperties} />
            </button>
          );
        })}
      </div>
      <DayAxis days={days} />
    </>
  );
}

function LineChart({
  days,
  activeKey,
  tooltipId,
  markEvents,
}: {
  days: readonly DaySummary[];
  activeKey: string | null;
  tooltipId: string;
  markEvents: ReturnType<typeof useActiveDay>['markEvents'];
}) {
  const peak = Math.max(1, ...days.map((day) => day.wpm));
  const points = days.map((day, index) => {
    const x = ((index + 0.5) / Math.max(1, days.length)) * 100;
    const y = 100 - (day.wpm / peak) * 84 - 8;
    return { day, x, y };
  });
  return (
    <>
      <div className="ui-day-chart-plot" data-plot="line">
        <svg viewBox="0 0 100 100" preserveAspectRatio="none" aria-hidden="true">
          <line x1="0" y1="50" x2="100" y2="50" />
          <polyline points={points.map(({ x, y }) => `${x},${y}`).join(' ')} />
        </svg>
        <div className="ui-day-chart-line-targets">
          {points.map(({ day, y }) => (
            <button
              key={day.key}
              type="button"
              data-active={activeKey === day.key}
              aria-label={markLabel(day, 'wpm')}
              aria-describedby={tooltipId}
              {...markEvents(day, 'wpm')}
            >
              <span style={{ '--day-chart-point': `${y}%` } as CSSProperties} />
            </button>
          ))}
        </div>
      </div>
      <DayAxis days={days} />
    </>
  );
}

function intensity(value: number, maximum: number): 0 | 1 | 2 | 3 | 4 {
  if (value <= 0) return 0;
  const ratio = value / Math.max(1, maximum);
  if (ratio > 0.75) return 4;
  if (ratio > 0.5) return 3;
  if (ratio > 0.25) return 2;
  return 1;
}

export function DayChart(props: DayChartProps) {
  const tooltipId = useId();
  const chartRef = useRef<HTMLElement>(null);
  const { active, dismiss, markEvents } = useActiveDay();
  const flatDays = props.kind === 'heatmap' ? props.weeks.flat() : props.days;
  const peak = Math.max(1, ...flatDays.map((day) => day.words));

  useEffect(() => {
    const onPointerDown = (event: PointerEvent) => {
      const target = event.target;
      const mark = target instanceof Element ? target.closest('button') : null;
      if (mark && chartRef.current?.contains(mark)) return;
      dismiss();
    };
    const onKeyDown = (event: globalThis.KeyboardEvent) => {
      if (event.key === 'Escape') dismiss();
    };
    document.addEventListener('pointerdown', onPointerDown);
    document.addEventListener('keydown', onKeyDown);
    return () => {
      document.removeEventListener('pointerdown', onPointerDown);
      document.removeEventListener('keydown', onKeyDown);
    };
  }, [dismiss]);

  return (
    <figure ref={chartRef} className="ui-day-chart" data-density={props.density ?? 'standard'} aria-label={props.ariaLabel}>
      <ChartTooltip active={active} id={tooltipId} />
      {props.kind === 'bars' ? (
        <BarsChart
          days={props.days}
          metric={props.metric}
          activeKey={active?.key ?? null}
          tooltipId={tooltipId}
          highlightLast={props.highlightLast ?? false}
          markEvents={markEvents}
        />
      ) : props.kind === 'line' ? (
        <LineChart
          days={props.days}
          activeKey={active?.key ?? null}
          tooltipId={tooltipId}
          markEvents={markEvents}
        />
      ) : (
        <div className="ui-day-chart-heatmap" role="group" aria-label={props.ariaLabel}>
          {props.weeks.map((week, weekIndex) => week.map((day) => (
            <button
              key={day.key}
              type="button"
              style={{ gridColumn: weekIndex + 1, gridRow: day.date.getDay() + 1 }}
              data-intensity={intensity(day.words, peak)}
              data-active={active?.key === day.key}
              aria-label={markLabel(day, 'words')}
              aria-describedby={tooltipId}
              {...markEvents(day, 'words')}
            />
          )))}
        </div>
      )}
    </figure>
  );
}
