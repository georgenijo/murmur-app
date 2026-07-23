import { useEffect, useMemo, useState } from 'react';
import type { CorrelationFilter } from '../../lib/eventFilters';
import {
  getPerformanceRun,
  type PerformanceRunKindV1,
  type PerformanceRunV1,
} from '../../lib/performance';
import {
  formatMilliseconds,
  formatTimestamp,
  inputSummary,
  kindLabel,
  peakResourceSummary,
  rateForRun,
  runLatencyMs,
  runOutcomeLabel,
  runtimeSummary,
} from '../../lib/performancePresentation';
import { RunDetail } from './RunDetail';

type OutcomeFilter =
  | 'all'
  | 'success'
  | 'noSpeech'
  | 'cancelled'
  | 'timedOut'
  | 'failed'
  | 'interrupted';

interface RunsViewProps {
  runs: PerformanceRunV1[];
  loading: boolean;
  error: string | null;
  cleared: boolean;
  clearing: boolean;
  clearError: string | null;
  onRetry: () => void;
  onClear: () => Promise<void>;
  onShowEvents: (filter: CorrelationFilter) => void;
}

function outcomeClasses(status: PerformanceRunV1['outcome']['status']): string {
  if (status === 'success') return 'bg-emerald-100 text-emerald-700 dark:bg-emerald-900/35 dark:text-emerald-300';
  if (status === 'noSpeech' || status === 'cancelled') {
    return 'bg-stone-200 text-stone-700 dark:bg-stone-700 dark:text-stone-300';
  }
  return 'bg-red-100 text-red-700 dark:bg-red-900/35 dark:text-red-300';
}

