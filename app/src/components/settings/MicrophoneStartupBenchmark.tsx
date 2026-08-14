import { useEffect, useMemo, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import type { AudioInputInventoryV1 } from '../../lib/audioDevices';
import {
  MICROPHONE_STARTUP_BENCHMARK_CYCLES,
  addMicrophoneStartupReport,
  cancelMicrophoneStartupBenchmark,
  clearMicrophoneStartupReports,
  loadMicrophoneStartupReports,
  parseMicrophoneStartupBenchmarkProgress,
  runMicrophoneStartupBenchmark,
  saveMicrophoneStartupReport,
  saveMicrophoneStartupReports,
  summarizeMicrophoneStartupBenchmark,
  type MicrophoneStartupBenchmarkProgress,
  type MicrophoneStartupBenchmarkReport,
  type MicrophoneStartupCycleResult,
} from '../../lib/microphoneStartupBenchmark';
import type { DictationStatus } from '../../lib/types';

function milliseconds(value: number | null): string {
  return value === null ? '—' : `${Math.round(value)} ms`;
}

function backendLabel(backend: 'auhal' | 'cpal' | null): string {
  if (backend === 'auhal') return 'AUHAL';
  if (backend === 'cpal') return 'CPAL';
  return '—';
}

function diagnosticLabel(value: string): string {
  const words = value.replace(/_/g, ' ');
  return `${words.charAt(0).toUpperCase()}${words.slice(1)}`;
}

function setupStep(cycle: MicrophoneStartupCycleResult): string {
  if (!cycle.lastSetupStep) return 'No native setup step reported';
  const transition = cycle.lastSetupTransition === 'completed' ? 'completed' : 'entered';
  return `${diagnosticLabel(cycle.lastSetupStep)} (${transition})`;
}

function progressLabel(progress: MicrophoneStartupBenchmarkProgress | null): string {
  if (!progress) return 'Preparing production capture path…';
  if (progress.phase === 'complete') return 'Finishing report…';
  if (progress.currentCycle === 0) return `${diagnosticLabel(progress.phase)} for exclusive microphone access…`;
  const backend = progress.backend ? ` · ${backendLabel(progress.backend)}` : '';
  const fallback = progress.fallbackOccurred ? ' · fallback' : '';
  const step = progress.lastSetupStep
    ? ` · ${diagnosticLabel(progress.lastSetupStep)} ${progress.lastSetupTransition ?? ''}`
    : '';
  return `Cycle ${progress.currentCycle} of ${progress.totalCycles}${backend}${fallback}${step}`;
}

function refusalHint({
  status,
  modelBenchmarkRunning,
  fileTranscribing,
  corpusBusy,
}: {
  status: DictationStatus;
  modelBenchmarkRunning: boolean;
  fileTranscribing: boolean;
  corpusBusy: boolean;
}): string | null {
  if (status !== 'idle') return 'Finish the current recording first.';
  if (modelBenchmarkRunning) return 'Finish or cancel the model benchmark first.';
  if (fileTranscribing) return 'Finish the file transcription first.';
  if (corpusBusy) return 'Finish or cancel the corpus recording first.';
  return null;
}

export function MicrophoneStartupBenchmark({
  status,
  deviceId,
  audioInventory,
  modelBenchmarkRunning,
  fileTranscribing,
  corpusBusy,
  outputDir,
  autoSave,
  onRunningChange,
}: {
  status: DictationStatus;
  deviceId: string;
  audioInventory: AudioInputInventoryV1 | null;
  modelBenchmarkRunning: boolean;
  fileTranscribing: boolean;
  corpusBusy: boolean;
  outputDir: string;
  autoSave: boolean;
  onRunningChange: (running: boolean) => void;
}) {
  const [reports, setReports] = useState<MicrophoneStartupBenchmarkReport[]>(loadMicrophoneStartupReports);
  const [selectedAt, setSelectedAt] = useState<string | null>(() => reports[0]?.startedAt ?? null);
  const [running, setRunning] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const [progress, setProgress] = useState<MicrophoneStartupBenchmarkProgress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [saveState, setSaveState] = useState<'idle' | 'saving' | 'saved'>('idle');
  const [listenerReady, setListenerReady] = useState(false);
  const mounted = useRef(true);
  const runningRef = useRef(false);
  const activeRunId = useRef<string | null>(null);
  const activeBenchmarkRunId = useRef<number | null>(null);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      if (runningRef.current && activeRunId.current) {
        void cancelMicrophoneStartupBenchmark(activeRunId.current).catch(() => {
          // The component is gone; the native run remains responsible for teardown.
        });
      }
    };
  }, []);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    listen<unknown>('microphone-startup-benchmark-progress', (event) => {
      if (disposed || !runningRef.current) return;
      const parsed = parseMicrophoneStartupBenchmarkProgress(event.payload);
      if (!parsed) {
        if (event.payload && typeof event.payload === 'object'
          && !Array.isArray(event.payload)
          && (event.payload as Record<string, unknown>).runId === activeRunId.current) {
          setError('Murmur sent invalid microphone benchmark progress.');
        }
        return;
      }
      if (parsed.runId !== activeRunId.current) return;
      if (activeBenchmarkRunId.current !== null
        && parsed.benchmarkRunId !== activeBenchmarkRunId.current) return;
      activeBenchmarkRunId.current = parsed.benchmarkRunId;
      setProgress(parsed);
    })
      .then((dispose) => {
        if (disposed) dispose();
        else {
          unlisten = dispose;
          setListenerReady(true);
        }
      })
      .catch((reason: unknown) => {
        if (!disposed) setError(`Could not watch microphone benchmark progress: ${String(reason)}`);
      });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const report = reports.find((item) => item.startedAt === selectedAt) ?? null;
  const summary = useMemo(() => report ? summarizeMicrophoneStartupBenchmark(report) : null, [report]);
  const blocked = refusalHint({
    status,
    modelBenchmarkRunning,
    fileTranscribing,
    corpusBusy,
  });
  const selectedDevice = deviceId === 'system_default'
    ? 'System Default'
    : audioInventory?.devices.find((device) => device.id === deviceId)?.name
      ?? 'Saved microphone (currently unavailable)';
  const progressPercent = progress
    ? Math.round((progress.completedCycles / progress.totalCycles) * 100)
    : 0;

  const handleRun = async () => {
    const runId = crypto.randomUUID();
    setError(null);
    setCopied(false);
    setProgress(null);
    setCancelling(false);
    runningRef.current = true;
    activeRunId.current = runId;
    activeBenchmarkRunId.current = null;
    setRunning(true);
    onRunningChange(true);
    try {
      const next = await runMicrophoneStartupBenchmark(
        runId,
        deviceId,
        MICROPHONE_STARTUP_BENCHMARK_CYCLES,
      );
      if (!mounted.current) return;
      if (activeBenchmarkRunId.current !== null
        && next.benchmarkRunId !== activeBenchmarkRunId.current) {
        throw new Error('Murmur returned a report for a different microphone benchmark.');
      }
      setReports((current) => {
        const updated = addMicrophoneStartupReport(current, next);
        if (!next.cancelled && next.completedCycles === next.requestedCycles) {
          saveMicrophoneStartupReports(updated.filter((item) => (
            !item.cancelled && item.completedCycles === item.requestedCycles
          )));
        }
        return updated;
      });
      setSelectedAt(next.startedAt);
      if (autoSave && !next.cancelled && next.completedCycles === next.requestedCycles) {
        try {
          await saveMicrophoneStartupReport(next, outputDir);
        } catch (reason) {
          if (mounted.current) setError(`Run completed, but the report could not be auto-saved: ${String(reason)}`);
        }
      }
    } catch (reason) {
      if (mounted.current) setError(String(reason));
    } finally {
      runningRef.current = false;
      activeRunId.current = null;
      activeBenchmarkRunId.current = null;
      if (mounted.current) {
        setRunning(false);
        setCancelling(false);
        onRunningChange(false);
      }
    }
  };

  const handleCancel = async () => {
    const runId = activeRunId.current;
    if (!runId) {
      setError('The microphone benchmark had already stopped.');
      return;
    }
    setError(null);
    setCancelling(true);
    try {
      const accepted = await cancelMicrophoneStartupBenchmark(runId);
      if (!accepted && mounted.current) {
        setCancelling(false);
        setError('The microphone benchmark had already stopped.');
      }
    } catch (reason) {
      if (mounted.current) {
        setCancelling(false);
        setError(`Could not cancel the microphone benchmark: ${String(reason)}`);
      }
    }
  };

  const copyReport = async () => {
    if (!report) return;
    try {
      await navigator.clipboard.writeText(JSON.stringify(report, null, 2));
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1600);
    } catch (reason) {
      setError(`Could not copy report: ${String(reason)}`);
    }
  };

  const saveReport = async () => {
    if (!report) return;
    setError(null);
    setSaveState('saving');
    try {
      await saveMicrophoneStartupReport(report, outputDir);
      if (!mounted.current) return;
      setSaveState('saved');
      window.setTimeout(() => {
        if (mounted.current) setSaveState('idle');
      }, 1600);
    } catch (reason) {
      if (mounted.current) {
        setSaveState('idle');
        setError(`Could not save report: ${String(reason)}`);
      }
    }
  };

  return (
    <section className="space-y-3 border-y border-outline-variant/30 py-5" aria-labelledby="microphone-startup-heading">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <div className="flex items-center gap-2">
            <h3 id="microphone-startup-heading" className="text-sm font-semibold text-on-surface">
              Microphone startup
            </h3>
            <span className="rounded-full bg-surface-container-high px-2 py-0.5 text-[10px] font-medium text-on-surface-variant">
              5 cycles
            </span>
          </div>
          <p className="mt-1 text-xs leading-relaxed text-on-surface-variant">
            Times the production capture path to its first audio buffer. No audio is transcribed or saved.
          </p>
        </div>
        <span className="max-w-36 truncate text-right text-[11px] text-on-surface-variant" title={selectedDevice}>
          {selectedDevice}
        </span>
      </div>

      {running ? (
        <div className="rounded-lg border border-primary/30 bg-primary/10 p-3">
          <div className="flex items-center justify-between gap-3 text-xs" aria-live="polite">
            <span className="min-w-0 truncate text-on-surface">{progressLabel(progress)}</span>
            <span className="shrink-0 tabular-nums text-on-surface">{progressPercent}%</span>
          </div>
          <div
            role="progressbar"
            aria-label="Microphone startup benchmark progress"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={progressPercent}
            className="mt-2 h-1.5 overflow-hidden rounded-full bg-surface-container-high"
          >
            <div className="h-full bg-primary transition-all duration-200" style={{ width: `${progressPercent}%` }} />
          </div>
          <button
            type="button"
            onClick={() => void handleCancel()}
            disabled={cancelling}
            className="mt-3 w-full rounded-lg border border-outline-variant/30 px-3 py-2 text-xs font-medium text-on-surface hover:bg-surface-container-low disabled:opacity-50"
          >
            {cancelling ? 'Stopping safely…' : 'Cancel microphone test'}
          </button>
        </div>
      ) : (
        <button
          type="button"
          onClick={() => void handleRun()}
          disabled={blocked !== null || !listenerReady}
          className="w-full rounded-lg border border-primary/40 bg-primary/10 px-4 py-2.5 text-sm font-semibold text-on-surface hover:bg-primary/15 disabled:cursor-not-allowed disabled:opacity-40"
        >
          Test microphone startup
        </button>
      )}
      {!running && blocked && <p className="text-xs text-primary">{blocked}</p>}
      {!running && !listenerReady && !blocked && !error && (
        <p className="text-xs text-on-surface-variant">Preparing microphone diagnostics…</p>
      )}
      {error && <p role="alert" className="text-xs text-error break-words">{error}</p>}

      {report && !running && summary && (
        <div className="space-y-3 rounded-xl border border-outline-variant/30 bg-surface-container-low p-3">
          <div className="flex flex-col items-start justify-between gap-2 sm:flex-row sm:gap-3">
            <div>
              <h4 className="text-xs font-semibold text-on-surface">Startup results</h4>
              <p className="mt-0.5 text-[11px] text-on-surface-variant">
                {report.completedCycles}/{report.requestedCycles} cycles
                {report.cancelled ? ' · cancelled early' : ''}
                {' · '}macOS · Murmur v{report.appVersion}
              </p>
            </div>
            <div className="flex shrink-0 gap-2">
              <button type="button" onClick={() => void copyReport()} className="rounded-md border border-outline-variant/30 px-2 py-1 text-[11px] font-medium text-on-surface hover:bg-surface-container-high">
                {copied ? 'Copied' : 'Copy JSON'}
              </button>
              <button type="button" disabled={saveState === 'saving'} onClick={() => void saveReport()} className="rounded-md border border-outline-variant/30 px-2 py-1 text-[11px] font-medium text-on-surface hover:bg-surface-container-high disabled:opacity-50">
                {saveState === 'saved' ? 'Saved' : saveState === 'saving' ? 'Saving…' : 'Save to file'}
              </button>
            </div>
          </div>

          <div className="grid grid-cols-4 divide-x divide-outline-variant/30 border-y border-outline-variant/30">
            {[
              ['Median', milliseconds(summary.medianMs)],
              ['P95', milliseconds(summary.p95Ms)],
              ['Ready', `${summary.readyCycles}/${report.completedCycles}`],
              ['Fallbacks', String(summary.fallbackCycles)],
            ].map(([label, value]) => (
              <div key={label} className="min-w-0 px-2 py-2 text-center">
                <div className="text-[10px] uppercase text-on-surface-variant">{label}</div>
                <div className="mt-0.5 truncate text-xs font-semibold tabular-nums text-on-surface" title={value}>{value}</div>
              </div>
            ))}
          </div>

          <p className="text-[11px] text-on-surface-variant">
            AUHAL won {summary.auhalWins}; CPAL won {summary.cpalWins}. Range {milliseconds(summary.minimumMs)}–{milliseconds(summary.maximumMs)}. Diagnostic cycles read the immutable production backend order and never retrain its first-PCM preference.
          </p>

          <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
            {(['auhal', 'cpal'] as const).map((backend) => {
              const metrics = summary.backendAttempts[backend];
              return (
                <div key={backend} className="rounded-lg border border-outline-variant/30 bg-surface-container-lowest p-2.5">
                  <div className="flex items-center justify-between gap-2">
                    <span className="text-xs font-semibold text-on-surface">{backendLabel(backend)}</span>
                    <span className="text-[10px] text-on-surface-variant">
                      {metrics.ready} ready · {metrics.failed} failed · {metrics.attempts} attempts
                    </span>
                  </div>
                  <div className="mt-2 grid grid-cols-3 gap-2 text-[10px] text-on-surface-variant">
                    <span>Median <strong className="block text-xs font-medium tabular-nums text-on-surface">{milliseconds(metrics.medianMs)}</strong></span>
                    <span>P95 <strong className="block text-xs font-medium tabular-nums text-on-surface">{milliseconds(metrics.p95Ms)}</strong></span>
                    <span>Max <strong className="block text-xs font-medium tabular-nums text-on-surface">{milliseconds(metrics.maximumMs)}</strong></span>
                  </div>
                </div>
              );
            })}
          </div>

          {reports.length > 1 && (
            <div className="flex items-center gap-2">
              <label htmlFor="microphone-startup-run" className="shrink-0 text-[11px] text-on-surface-variant">Saved run</label>
              <select
                id="microphone-startup-run"
                value={selectedAt ?? ''}
                onChange={(event) => setSelectedAt(event.target.value)}
                className="min-w-0 flex-1 rounded-md border border-on-surface-variant bg-surface-container-lowest px-2 py-1.5 text-xs text-on-surface focus:outline-none focus:ring-2 focus:ring-primary"
              >
                {reports.map((item) => (
                  <option key={item.startedAt} value={item.startedAt}>
                    {new Date(item.startedAt).toLocaleString()} · {item.completedCycles}/{item.requestedCycles} cycles
                  </option>
                ))}
              </select>
              <button
                type="button"
                onClick={() => {
                  clearMicrophoneStartupReports();
                  setReports([]);
                  setSelectedAt(null);
                }}
                className="shrink-0 px-2 py-1.5 text-xs text-on-surface-variant hover:text-error"
              >
                Clear
              </button>
            </div>
          )}

          {report.cycles.length > 0 ? (
            <div className="overflow-x-auto">
              <table className="w-full min-w-[34rem] text-[11px]">
                <thead className="text-left text-on-surface-variant">
                  <tr className="border-b border-outline-variant/30">
                    <th className="py-1.5 pr-2 font-medium">Cycle</th>
                    <th className="px-2 py-1.5 font-medium">Result</th>
                    <th className="px-2 py-1.5 font-medium">Backend / order</th>
                    <th className="px-2 py-1.5 text-right font-medium">First PCM</th>
                    <th className="pl-2 py-1.5 font-medium">Attempts / last setup step</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-outline-variant/30 text-on-surface">
                  {report.cycles.map((cycle) => (
                    <tr key={cycle.cycle}>
                      <td className="py-2 pr-2 tabular-nums">{cycle.cycle}</td>
                      <td className={`px-2 py-2 font-medium ${cycle.outcome === 'failed' ? 'text-error' : cycle.outcome === 'ready' ? 'text-success' : 'text-on-surface-variant'}`}>
                        {cycle.outcome === 'ready' ? 'Ready' : cycle.outcome === 'failed' ? 'Failed' : 'Cancelled'}
                        {cycle.fallbackOccurred ? ' · fallback' : ''}
                      </td>
                      <td className="px-2 py-2">
                        <span className="font-medium">{backendLabel(cycle.backend)}</span>
                        <span className="block text-[10px] text-on-surface-variant">
                          {cycle.backendOrder.map(backendLabel).join(' → ')} · {cycle.backendOrderSource === 'default' ? 'default order' : 'session memo'}
                        </span>
                      </td>
                      <td className="px-2 py-2 text-right tabular-nums">{milliseconds(cycle.cycleStartToFirstPcmMs)}</td>
                      <td className="pl-2 py-2 text-on-surface-variant">
                        {cycle.failureKind && <span className="block font-medium text-error">{diagnosticLabel(cycle.failureKind)}</span>}
                        {setupStep(cycle)}
                        {cycle.attempts.map((attempt) => (
                          <span key={`${attempt.resolutionPass}:${attempt.attemptIndex}`} className="mt-0.5 block text-[10px]">
                            P{attempt.resolutionPass} {backendLabel(attempt.backend)} · {attempt.outcome}
                            {attempt.attemptStartToFirstPcmMs !== null
                              ? ` in ${milliseconds(attempt.attemptStartToFirstPcmMs)}`
                              : attempt.activeElapsedMs === null
                                ? ' before elapsed timing was confirmed'
                                : ''}
                            {attempt.activeElapsedMs !== null ? ` · ${milliseconds(attempt.activeElapsedMs)} active` : ''}
                            {' · '}{milliseconds(attempt.attemptBudgetMs)} budget
                            {attempt.failureKind
                              ? ` · ${diagnosticLabel(attempt.failureKind)}${attempt.failurePhase ? ` (${diagnosticLabel(attempt.failurePhase)})` : ''}`
                              : ''}
                            {attempt.lastSetupStep ? ` · ${diagnosticLabel(attempt.lastSetupStep)} ${attempt.lastSetupTransition ?? ''}` : ''}
                          </span>
                        ))}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          ) : (
            <p className="text-[11px] text-on-surface-variant">The run was cancelled before its first cycle completed.</p>
          )}
        </div>
      )}
    </section>
  );
}
