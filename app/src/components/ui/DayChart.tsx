import { useCallback, useEffect, useId, useRef, useState, type CSSProperties, type FocusEvent, type MouseEvent } from 'react';
import type { DaySummary } from '../../lib/stats';
import { dayChartScale, hasSpeed, heatmapIntensity, speedPoints, speedSegments, type ChartScale } from '../../lib/dayChartScale';

type DayMetric = 'words' | 'recordings' | 'wpm';

interface DayChartBase {
  ariaLabel: string;
  density?: 'compact' | 'standard';
  presentation?: 'insights';
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
  if (metric === 'wpm') return hasSpeed(day) ? `${day.wpm} WPM` : 'No WPM data';
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

function DayAxis({ days, insights }: { days: readonly DaySummary[]; insights: boolean }) {
  return (
    <div className="ui-day-chart-axis" aria-hidden="true">
      {days.map((day) => (
        <span key={day.key}>{day.date.toLocaleDateString(undefined, { weekday: insights ? 'short' : 'narrow' })}</span>
      ))}
    </div>
  );
}

function ValueAxis({ scale, empty = false }: { scale: ChartScale; empty?: boolean }) {
  return <>
    <div className="insights-chart-grid" aria-hidden="true"><span /><span /><span /></div>
    <div className="insights-value-axis" aria-hidden="true">
      {scale.ticks.map((tick, index) => <span key={index}>{empty && index !== 2 ? '' : empty ? '0' : tick.toLocaleString()}</span>)}
    </div>
  </>;
}

function DailyValues({ days, metric, ariaLabel }: { days: readonly DaySummary[]; metric: DayMetric; ariaLabel: string }) {
  return <div className="sr-only insights-values-accessible"><table>
    <caption>{ariaLabel} daily values</caption>
    <thead><tr><th scope="col">Date</th><th scope="col">{metric === 'wpm' ? 'Words per minute' : metric === 'words' ? 'Words' : 'Recordings'}</th>{metric !== 'recordings' && <th scope="col">Recordings</th>}</tr></thead>
    <tbody>{days.map(day => <tr key={day.key}><th scope="row">{dayLabel(day)}</th><td>{metric === 'wpm' && !hasSpeed(day) ? 'No WPM data' : metricValue(day, metric).toLocaleString()}</td>{metric !== 'recordings' && <td>{day.recordings.toLocaleString()}</td>}</tr>)}</tbody>
  </table></div>;
}

function BarsChart({ days, metric, activeKey, tooltipId, highlightLast, markEvents, insights }: {
  days: readonly DaySummary[];
  metric: 'words' | 'recordings';
  activeKey: string | null;
  tooltipId: string;
  highlightLast: boolean;
  markEvents: ReturnType<typeof useActiveDay>['markEvents'];
  insights: boolean;
}) {
  const scale = dayChartScale(days, metric);
  const peak = insights ? scale.max : Math.max(1, ...days.map(day => metricValue(day, metric)));
  const empty = insights && !days.some(day => day.recordings > 0 || day[metric] > 0);
  return <>
    <div className="ui-day-chart-plot" data-plot="bars" data-empty={empty} data-axis-min={insights ? scale.min : undefined} data-axis-max={insights && !empty ? scale.max : undefined}>
      {insights && <ValueAxis scale={scale} empty={empty} />}
      {empty ? <p className="insights-empty-message"><span>Dictate something and your activity will show up here.</span></p> : days.map((day, index) => (
        <button key={day.key} type="button" className="ui-day-chart-bar" data-active={activeKey === day.key}
          data-highlighted={highlightLast && index === days.length - 1} aria-label={markLabel(day, metric)} aria-describedby={tooltipId} {...markEvents(day, metric)}>
          <span style={{ '--day-chart-value': `${(insights ? metricValue(day, metric) / peak : Math.max(0.08, metricValue(day, metric) / peak)) * 100}%` } as CSSProperties} />
        </button>
      ))}
    </div>
    <DayAxis days={days} insights={insights} />
  </>;
}

function LineChart({ days, activeKey, tooltipId, markEvents, insights }: {
  days: readonly DaySummary[];
  activeKey: string | null;
  tooltipId: string;
  markEvents: ReturnType<typeof useActiveDay>['markEvents'];
  insights: boolean;
}) {
  const scale = insights ? dayChartScale(days, 'wpm') : { min: 0, max: Math.max(1, ...days.filter(hasSpeed).map(day => day.wpm)), ticks: [] };
  const points = speedPoints(days, scale).map(point => insights || point.y === null ? point : { ...point, y: point.y * 0.84 + 8 });
  const empty = insights && !days.some(day => day.recordings > 0);
  const noSpeed = insights && !days.some(hasSpeed);
  return <>
    <div className="ui-day-chart-plot" data-plot="line" data-empty={empty} data-axis-min={insights ? scale.min : undefined} data-axis-max={insights && !empty ? scale.max : undefined}>
      {insights && <ValueAxis scale={scale} empty={empty} />}
      {empty ? <p className="insights-empty-message"><span>Dictate something and your activity will show up here.</span></p> : <>
        <svg viewBox="0 0 100 100" preserveAspectRatio="none" aria-hidden="true">
          {!insights && <line x1="0" y1="50" x2="100" y2="50" />}
          {speedSegments(points).map((segment, index) => <polyline key={index} points={segment.map(({ x, y }) => `${x},${y}`).join(' ')} />)}
        </svg>
        {noSpeed && <p className="insights-empty-message"><span>No speed data for these days.</span></p>}
        <div className="ui-day-chart-line-targets">
          {points.map(({ day, y }) => <button key={day.key} type="button" data-active={activeKey === day.key}
            aria-label={markLabel(day, 'wpm')} aria-describedby={tooltipId} {...markEvents(day, 'wpm')}>
            {y !== null && <span style={{ '--day-chart-point': `${y}%` } as CSSProperties} />}
          </button>)}
        </div>
      </>}
    </div>
    <DayAxis days={days} insights={insights} />
  </>;
}

export function DayChart(props: DayChartProps) {
  const tooltipId = useId();
  const insights = props.presentation === 'insights';
  const chartRef = useRef<HTMLElement>(null);
  const { active, dismiss, markEvents } = useActiveDay();
  const flatDays = props.kind === 'heatmap' ? props.weeks.flat() : props.days;
  const peak = Math.max(1, ...flatDays.map((day) => day.words));
  const scale = props.kind === 'line' && insights ? dayChartScale(props.days, 'wpm') : null;
  const accessibleLabel = scale
    ? !flatDays.some(day => day.recordings > 0) ? `${props.ariaLabel}: no dictations yet`
      : !flatDays.some(hasSpeed) ? `${props.ariaLabel}: no speed data for these days`
      : `${props.ariaLabel}; speed axis ${scale.min} to ${scale.max} words per minute; days without dictations are gaps`
    : props.ariaLabel;

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
    <figure ref={chartRef} className="ui-day-chart" data-density={props.density ?? 'standard'} aria-label={accessibleLabel}>
      <ChartTooltip active={active} id={tooltipId} />
      {props.kind === 'bars' ? (
        <BarsChart
          days={props.days}
          metric={props.metric}
          activeKey={active?.key ?? null}
          tooltipId={tooltipId}
          highlightLast={props.highlightLast ?? false}
          markEvents={markEvents}
          insights={insights}
        />
      ) : props.kind === 'line' ? (
        <LineChart
          days={props.days}
          activeKey={active?.key ?? null}
          tooltipId={tooltipId}
          markEvents={markEvents}
          insights={insights}
        />
      ) : (
        <>
        <div className="ui-day-chart-calendar">
          {!insights && flatDays.length > 0 && (
            <p className="ui-day-chart-date-range">
              {dayLabel(flatDays[0])} – {dayLabel(flatDays[flatDays.length - 1])}
            </p>
          )}
          <div className="ui-day-chart-months" aria-hidden="true">
            {props.weeks.map((week, index) => (
              <span key={index}>
                {week[0] && (index === 0 || week[0].date.getMonth() !== props.weeks[index - 1]?.[0]?.date.getMonth())
                  ? week[0].date.toLocaleDateString(undefined, { month: 'short' }) : ''}
              </span>
            ))}
          </div>
          <div className="ui-day-chart-weekdays" aria-hidden="true">
            {Array.from({ length: 7 }, (_, weekday) => (
              <span key={weekday}>{new Date(2026, 0, 4 + weekday).toLocaleDateString(undefined, { weekday: 'short' })}</span>
            ))}
          </div>
          <div className="ui-day-chart-heatmap" role="group" aria-label={props.ariaLabel}>
            {props.weeks.map((week, weekIndex) => week.map((day) => (
              <button
                key={day.key}
                type="button"
                style={{ gridColumn: weekIndex + 1, gridRow: day.date.getDay() + 1 }}
                data-intensity={heatmapIntensity(day.words, peak)}
                data-active={active?.key === day.key}
                aria-label={markLabel(day, 'words')}
                aria-describedby={tooltipId}
                {...markEvents(day, 'words')}
              />
            )))}
          </div>
        </div>
        {insights && <div className="insights-heatmap-legend" aria-label="Activity intensity: less to more">
          <span>Less</span>{[0, 1, 2, 3, 4].map(level => <i key={level} data-intensity={level} aria-hidden="true" style={{ background: `var(--ui-chart-${level})` }} />)}<span>More</span>
        </div>}
        {!insights && flatDays.length > 0 && (
          <details className="ui-day-chart-values">
            <summary>Daily values</summary>
            <div>
              <table>
                <caption className="sr-only">{props.ariaLabel} daily values</caption>
                <thead><tr><th scope="col">Date</th><th scope="col">Words</th><th scope="col">Recordings</th></tr></thead>
                <tbody>{flatDays.map((day) => (
                  <tr key={day.key}><th scope="row">{dayLabel(day)}</th><td>{day.words.toLocaleString()}</td><td>{day.recordings.toLocaleString()}</td></tr>
                ))}</tbody>
              </table>
            </div>
          </details>
        )}
        </>
      )}
      {insights && <DailyValues days={flatDays} metric={props.metric} ariaLabel={accessibleLabel} />}
    </figure>
  );
}
