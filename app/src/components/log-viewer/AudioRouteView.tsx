import type { AppEvent } from '../../lib/events';

interface AudioRouteViewProps {
  events: AppEvent[];
}

interface RoutePoint {
  timestamp: string;
  label: string;
  input: string;
  output: string;
  inputRate: number;
  outputRate: number;
  inputBluetooth: boolean;
  outputBluetooth: boolean;
  kind: 'snapshot' | 'capture' | 'warning' | 'focus';
}

function asString(value: unknown, fallback = 'unknown'): string {
  return typeof value === 'string' && value.length > 0 ? value : fallback;
}

function asNumber(value: unknown): number {
  return typeof value === 'number' ? value : 0;
}

function asBoolean(value: unknown): boolean {
  return value === true;
}

function timeLabel(timestamp: string): string {
  const date = new Date(timestamp);
  if (Number.isNaN(date.getTime())) return timestamp;
  return date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });
}

function extractRoutePoints(events: AppEvent[]): RoutePoint[] {
  return events
    .filter((event) =>
      event.stream === 'audio' &&
      (
        event.summary === 'audio route snapshot' ||
        event.summary === 'audio capture route selected' ||
        event.summary === 'audio capture stream started' ||
        event.summary === 'audio capture stream stopped' ||
        event.summary.includes('Bluetooth-like input selected')
      )
    )
    .slice(-60)
    .map((event) => {
      if (event.summary === 'audio capture stream started') {
        return {
          timestamp: event.timestamp,
          label: 'capture started',
          input: '',
          output: '',
          inputRate: 0,
          outputRate: 0,
          inputBluetooth: false,
          outputBluetooth: false,
          kind: 'capture' as const,
        };
      }
      if (event.summary === 'audio capture stream stopped') {
        return {
          timestamp: event.timestamp,
          label: 'capture stopped',
          input: '',
          output: '',
          inputRate: 0,
          outputRate: 0,
          inputBluetooth: false,
          outputBluetooth: false,
          kind: 'capture' as const,
        };
      }
      if (event.summary.includes('Bluetooth-like input selected')) {
        return {
          timestamp: event.timestamp,
          label: 'Bluetooth warning',
          input: '',
          output: '',
          inputRate: 0,
          outputRate: 0,
          inputBluetooth: true,
          outputBluetooth: false,
          kind: 'warning' as const,
        };
      }

      const label = event.summary === 'audio route snapshot'
        ? asString(event.data.reason, 'snapshot').replace(/_/g, ' ')
        : 'recording route';
      return {
        timestamp: event.timestamp,
        label,
        input: asString(event.data.input_device),
        output: asString(event.data.output_device),
        inputRate: asNumber(event.data.input_sample_rate ?? event.data.sample_rate),
        outputRate: asNumber(event.data.output_sample_rate),
        inputBluetooth: asBoolean(event.data.input_bluetooth_like),
        outputBluetooth: asBoolean(event.data.output_bluetooth_like),
        kind: event.summary === 'audio route snapshot' ? 'snapshot' : 'capture',
      };
    });
}

function latestRoute(points: RoutePoint[]): RoutePoint | undefined {
  return [...points].reverse().find((point) => point.input || point.output);
}

function Timeline({ points }: { points: RoutePoint[] }) {
  if (points.length === 0) return null;

  const width = 700;
  const height = 84;
  const padding = { left: 28, right: 28, top: 24, bottom: 24 };
  const innerW = width - padding.left - padding.right;
  const step = points.length > 1 ? innerW / (points.length - 1) : 0;

  const colorFor = (point: RoutePoint) => {
    if (point.kind === 'warning' || point.inputBluetooth || point.outputBluetooth) return '#f59e0b';
    if (point.label.includes('started')) return '#10b981';
    if (point.label.includes('stopped')) return '#64748b';
    return '#3b82f6';
  };

  return (
    <svg viewBox={`0 0 ${width} ${height}`} className="w-full" preserveAspectRatio="xMidYMid meet">
      <line
        x1={padding.left}
        y1={height / 2}
        x2={width - padding.right}
        y2={height / 2}
        stroke="currentColor"
        strokeOpacity={0.12}
      />
      {points.map((point, index) => {
        const x = padding.left + (points.length > 1 ? index * step : innerW / 2);
        return (
          <g key={`${point.timestamp}-${index}`}>
            <circle cx={x} cy={height / 2} r={5} fill={colorFor(point)} />
            {(index === 0 || index === points.length - 1) && (
              <text x={x} y={height - 6} textAnchor="middle" className="fill-stone-400 dark:fill-stone-500" fontSize="9">
                {timeLabel(point.timestamp)}
              </text>
            )}
          </g>
        );
      })}
    </svg>
  );
}

