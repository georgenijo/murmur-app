import type { CorrelationFilter } from '../../lib/eventFilters';
import type {
  MeasurementV1,
  PerformanceRunV1,
  ResourceRangeV1,
  StageTimingV1,
} from '../../lib/performance';
import {
  acceleratorLabel,
  backendLabel,
  correlationFilterForRun,
  correlationLabel,
  formatBytes,
  formatMeasurement,
  formatMilliseconds,
  formatPercent,
  formatTimestamp,
  inputSummary,
  orderedStages,
  rateForRun,
  resourceDelta,
  runLatencyMs,
  runOutcomeDetail,
  runOutcomeLabel,
  stageLabel,
} from '../../lib/performancePresentation';

interface RunDetailProps {
  run: PerformanceRunV1;
  onBack: () => void;
  onShowEvents: (filter: CorrelationFilter) => void;
}

function outcomeClasses(status: PerformanceRunV1['outcome']['status']): string {
  if (status === 'success') return 'bg-emerald-100 text-emerald-700 dark:bg-emerald-900/35 dark:text-emerald-300';
  if (status === 'noSpeech' || status === 'cancelled') {
    return 'bg-stone-200 text-stone-700 dark:bg-stone-700 dark:text-stone-300';
  }
  return 'bg-red-100 text-red-700 dark:bg-red-900/35 dark:text-red-300';
}

function SummaryCard({
  label,
  value,
  detail,
}: {
  label: string;
  value: string;
  detail?: string;
}) {
  return (
    <div className="rounded-xl border border-outline-variant/10 bg-surface-container-lowest p-3 shadow-sm">
      <div className="text-[10px] font-medium uppercase tracking-wider text-on-surface-variant">
        {label}
      </div>
      <div className="mt-1 text-base font-semibold tabular-nums text-on-surface">{value}</div>
      {detail && <div className="mt-0.5 text-[10px] text-on-surface-variant">{detail}</div>}
    </div>
  );
}

function MeasurementCell({
  measurement,
  format,
}: {
  measurement: MeasurementV1<number>;
  format: (value: number) => string;
}) {
  const value = formatMeasurement(measurement, format);
  return (
    <td className={`px-2 py-2 text-right tabular-nums ${
      value.status === 'measured' ? 'text-on-surface' : 'text-on-surface-variant'
    }`} title={value.detail}>
      {value.text}
    </td>
  );
}

function ResourceRangeRow({
  label,
  scope,
  range,
  format,
}: {
  label: string;
  scope: string;
  range: ResourceRangeV1<number>;
  format: (value: number) => string;
}) {
  return (
    <tr className="border-t border-outline-variant/10">
      <th scope="row" className="px-2 py-2 text-left font-medium text-on-surface">
        {label}
        <span className="ml-1 font-normal text-on-surface-variant">· {scope}</span>
      </th>
      <MeasurementCell measurement={range.start} format={format} />
      <MeasurementCell measurement={range.average} format={format} />
      <MeasurementCell measurement={range.peak} format={format} />
      <MeasurementCell measurement={range.end} format={format} />
    </tr>
  );
}

function WaterfallRow({ timing, maximum }: { timing: StageTimingV1; maximum: number }) {
  const value = formatMeasurement(timing.durationMs, formatMilliseconds);
  const measuredValue = timing.durationMs.status === 'measured'
    ? timing.durationMs.value
    : null;
  const width = measuredValue !== null
    ? measuredValue === 0
      ? 0
      : Math.max(1.5, (measuredValue / maximum) * 100)
    : 0;
  const outcomeColor = timing.outcome === 'failed'
    ? 'bg-error'
    : timing.outcome === 'fallback'
      ? 'bg-amber-500'
      : 'bg-primary';

  return (
    <li className="grid grid-cols-[minmax(130px,0.8fr)_minmax(150px,2fr)_90px] items-center gap-3 py-1.5">
      <div className="min-w-0">
        <div className="truncate text-xs font-medium text-on-surface">{stageLabel(timing.stage)}</div>
        <div className="text-[10px] capitalize text-on-surface-variant">{timing.outcome}</div>
      </div>
      <div className="relative h-5 overflow-hidden rounded-md bg-surface-container">
        {measuredValue !== null ? (
          measuredValue === 0 ? (
            <span className={`absolute inset-y-0 left-0 w-0.5 ${outcomeColor}`} />
          ) : (
            <span
              className={`absolute inset-y-0 left-0 rounded-md ${outcomeColor}`}
              style={{ width: `${width}%` }}
            />
          )
        ) : (
          <span className="absolute inset-0 bg-[repeating-linear-gradient(135deg,transparent,transparent_5px,rgba(120,113,108,0.10)_5px,rgba(120,113,108,0.10)_10px)]" />
        )}
      </div>
      <div
        className={`text-right text-xs font-medium tabular-nums ${
          measuredValue !== null ? 'text-on-surface' : 'text-on-surface-variant'
        }`}
        title={value.detail}
      >
        {value.text}
      </div>
    </li>
  );
}

