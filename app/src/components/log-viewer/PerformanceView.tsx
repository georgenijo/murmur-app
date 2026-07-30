import { useEffect, useMemo, useState } from 'react';
import type { MeasurementV1, ResourceSampleV1 } from '../../lib/performance';
import type { PerformanceHealth } from '../../lib/hooks/usePerformanceHealth';
import {
  formatBytes,
  formatMeasurement,
  formatPercent,
  type FormattedMeasurement,
} from '../../lib/performancePresentation';
import { PerformanceChart, type ResourceChartSeries } from './PerformanceChart';

interface PerformanceViewProps {
  samples: ResourceSampleV1[];
  loading: boolean;
  error: string | null;
  health: PerformanceHealth;
  onRetry: () => void;
}

interface MetricCardProps {
  label: string;
  scope: string;
  value: FormattedMeasurement;
  accent?: string;
}

function MetricCard({ label, scope, value, accent = 'var(--murmur-primary)' }: MetricCardProps) {
  return (
    <div className="min-w-0 rounded-xl border border-outline-variant/10 bg-surface-container-lowest p-3 shadow-sm">
      <div className="flex items-center gap-2">
        <span className="h-2 w-2 shrink-0 rounded-full" style={{ background: accent }} />
        <span className="truncate text-[11px] font-medium text-on-surface-variant">{label}</span>
      </div>
      <div className={`mt-1 truncate text-lg font-semibold tabular-nums ${
        value.status === 'measured' ? 'text-on-surface' : 'text-on-surface-variant'
      }`}>
        {value.text}
      </div>
      <div className="mt-1 min-h-4 text-[10px] leading-4 text-on-surface-variant">
        {value.detail ? `${scope} · ${value.detail}` : scope}
      </div>
    </div>
  );
}

function textValue(text: string | null, loading: boolean, unavailable = 'Unavailable'): FormattedMeasurement {
  if (text) return { text, status: 'measured' };
  if (loading) return { text: 'Loading…', status: 'unavailable' };
  return { text: unavailable, status: 'unavailable' };
}

function pipelineLabel(health: PerformanceHealth): string | null {
  if (health.transformStatus && health.transformStatus !== 'idle') {
    return `Transform · ${health.transformStatus.replace('_', ' ')}`;
  }
  if (health.dictationStatus) {
    return `Dictation · ${health.dictationStatus}`;
  }
  return null;
}

function currentMeasurement(
  sample: ResourceSampleV1 | undefined,
  select: (value: ResourceSampleV1) => MeasurementV1<number>,
  format: (value: number) => string,
): FormattedMeasurement {
  if (!sample) return { text: 'Unavailable', status: 'unavailable', detail: 'No samples' };
  return formatMeasurement(select(sample), format);
}

const CPU_SERIES: ResourceChartSeries[] = [
  {
    key: 'host-cpu',
    label: 'Host CPU',
    color: 'var(--murmur-primary)',
    measurement: sample => sample.host.cpuPercent,
  },
  {
    key: 'main-cpu',
    label: 'Murmur CPU',
    color: '#d97706',
    measurement: sample => sample.mainProcess.cpuPercent,
  },
  {
    key: 'sidecar-cpu',
    label: 'Sidecar CPU',
    color: '#7c3aed',
    measurement: sample => sample.sidecarProcess.cpuPercent,
  },
];

const MEMORY_SERIES: ResourceChartSeries[] = [
  {
    key: 'main-rss',
    label: 'Main RSS',
    color: 'var(--murmur-error)',
    measurement: sample => sample.mainProcess.rssBytes,
  },
  {
    key: 'rust-heap',
    label: 'Rust heap',
    color: '#d97706',
    measurement: sample => sample.mainProcess.rustHeapBytes,
  },
  {
    key: 'ffi-heap',
    label: 'FFI / native heap',
    color: '#2563eb',
    measurement: sample => sample.mainProcess.ffiNativeHeapBytes,
  },
  {
    key: 'sidecar-rss',
    label: 'Sidecar RSS',
    color: '#7c3aed',
    measurement: sample => sample.sidecarProcess.rssBytes,
  },
];