export function RunsView({
  runs,
  loading,
  error,
  cleared,
  clearing,
  clearError,
  onRetry,
  onClear,
  onShowEvents,
}: RunsViewProps) {
  const [kind, setKind] = useState<'all' | PerformanceRunKindV1>('all');
  const [outcome, setOutcome] = useState<OutcomeFilter>('all');
  const [selectedRunId, setSelectedRunId] = useState<string | null>(null);
  const [selectedRun, setSelectedRun] = useState<PerformanceRunV1 | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);
  const [detailError, setDetailError] = useState<string | null>(null);

  useEffect(() => {
    if (!selectedRunId) {
      setSelectedRun(null);
      setDetailError(null);
      return;
    }
    let cancelled = false;
    setDetailLoading(true);
    setDetailError(null);
    void getPerformanceRun(selectedRunId)
      .then(run => {
        if (cancelled) return;
        if (!run) {
          setDetailError('This run is no longer available in bounded diagnostics history.');
          setSelectedRun(null);
        } else {
          setSelectedRun(run);
        }
      })
      .catch(reason => {
        if (!cancelled) {
          setDetailError(reason instanceof Error ? reason.message : String(reason));
          setSelectedRun(null);
        }
      })
      .finally(() => {
        if (!cancelled) setDetailLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [selectedRunId]);

  const filteredRuns = useMemo(() => runs.filter(run =>
    (kind === 'all' || run.kind === kind)
      && (outcome === 'all' || run.outcome.status === outcome)), [kind, outcome, runs]);

  const confirmClear = async () => {
    const confirmed = window.confirm(
      'Clear local Performance runs and resource samples?\n\n'
      + 'This does not remove Events, logs, transcription history, settings, knowledge, or benchmark reports.',
    );
    if (!confirmed) return;
    try {
      await onClear();
      setSelectedRunId(null);
    } catch {
      // The hook owns the inline error state; keep the current detail selection.
    }
  };

  if (selectedRun) {
    return (
      <RunDetail
        run={selectedRun}
        onBack={() => setSelectedRunId(null)}
        onShowEvents={onShowEvents}
      />
    );
  }

  return (
    <div className="flex min-h-full flex-col p-4">
      <div className="mb-4 flex flex-wrap items-end justify-between gap-3">
        <div>
          <h2 className="text-sm font-semibold text-on-surface">Runs</h2>
          <p className="text-[11px] text-on-surface-variant">
            Newest 200 local, content-free performance records
          </p>
        </div>
        <div className="flex flex-wrap items-end gap-2">
          <label className="text-[10px] font-medium uppercase tracking-wider text-on-surface-variant">
            Kind
            <select
              value={kind}
              onChange={event => setKind(event.target.value as 'all' | PerformanceRunKindV1)}
              className="mt-1 block rounded-lg border border-outline-variant/20 bg-surface-container-lowest px-2 py-1.5 text-xs normal-case tracking-normal text-on-surface outline-none focus:border-primary focus:ring-1 focus:ring-primary"
            >
              <option value="all">All kinds</option>
              <option value="dictation">Dictation</option>
              <option value="fileTranscription">File transcription</option>
              <option value="selectedTextTransform">Selected-text transform</option>
            </select>
          </label>
          <label className="text-[10px] font-medium uppercase tracking-wider text-on-surface-variant">
            Outcome
            <select
              value={outcome}
              onChange={event => setOutcome(event.target.value as OutcomeFilter)}
              className="mt-1 block rounded-lg border border-outline-variant/20 bg-surface-container-lowest px-2 py-1.5 text-xs normal-case tracking-normal text-on-surface outline-none focus:border-primary focus:ring-1 focus:ring-primary"
            >
              <option value="all">All outcomes</option>
              <option value="success">Success</option>
              <option value="noSpeech">No speech</option>
              <option value="cancelled">Cancelled</option>
              <option value="timedOut">Timed out</option>
              <option value="failed">Failed</option>
              <option value="interrupted">Interrupted</option>
            </select>
          </label>
          <button
            type="button"
            onClick={onRetry}
            className="rounded-lg border border-outline-variant/15 px-2.5 py-1.5 text-xs font-medium text-on-surface-variant hover:bg-surface-container focus:outline-none focus-visible:ring-2 focus-visible:ring-primary"
          >
            Refresh
          </button>
          <button
            type="button"
            disabled={clearing}
            onClick={() => void confirmClear()}
            className="rounded-lg border border-error/20 px-2.5 py-1.5 text-xs font-medium text-error hover:bg-error/10 focus:outline-none focus-visible:ring-2 focus-visible:ring-error disabled:opacity-50"
          >
            {clearing ? 'Clearing…' : 'Clear Performance Data'}
          </button>
        </div>
      </div>

      {(error || clearError || detailError) && (
        <div role="alert" className="mb-3 rounded-xl border border-error/20 bg-error/10 px-3 py-2 text-xs text-error">
          {detailError ?? clearError ?? 'Runs could not be refreshed. Existing records remain visible.'}
        </div>
      )}

      {detailLoading && (
        <div aria-label="Loading run detail" className="mb-3 h-20 animate-pulse rounded-xl bg-surface-container" />
      )}

      {loading && runs.length === 0 ? (
        <div aria-label="Loading performance runs" className="space-y-2">
          {Array.from({ length: 5 }, (_, index) => (
            <div key={index} className="h-12 animate-pulse rounded-xl bg-surface-container" />
          ))}
        </div>
      ) : runs.length === 0 ? (
        <div className="flex flex-1 items-center justify-center">
          <div className="max-w-sm rounded-2xl border border-dashed border-outline-variant/30 bg-surface-container-low p-8 text-center">
            <div className="text-sm font-medium text-on-surface">
              {cleared ? 'Performance data was cleared' : error ? 'Run history unavailable' : 'No performance runs yet'}
            </div>
            <p className="mt-1 text-xs text-on-surface-variant">
              {cleared
                ? 'New dictation, file transcription, and selected-text transform runs will appear here.'
                : error
                  ? 'Retry to reconnect to the local diagnostics store.'
                  : 'Complete a local run to create a privacy-safe diagnostic record.'}
            </p>
          </div>
        </div>
      ) : filteredRuns.length === 0 ? (
        <div className="flex flex-1 items-center justify-center text-center">
          <div>
            <div className="text-sm font-medium text-on-surface">No runs match these filters</div>
            <button
              type="button"
              onClick={() => { setKind('all'); setOutcome('all'); }}
              className="mt-2 text-xs font-medium text-primary hover:underline focus:outline-none focus-visible:ring-2 focus-visible:ring-primary"
            >
              Reset filters
            </button>
          </div>
        </div>
      ) : (
        <div className="overflow-x-auto rounded-xl border border-outline-variant/15 bg-surface-container-lowest">
          <table className="w-full min-w-[900px] text-left text-xs">
            <thead className="sticky top-0 bg-surface-container-low text-[10px] uppercase tracking-wider text-on-surface-variant">
              <tr>
                <th scope="col" className="px-3 py-2">Run</th>
                <th scope="col" className="px-3 py-2">Runtime</th>
                <th scope="col" className="px-3 py-2">Outcome</th>
                <th scope="col" className="px-3 py-2 text-right">Input</th>
                <th scope="col" className="px-3 py-2 text-right">Latency</th>
                <th scope="col" className="px-3 py-2 text-right">Rate</th>
                <th scope="col" className="px-3 py-2">Peak resources</th>
              </tr>
            </thead>
            <tbody>
              {filteredRuns.map(run => {
                const input = inputSummary(run);
                const rate = rateForRun(run);
                return (
                  <tr key={run.runId} className="border-t border-outline-variant/10 hover:bg-surface-container-low">
                    <td className="px-3 py-2">
                      <button
                        type="button"
                        onClick={() => setSelectedRunId(run.runId)}
                        className="text-left focus:outline-none focus-visible:ring-2 focus-visible:ring-primary"
                        aria-label={`View details for ${kindLabel(run.kind)} at ${formatTimestamp(run.startedAtMs)}`}
                      >
                        <span className="block font-semibold text-on-surface">{kindLabel(run.kind)}</span>
                        <span className="block whitespace-nowrap text-[10px] text-on-surface-variant">
                          {formatTimestamp(run.startedAtMs)}
                        </span>
                      </button>
                    </td>
                    <td className="max-w-64 px-3 py-2 text-on-surface-variant">
                      <span className="line-clamp-2">{runtimeSummary(run)}</span>
                    </td>
                    <td className="px-3 py-2">
                      <span className={`whitespace-nowrap rounded-full px-2 py-0.5 text-[10px] font-semibold ${outcomeClasses(run.outcome.status)}`}>
                        {runOutcomeLabel(run.outcome)}
                      </span>
                    </td>
                    <td className="whitespace-nowrap px-3 py-2 text-right tabular-nums text-on-surface" title={input.detail}>
                      {input.text}
                    </td>
                    <td className="whitespace-nowrap px-3 py-2 text-right tabular-nums text-on-surface">
                      {formatMilliseconds(runLatencyMs(run))}
                    </td>
                    <td className="whitespace-nowrap px-3 py-2 text-right tabular-nums text-on-surface" title={rate.detail}>
                      <span className="block">{rate.text}</span>
                      <span className="block text-[9px] text-on-surface-variant">{rate.label}</span>
                    </td>
                    <td className="max-w-60 px-3 py-2 text-[10px] text-on-surface-variant">
                      {peakResourceSummary(run)}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