export function RunDetail({ run, onBack, onShowEvents }: RunDetailProps) {
  const stages = orderedStages(run);
  const measuredDurations = stages.flatMap(stage =>
    stage.durationMs.status === 'measured' ? [stage.durationMs.value] : []);
  const maximum = Math.max(...measuredDurations, 1);
  const rate = rateForRun(run);
  const input = inputSummary(run);
  const mainPeak = formatMeasurement(run.resources.mainProcess.rssBytes.peak, formatBytes);
  const sidecarPeak = formatMeasurement(run.resources.sidecarProcess.rssBytes.peak, formatBytes);
  const hostPeak = formatMeasurement(run.resources.host.cpuPercent.peak, formatPercent);
  const mainDelta = resourceDelta(
    run.resources.mainProcess.rssBytes.start,
    run.resources.mainProcess.rssBytes.end,
  );
  const outcomeDetail = runOutcomeDetail(run.outcome);

  return (
    <div className="flex flex-col gap-5 p-4">
      <header className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <button
            type="button"
            onClick={onBack}
            className="mb-2 rounded-lg px-2 py-1 text-xs font-medium text-on-surface-variant hover:bg-surface-container focus:outline-none focus-visible:ring-2 focus-visible:ring-primary"
          >
            ← All runs
          </button>
          <div className="flex flex-wrap items-center gap-2">
            <h2 className="text-lg font-semibold text-on-surface">{correlationLabel(run)}</h2>
            <span className={`rounded-full px-2 py-0.5 text-[10px] font-semibold ${outcomeClasses(run.outcome.status)}`}>
              {runOutcomeLabel(run.outcome)}
            </span>
          </div>
          <p className="mt-1 text-xs text-on-surface-variant">
            {formatTimestamp(run.startedAtMs)} · Murmur {run.appVersion}
            {outcomeDetail ? ` · ${outcomeDetail}` : ''}
          </p>
        </div>
        <button
          type="button"
          onClick={() => onShowEvents(correlationFilterForRun(run))}
          className="rounded-lg bg-primary px-3 py-1.5 text-xs font-semibold text-on-primary shadow-sm hover:opacity-90 focus:outline-none focus-visible:ring-2 focus-visible:ring-primary"
        >
          Show correlated Events
        </button>
      </header>

      <section aria-labelledby="run-summary-heading">
        <h3 id="run-summary-heading" className="mb-2 text-xs font-semibold uppercase tracking-wider text-on-surface-variant">
          Run summary
        </h3>
        <div className="grid grid-cols-2 gap-3 md:grid-cols-4">
          <SummaryCard label="Total latency" value={formatMilliseconds(runLatencyMs(run))} detail="Start to terminal outcome" />
          <SummaryCard label="Input" value={input.text} detail={input.detail} />
          <SummaryCard label={rate.label} value={rate.text} detail={rate.detail} />
          <SummaryCard label="Main RSS peak" value={mainPeak.text} detail={mainPeak.detail} />
          <SummaryCard label="Main RSS delta" value={mainDelta.text} detail={mainDelta.detail} />
          <SummaryCard label="Host CPU peak" value={hostPeak.text} detail="Whole-host utilization" />
          <SummaryCard label="Sidecar RSS peak" value={sidecarPeak.text} detail={sidecarPeak.detail ?? 'Local LLM helper'} />
          <SummaryCard label="Accelerator utilization" value="Unavailable" detail="No production GPU or ANE percentage" />
        </div>
      </section>

      <section aria-labelledby="runtime-heading">
        <h3 id="runtime-heading" className="mb-2 text-xs font-semibold uppercase tracking-wider text-on-surface-variant">
          Runtime identity
        </h3>
        {run.runtimes.length === 0 ? (
          <div className="rounded-xl bg-surface-container-low p-3 text-xs text-on-surface-variant">
            Runtime identity unavailable
          </div>
        ) : (
          <div className="grid gap-2 md:grid-cols-2">
            {run.runtimes.map((runtime, index) => (
              <div key={`${runtime.role}-${index}`} className="rounded-xl border border-outline-variant/10 bg-surface-container-lowest p-3 text-xs">
                <div className="font-semibold text-on-surface">{runtime.modelId}</div>
                <div className="mt-1 text-on-surface-variant">
                  {backendLabel(runtime)} · {acceleratorLabel(runtime)} · {runtime.warmState}
                </div>
                <div className="mt-1 text-[10px] uppercase tracking-wider text-on-surface-variant">
                  {runtime.role}
                </div>
              </div>
            ))}
          </div>
        )}
      </section>

      <section aria-labelledby="waterfall-heading" className="rounded-2xl border border-outline-variant/15 bg-surface-container-lowest p-4 shadow-sm">
        <div className="mb-3">
          <h3 id="waterfall-heading" className="text-sm font-semibold text-on-surface">Phase waterfall</h3>
          <p className="mt-0.5 text-[11px] text-on-surface-variant">
            Canonical stage order with duration contribution. V1 does not record absolute offsets, so none are inferred.
          </p>
        </div>
        <ol aria-label="Ordered run stages">
          {stages.map(stage => <WaterfallRow key={stage.stage} timing={stage} maximum={maximum} />)}
        </ol>
        {run.followUps.length > 0 && (
          <div className="mt-3 border-t border-outline-variant/10 pt-3">
            <div className="mb-1 text-[10px] font-semibold uppercase tracking-wider text-on-surface-variant">
              Correlated follow-ups
            </div>
            {run.followUps.map((followUp, index) => {
              const duration = formatMeasurement(followUp.durationMs, formatMilliseconds);
              return (
                <div key={`${followUp.kind}-${followUp.atMs}-${index}`} className="flex items-center justify-between py-1 text-xs">
                  <span className="font-medium capitalize text-on-surface">{followUp.kind}</span>
                  <span className="text-on-surface-variant">
                    {new Date(followUp.atMs).toLocaleTimeString()} · {followUp.outcome} · {duration.text}
                  </span>
                </div>
              );
            })}
          </div>
        )}
      </section>

      <section aria-labelledby="resources-heading">
        <div className="mb-2">
          <h3 id="resources-heading" className="text-sm font-semibold text-on-surface">Resource summary</h3>
          <p className="text-[11px] text-on-surface-variant">
            {run.resources.sampleCount} typed sample{run.resources.sampleCount === 1 ? '' : 's'} during this run
          </p>
        </div>
        <div className="overflow-x-auto rounded-xl border border-outline-variant/15">
          <table className="w-full min-w-[620px] text-xs">
            <thead className="bg-surface-container-low text-[10px] uppercase tracking-wider text-on-surface-variant">
              <tr>
                <th scope="col" className="px-2 py-2 text-left">Metric and scope</th>
                <th scope="col" className="px-2 py-2 text-right">Start</th>
                <th scope="col" className="px-2 py-2 text-right">Average</th>
                <th scope="col" className="px-2 py-2 text-right">Peak</th>
                <th scope="col" className="px-2 py-2 text-right">End</th>
              </tr>
            </thead>
            <tbody>
              <ResourceRangeRow label="CPU" scope="Whole host" range={run.resources.host.cpuPercent} format={formatPercent} />
              <ResourceRangeRow label="CPU" scope="Murmur main process" range={run.resources.mainProcess.cpuPercent} format={formatPercent} />
              <ResourceRangeRow label="RSS" scope="Murmur main process" range={run.resources.mainProcess.rssBytes} format={formatBytes} />
              <ResourceRangeRow label="Rust heap" scope="Murmur main process" range={run.resources.mainProcess.rustHeapBytes} format={formatBytes} />
              <ResourceRangeRow label="FFI / native heap" scope="Murmur main process" range={run.resources.mainProcess.ffiNativeHeapBytes} format={formatBytes} />
              <ResourceRangeRow label="CPU" scope="Local LLM sidecar" range={run.resources.sidecarProcess.cpuPercent} format={formatPercent} />
              <ResourceRangeRow label="RSS" scope="Local LLM sidecar" range={run.resources.sidecarProcess.rssBytes} format={formatBytes} />
            </tbody>
          </table>
        </div>
      </section>

      <footer className="break-all rounded-xl bg-surface-container-low px-3 py-2 font-mono text-[10px] text-on-surface-variant">
        Run ID: {run.runId}
      </footer>
    </div>
  );
}
