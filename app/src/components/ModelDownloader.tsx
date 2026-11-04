import { useState, useEffect, useRef, type ReactNode } from 'react';
import { listen } from '@tauri-apps/api/event';
import { cn } from '../lib/sona-utils';
import { cancelModelDownload, downloadModel } from '../lib/dictation';
import { AVAILABLE_MODEL_OPTIONS, type ModelOption } from '../lib/settings';
import { preferredInstalledModel, useInstalledModels } from '../lib/hooks/useInstalledModels';
import {
  correlatedModelDownloadAttempt,
  modelDownloadLabel,
  modelDownloadPercent,
  type ModelDownloadProgress,
} from '../lib/modelDownload';
import {
  modelGuidanceSummary,
  modelMemoryWarning,
  useModelHardwareGuidance,
} from '../lib/modelHardwareGuidance';

const MODEL_DESCRIPTIONS: Record<string, string> = {
  'parakeet-tdt-0.6b-v3-coreml': 'Fastest on Apple Silicon — multilingual, Apple Neural Engine',
  'parakeet-tdt-0.6b-v2-fp16': 'Fast CPU fallback — English only',
  'large-v3-turbo': 'Highest accuracy, slower (1-2 seconds)',
  'base.en': 'Good balance of speed and accuracy',
};

/**
 * Subset of models shown on the initial download screen (first = default).
 * Installed models outside this curated list are also offered.
 */
export const DOWNLOAD_MODEL_KEYS: ModelOption[] = [
  'parakeet-tdt-0.6b-v3-coreml',
  'parakeet-tdt-0.6b-v2-fp16',
  'large-v3-turbo',
  'base.en',
];
const MODELS = AVAILABLE_MODEL_OPTIONS.map((opt) => ({
  name: opt.value, label: opt.label, size: opt.size, description: MODEL_DESCRIPTIONS[opt.value] ?? '',
}));

interface Props {
  initialModel: ModelOption;
  onComplete: (model: ModelOption) => void;
  /** Notifies embedders when a download starts/stops (e.g. to lock navigation). */
  onDownloadingChange?: (downloading: boolean) => void;
  /**
   * Per-model on-disk status from the embedder's probe. Installed rows show an
   * "Installed" chip instead of the size, and selecting one turns the primary
   * button into Continue (completes without downloading). Omitted = unknown,
   * every row behaves as before.
   */
  installedModels?: Partial<Record<ModelOption, boolean>>;
  /** Lets an embedder place the live primary action in its own navigation row. */
  renderPrimaryAction?: (action: {
    label: string;
    disabled: boolean;
    onActivate: () => void;
  }) => ReactNode;
}

interface DownloadOperation {
  requestId: string;
  model: ModelOption;
  cancelled: boolean;
  started: boolean;
  done: Promise<void>;
}

type DownloadState =
  | { phase: 'idle' }
  | { phase: 'downloading'; progress: ModelDownloadProgress }
  | { phase: 'error'; message: string };

/**
 * Model picker + download progress card, without the full-screen chrome.
 * Used by the standalone first-launch gate (ModelDownloader) and embedded in
 * the onboarding wizard's model step (OnboardingFlow).
 */
