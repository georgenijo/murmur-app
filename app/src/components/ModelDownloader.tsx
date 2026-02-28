import { useState, useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { MODEL_OPTIONS, type ModelOption } from '../lib/settings';

const MODEL_DESCRIPTIONS: Record<string, string> = {
  'moonshine-tiny': 'Fastest — sub-20ms for typical dictation',
  'moonshine-base': 'Better accuracy, still very fast',
  'large-v3-turbo': 'Highest accuracy, slower (1-2 seconds)',
  'base.en': 'Good balance of speed and accuracy',
};

/** Subset of models shown on the initial download screen. */
const DOWNLOAD_MODEL_KEYS: ModelOption[] = ['moonshine-tiny', 'moonshine-base', 'large-v3-turbo', 'base.en'];
const MODELS = DOWNLOAD_MODEL_KEYS.map((key) => {
  const opt = MODEL_OPTIONS.find((m) => m.value === key)!;
  return { name: opt.value, label: opt.label, size: opt.size, description: MODEL_DESCRIPTIONS[key] ?? '' };
});

interface Props {
  onComplete: () => void;
}

type DownloadState =
  | { phase: 'idle' }
  | { phase: 'downloading'; received: number; total: number }
  | { phase: 'error'; message: string };

export function ModelDownloader({ onComplete }: Props) {
  const [selected, setSelected] = useState<string>(MODELS[0].name);
  const [downloadState, setDownloadState] = useState<DownloadState>({ phase: 'idle' });
  const downloadUnlistenRef = useRef<(() => void) | null>(null);

  useEffect(() => {
    return () => {
      downloadUnlistenRef.current?.();
      downloadUnlistenRef.current = null;
    };
  }, []);

  const handleDownload = async () => {
    setDownloadState({ phase: 'downloading', received: 0, total: 0 });

    const unlisten = await listen<{ received: number; total: number }>(
      'download-progress',
      (event) => {
        setDownloadState({
          phase: 'downloading',
          received: event.payload.received,
          total: event.payload.total,
        });
      }
    );
    downloadUnlistenRef.current = unlisten;

    try {
      await invoke('download_model', { modelName: selected });
      unlisten();
      downloadUnlistenRef.current = null;
      onComplete();
    } catch (err) {
      unlisten();
      downloadUnlistenRef.current = null;
      setDownloadState({
        phase: 'error',
        message: String(err),
      });
    }
  };

  const progressPercent =
    downloadState.phase === 'downloading' && downloadState.total > 0
      ? Math.round((downloadState.received / downloadState.total) * 100)
      : null;

  const isDownloading = downloadState.phase === 'downloading';

  return (
    <div className="h-screen bg-stone-50 dark:bg-stone-900 flex flex-col items-center justify-center p-8">
      <div className="w-full max-w-md">
        <h1 className="text-xl font-semibold text-stone-800 dark:text-stone-100 mb-1">
          Download a Model
        </h1>
        <p className="text-sm text-stone-500 dark:text-stone-400 mb-6">
          A model file is required for transcription. Choose one to download.
        </p>

        <div className="space-y-2 mb-6">
          {MODELS.map((model) => (
            <button
              key={model.name}
              onClick={() => !isDownloading && setSelected(model.name)}
              disabled={isDownloading}
              className={`w-full text-left px-4 py-3 rounded-lg border transition-colors ${
                selected === model.name
                  ? 'border-blue-500 bg-blue-50 dark:bg-blue-900/20 dark:border-blue-400'
                  : 'border-stone-200 dark:border-stone-700 bg-white dark:bg-stone-800 hover:border-stone-300 dark:hover:border-stone-600'
              } ${isDownloading ? 'opacity-50 cursor-not-allowed' : 'cursor-pointer'}`}
            >
              <div className="flex items-center justify-between">
                <span className="text-sm font-medium text-stone-800 dark:text-stone-100">
                  {model.label}
                </span>
                <span className="text-xs text-stone-400 dark:text-stone-500 font-mono">
                  {model.size}
                </span>
              </div>
              <p className="text-xs text-stone-500 dark:text-stone-400 mt-0.5">
                {model.description}
              </p>
            </button>
          ))}
        </div>

        {isDownloading && (
          <div className="mb-4">
            <div className="flex justify-between text-xs text-stone-500 dark:text-stone-400 mb-1">
              <span>Downloading…</span>
              {progressPercent !== null ? (
                <span>{progressPercent}%</span>
              ) : (
                <span>Starting…</span>
              )}
            </div>
            <div className="w-full h-2 bg-stone-200 dark:bg-stone-700 rounded-full overflow-hidden">
              <div
                className="h-full bg-blue-500 rounded-full transition-all duration-200"
                style={{ width: `${progressPercent ?? 0}%` }}
              />
            </div>
            {downloadState.total > 0 && (
              <p className="text-xs text-stone-400 dark:text-stone-500 mt-1 text-right">
                {(downloadState.received / 1024 / 1024).toFixed(1)} /{' '}
                {(downloadState.total / 1024 / 1024).toFixed(1)} MB
              </p>
            )}
          </div>
        )}

        {downloadState.phase === 'error' && (
          <div className="mb-4 px-4 py-3 bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 rounded-lg">
            <p className="text-sm text-red-600 dark:text-red-400">{downloadState.message}</p>
          </div>
        )}

        <button
          onClick={handleDownload}
          disabled={isDownloading}
          className="w-full py-2.5 px-4 bg-blue-600 hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed text-white text-sm font-medium rounded-lg transition-colors"
        >
          {isDownloading
            ? 'Downloading…'
            : downloadState.phase === 'error'
            ? 'Retry Download'
            : 'Download'}
        </button>
      </div>
    </div>
  );
}
