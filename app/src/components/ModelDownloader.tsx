import { useState, useEffect, useRef, type ReactNode } from 'react';
import { Check, CircleAlert, Cpu } from 'lucide-react';
import { WindowHeader } from './ui/WindowHeader';
import { StepHeading } from './onboarding/StepHeading';
import { createPortal } from 'react-dom';
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
  modelMemoryWarning,
  useModelHardwareGuidance,
} from '../lib/modelHardwareGuidance';

const MODEL_DESCRIPTIONS: Record<string, string> = {
  'parakeet-tdt-0.6b-v3-coreml': 'Fastest transcription on Apple Silicon · Multilingual',
  'parakeet-tdt-0.6b-v2-fp16': 'Fast transcription · English only',
  'large-v3-turbo': 'Highest accuracy · Multilingual',
  'base.en': 'Balanced speed and accuracy · English only',
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
    isNavigation: boolean;
    isCancellation: boolean;
  }) => ReactNode;
  /**
   * Fixed host for progress, cancel, error, and memory notices. Embedders whose
   * model list scrolls (the setup wizard's fixed-height frame) pass a footer
   * element so download status stays visible instead of resting below the fold.
   */
  statusPortalTarget?: HTMLElement | null;
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
  statusPortalTarget,
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
    ? 'Try again'
    : `Download · ${MODELS.find((model) => model.name === selected)!.size}`;
  const activatePrimary = () => {
    if (selectedInstalled) {
      onComplete(selected);
      return;
    }
    void handleDownload();
  };

  const action = {
    label: isDownloading ? cancelling ? 'Cancelling…' : 'Cancel' : primaryLabel,
    disabled: isDownloading && cancelling,
    onActivate: isDownloading ? () => void handleCancel() : activatePrimary,
    isNavigation: selectedInstalled,
    isCancellation: isDownloading,
  };
  const statusContent = <>
    {isDownloading && <div className="model-download-progress" aria-live="polite" aria-busy="true">
      <p className="model-progress-label">{cancelling ? 'Cancelling…' : progressPercent === null ? modelDownloadLabel(progress!) : `Downloading · ${progressPercent}%`}</p>
      <div className="model-progress-track">
        <div role="progressbar" aria-label="Speech model download" aria-valuenow={progressPercent ?? undefined} aria-valuemin={0} aria-valuemax={100} aria-valuetext={progressPercent === null ? modelDownloadLabel(progress!) : undefined}
          className={cn('model-progress-fill', progressPercent === null && 'model-download-indeterminate')}
          style={progressPercent === null ? undefined : { width: `${progressPercent}%` }} />
      </div>
      {progress && progress.total > 0 && progressPercent !== null && <p className="model-progress-size">{(progress.received / 1024 / 1024).toFixed(1)} / {(progress.total / 1024 / 1024).toFixed(1)} MB</p>}
    </div>}
    {downloadState.phase === 'error' && <div role="alert" className="model-download-error"><CircleAlert size={18} aria-hidden="true" /><span>{downloadState.message}</span></div>}
    {!renderPrimaryAction && <button type="button" onClick={action.onActivate} disabled={action.disabled} className={cn('model-download-action', isDownloading ? 'dialog-pill-btn' : 'first-run-primary')}>{action.label}</button>}
    {isDownloading && <p className="model-download-help">You can cancel and choose another model.</p>}
    {downloadState.phase === 'error' && selected === 'parakeet-tdt-0.6b-v3-coreml' && <div className="model-fallback-actions">
      <button type="button" onClick={() => chooseFallback('base.en')} className="dialog-pill-btn">Use Whisper base</button>
      <button type="button" onClick={() => chooseFallback('parakeet-tdt-0.6b-v2-fp16')} className="dialog-pill-btn">Use CPU Parakeet</button>
    </div>}
    {cancelError && <p role="alert" className="model-cancel-error">{cancelError}</p>}
  </>;

  return <div className="model-download-panel" data-downloading={isDownloading || undefined} data-embedded={!!renderPrimaryAction || undefined}>
    <div className="model-options">
      {offeredModels.map((model) => <button key={model.name} id={`download-model-${model.name}`} type="button" aria-pressed={selected === model.name}
        aria-disabled={isDownloading} disabled={isDownloading} onClick={() => selectModel(model.name)} className="dialog-card model-option">
        <div className="model-option-heading">
          <span className="model-option-name">{model.label}
            {hardwareGuidance?.recommendedModel === model.name && <span className="model-recommendation">Recommended for this Mac</span>}
            {hardwareGuidance?.warnedModels.includes(model.name) && <span className="model-memory-tag">Higher memory</span>}
          </span>
          <span className="model-option-size">{installedModels?.[model.name] ? 'Installed' : model.size}</span>
        </div>
        <p>{model.name === 'parakeet-tdt-0.6b-v3-coreml' && hardwareGuidance?.chipTier !== 'unknown' && hardwareGuidance ? 'Fastest transcription on this Mac · Multilingual' : model.description}</p>
        {selected === model.name && <Check className="model-selection" size={14} aria-hidden="true" />}
      </button>)}
    </div>
    {statusPortalTarget ? createPortal(statusContent, statusPortalTarget) : <div className="model-download-status">{statusContent}</div>}
    {selectedMemoryWarning && (statusPortalTarget
      ? createPortal(<p role="status" className="model-memory-warning">{selectedMemoryWarning}</p>, statusPortalTarget)
      : <p role="status" className="model-memory-warning">{selectedMemoryWarning}</p>)}
    {renderPrimaryAction?.(action)}
  </div>;
}

/**
 * Full-screen first-launch gate shown when the selected model is missing but
 * onboarding has already completed (e.g. the model file was deleted).
 */
export function ModelDownloader({ initialModel, onComplete }: Props) {
  const { installedModels, error, retry } = useInstalledModels();
  return (
    <div className="model-downloader h-full min-h-0 bg-background flex flex-col items-center justify-center">
      <WindowHeader showWordmark={false} />
      <div className="model-downloader-column">
        <StepHeading icon={Cpu} title="Speech model" subtitle="Download a local model once, then transcribe offline." />
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
