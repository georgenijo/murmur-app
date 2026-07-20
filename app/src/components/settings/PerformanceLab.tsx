import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import {
  BenchmarkModel,
  BenchmarkModelResult,
  BenchmarkPreset,
  BenchmarkProgress,
  BenchmarkReport,
  MAX_SAVED_BENCHMARK_REPORTS,
  addBenchmarkReport,
  cancelBenchmark,
  clearBenchmarkReports,
  getBenchmarkActivity,
  getBenchmarkModels,
  loadBenchmarkReports,
  openBenchmarkOutputFolder,
  runBenchmark,
  saveBenchmarkReport,
  saveBenchmarkReports,
} from '../../lib/benchmark';
import { downloadModel } from '../../lib/dictation';
import {
  modelDownloadLabel,
  modelDownloadPercent,
  type ModelDownloadProgress,
} from '../../lib/modelDownload';
import type { Settings } from '../../lib/settings';
import type { DictationStatus } from '../../lib/types';

const PRESETS: { id: BenchmarkPreset; label: string; detail: string }[] = [
  { id: 'quick', label: 'Quick', detail: '2 clips x 3 runs' },
  { id: 'standard', label: 'Standard', detail: '7 clips x 5 runs' },
  { id: 'thorough', label: 'Thorough', detail: '9 clips x 10 runs' },
];

function milliseconds(value: number | null): string {
  return value === null ? '-' : `${Math.round(value)} ms`;
}

function percentage(value: number | null): string {
  return value === null ? '-' : `${(value * 100).toFixed(1)}%`;
}

function speed(value: number | null): string {
  return value && value > 0 ? `${Math.round(1 / value)}x` : '-';
}

function modelLabel(report: BenchmarkReport, modelName: string | null): string {
  if (!modelName) return '-';
  return report.results.find((result) => result.modelName === modelName)?.label ?? modelName;
}

type LatencyResult = BenchmarkModelResult & { warmMedianMs: number; warmP95Ms: number };
type AccuracyResult = BenchmarkModelResult & { normalizedWordErrorRate: number };

function latencyResults(report: BenchmarkReport): LatencyResult[] {
  return report.results.filter((result): result is LatencyResult => (
    !result.error && result.warmMedianMs !== null && result.warmP95Ms !== null
  ));
}

function accuracyResults(report: BenchmarkReport): AccuracyResult[] {
  return report.results.filter((result): result is AccuracyResult => (
    !result.error && result.normalizedWordErrorRate !== null
  ));
}