export function ModelDownloadPanel({
  initialModel,
  onComplete,
  onDownloadingChange,
  installedModels,
  renderPrimaryAction,
}: Props) {
  const hardwareGuidance = useModelHardwareGuidance();
  const offeredModels = [
    ...DOWNLOAD_MODEL_KEYS,
    ...MODELS.filter((model) => !DOWNLOAD_MODEL_KEYS.includes(model.name) && installedModels?.[model.name]).map((model) => model.name),
  ].map((name) => MODELS.find((model) => model.name === name)!);
  const [selected, setSelected] = useState<ModelOption>(
    offeredModels.some((model) => model.name === initialModel) ? initialModel : offeredModels[0].name
  );
  const [downloadState, setDownloadState] = useState<DownloadState>({ phase: 'idle' });
  const operationRef = useRef<DownloadOperation | null>(null);
  const [cancelling, setCancelling] = useState(false);
  const [cancelError, setCancelError] = useState<string | null>(null);
  const downloadUnlistenRef = useRef<(() => void) | null>(null);
  const activeAttemptRef = useRef<number | null>(null);

  useEffect(() => {
    return () => {
      const operation = operationRef.current;
      operationRef.current = null;
      if (operation) {
        operation.cancelled = true;
        if (operation.started) void cancelModelDownload(operation.model, operation.requestId).catch(() => {});
      }
      downloadUnlistenRef.current?.();
      downloadUnlistenRef.current = null;
      activeAttemptRef.current = null;
    };
  }, []);

  const handleDownload = () => {
    if (operationRef.current) return;
    const requestedModel = selected;
    const operation: DownloadOperation = {
      requestId: crypto.randomUUID(), model: requestedModel,
      cancelled: false, started: false, done: Promise.resolve(),
    };
    operationRef.current = operation;
    activeAttemptRef.current = null;
    setCancelError(null);
    onDownloadingChange?.(true);
    setDownloadState({
      phase: 'downloading',
      progress: { modelName: requestedModel, received: 0, total: 0, phase: 'downloading' },
    });
    operation.done = (async () => {
      let unlisten: (() => void) | null = null;
      try {
        unlisten = await listen<ModelDownloadProgress>('download-progress', (event) => {
          if (operationRef.current !== operation || operation.cancelled) return;
          const progress = event.payload;
          if (progress.requestId !== operation.requestId) return;
          const attemptId = correlatedModelDownloadAttempt(progress, requestedModel, activeAttemptRef.current);
          if (attemptId === undefined) return;
          activeAttemptRef.current = attemptId;
          setDownloadState({ phase: 'downloading', progress });
        });
        const nativeUnlisten = unlisten;
        let listening = true;
        unlisten = () => {
          if (!listening) return;
          listening = false;
          nativeUnlisten();
        };
        if (operationRef.current !== operation || operation.cancelled) return;
        downloadUnlistenRef.current = unlisten;
        operation.started = true;
        await downloadModel(requestedModel, operation.requestId);
        if (operationRef.current !== operation || operation.cancelled) return;
        operationRef.current = null;
        onDownloadingChange?.(false);
        onComplete(requestedModel);
      } catch (err) {
        if (operationRef.current !== operation || operation.cancelled) return;
        operationRef.current = null;
        onDownloadingChange?.(false);
        setDownloadState({ phase: 'error', message: String(err) });
      } finally {
        unlisten?.();
        if (downloadUnlistenRef.current === unlisten) downloadUnlistenRef.current = null;
      }
    })();
  };

  const handleCancel = async () => {
    const operation = operationRef.current;
    if (!operation) return;
    operation.cancelled = true;
    setCancelling(true);
    setCancelError(null);
    try {
      if (operation.started) await cancelModelDownload(operation.model, operation.requestId);
      // A pending listen has no native owner. Its continuation checks cancelled.
      if (operation.started) await operation.done;
      if (operationRef.current !== operation) return;
      operationRef.current = null;
      activeAttemptRef.current = null;
      downloadUnlistenRef.current?.();
      downloadUnlistenRef.current = null;
      setDownloadState({ phase: 'idle' });
      setCancelling(false);
      onDownloadingChange?.(false);
    } catch (error) {
      if (operationRef.current !== operation) return;
      // Keep navigation locked until native termination is confirmed.
      setCancelError(String(error));
      setCancelling(false);
    }
  };

  const progress = downloadState.phase === 'downloading' ? downloadState.progress : null;
  const progressPercent = progress ? modelDownloadPercent(progress) : null;

  const isDownloading = downloadState.phase === 'downloading';
  const selectedInstalled = installedModels?.[selected] === true;
  const selectedMemoryWarning = hardwareGuidance
    ? modelMemoryWarning(hardwareGuidance, selected)
    : null;
  const selectModel = (model: ModelOption) => {
    if (isDownloading) return;
    setSelected(model);
    setDownloadState({ phase: 'idle' });
  };
  const chooseFallback = (model: ModelOption) => {
    selectModel(model);
    window.requestAnimationFrame(() => {
      document.getElementById(`download-model-${model}`)?.focus();
    });
  };
  const primaryLabel = isDownloading
    ? progress ? modelDownloadLabel(progress) : 'Starting…'
    : selectedInstalled
    ? 'Continue'
    : downloadState.phase === 'error'
    ? 'Retry download'
    : 'Download';
  const activatePrimary = () => {
    if (selectedInstalled) {
      onComplete(selected);
      return;
    }
    void handleDownload();
  };

  return (
    <div>
      {hardwareGuidance && (
        <p className="mb-3 text-xs text-on-surface-variant" data-testid="model-guidance-summary">
          {modelGuidanceSummary(hardwareGuidance)}
        </p>
      )}
      <div className="space-y-2 mb-6">
          {offeredModels.map((model) => (
            <button
              key={model.name}
              id={`download-model-${model.name}`}
              type="button"
              aria-pressed={selected === model.name}
              onClick={() => selectModel(model.name)}
              disabled={isDownloading}
              className={cn(
                'dialog-card w-full px-4 py-3 text-left transition-colors',
                selected === model.name
                  ? 'border-primary/45 bg-[var(--ui-tint-accent-subtle)] shadow-[var(--ui-shadow-2)]'
                  : 'hover:border-primary/30',
                isDownloading ? 'opacity-50 cursor-not-allowed' : 'cursor-pointer',
              )}
            >
              <div className="flex items-center justify-between gap-3">
                <span className="text-sm font-medium text-on-surface">
                  {model.label}
                </span>
                <span className="flex shrink-0 items-center gap-1.5">
                  {hardwareGuidance?.recommendedModel === model.name && (
                    <span className="rounded-full bg-[var(--ui-tint-accent-strong)] px-2 py-0.5 text-xs font-semibold text-on-surface">
                      Recommended
                    </span>
                  )}
                  {hardwareGuidance?.warnedModels.includes(model.name) && (
                    <span className="rounded-full bg-warning/15 px-2 py-0.5 text-xs font-semibold text-warning">
                      Higher memory
                    </span>
                  )}
                  {installedModels?.[model.name] ? (
                    <span className="rounded-full bg-[var(--ui-tint-accent-strong)] px-2 py-0.5 text-[11px] font-medium text-on-surface">
                      Installed
                    </span>
                  ) : (
                    <span className="text-xs text-on-surface-variant font-mono">
                      {model.size}
                    </span>
                  )}
                </span>
              </div>
              <p className="text-xs text-on-surface-variant mt-0.5">
                {model.description}
              </p>
            </button>
          ))}
        </div>

        {selectedMemoryWarning && (
          <div role="status" className="dialog-toast mb-4 border-warning/30 bg-warning/10 px-4 py-3 text-xs text-on-surface">
            {selectedMemoryWarning}
          </div>
        )}

        {isDownloading && (
          <div className="mb-4" aria-live="polite" aria-busy="true">
            <div className="flex justify-between text-xs text-on-surface-variant mb-1">
              <span>{cancelling ? 'Cancelling…' : progress ? modelDownloadLabel(progress) : 'Starting…'}</span>
              {progressPercent !== null ? (
                <span>{progressPercent}%</span>
              ) : (
                <span>Working…</span>
              )}
            </div>
            <div className="w-full h-2 overflow-hidden rounded-full bg-[var(--ui-tint-sunken)]">
              <div
                role="progressbar"
                aria-valuenow={progressPercent ?? undefined}
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuetext={progressPercent === null ? 'Model installation in progress' : undefined}
                className={cn(
                  'h-full rounded-full bg-[linear-gradient(140deg,var(--murmur-primary),var(--murmur-primary-dim))]',
                  progressPercent === null ? 'model-download-indeterminate' : 'transition-all duration-200',
                )}
                style={progressPercent === null ? undefined : { width: `${progressPercent}%` }}
              />
            </div>
            {progress && progress.total > 0 && progress.phase !== 'installing' && (
              <p className="text-xs text-on-surface-variant mt-1 text-right">
                {(progress.received / 1024 / 1024).toFixed(1)} /{' '}
                {(progress.total / 1024 / 1024).toFixed(1)} MB
              </p>
            )}
          </div>
        )}

        {isDownloading && (
          <div className="mb-4">
            <button type="button" onClick={() => void handleCancel()} disabled={cancelling}
              className="dialog-pill-btn px-4 py-2 text-[length:var(--ui-font-body)] text-on-surface disabled:opacity-50">
              {cancelling ? 'Cancelling…' : 'Cancel download'}
            </button>
            <p className="mt-2 text-[length:var(--ui-font-caption)] text-on-surface-variant">Cancel to go back or choose another model. Retry resumes supported downloads.</p>
            {cancelError && <p role="alert" className="mt-2 text-[length:var(--ui-font-body)] text-error">{cancelError}</p>}
          </div>
        )}

        {downloadState.phase === 'error' && (
          <div role="alert" className="dialog-toast mb-4 border-error/30 bg-error/10 px-4 py-3">
            <p className="text-sm text-error">{downloadState.message}</p>
            {selected === 'parakeet-tdt-0.6b-v3-coreml' && (
              <div className="mt-3 flex flex-wrap gap-2">
                <button
                  type="button"
                  onClick={() => chooseFallback('base.en')}
                  className="dialog-pill-btn px-2.5 py-1.5 text-xs text-on-surface hover:bg-surface-container"
                >
                  Use Whisper base
                </button>
                <button
                  type="button"
                  onClick={() => chooseFallback('parakeet-tdt-0.6b-v2-fp16')}
                  className="dialog-pill-btn px-2.5 py-1.5 text-xs text-on-surface hover:bg-surface-container"
                >
                  Use CPU Parakeet
                </button>
              </div>
            )}
          </div>
        )}

        {/* An installed selection completes immediately — same path a finished
            download takes — so nothing on disk is ever re-downloaded. */}
        {renderPrimaryAction ? renderPrimaryAction({
          label: primaryLabel,
          disabled: isDownloading,
          onActivate: activatePrimary,
        }) : (
          <button
            type="button"
            onClick={activatePrimary}
            disabled={isDownloading}
            className="w-full rounded-[var(--ui-radius-pill)] bg-[linear-gradient(140deg,var(--murmur-primary),var(--murmur-primary-dim))] px-4 py-2.5 text-sm font-semibold text-on-primary shadow-[var(--ui-shadow-accent)] transition-colors hover:brightness-105 disabled:cursor-not-allowed disabled:opacity-50"
          >
            {primaryLabel}
          </button>
        )}
    </div>
  );
}

/**
 * Full-screen first-launch gate shown when the selected model is missing but
 * onboarding has already completed (e.g. the model file was deleted).
 */
export function ModelDownloader({ initialModel, onComplete }: Props) {
  const { installedModels, error, retry } = useInstalledModels();
  return (
    <div className="h-full min-h-0 bg-background flex flex-col items-center justify-center p-8">
      <div className="w-full max-w-md">
        <h1 className="text-xl font-semibold tracking-[var(--ui-track-title,-0.01em)] text-on-surface mb-1">
          Download a Model
        </h1>
        <p className="text-sm text-on-surface-variant mb-6">
          A model file is required for transcription. Choose an installed model or download one.
        </p>
        {error ? (
          <div role="alert">
            <p>Could not check installed models.</p>
            <button type="button" onClick={retry} className="dialog-pill-btn mt-3 px-4 py-2">Retry</button>
          </div>
        ) : installedModels === null ? (
          <p role="status">Checking installed models…</p>
        ) : (
          <ModelDownloadPanel initialModel={preferredInstalledModel(initialModel, installedModels)} installedModels={installedModels} onComplete={onComplete} />
        )}
      </div>
    </div>
  );
}
