import { useState, useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { AVAILABLE_MODEL_OPTIONS, type ModelOption } from '../lib/settings';
import {
  modelDownloadLabel,
  modelDownloadPercent,
  type ModelDownloadProgress,
} from '../lib/modelDownload';

const MODEL_DESCRIPTIONS: Record<string, string> = {
  'parakeet-tdt-0.6b-v3-coreml': 'Fastest on Apple Silicon — multilingual, Apple Neural Engine (recommended)',
  'parakeet-tdt-0.6b-v2-fp16': 'Fast CPU fallback — English only',
  'large-v3-turbo': 'Highest accuracy, slower (1-2 seconds)',
  'base.en': 'Good balance of speed and accuracy',
};

/** Subset of models shown on the initial download screen (first = default). */
const DOWNLOAD_MODEL_KEYS: ModelOption[] = [
  'parakeet-tdt-0.6b-v3-coreml',
  'parakeet-tdt-0.6b-v2-fp16',
  'large-v3-turbo',
  'base.en',
];
const MODELS = DOWNLOAD_MODEL_KEYS.map((key) => {
  const opt = AVAILABLE_MODEL_OPTIONS.find((m) => m.value === key);
  if (!opt) return null;
  return { name: opt.value, label: opt.label, size: opt.size, description: MODEL_DESCRIPTIONS[key] ?? '' };
}).filter((model): model is NonNullable<typeof model> => model !== null);

interface Props {
  initialModel: ModelOption;
  onComplete: (model: ModelOption) => void;
  /** Notifies embedders when a download starts/stops (e.g. to lock navigation). */
  onDownloadingChange?: (downloading: boolean) => void;
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
export function ModelDownloadPanel({ initialModel, onComplete, onDownloadingChange }: Props) {
  const [selected, setSelected] = useState<ModelOption>(
    MODELS.some((model) => model.name === initialModel) ? initialModel : MODELS[0].name
  );
  const [downloadState, setDownloadState] = useState<DownloadState>({ phase: 'idle' });
  const downloadUnlistenRef = useRef<(() => void) | null>(null);

  useEffect(() => {
    return () => {
      downloadUnlistenRef.current?.();
      downloadUnlistenRef.current = null;
    };
  }, []);

  const handleDownload = async () => {
    onDownloadingChange?.(true);
    setDownloadState({
      phase: 'downloading',
      progress: { received: 0, total: 0, phase: 'downloading' },
    });

    // Single try/catch covering listen() as well as the download itself, so a
    // listen failure can't strand the downloading state (or an embedder's
    // navigation lock) forever.
    try {
      const unlisten = await listen<ModelDownloadProgress>(
        'download-progress',
        (event) => {
          setDownloadState({
            phase: 'downloading',
            progress: event.payload,
          });
        }
      );
      downloadUnlistenRef.current = unlisten;

      await invoke('download_model', { modelName: selected });
      unlisten();
      downloadUnlistenRef.current = null;
      onDownloadingChange?.(false);
      onComplete(selected);
    } catch (err) {
      downloadUnlistenRef.current?.();
      downloadUnlistenRef.current = null;
      onDownloadingChange?.(false);
      setDownloadState({
        phase: 'error',
        message: String(err),
      });
    }
  };

  const progress = downloadState.phase === 'downloading' ? downloadState.progress : null;
  const progressPercent = progress ? modelDownloadPercent(progress) : null;

  const isDownloading = downloadState.phase === 'downloading';

  return (
    <div>
      <div className="space-y-2 mb-6">
          {MODELS.map((model) => (
            <button
              key={model.name}
              onClick={() => !isDownloading && setSelected(model.name)}
              disabled={isDownloading}
              className={`w-full text-left px-4 py-3 rounded-lg border transition-colors ${
                selected === model.name
                  ? 'border-primary bg-surface-container-low'
                  : 'border-outline-variant/20 bg-surface-container-lowest hover:border-primary/50'
              } ${isDownloading ? 'opacity-50 cursor-not-allowed' : 'cursor-pointer'}`}
            >
              <div className="flex items-center justify-between">
                <span className="text-sm font-medium text-on-surface">
                  {model.label}
                </span>
                <span className="text-xs text-on-surface-variant font-mono">
                  {model.size}
                </span>
              </div>
              <p className="text-xs text-on-surface-variant mt-0.5">
                {model.description}
              </p>
            </button>
          ))}
        </div>

        {isDownloading && (
          <div className="mb-4">
            <div className="flex justify-between text-xs text-on-surface-variant mb-1">
              <span>{progress ? modelDownloadLabel(progress) : 'Starting...'}</span>
              {progressPercent !== null ? (
                <span>{progressPercent}%</span>
              ) : (
                <span>Working...</span>
              )}
            </div>
            <div className="w-full h-2 bg-surface-container-highest rounded-full overflow-hidden">
              <div
                role="progressbar"
                aria-valuenow={progressPercent ?? undefined}
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuetext={progressPercent === null ? 'Model installation in progress' : undefined}
                className={`h-full bg-primary rounded-full ${
                  progressPercent === null
                    ? 'model-download-indeterminate'
                    : 'transition-all duration-200'
                }`}
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

        {downloadState.phase === 'error' && (
          <div className="mb-4 rounded-lg border border-error/30 bg-error/10 px-4 py-3">
            <p className="text-sm text-error">{downloadState.message}</p>
          </div>
        )}

        <button
          onClick={handleDownload}
          disabled={isDownloading}
          className="w-full rounded-lg bg-primary px-4 py-2.5 text-sm font-medium text-on-primary transition-colors hover:bg-primary-dim disabled:cursor-not-allowed disabled:opacity-50"
        >
          {isDownloading
            ? progress ? modelDownloadLabel(progress) : 'Starting...'
            : downloadState.phase === 'error'
            ? 'Retry Download'
            : 'Download'}
        </button>
    </div>
  );
}

/**
 * Full-screen first-launch gate shown when the selected model is missing but
 * onboarding has already completed (e.g. the model file was deleted).
 */
export function ModelDownloader({ initialModel, onComplete }: Props) {
  return (
    <div className="h-screen bg-background flex flex-col items-center justify-center p-8">
      <div className="w-full max-w-md">
        <h1 className="text-xl font-semibold text-on-surface mb-1">
          Download a Model
        </h1>
        <p className="text-sm text-on-surface-variant mb-6">
          A model file is required for transcription. Choose one to download.
        </p>
        <ModelDownloadPanel initialModel={initialModel} onComplete={onComplete} />
      </div>
    </div>
  );
}