export function AudioRouteView({ events }: AudioRouteViewProps) {
  const points = extractRoutePoints(events);
  const latest = latestRoute(points);
  const bluetoothActive = latest?.inputBluetooth || latest?.outputBluetooth;

  return (
    <div className="p-4 space-y-4">
      <div className="grid grid-cols-3 gap-3">
        <div className="rounded-lg border border-stone-200 dark:border-stone-700 p-3">
          <div className="text-[11px] text-stone-500 dark:text-stone-400 mb-1">Input</div>
          <div className="text-sm font-medium text-stone-900 dark:text-stone-100 truncate">
            {latest?.input || 'unknown'}
          </div>
          <div className="mt-1 text-xs text-stone-500 dark:text-stone-400">
            {latest?.inputRate ? `${latest.inputRate} Hz` : 'no rate'}
          </div>
        </div>
        <div className="rounded-lg border border-stone-200 dark:border-stone-700 p-3">
          <div className="text-[11px] text-stone-500 dark:text-stone-400 mb-1">Output</div>
          <div className="text-sm font-medium text-stone-900 dark:text-stone-100 truncate">
            {latest?.output || 'unknown'}
          </div>
          <div className="mt-1 text-xs text-stone-500 dark:text-stone-400">
            {latest?.outputRate ? `${latest.outputRate} Hz` : 'no rate'}
          </div>
        </div>
        <div className={`rounded-lg border p-3 ${
          bluetoothActive
            ? 'border-amber-300 dark:border-amber-700 bg-amber-50 dark:bg-amber-900/20'
            : 'border-stone-200 dark:border-stone-700'
        }`}>
          <div className="text-[11px] text-stone-500 dark:text-stone-400 mb-1">Route Risk</div>
          <div className={`text-sm font-medium ${
            bluetoothActive ? 'text-amber-700 dark:text-amber-300' : 'text-stone-900 dark:text-stone-100'
          }`}>
            {bluetoothActive ? 'Bluetooth call route likely' : 'No Bluetooth route detected'}
          </div>
          <div className="mt-1 text-xs text-stone-500 dark:text-stone-400">
            {points.length} route events
          </div>
        </div>
      </div>

      <div className="rounded-lg border border-stone-200 dark:border-stone-700 p-3">
        <div className="flex items-center justify-between mb-2">
          <h2 className="text-xs font-semibold text-stone-700 dark:text-stone-300">Audio Route Timeline</h2>
          <div className="flex items-center gap-3 text-[10px] text-stone-500 dark:text-stone-400">
            <span><span className="inline-block w-2 h-2 rounded-full bg-blue-500 mr-1" />snapshot</span>
            <span><span className="inline-block w-2 h-2 rounded-full bg-emerald-500 mr-1" />start</span>
            <span><span className="inline-block w-2 h-2 rounded-full bg-amber-500 mr-1" />Bluetooth</span>
          </div>
        </div>
        {points.length === 0 ? (
          <div className="flex items-center justify-center h-24 text-sm text-stone-400 dark:text-stone-500">
            No audio route events yet
          </div>
        ) : (
          <Timeline points={points} />
        )}
      </div>

      <div className="rounded-lg border border-stone-200 dark:border-stone-700 overflow-hidden">
        {points.length === 0 ? null : points.slice().reverse().map((point, index) => (
          <div
            key={`${point.timestamp}-${index}`}
            className="grid grid-cols-[84px_1fr_1fr_1fr] gap-3 px-3 py-2 text-xs border-b last:border-b-0 border-stone-100 dark:border-stone-800"
          >
            <span className="text-stone-400 dark:text-stone-500 tabular-nums">{timeLabel(point.timestamp)}</span>
            <span className="font-medium text-stone-700 dark:text-stone-300 truncate">{point.label}</span>
            <span className="text-stone-500 dark:text-stone-400 truncate">{point.input || '-'}</span>
            <span className="text-stone-500 dark:text-stone-400 truncate">{point.output || '-'}</span>
          </div>
        ))}
      </div>
    </div>
  );
}