function LatencyChart({ report }: { report: BenchmarkReport }) {
  const results = latencyResults(report);
  const maximum = Math.max(...results.map((result) => result.warmP95Ms), 1);
  return (
    <div>
      <div className="mb-2 flex items-center justify-between gap-3">
        <h4 className="text-xs font-semibold text-on-surface dark:text-on-surface-variant">Inference latency</h4>
        <div className="flex gap-3 text-[10px] text-on-surface-variant dark:text-on-surface-variant">
          <span><i className="inline-block h-1.5 w-3 rounded-sm bg-emerald-500 mr-1" />Median</span>
          <span><i className="inline-block h-1.5 w-3 rounded-sm bg-surface-container-high dark:bg-surface-container-high mr-1" />P95</span>
        </div>
      </div>
      <div className="space-y-2">
        {results.map((result) => (
          <div key={result.modelName} className="grid grid-cols-[6.5rem_1fr_4.5rem] items-center gap-2">
            <span className="truncate text-[11px] font-medium text-on-surface dark:text-on-surface-variant" title={result.label}>{result.label}</span>
            <div className="relative h-4 rounded-sm bg-surface-container-low dark:bg-surface-container-high overflow-hidden">
              <div
                className="absolute inset-y-0 left-0 bg-surface-container-high dark:bg-surface-container-high"
                style={{ width: `${(result.warmP95Ms / maximum) * 100}%` }}
              />
              <div
                className="absolute left-0 top-[5px] h-1.5 bg-emerald-500"
                style={{ width: `${(result.warmMedianMs / maximum) * 100}%` }}
              />
            </div>
            <span className="text-right text-[10px] tabular-nums text-on-surface-variant dark:text-on-surface-variant">
              {Math.round(result.warmMedianMs)} / {Math.round(result.warmP95Ms)}
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}

function AccuracyChart({ report }: { report: BenchmarkReport }) {
  const results = accuracyResults(report);
  return (
    <div>
      <div className="mb-2 flex items-center justify-between gap-3">
        <h4 className="text-xs font-semibold text-on-surface dark:text-on-surface-variant">Word accuracy</h4>
        <span className="text-[10px] text-on-surface-variant dark:text-on-surface-variant">Normalized / higher is better</span>
      </div>
      <div className="space-y-2">
        {results.map((result) => {
          const accuracy = Math.max(0, 1 - result.normalizedWordErrorRate);
          return (
            <div key={result.modelName} className="grid grid-cols-[6.5rem_1fr_3rem] items-center gap-2">
              <span className="truncate text-[11px] font-medium text-on-surface dark:text-on-surface-variant" title={result.label}>{result.label}</span>
              <div className="h-2.5 rounded-sm bg-surface-container-low dark:bg-surface-container-high overflow-hidden">
                <div className="h-full bg-amber-400 dark:bg-amber-500" style={{ width: `${accuracy * 100}%` }} />
              </div>
              <span className="text-right text-[10px] tabular-nums text-on-surface-variant dark:text-on-surface-variant">
                {(accuracy * 100).toFixed(1)}%
              </span>
            </div>
          );
        })}
      </div>
    </div>
  );
}

export function PerformanceLab({ status, settings, onUpdateSettings }: {
  status: DictationStatus;
  settings: Settings;
  onUpdateSettings: (updates: Partial<Settings>) => void;
}) {
  const [models, setModels] = useState<BenchmarkModel[]>([]);
  const [selected, setSelected] = useState<string[]>([]);
  const [preset, setPreset] = useState<BenchmarkPreset>('standard');
  const [progress, setProgress] = useState<BenchmarkProgress | null>(null);
  const [dashboard, setDashboard] = useState<{
    reports: BenchmarkReport[];
    selectedAt: string | null;
  }>(() => {
    const reports = loadBenchmarkReports();
    return { reports, selectedAt: reports[0]?.createdAt ?? null };
  });
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [saveState, setSaveState] = useState<'idle' | 'saving' | 'saved'>('idle');
  const [downloading, setDownloading] = useState<string | null>(null);
  const [downloadProgress, setDownloadProgress] = useState<ModelDownloadProgress | null>(null);
  const [fileTranscribing, setFileTranscribing] = useState(false);
  const mounted = useRef(true);
  const runningRef = useRef(false);

  const refreshModels = useCallback(async () => {
    const catalog = await getBenchmarkModels();
    if (!mounted.current) return;
    setModels(catalog);
    setSelected((current) => {
      const installed = new Set(catalog.filter((model) => model.installed).map((model) => model.modelName));
      const retained = current.filter((name) => installed.has(name));
      if (retained.length > 0) return retained;
      return catalog.filter((model) => model.installed).map((model) => model.modelName);
    });
  }, []);

  useEffect(() => {
    mounted.current = true;
    refreshModels().catch((reason: unknown) => setError(String(reason)));
    return () => {
      mounted.current = false;
      if (runningRef.current) void cancelBenchmark();
    };
  }, [refreshModels]);

  useEffect(() => {
    runningRef.current = running;
  }, [running]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    listen<BenchmarkProgress>('benchmark-progress', (event) => setProgress(event.payload))
      .then((dispose) => {
        if (disposed) dispose();
        else unlisten = dispose;
      })
      .catch((reason: unknown) => {
        if (!disposed) setError(`Could not watch benchmark progress: ${String(reason)}`);
      });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    getBenchmarkActivity()
      .then((activity) => {
        if (!disposed) setFileTranscribing(activity.fileTranscribing);
      })
      .catch(() => {});
    listen<boolean>('file-transcription-status-changed', (event) => {
      if (!disposed) setFileTranscribing(event.payload);
    })
      .then((dispose) => {
        if (disposed) dispose();
        else unlisten = dispose;
      })
      .catch(() => {});
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const installedCount = models.filter((model) => model.installed).length;
  const report = dashboard.reports.find((item) => item.createdAt === dashboard.selectedAt) ?? null;
  const progressPercent = progress && progress.total > 0
    ? Math.round((progress.completed / progress.total) * 100)
    : 0;
  const canRun = selected.length > 0 && !running && status === 'idle' && !fileTranscribing;

  const selectedSet = useMemo(() => new Set(selected), [selected]);
  const toggleModel = (modelName: string) => {
    setSelected((current) => current.includes(modelName)
      ? current.filter((name) => name !== modelName)
      : [...current, modelName]);
  };

  const handleRun = async () => {
    setError(null);
    setCopied(false);
    setProgress(null);
    runningRef.current = true;
    setRunning(true);
    try {
      const next = await runBenchmark(selected, preset);
      if (!mounted.current) return;
      setDashboard((current) => {
        const reports = saveBenchmarkReports(addBenchmarkReport(current.reports, next));
        return { reports, selectedAt: next.createdAt };
      });
      if (settings.benchmarkAutoSave) {
        // Best-effort: a write failure must not fail the completed run.
        try {
          await saveBenchmarkReport(next, settings.benchmarkOutputDir);
        } catch (reason) {
          if (mounted.current) setError(`Could not auto-save report: ${String(reason)}`);
        }
      }
    } catch (reason) {
      if (mounted.current && String(reason) !== 'Benchmark cancelled') setError(String(reason));
    } finally {
      runningRef.current = false;
      if (mounted.current) setRunning(false);
    }
  };

  const handleCancel = async () => {
    await cancelBenchmark();
  };

  const handleDownload = async (modelName: string) => {
    setError(null);
    setDownloading(modelName);
    setDownloadProgress(null);
    let unlisten: (() => void) | undefined;
    try {
      unlisten = await listen<ModelDownloadProgress>('download-progress', (event) => {
        setDownloadProgress(event.payload);
      });
      await downloadModel(modelName);
      await refreshModels();
      setSelected((current) => current.includes(modelName) ? current : [...current, modelName]);
    } catch (reason) {
      setError(String(reason));
    } finally {
      unlisten?.();
      setDownloading(null);
      setDownloadProgress(null);
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
      await saveBenchmarkReport(report, settings.benchmarkOutputDir);
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

  const revealFolder = async () => {
    try {
      await openBenchmarkOutputFolder(settings.benchmarkOutputDir);
    } catch (reason) {
      setError(`Could not open output folder: ${String(reason)}`);
    }
  };

  const chooseFolder = async () => {
    try {
      const selected = await open({ directory: true, multiple: false });
      if (typeof selected === 'string') onUpdateSettings({ benchmarkOutputDir: selected });
    } catch {
      // Cancellation leaves the stored folder untouched.
    }
  };

  return (
    <div className="space-y-6">
      <div className="rounded-xl border border-amber-500/30 bg-amber-500/10 p-3 text-xs text-on-surface-variant">
        <p className="font-medium text-on-surface">Directional results, not a universal model ranking</p>
        <p className="mt-1">This lab compares installed models on this Mac with a small, clean synthetic English corpus. It does not represent your voice, microphone, accent, room, or every dictation workload.</p>
      </div>
      <section>
        <div className="flex items-end justify-between gap-4 mb-3">
          <div>
            <h3 className="text-sm font-semibold text-on-surface">Models</h3>
            <p className="mt-0.5 text-xs text-on-surface-variant dark:text-on-surface-variant">
              {installedCount} installed. All tests run locally.
            </p>
          </div>
          {installedCount > 0 && (
            <button
              type="button"
              disabled={running}
              onClick={() => setSelected(models.filter((model) => model.installed).map((model) => model.modelName))}
              className="text-xs text-on-surface dark:text-on-surface-variant hover:text-on-surface dark:hover:text-white disabled:opacity-50"
            >
              Select installed
            </button>
          )}
        </div>

        <div className="border-y border-outline-variant/30 dark:border-outline-variant/30 divide-y divide-outline-variant/30 dark:divide-outline-variant/30">
          {models.filter((model) => model.supported).map((model) => (
            <div key={model.modelName} className="min-h-14 flex items-center gap-3 py-2.5">
              <input
                type="checkbox"
                aria-label={`Benchmark ${model.label}`}
                checked={selectedSet.has(model.modelName)}
                disabled={!model.installed || running}
                onChange={() => toggleModel(model.modelName)}
                className="h-4 w-4 accent-primary dark:accent-primary"
              />
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <span className="text-sm font-medium text-on-surface dark:text-on-surface">{model.label}</span>
                  <span className="text-[11px] text-on-surface-variant dark:text-on-surface-variant">{model.size}</span>
                </div>
                <p className="text-xs text-on-surface-variant dark:text-on-surface-variant truncate">
                  {model.backend} / {model.accelerator}
                </p>
              </div>
              {!model.installed && (
                <button
                  type="button"
                  disabled={downloading !== null || running}
                  onClick={() => handleDownload(model.modelName)}
                  className="shrink-0 px-2.5 py-1.5 text-xs font-medium border border-outline-variant/30 dark:border-outline-variant/30 rounded-md text-on-surface dark:text-on-surface-variant hover:bg-surface-container-low dark:hover:bg-primary/80 disabled:opacity-50"
                >
                  {downloading === model.modelName
                    ? downloadProgress === null
                      ? 'Starting...'
                      : modelDownloadPercent(downloadProgress) === null
                        ? modelDownloadLabel(downloadProgress)
                        : `${modelDownloadPercent(downloadProgress)}%`
                    : 'Download'}
                </button>
              )}
            </div>
          ))}
        </div>
      </section>

      <section>
        <h3 className="mb-2 text-sm font-semibold text-on-surface">Test Length</h3>
        <div className="grid grid-cols-3 gap-1 p-1 bg-surface-container-low dark:bg-surface-container-high rounded-lg">
          {PRESETS.map((option) => (
            <button
              type="button"
              key={option.id}
              disabled={running}
              onClick={() => setPreset(option.id)}
              className={`min-w-0 px-2 py-2 rounded-md transition-colors disabled:opacity-50 ${
                preset === option.id
                  ? 'bg-surface-container-lowest dark:bg-surface-container-high shadow-sm text-on-surface dark:text-on-surface'
                  : 'text-on-surface-variant hover:text-on-surface'
              }`}
            >
              <span className="block text-xs font-semibold">{option.label}</span>
              <span className="block mt-0.5 text-[10px] whitespace-normal leading-tight">{option.detail}</span>
            </button>
          ))}
        </div>
      </section>

      <section>
        {running ? (
          <div className="space-y-2">
            <div className="flex items-center justify-between gap-3 text-xs">
              <span className="min-w-0 truncate text-on-surface dark:text-on-surface-variant">
                {progress ? `${progress.modelLabel}${progress.fixture ? ` / ${progress.fixture}` : ''} / ${progress.phase}` : 'Starting benchmark'}
              </span>
              <span className="shrink-0 tabular-nums text-on-surface-variant">{progressPercent}%</span>
            </div>
            <div className="h-1.5 overflow-hidden rounded-full bg-surface-container-high dark:bg-surface-container-high">
              <div className="h-full bg-emerald-500 transition-all duration-200" style={{ width: `${progressPercent}%` }} />
            </div>
            <button
              type="button"
              onClick={handleCancel}
              className="w-full rounded-lg border border-outline-variant/30 px-3 py-2 text-xs font-medium text-on-surface hover:bg-surface-container-low"
            >
              Cancel
            </button>
          </div>
        ) : (
          <button
            type="button"
            onClick={handleRun}
            disabled={!canRun}
            className="w-full px-4 py-2.5 text-sm font-semibold rounded-lg bg-primary dark:bg-surface-container-low text-white dark:text-on-surface hover:bg-primary/80 dark:hover:bg-surface-container-lowest disabled:opacity-40 disabled:cursor-not-allowed"
          >
            Run Benchmark
          </button>
        )}
        {status !== 'idle' && (
          <p className="mt-2 text-xs text-amber-600 dark:text-amber-400">Finish the current recording first.</p>
        )}
        {fileTranscribing && (
          <p className="mt-2 text-xs text-amber-600 dark:text-amber-400">Finish the file transcription first.</p>
        )}
        {error && (
          <p className="mt-2 text-xs text-red-600 dark:text-red-400 break-words">{error}</p>
        )}
      </section>

      {report && !running && (
        <section className="space-y-4 border-t border-outline-variant/30 dark:border-outline-variant/30 pt-5">
          <div className="flex items-center justify-between gap-3">
            <div>
              <h3 className="text-sm font-semibold text-on-surface">Results</h3>
              <p className="mt-0.5 text-[11px] text-on-surface-variant dark:text-on-surface-variant">
                {report.environment
                  ? `${report.environment.os}${report.environment.osVersion ? ` ${report.environment.osVersion}` : ''} / ${report.environment.chip ?? report.environment.hardwareModel ?? report.environment.architecture}${report.environment.memoryMb ? ` / ${Math.round(report.environment.memoryMb / 1024)} GB RAM` : ''}`
                  : report.platform}
                {' / '}Murmur v{report.appVersion} / {report.preset}
              </p>
              <p
                className="mt-0.5 text-[11px] text-on-surface-variant dark:text-on-surface-variant"
                title="One-time shared backend init (Metal shader compilation, ANE compile cache, etc.) measured once before per-model timing, so it doesn't skew any single model's cold-load number."
              >
                Shared init (one-time): {milliseconds(report.sharedInitMs)}
              </p>
            </div>
            <div className="flex shrink-0 items-center gap-2">
              <button
                type="button"
                onClick={copyReport}
                className="px-2.5 py-1.5 text-xs font-medium border border-outline-variant/30 dark:border-outline-variant/30 rounded-md text-on-surface dark:text-on-surface-variant hover:bg-surface-container-low dark:hover:bg-primary/80"
              >
                {copied ? 'Copied' : 'Copy JSON'}
              </button>
              <button
                type="button"
                onClick={saveReport}
                disabled={saveState === 'saving'}
                className="px-2.5 py-1.5 text-xs font-medium border border-outline-variant/30 dark:border-outline-variant/30 rounded-md text-on-surface dark:text-on-surface-variant hover:bg-surface-container-low dark:hover:bg-primary/80 disabled:opacity-50"
              >
                {saveState === 'saved' ? 'Saved' : saveState === 'saving' ? 'Saving…' : 'Save to file'}
              </button>
              <button
                type="button"
                onClick={revealFolder}
                title="Open output folder"
                aria-label="Open output folder"
                className="p-1.5 border border-outline-variant/30 dark:border-outline-variant/30 rounded-md text-on-surface-variant hover:bg-surface-container-low dark:hover:bg-primary/80"
              >
                <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="currentColor" className="h-4 w-4">
                  <path d="M3.75 3A1.75 1.75 0 0 0 2 4.75v10.5c0 .966.784 1.75 1.75 1.75h12.5A1.75 1.75 0 0 0 18 15.25v-8.5A1.75 1.75 0 0 0 16.25 5h-6.19l-1.2-1.44A1.75 1.75 0 0 0 7.52 3H3.75Z" />
                </svg>
              </button>
            </div>
          </div>

          <div className="rounded-lg border border-outline-variant/30 bg-surface-container-low p-3 text-[11px] leading-relaxed text-on-surface-variant">
            <p className="font-medium text-on-surface">What this run measured</p>
            <p className="mt-1">
              {report.corpus
                ? `${report.corpus.fixtureCount} ${report.corpus.language.toUpperCase()} synthetic clips / ${report.corpus.referenceWords} reference words / ${report.iterations} measured runs per clip.`
                : `${report.results.find((result) => !result.error)?.fixtures.length ?? 0} synthetic clips / ${report.iterations} measured runs per clip (legacy saved report).`}
            </p>
            <p className="mt-1">{report.configuration?.executionPath ?? 'Full-buffer final transcription after recording stops'}; VAD threshold {report.configuration?.vadThreshold ?? 0.5}; {report.configuration?.transcriptTransformProfile ?? 'default local delivery pipeline'}.</p>
            {report.configuration && <p className="mt-1">Model run order: {report.configuration.modelRunOrder.join(' → ') || 'none'}. Shared-init order: {report.configuration.sharedInitOrder.join(' → ') || 'none'}.</p>}
            <p className="mt-1">{report.corpus?.limitation ?? 'Directional local comparison only; the synthetic corpus is not representative of natural speakers or recording environments.'}</p>
          </div>

          <div className="rounded-lg border border-outline-variant/30 bg-surface-container-low p-3 text-[11px] leading-relaxed text-on-surface-variant">
            <p className="font-medium text-on-surface">Report export folder</p>
            <p className="mt-1 break-all rounded-md border border-outline-variant/30 bg-surface-container-lowest px-2.5 py-1.5 text-on-surface">
              {settings.benchmarkOutputDir || 'Documents/Murmur (default)'}
            </p>
            <div className="mt-2 flex gap-3">
              <button type="button" onClick={() => void chooseFolder()} className="font-medium text-on-surface-variant underline hover:text-primary">Choose Folder</button>
              {settings.benchmarkOutputDir && <button type="button" onClick={() => onUpdateSettings({ benchmarkOutputDir: '' })} className="font-medium text-on-surface-variant underline hover:text-primary">Reset to default</button>}
            </div>
            <label className="mt-3 flex items-center gap-2 text-on-surface">
              <input
                type="checkbox"
                checked={settings.benchmarkAutoSave}
                onChange={() => onUpdateSettings({ benchmarkAutoSave: !settings.benchmarkAutoSave })}
                className="h-3.5 w-3.5 accent-primary"
              />
              <span>Auto-save every run to this folder (survives the {MAX_SAVED_BENCHMARK_REPORTS}-run in-app limit)</span>
            </label>
          </div>

          <div className="flex items-center gap-2">
            <label htmlFor="benchmark-run" className="shrink-0 text-[11px] text-on-surface-variant dark:text-on-surface-variant">Saved run</label>
            <select
              id="benchmark-run"
              value={dashboard.selectedAt ?? ''}
              onChange={(event) => setDashboard((current) => ({ ...current, selectedAt: event.target.value }))}
              className="min-w-0 flex-1 px-2 py-1.5 text-xs rounded-md border border-outline-variant/30 dark:border-outline-variant/30 bg-surface-container-lowest dark:bg-surface-container-high text-on-surface dark:text-on-surface-variant focus:outline-none focus:ring-2 focus:ring-primary"
            >
              {dashboard.reports.map((item) => (
                <option key={item.createdAt} value={item.createdAt}>
                  {new Date(item.createdAt).toLocaleString()} / {item.results.length} model{item.results.length === 1 ? '' : 's'}
                </option>
              ))}
            </select>
            <button
              type="button"
              onClick={() => {
                clearBenchmarkReports();
                setDashboard({ reports: [], selectedAt: null });
              }}
              className="shrink-0 px-2 py-1.5 text-xs text-on-surface-variant dark:text-on-surface-variant hover:text-red-600 dark:hover:text-red-400"
            >
              Clear
            </button>
          </div>

          <div className="grid grid-cols-3 border-y border-outline-variant/30 dark:border-outline-variant/30 divide-x divide-outline-variant/30 dark:divide-outline-variant/30">
            {[
              ['Fastest', modelLabel(report, report.recommendations.fastest)],
              ['Accurate', modelLabel(report, report.recommendations.mostAccurate)],
              ['Balanced', modelLabel(report, report.recommendations.balanced)],
            ].map(([label, value]) => (
              <div key={label} className="min-w-0 px-2 py-2.5 text-center">
                <div className="text-[10px] uppercase text-on-surface-variant dark:text-on-surface-variant">{label}</div>
                <div className="mt-1 text-xs font-semibold text-on-surface dark:text-on-surface truncate" title={value}>{value}</div>
              </div>
            ))}
          </div>

          <div className="space-y-4 border-b border-outline-variant/30 dark:border-outline-variant/30 pb-4">
            <LatencyChart report={report} />
            <AccuracyChart report={report} />
          </div>

          <div>
            <h4 className="mb-1 text-xs font-semibold text-on-surface dark:text-on-surface-variant">Metrics</h4>
            <table className="w-full table-fixed text-[11px]">
              <colgroup>
                <col className="w-[28%]" />
                <col className="w-[14%]" />
                <col className="w-[13%]" />
                <col className="w-[13%]" />
                <col className="w-[16%]" />
                <col className="w-[16%]" />
              </colgroup>
              <thead className="text-left text-on-surface-variant dark:text-on-surface-variant">
                <tr className="border-b border-outline-variant/30 dark:border-outline-variant/30">
                  <th className="py-2 pr-3 font-medium">Model</th>
                  <th className="px-2 py-2 font-medium text-right">Median</th>
                  <th className="px-2 py-2 font-medium text-right">P95</th>
                  <th className="px-2 py-2 font-medium text-right">Speed</th>
                  <th className="px-2 py-2 font-medium text-right">WER norm (raw)</th>
                  <th className="pl-2 py-2 font-medium text-right">Delivered (raw)</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-outline-variant/30 dark:divide-outline-variant/30 text-on-surface dark:text-on-surface-variant">
                {report.results.map((result) => (
                  <tr key={result.modelName}>
                    <td className="py-2.5 pr-3">
                      <span className="font-medium text-on-surface dark:text-on-surface">{result.label}</span>
                      <span className="block text-[10px] text-on-surface-variant">{result.accelerator}</span>
                    </td>
                    {result.error ? (
                      <td colSpan={5} className="px-2 py-2.5 text-red-600 dark:text-red-400">{result.error}</td>
                    ) : (
                      <>
                        <td className="px-2 py-2.5 text-right tabular-nums">{milliseconds(result.warmMedianMs)}</td>
                        <td className="px-2 py-2.5 text-right tabular-nums">{milliseconds(result.warmP95Ms)}</td>
                        <td className="px-2 py-2.5 text-right tabular-nums">{speed(result.realtimeFactor)}</td>
                        <td className="px-2 py-2.5 text-right tabular-nums">
                          {percentage(result.normalizedWordErrorRate)}
                          <span className="text-on-surface-variant dark:text-on-surface-variant"> ({percentage(result.wordErrorRate)})</span>
                        </td>
                        <td className="pl-2 py-2.5 text-right tabular-nums">
                          {percentage(result.deliveredNormalizedWordErrorRate)}
                          <span className="text-on-surface-variant dark:text-on-surface-variant"> ({percentage(result.deliveredWordErrorRate)})</span>
                        </td>
                      </>
                    )}
                  </tr>
                ))}
              </tbody>
            </table>
          </div>

          <p className="text-[11px] leading-relaxed text-on-surface-variant dark:text-on-surface-variant">
            WER counts changed, missing, and extra words against the known transcript. Normalized WER first ignores formatting and number/unit spelling (16 kHz = sixteen kilohertz, front end = frontend) so it reflects recognition, not formatting; raw WER is shown in parentheses. Delivered WER scores the text after the production transform pipeline (dev-vocab prompt for Whisper, then cleanup / correction / formatting) — what actually reaches the clipboard — again shown normalized with raw in parentheses. P95 uses nearest-rank selection over the measured warm runs; with only 3, 5, or 10 samples it is a coarse tail-latency signal. Accuracy ranking and the Accurate/Balanced picks use the normalized recognition number. Fastest is the strict minimum duration-weighted speed. Balanced favors lower memory among models within 2 accuracy points and an inclusive 10% band of the fastest eligible speed.
          </p>

          <div className="space-y-2">
            {report.results.filter((result) => !result.error).map((result) => (
              <details key={result.modelName} className="border-t border-outline-variant/30 dark:border-outline-variant/30 pt-2">
                <summary className="cursor-pointer text-xs font-medium text-on-surface dark:text-on-surface-variant">
                  {result.label} details
                </summary>
                <div className="mt-2 space-y-3">
                  <div className="grid grid-cols-2 gap-x-4 gap-y-1 text-[11px] text-on-surface-variant dark:text-on-surface-variant">
                    <span title="Per-model load measured after the separate one-time shared backend initialization shown above.">Model load after shared init</span><span className="text-right tabular-nums">{milliseconds(result.modelLoadMs)}</span>
                    <span>First inference</span><span className="text-right tabular-nums">{milliseconds(result.firstInferenceMs)}</span>
                    <span>Download size</span><span className="text-right tabular-nums">{result.downloadSize ?? 'Not recorded'}</span>
                    <span title="Observed process RSS delta. Models run sequentially in one process, so allocator retention from an earlier model can inflate a later model's baseline. This is not download size or isolated peak memory.">Observed memory increase</span><span className="text-right tabular-nums">{result.memoryDeltaMb} MB (rough)</span>
                  </div>
                  {result.fixtures.map((fixture) => (
                    <div key={fixture.fixtureId} className="text-[11px] leading-relaxed">
                      <div className="flex justify-between gap-3 font-medium text-on-surface dark:text-on-surface-variant">
                        <span>
                          {fixture.label} / {fixture.audioSeconds.toFixed(1)}s
                          {fixture.normalizedWordErrors === 0 && (
                            <span
                              className="ml-1 text-emerald-500 dark:text-emerald-400"
                              title="This model scored a perfect normalized transcript on this clip — the clip does not distinguish it from other top models."
                            >
                              (saturated)
                            </span>
                          )}
                        </span>
                        <span>
                          {fixture.normalizedWordErrors}/{fixture.normalizedReferenceWords} errors
                          <span className="text-on-surface-variant dark:text-on-surface-variant"> ({fixture.wordErrors}/{fixture.referenceWords} raw)</span>
                        </span>
                      </div>
                      <div className="mt-1 grid gap-1 text-on-surface-variant dark:text-on-surface-variant">
                        <p><span className="font-medium">Reference:</span> {fixture.reference}</p>
                        <p><span className="font-medium">Output:</span> {fixture.transcript || '(empty)'}</p>
                        <p>
                          <span className="font-medium">Delivered:</span> {fixture.deliveredTranscript || '(empty)'}
                          <span className="text-on-surface-variant dark:text-on-surface-variant"> — {fixture.deliveredNormalizedWordErrors}/{fixture.normalizedReferenceWords} errors ({fixture.deliveredWordErrors}/{fixture.referenceWords} raw)</span>
                          {fixture.deliveredTransformFailed && (
                            <span className="text-amber-600 dark:text-amber-400"> — transform failed, showing raw</span>
                          )}
                        </p>
                      </div>
                    </div>
                  ))}
                </div>
              </details>
            ))}
          </div>
        </section>
      )}
    </div>
  );
}
