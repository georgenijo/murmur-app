import { useState, useEffect, useRef, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getVersion } from '@tauri-apps/api/app';
import {
  Settings, RecordingMode, DEFAULT_SETTINGS,
  MODEL_OPTIONS, MOONSHINE_MODELS, WHISPER_MODELS, DOUBLE_TAP_KEY_OPTIONS, RECORDING_MODE_OPTIONS,
} from '../../lib/settings';
import { Select } from '../ui/Select';
import type { DictationStatus } from '../../lib/types';
import type { UpdateStatus } from '../../lib/updater';

function PasteDelaySlider({ value, onCommit }: { value: number; onCommit: (v: number) => void }) {
  const [draft, setDraft] = useState(value);
  useEffect(() => { setDraft(value); }, [value]);

  return (
    <div className="mt-3">
      <div className="flex items-center justify-between mb-1">
        <label className="text-xs text-stone-600 dark:text-stone-400">
          Paste Delay
        </label>
        <span className="text-xs font-medium text-stone-700 dark:text-stone-300">
          {draft}ms
        </span>
      </div>
      <input
        type="range"
        min={10}
        max={500}
        step={10}
        value={draft}
        onChange={(e) => setDraft(Number(e.target.value))}
        onPointerUp={() => onCommit(draft)}
        className="w-full h-1.5 bg-stone-200 dark:bg-stone-600 rounded-full appearance-none cursor-pointer accent-stone-800 dark:accent-stone-300"
      />
      <p className="mt-1 text-xs text-stone-500 dark:text-stone-400">
        Delay before paste. Increase if paste lands in the wrong window.
      </p>
    </div>
  );
}

interface SettingsPanelProps {
  isOpen: boolean;
  onClose: () => void;
  settings: Settings;
  onUpdateSettings: (updates: Partial<Settings>) => void;
  status: DictationStatus;
  onResetStats: () => void;
  onViewLogs: () => void;
  accessibilityGranted: boolean | null;
  onCheckForUpdate: () => Promise<void>;
  updateStatus: UpdateStatus;
}

