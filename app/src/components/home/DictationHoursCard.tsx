import { useId, useMemo } from 'react';
import { formatHour, formatHourRange, getDictationHours, periodShare } from '../../lib/dictationHours';
import type { HistoryEntry } from '../../lib/history';
import AnimatedSwitch from '../ui/animated-switch/animated-switch';

interface DictationHoursCardProps {
  entries: readonly HistoryEntry[];
  enabled: boolean;
  onEnabledChange: (enabled: boolean) => void;
}

const AXIS_HOURS = [0, 6, 12, 18];

function plural(value: number, one: string, many: string): string {
  return `${value.toLocaleString()} ${value === 1 ? one : many}`;
}

export function DictationHoursCard({ entries, enabled, onEnabledChange }: DictationHoursCardProps) {
  const titleId = useId();
  const hours = useMemo(() => (enabled ? getDictationHours(entries) : null), [entries, enabled]);
  const state = !enabled ? 'off' : hours && hours.total > 0 ? 'on' : 'empty';
  const max = hours ? Math.max(1, ...hours.counts) : 1;
  const peak = hours?.peakHour ?? null;

  return (
    <section className="insights-hours" aria-labelledby={titleId} data-hours-state={state}>
      <div className="insights-hours-header">
        <h2 id={titleId}>When you dictate</h2>
        <AnimatedSwitch size="sm" aria-label="Show when you dictate" checked={enabled} onCheckedChange={onEnabledChange} />
      </div>
      {state === 'off' && (
        <p className="insights-hours-hint">Turn on to see the hours you dictate most, from the transcripts saved in History. Dictation itself is unchanged.</p>
      )}
      {state === 'empty' && (
        <p className="insights-hours-hint">Appears after your first spoken dictation is saved in History.</p>
      )}
      {state === 'on' && hours && peak !== null && (
        <>
          <p className="insights-hours-summary">
            Most active <strong>{formatHourRange(peak)}</strong>
            <small>· From {plural(hours.total, 'dictation', 'dictations')} in History</small>
          </p>
          <div
            className="insights-hours-plot"
            role="img"
            aria-label={`Dictations by hour of day. Busiest hour ${formatHourRange(peak)} with ${plural(hours.counts[peak], 'dictation', 'dictations')}.`}
          >
            <div className="insights-hours-bars" aria-hidden="true">
              {hours.counts.map((count, hour) => (
                <span
                  key={hour}
                  data-hour={hour}
                  data-peak={hour === peak || undefined}
                  title={`${formatHourRange(hour)}: ${plural(count, 'dictation', 'dictations')}`}
                  style={{ height: count > 0 ? `${Math.max(6, Math.round((count / max) * 100))}%` : undefined }}
                />
              ))}
            </div>
            <div className="insights-hours-axis" aria-hidden="true">
              {AXIS_HOURS.map((hour) => <span key={hour} style={{ gridColumnStart: hour + 1 }}>{formatHour(hour)}</span>)}
            </div>
          </div>
          <table className="ui-visually-hidden">
            <caption>Dictations by hour of day</caption>
            <thead><tr><th scope="col">Hour</th><th scope="col">Dictations</th></tr></thead>
            <tbody>
              {hours.counts.map((count, hour) => (
                <tr key={hour}><th scope="row">{formatHourRange(hour)}</th><td>{count}</td></tr>
              ))}
            </tbody>
          </table>
          <dl className="insights-hours-periods" aria-label="Share of dictations by time of day">
            {hours.periods.map((period) => (
              <div key={period.id} data-period={period.id} title={`${formatHour(period.startHour)} – ${formatHour(period.endHour)}`}>
                <dt>{period.label}</dt>
                <dd>{periodShare(period.count, hours.total)}%</dd>
              </div>
            ))}
          </dl>
        </>
      )}
    </section>
  );
}