export function PerformanceView({
  samples,
  loading,
  error,
  health,
  onRetry,
}: PerformanceViewProps) {
  const [selectedIndex, setSelectedIndex] = useState(Math.max(samples.length - 1, 0));

  useEffect(() => {
    setSelectedIndex(current => {
      if (samples.length === 0) return 0;
      const latest = samples.length - 1;
      return current >= samples.length - 2 ? latest : Math.min(current, latest);
    });
  }, [samples.length]);

  const selected = samples[selectedIndex];
  const runtime = health.runtime;
  const healthCards = useMemo(() => [
    {
      label: 'Pipeline state',
      scope: 'Current local pipeline',
      value: textValue(pipelineLabel(health), health.loading),
    },
    {
      label: 'Active model',
      scope: 'Configured dictation model',
      value: textValue(runtime?.label ?? health.modelName, health.loading),
    },
    {
      label: 'Backend',
      scope: 'Current transcription backend',
      value: textValue(runtime?.backend ?? null, health.loading),
    },
    {
      label: 'Accelerator identity',
      scope: 'Configured backend identity, not utilization',
      value: textValue(runtime?.accelerator ?? null, health.loading),
    },
    {
      label: 'Accelerator utilization',
      scope: 'No production GPU or ANE percentage',
      value: {
        text: 'Unavailable',
        status: 'unavailable' as const,
      },
    },
  ], [health, runtime]);

  return (
    <div className="flex flex-col gap-5 p-4">
      <section aria-labelledby="runtime-health-heading">
        <div className="mb-2 flex items-end justify-between gap-3">
          <div>
            <h2 id="runtime-health-heading" className="text-sm font-semibold text-on-surface">
              Live health
            </h2>
            <p className="text-[11px] text-on-surface-variant">
              Local runtime identity and pipeline state
            </p>
          </div>
          <button
            type="button"
            onClick={onRetry}
            className="rounded-lg border border-outline-variant/15 px-2.5 py-1 text-xs font-medium text-on-surface-variant hover:bg-surface-container focus:outline-none focus-visible:ring-2 focus-visible:ring-primary"
          >
            Refresh
          </button>
        </div>
        {(health.error || error) && (
          <div role="alert" className="mb-3 rounded-xl border border-error/20 bg-error/10 px-3 py-2 text-xs text-error">
            Some live diagnostics could not be refreshed. Existing measured data remains visible.
          </div>
        )}
        <div className="grid grid-cols-2 gap-3 lg:grid-cols-5">
          {healthCards.map(card => <MetricCard key={card.label} {...card} />)}
        </div>
      </section>

      <section aria-labelledby="resource-health-heading">
        <div className="mb-2 flex flex-wrap items-end justify-between gap-3">
          <div>
            <h2 id="resource-health-heading" className="text-sm font-semibold text-on-surface">
              Scoped resources
            </h2>
            <p className="text-[11px] text-on-surface-variant">
              Persistent typed samples · gaps remain unavailable, never zero
            </p>
          </div>
          {samples.length > 0 && (
            <label className="flex min-w-56 items-center gap-2 text-[11px] text-on-surface-variant">
              Timeline cursor
              <input
                type="range"
                min={0}
                max={Math.max(samples.length - 1, 0)}
                value={selectedIndex}
                onChange={event => setSelectedIndex(Number(event.target.value))}
                aria-label="Shared resource chart timeline cursor"
                className="min-w-32 flex-1 accent-primary"
              />
              <span className="w-16 text-right tabular-nums">
                {selected
                  ? new Date(selected.observedAtMs).toLocaleTimeString([], {
                    hour: 'numeric',
                    minute: '2-digit',
                    second: '2-digit',
                  })
                  : '—'}
              </span>
            </label>
          )}
        </div>

        {loading && samples.length === 0 ? (
          <div aria-label="Loading resource samples" className="grid grid-cols-2 gap-3 md:grid-cols-4">
            {Array.from({ length: 4 }, (_, index) => (
              <div key={index} className="h-24 animate-pulse rounded-xl bg-surface-container" />
            ))}
          </div>
        ) : samples.length === 0 ? (
          <div className="rounded-2xl border border-dashed border-outline-variant/30 bg-surface-container-low p-8 text-center">
            <div className="text-sm font-medium text-on-surface">
              {error ? 'Resource samples unavailable' : 'Waiting for the first resource sample'}
            </div>
            <p className="mt-1 text-xs text-on-surface-variant">
              {error
                ? 'Retry to reconnect to the local diagnostics store.'
                : 'Typed host and process measurements will appear here shortly.'}
            </p>
          </div>
        ) : (
          <>
            <div className="grid grid-cols-2 gap-3 md:grid-cols-4">
              <MetricCard
                label="Host CPU"
                scope="Whole-host utilization · 0–100%"
                value={currentMeasurement(selected, sample => sample.host.cpuPercent, formatPercent)}
              />
              <MetricCard
                label="Murmur CPU"
                scope="Main process · 100% equals one logical core"
                value={currentMeasurement(selected, sample => sample.mainProcess.cpuPercent, formatPercent)}
                accent="#d97706"
              />
              <MetricCard
                label="Main-process RSS"
                scope="Physical resident memory"
                value={currentMeasurement(selected, sample => sample.mainProcess.rssBytes, formatBytes)}
                accent="var(--murmur-error)"
              />
              <MetricCard
                label="Rust heap"
                scope="Murmur Rust malloc zone"
                value={currentMeasurement(selected, sample => sample.mainProcess.rustHeapBytes, formatBytes)}
                accent="#d97706"
              />
              <MetricCard
                label="FFI / native heap"
                scope="Non-Rust malloc zones · not GPU memory"
                value={currentMeasurement(selected, sample => sample.mainProcess.ffiNativeHeapBytes, formatBytes)}
                accent="#2563eb"
              />
              <MetricCard
                label="Sidecar CPU"
                scope="Local LLM helper process"
                value={currentMeasurement(selected, sample => sample.sidecarProcess.cpuPercent, formatPercent)}
                accent="#7c3aed"
              />
              <MetricCard
                label="Sidecar RSS"
                scope="Local LLM helper resident memory"
                value={currentMeasurement(selected, sample => sample.sidecarProcess.rssBytes, formatBytes)}
                accent="#7c3aed"
              />
            </div>
            <PerformanceChart
              title="CPU timeline"
              scope="Host, Murmur main process, and local sidecar remain separate scopes"
              unit="percent"
              samples={samples}
              series={CPU_SERIES}
              selectedIndex={selectedIndex}
              onSelectedIndexChange={setSelectedIndex}
              formatValue={formatPercent}
            />
            <PerformanceChart
              title="Memory timeline"
              scope="RSS and allocator zones are process-scoped; none represent GPU or ANE utilization"
              unit="bytes"
              samples={samples}
              series={MEMORY_SERIES}
              selectedIndex={selectedIndex}
              onSelectedIndexChange={setSelectedIndex}
              formatValue={formatBytes}
            />
          </>
        )}
      </section>
    </div>
  );
}