export function SettingsPanel({ isOpen, onClose, settings, onUpdateSettings, status, onResetStats, onViewLogs, accessibilityGranted, onCheckForUpdate, updateStatus }: SettingsPanelProps) {
  const [confirmReset, setConfirmReset] = useState(false);
  const [confirmResetApp, setConfirmResetApp] = useState(false);
  const [deleteLogs, setDeleteLogs] = useState(true);
  const [version, setVersion] = useState('');

  useEffect(() => { getVersion().then(setVersion); }, []);
  const confirmResetTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const confirmResetAppTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const handleResetClick = () => {
    if (confirmReset) {
      if (confirmResetTimeoutRef.current) clearTimeout(confirmResetTimeoutRef.current);
      confirmResetTimeoutRef.current = null;
      setConfirmReset(false);
      onResetStats();
    } else {
      setConfirmReset(true);
      confirmResetTimeoutRef.current = setTimeout(() => {
        setConfirmReset(false);
        confirmResetTimeoutRef.current = null;
      }, 3000);
    }
  };

  const handleResetAppDataClick = async () => {
    if (confirmResetApp) {
      if (confirmResetAppTimeoutRef.current) clearTimeout(confirmResetAppTimeoutRef.current);
      confirmResetAppTimeoutRef.current = null;
      setConfirmResetApp(false);
      localStorage.clear();
      if (deleteLogs) {
        try { await invoke('clear_logs'); } catch (_) { /* best effort */ }
      }
      window.location.reload();
    } else {
      setConfirmResetApp(true);
      confirmResetAppTimeoutRef.current = setTimeout(() => {
        setConfirmResetApp(false);
        confirmResetAppTimeoutRef.current = null;
      }, 3000);
    }
  };

  const handleRequestPermission = () => invoke('request_accessibility_permission');

  // Model availability check and inline download
  const [modelAvailable, setModelAvailable] = useState<boolean | null>(null);
  const [modelDownload, setModelDownload] = useState<
    | { phase: 'idle' }
    | { phase: 'downloading'; received: number; total: number }
    | { phase: 'error'; message: string }
  >({ phase: 'idle' });
  const downloadUnlistenRef = useRef<(() => void) | null>(null);
  const downloadModelRef = useRef<string | null>(null);

  useEffect(() => {
    let stale = false;
    setModelAvailable(null);
    setModelDownload({ phase: 'idle' });
    downloadModelRef.current = null;
    invoke<boolean>('check_specific_model_exists', { modelName: settings.model })
      .then((v) => { if (!stale) setModelAvailable(v); })
      .catch(() => { if (!stale) setModelAvailable(null); });
    return () => { stale = true; };
  }, [settings.model]);

  useEffect(() => {
    return () => {
      downloadUnlistenRef.current?.();
      downloadUnlistenRef.current = null;
    };
  }, []);

  const handleModelDownload = useCallback(async () => {
    const modelName = settings.model;
    downloadModelRef.current = modelName;
    setModelDownload({ phase: 'downloading', received: 0, total: 0 });
    let unlisten: (() => void) | null = null;
    try {
      unlisten = await listen<{ received: number; total: number }>(
        'download-progress',
        (event) => {
          if (downloadModelRef.current !== modelName) return;
          setModelDownload({
            phase: 'downloading',
            received: event.payload.received,
            total: event.payload.total,
          });
        }
      );
      downloadUnlistenRef.current = unlisten;
      await invoke('download_model', { modelName });
      unlisten();
      downloadUnlistenRef.current = null;
      if (downloadModelRef.current === modelName) {
        downloadModelRef.current = null;
        setModelDownload({ phase: 'idle' });
        setModelAvailable(true);
      }
    } catch (err) {
      unlisten?.();
      downloadUnlistenRef.current = null;
      if (downloadModelRef.current === modelName) {
        downloadModelRef.current = null;
        setModelDownload({ phase: 'error', message: String(err) });
      }
    }
  }, [settings.model]);

  // Audio device enumeration
  const [audioDevices, setAudioDevices] = useState<string[]>([]);
  useEffect(() => {
    if (!isOpen) return;
    invoke<string[]>('list_audio_devices')
      .then(setAudioDevices)
      .catch(() => setAudioDevices([]));
  }, [isOpen]);

  const savedDeviceMissing =
    settings.microphone !== DEFAULT_SETTINGS.microphone &&
    audioDevices.length > 0 &&
    !audioDevices.includes(settings.microphone);

  const isDoubleTap = settings.recordingMode === 'double_tap';
  const isBoth = settings.recordingMode === 'both';
  const keyLabel = isBoth ? 'Trigger Key' : isDoubleTap ? 'Double-Tap Key' : 'Hold Key';
  const keyHelpText = isBoth
    ? 'Hold to record, or double-tap to start and single tap to stop'
    : isDoubleTap
      ? 'Double-tap to start recording, single tap to stop'
      : 'Hold to start recording, release to stop';
  const isRecording = status !== 'idle';
  const selectedModel = MODEL_OPTIONS.find(m => m.value === settings.model);
  const downloadProgressPercent =
    modelDownload.phase === 'downloading' && modelDownload.total > 0
      ? Math.round((modelDownload.received / modelDownload.total) * 100)
      : null;

  return (
    <aside
      className={`shrink-0 border-l border-stone-200 dark:border-stone-700 bg-white dark:bg-stone-800 transition-all duration-200 ${
        isOpen ? 'w-[280px] overflow-y-auto' : 'w-0 overflow-hidden'
      }`}
    >
      {/* Header */}
      <div className="flex items-center justify-between p-4 border-b border-stone-200 dark:border-stone-700">
        <h2 className="text-sm font-semibold text-stone-900 dark:text-stone-100">Settings</h2>
        <button
          onClick={onClose}
          className="p-1 rounded-md hover:bg-stone-100 dark:hover:bg-stone-700 transition-colors"
        >
          <svg className="w-4 h-4 text-stone-500" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>

      {/* Content */}
      <div className="p-4 space-y-6">
        {/* Model Selector */}
        <div>
          <label className="block text-sm font-medium text-stone-700 dark:text-stone-300 mb-2">
            Transcription Model
          </label>
          <Select
            value={settings.model}
            onChange={(value) => onUpdateSettings({ model: value })}
            disabled={isRecording}
            items={[
              {
                label: 'Moonshine (Fast, CPU)',
                options: MOONSHINE_MODELS.map((m) => ({ value: m.value, label: `${m.label} (${m.size})` })),
              },
              {
                label: 'Whisper (Metal GPU)',
                options: WHISPER_MODELS.map((m) => ({ value: m.value, label: `${m.label} (${m.size})` })),
              },
            ]}
          />
          <p className="mt-1 text-xs text-stone-500 dark:text-stone-400">
            Moonshine runs on CPU; Whisper uses Metal GPU. Larger models are more accurate but slower.
          </p>
          {isRecording && (
            <p className="mt-1 text-xs text-amber-600 dark:text-amber-400">
              Stop recording before changing model
            </p>
          )}

          {/* Model not downloaded — inline download prompt */}
          {modelAvailable === false && modelDownload.phase === 'idle' && (
            <div className="mt-2 flex items-center gap-2 px-3 py-2 text-xs bg-amber-50 dark:bg-amber-900/20 border border-amber-200 dark:border-amber-800 rounded-lg text-amber-700 dark:text-amber-400">
              <span className="w-2 h-2 rounded-full bg-amber-500 flex-shrink-0" />
              <span>Model not downloaded</span>
              <button
                onClick={handleModelDownload}
                className="underline hover:no-underline ml-auto flex-shrink-0"
              >
                Download
              </button>
            </div>
          )}

          {/* Download in progress */}
          {modelDownload.phase === 'downloading' && (
            <div className="mt-2">
              <div className="flex justify-between text-xs text-stone-500 dark:text-stone-400 mb-1">
                <span>Downloading…</span>
                {downloadProgressPercent !== null ? (
                  <span>{downloadProgressPercent}%</span>
                ) : (
                  <span>Starting…</span>
                )}
              </div>
              <div className="w-full h-1.5 bg-stone-200 dark:bg-stone-700 rounded-full overflow-hidden">
                <div
                  role="progressbar"
                  aria-valuenow={downloadProgressPercent ?? 0}
                  aria-valuemin={0}
                  aria-valuemax={100}
                  aria-valuetext={`Download progress: ${downloadProgressPercent ?? 0} percent`}
                  className="h-full bg-blue-500 rounded-full transition-all duration-200"
                  style={{ width: `${downloadProgressPercent ?? 0}%` }}
                />
              </div>
            </div>
          )}

          {/* Download error */}
          {modelDownload.phase === 'error' && (
            <div className="mt-2 flex items-center gap-2 px-3 py-2 text-xs bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 rounded-lg text-red-600 dark:text-red-400">
              <span>{modelDownload.message}</span>
              <button
                onClick={handleModelDownload}
                className="underline hover:no-underline ml-auto flex-shrink-0"
              >
                Retry
              </button>
            </div>
          )}
        </div>

        {/* Microphone Selector */}
        <div>
          <label className="block text-sm font-medium text-stone-700 dark:text-stone-300 mb-2">
            Microphone
          </label>
          <Select
            value={settings.microphone}
            onChange={(value) => onUpdateSettings({ microphone: value })}
            disabled={isRecording}
            items={[
              { value: 'system_default', label: 'System Default' },
              ...audioDevices.map((name) => ({ value: name, label: name })),
            ]}
          />
          {savedDeviceMissing && (
            <div className="mt-2 flex items-center gap-2 px-3 py-2 text-xs bg-amber-50 dark:bg-amber-900/20 border border-amber-200 dark:border-amber-800 rounded-lg text-amber-700 dark:text-amber-400">
              <span className="w-2 h-2 rounded-full bg-amber-500 flex-shrink-0" />
              <span>Selected device not found — will use System Default</span>
            </div>
          )}
        </div>

        {/* Recording Trigger */}
        <div>
          <label className="block text-sm font-medium text-stone-700 dark:text-stone-300 mb-2">
            Recording Trigger
          </label>
          <div className="flex gap-2">
            {RECORDING_MODE_OPTIONS.map((option) => (
              <button
                key={option.value}
                disabled={isRecording}
                onClick={() => onUpdateSettings({ recordingMode: option.value as RecordingMode })}
                className={`flex-1 px-3 py-2 rounded-lg text-xs font-medium border transition-colors ${
                  settings.recordingMode === option.value
                    ? 'bg-stone-800 dark:bg-stone-200 text-white dark:text-stone-900 border-stone-800 dark:border-stone-200'
                    : 'bg-white dark:bg-stone-700 text-stone-700 dark:text-stone-300 border-stone-300 dark:border-stone-600 hover:bg-stone-50 dark:hover:bg-stone-600'
                } ${isRecording ? 'opacity-50 cursor-not-allowed' : ''}`}
              >
                {option.label}
              </button>
            ))}
          </div>
          {isRecording && (
            <p className="mt-1 text-xs text-amber-600 dark:text-amber-400">
              Stop recording before changing mode
            </p>
          )}
        </div>

        {/* Accessibility notice — both modes use rdev which requires it */}
        {accessibilityGranted === false && (
          <div className="flex items-center gap-2 px-3 py-2 text-xs bg-amber-50 dark:bg-amber-900/20 border border-amber-200 dark:border-amber-800 rounded-lg text-amber-700 dark:text-amber-400">
            <span className="w-2 h-2 rounded-full bg-amber-500 flex-shrink-0" />
            <span>Accessibility permission required for keyboard detection</span>
            <button
              onClick={handleRequestPermission}
              className="underline hover:no-underline ml-auto flex-shrink-0"
            >
              Grant
            </button>
          </div>
        )}

        {/* Trigger Key Selector */}
        <div>
          <label className="block text-sm font-medium text-stone-700 dark:text-stone-300 mb-2">
            {keyLabel}
          </label>
          <Select
            value={settings.doubleTapKey}
            onChange={(value) => onUpdateSettings({ doubleTapKey: value })}
            disabled={isRecording}
            items={DOUBLE_TAP_KEY_OPTIONS}
          />
          <p className="mt-1 text-xs text-stone-500 dark:text-stone-400">
            {keyHelpText}
          </p>
        </div>

        {/* Auto-Paste Toggle */}
        <div>
          <div className="flex items-center justify-between">
            <div>
              <label className="block text-sm font-medium text-stone-700 dark:text-stone-300">
                Auto-Paste
              </label>
              <p className="mt-1 text-xs text-stone-500 dark:text-stone-400">
                Automatically paste transcription (requires Accessibility permission)
              </p>
            </div>
            <button
              type="button"
              role="switch"
              aria-checked={settings.autoPaste}
              aria-label="Auto paste"
              onClick={() => onUpdateSettings({ autoPaste: !settings.autoPaste })}
              className={`relative inline-flex shrink-0 h-6 w-11 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-stone-500 focus:ring-offset-2 ${
                settings.autoPaste ? 'bg-stone-800 dark:bg-stone-300' : 'bg-stone-300 dark:bg-stone-500'
              }`}
            >
              <span
                className={`inline-block h-4 w-4 transform rounded-full bg-white shadow transition-transform ${
                  settings.autoPaste ? 'translate-x-6' : 'translate-x-1'
                }`}
              />
            </button>
          </div>
          {settings.autoPaste && accessibilityGranted !== null && (
            <div className={`mt-2 flex items-center gap-2 text-xs ${
              accessibilityGranted
                ? 'text-emerald-600 dark:text-emerald-400'
                : 'text-amber-600 dark:text-amber-400'
            }`}>
              <span className={`w-2 h-2 rounded-full ${
                accessibilityGranted ? 'bg-emerald-500' : 'bg-amber-500'
              }`} />
              <span>
                {accessibilityGranted
                  ? 'Accessibility permission granted'
                  : 'Accessibility permission required'}
              </span>
              {accessibilityGranted === false && (
                <button
                  onClick={handleRequestPermission}
                  className="underline hover:no-underline"
                >
                  Grant
                </button>
              )}
            </div>
          )}
          {settings.autoPaste && (
            <PasteDelaySlider
              value={settings.autoPasteDelayMs}
              onCommit={(value) => onUpdateSettings({ autoPasteDelayMs: value })}
            />
          )}
        </div>

        {/* Launch at Login Toggle */}
        <div>
          <div className="flex items-center justify-between">
            <div>
              <label className="block text-sm font-medium text-stone-700 dark:text-stone-300">
                Launch at Login
              </label>
              <p className="mt-1 text-xs text-stone-500 dark:text-stone-400">
                Automatically start when you log in
              </p>
            </div>
            <button
              type="button"
              role="switch"
              aria-checked={settings.launchAtLogin}
              aria-label="Launch at login"
              onClick={() => onUpdateSettings({ launchAtLogin: !settings.launchAtLogin })}
              className={`relative inline-flex shrink-0 h-6 w-11 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-stone-500 focus:ring-offset-2 ${
                settings.launchAtLogin ? 'bg-stone-800 dark:bg-stone-300' : 'bg-stone-300 dark:bg-stone-500'
              }`}
            >
              <span
                className={`inline-block h-4 w-4 transform rounded-full bg-white shadow transition-transform ${
                  settings.launchAtLogin ? 'translate-x-6' : 'translate-x-1'
                }`}
              />
            </button>
          </div>
        </div>

        {/* Model Info */}
        <div className="pt-4 border-t border-stone-200 dark:border-stone-700">
          <h3 className="text-sm font-medium text-stone-700 dark:text-stone-300 mb-2">
            Current Model
          </h3>
          <div className="text-sm text-stone-600 dark:text-stone-400">
            <p><strong>Model:</strong> {selectedModel?.label}</p>
            <p><strong>Backend:</strong> {selectedModel
              ? (selectedModel.backend === 'moonshine' ? 'Moonshine (CPU)' : 'Whisper (Metal GPU)')
              : 'Unknown'}
            </p>
            <p><strong>Size:</strong> {selectedModel?.size}</p>
          </div>
        </div>

        {/* Reset Stats */}
        <div className="pt-4 border-t border-stone-200 dark:border-stone-700">
          <h3 className="text-sm font-medium text-stone-700 dark:text-stone-300 mb-2">
            Statistics
          </h3>
          <button
            onClick={handleResetClick}
            aria-label={confirmReset ? 'Confirm reset statistics' : 'Reset statistics'}
            className={`w-full px-3 py-2 rounded-lg text-xs font-medium border transition-colors ${
              confirmReset
                ? 'border-red-400 dark:border-red-600 bg-red-50 dark:bg-red-900/20 text-red-700 dark:text-red-400 hover:bg-red-100 dark:hover:bg-red-900/40'
                : 'border-stone-300 dark:border-stone-600 bg-white dark:bg-stone-700 text-stone-700 dark:text-stone-300 hover:bg-stone-50 dark:hover:bg-stone-600'
            }`}
          >
            {confirmReset ? 'Confirm Reset' : 'Reset Stats'}
          </button>
        </div>

        {/* Logs */}
        <div className="pt-4 border-t border-stone-200 dark:border-stone-700">
          <h3 className="text-sm font-medium text-stone-700 dark:text-stone-300 mb-2">
            Logs
          </h3>
          <button
            onClick={onViewLogs}
            className="w-full px-3 py-2 rounded-lg text-xs font-medium border border-stone-300 dark:border-stone-600 bg-white dark:bg-stone-700 text-stone-700 dark:text-stone-300 hover:bg-stone-50 dark:hover:bg-stone-600 transition-colors"
          >
            View Logs
          </button>
        </div>

        {/* Updates */}
        <div className="pt-4 border-t border-stone-200 dark:border-stone-700">
          <h3 className="text-sm font-medium text-stone-700 dark:text-stone-300 mb-2">
            Updates
          </h3>
          <button
            onClick={onCheckForUpdate}
            disabled={updateStatus.phase === 'checking' || updateStatus.phase === 'downloading'}
            className="w-full px-3 py-2 rounded-lg text-xs font-medium border border-stone-300 dark:border-stone-600 bg-white dark:bg-stone-700 text-stone-700 dark:text-stone-300 hover:bg-stone-50 dark:hover:bg-stone-600 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {updateStatus.phase === 'checking' ? 'Checking...' : 'Check for Updates'}
          </button>
          {updateStatus.phase === 'up-to-date' && (
            <p className="mt-1.5 text-xs text-emerald-600 dark:text-emerald-400">
              You're up to date
            </p>
          )}
          {updateStatus.phase === 'available' && (
            <p className="mt-1.5 text-xs text-blue-600 dark:text-blue-400">
              v{updateStatus.version} available
            </p>
          )}
          {updateStatus.phase === 'error' && (
            <p className="mt-1.5 text-xs text-red-600 dark:text-red-400">
              Update check failed
            </p>
          )}
        </div>

        {/* Reset App Data */}
        <div className="pt-4 border-t border-stone-200 dark:border-stone-700">
          <h3 className="text-sm font-medium text-stone-700 dark:text-stone-300 mb-2">
            Danger Zone
          </h3>
          <label className="flex items-center gap-2 mb-2 text-xs text-stone-600 dark:text-stone-400 cursor-pointer">
            <input
              type="checkbox"
              checked={deleteLogs}
              onChange={(e) => setDeleteLogs(e.target.checked)}
              className="rounded border-stone-300 dark:border-stone-600"
            />
            Also delete log files
          </label>
          <button
            onClick={handleResetAppDataClick}
            aria-label={confirmResetApp ? 'Confirm reset app data' : 'Reset app data'}
            className={`w-full px-3 py-2 rounded-lg text-xs font-medium border transition-colors ${
              confirmResetApp
                ? 'border-red-400 dark:border-red-600 bg-red-50 dark:bg-red-900/20 text-red-700 dark:text-red-400 hover:bg-red-100 dark:hover:bg-red-900/40'
                : 'border-stone-300 dark:border-stone-600 bg-white dark:bg-stone-700 text-stone-700 dark:text-stone-300 hover:bg-stone-50 dark:hover:bg-stone-600'
            }`}
          >
            {confirmResetApp ? 'Confirm Reset' : 'Reset App Data'}
          </button>
          <p className="mt-1.5 text-xs text-stone-500 dark:text-stone-400">
            Clears settings, statistics, and transcription history. The app will reload with defaults.
          </p>
        </div>
      </div>

      {/* Footer */}
      {version && (
        <div className="px-4 py-3 border-t border-stone-200 dark:border-stone-700 text-center">
          <span className="text-xs text-stone-400 dark:text-stone-500">v{version}</span>
        </div>
      )}
    </aside>
  );
}
