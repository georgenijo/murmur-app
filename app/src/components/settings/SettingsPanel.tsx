import { useState, useEffect, useRef, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import { getVersion } from '@tauri-apps/api/app';
import {
  Settings, RecordingMode, DEFAULT_SETTINGS,
  AVAILABLE_MODEL_OPTIONS, DOUBLE_TAP_KEY_OPTIONS, RECORDING_MODE_OPTIONS,
  IDLE_TIMEOUT_OPTIONS, LANGUAGE_OPTIONS, AppProfile, VoiceCommand,
} from '../../lib/settings';
import { Select } from '../ui/Select';
import { SettingsSection } from './SettingsSection';
import { VocabScanStrip } from './VocabScanStrip';
import { useVocabScan } from '../../lib/hooks/useVocabScan';
import { countVocabTokens } from '../../lib/dictation';
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

function CustomVocabularyTextarea({ value, onCommit }: { value: string; onCommit: (v: string) => void }) {
  const [draft, setDraft] = useState(value);
  const [tokenCount, setTokenCount] = useState<number | null>(null);
  useEffect(() => { setDraft(value); }, [value]);

  useEffect(() => {
    if (!draft.trim()) { setTokenCount(null); return; }
    let stale = false;
    countVocabTokens(draft)
      .then((count) => { if (!stale) setTokenCount(count); })
      .catch(() => { if (!stale) setTokenCount(null); });
    return () => { stale = true; };
  }, [draft]);

  return (
    <div>
      <label className="block text-sm font-medium text-stone-700 dark:text-stone-300 mb-2">
        Custom Vocabulary
      </label>
      <textarea
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={() => { if (draft !== value) onCommit(draft); }}
        placeholder="e.g. Tauri, Claude, whisper-rs, macOS"
        rows={3}
        autoComplete="off"
        autoCorrect="off"
        autoCapitalize="off"
        spellCheck={false}
        className="w-full px-3 py-2 text-xs rounded-lg border border-stone-300 dark:border-stone-600 bg-white dark:bg-stone-700 text-stone-900 dark:text-stone-100 placeholder-stone-400 dark:placeholder-stone-500 focus:outline-none focus:ring-2 focus:ring-stone-500 resize-y"
      />
      <div className="mt-1.5 flex items-start justify-between gap-2">
        <p className="text-xs text-stone-500 dark:text-stone-400">
          Comma-separated. Whisper models only.
        </p>
        {draft.trim().length > 0 && (() => {
          const displayCount = tokenCount ?? Math.ceil(draft.trim().length / 4);
          const isEstimate = tokenCount === null;
          return (
            <span className={`text-xs tabular-nums whitespace-nowrap ${displayCount > 200 ? 'text-amber-600 dark:text-amber-400' : 'text-stone-400 dark:text-stone-500'}`}>
              {isEstimate ? `~${displayCount}` : displayCount} tokens
            </span>
          );
        })()}
      </div>
    </div>
  );
}

function VadSensitivitySlider({ value, onCommit }: { value: number; onCommit: (v: number) => void }) {
  const [draft, setDraft] = useState(value);
  useEffect(() => { setDraft(value); }, [value]);

  return (
    <div>
      <div className="flex items-center justify-between mb-1">
        <label className="text-xs text-stone-600 dark:text-stone-400">
          Sensitivity
        </label>
        <span className="text-xs font-medium text-stone-700 dark:text-stone-300">
          {draft}%
        </span>
      </div>
      <input
        type="range"
        min={0}
        max={100}
        step={5}
        value={draft}
        onChange={(e) => setDraft(Number(e.target.value))}
        onPointerUp={() => onCommit(draft)}
        className="w-full h-1.5 bg-stone-200 dark:bg-stone-600 rounded-full appearance-none cursor-pointer accent-stone-800 dark:accent-stone-300"
      />
      <p className="mt-1 text-xs text-stone-500 dark:text-stone-400">
        Higher = keeps more audio. Lower = trims silence more aggressively.
      </p>
    </div>
  );
}

// Per-app profiles: simple add/remove list mapping a macOS bundle id to an
// auto-paste override. The override cycles Default -> On -> Off so the whole
// control fits in one tap-through button.
function AppProfilesEditor({ profiles, onChange }: {
  profiles: AppProfile[];
  onChange: (next: AppProfile[]) => void;
}) {
  const [bundleId, setBundleId] = useState('');
  const [label, setLabel] = useState('');

  const handleAdd = () => {
    const trimmedId = bundleId.trim();
    if (!trimmedId) return;
    if (profiles.some((p) => p.bundleId === trimmedId)) {
      // Already have a profile for this app — clear the inputs and bail.
      setBundleId('');
      setLabel('');
      return;
    }
    onChange([
      ...profiles,
      { bundleId: trimmedId, label: label.trim(), autoPasteOverride: null, cleanupOverride: null },
    ]);
    setBundleId('');
    setLabel('');
  };

  const handleRemove = (id: string) => {
    onChange(profiles.filter((p) => p.bundleId !== id));
  };

  // Cycle a tri-state override: Default (null) -> On (true) -> Off (false) -> Default.
  const cycle = (value: boolean | null): boolean | null =>
    value === null ? true : value === true ? false : null;

  const cyclePaste = (id: string) => {
    onChange(profiles.map((p) =>
      p.bundleId === id ? { ...p, autoPasteOverride: cycle(p.autoPasteOverride) } : p));
  };

  const cycleCleanup = (id: string) => {
    onChange(profiles.map((p) =>
      p.bundleId === id ? { ...p, cleanupOverride: cycle(p.cleanupOverride) } : p));
  };

  const chipLabel = (kind: string, value: boolean | null) =>
    value === null ? `${kind}: Default` : value ? `${kind}: On` : `${kind}: Off`;

  const chipClass = (value: boolean | null) =>
    value === null
      ? 'border-stone-300 dark:border-stone-600 bg-white dark:bg-stone-700 text-stone-600 dark:text-stone-300'
      : value
        ? 'border-emerald-300 dark:border-emerald-700 bg-emerald-50 dark:bg-emerald-900/20 text-emerald-700 dark:text-emerald-400'
        : 'border-amber-300 dark:border-amber-700 bg-amber-50 dark:bg-amber-900/20 text-amber-700 dark:text-amber-400';

  return (
    <div>
      <p className="mb-2 text-xs text-stone-500 dark:text-stone-400">
        Override auto-paste and transcript cleanup for specific apps by bundle id
        (e.g. <span className="font-mono">com.apple.Terminal</span>). The frontmost
        app when you finish dictating decides the behavior. Each toggle cycles
        Default → On → Off.
      </p>

      {profiles.length > 0 && (
        <ul className="mb-3 space-y-1.5">
          {profiles.map((p) => (
            <li
              key={p.bundleId}
              className="flex flex-col gap-2 px-2.5 py-2 rounded-lg border border-stone-200 dark:border-stone-600 bg-stone-50 dark:bg-stone-700/40"
            >
              <div className="flex items-center gap-2">
                <div className="min-w-0 flex-1">
                  {p.label && (
                    <div className="text-xs font-medium text-stone-700 dark:text-stone-300 truncate">
                      {p.label}
                    </div>
                  )}
                  <div className="text-xs font-mono text-stone-500 dark:text-stone-400 truncate">
                    {p.bundleId}
                  </div>
                </div>
                <button
                  type="button"
                  onClick={() => handleRemove(p.bundleId)}
                  aria-label={`Remove profile for ${p.label || p.bundleId}`}
                  className="shrink-0 p-1 rounded-md text-stone-400 hover:text-red-600 hover:bg-stone-100 dark:hover:bg-stone-600 transition-colors"
                >
                  <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
                  </svg>
                </button>
              </div>
              <div className="flex gap-1.5">
                <button
                  type="button"
                  onClick={() => cyclePaste(p.bundleId)}
                  aria-label={`Auto-paste for ${p.label || p.bundleId}: ${chipLabel('Paste', p.autoPasteOverride)}`}
                  className={`flex-1 px-2 py-1 rounded-md text-xs font-medium border transition-colors ${chipClass(p.autoPasteOverride)}`}
                >
                  {chipLabel('Paste', p.autoPasteOverride)}
                </button>
                <button
                  type="button"
                  onClick={() => cycleCleanup(p.bundleId)}
                  aria-label={`Cleanup for ${p.label || p.bundleId}: ${chipLabel('Clean', p.cleanupOverride)}`}
                  className={`flex-1 px-2 py-1 rounded-md text-xs font-medium border transition-colors ${chipClass(p.cleanupOverride)}`}
                >
                  {chipLabel('Clean', p.cleanupOverride)}
                </button>
              </div>
            </li>
          ))}
        </ul>
      )}

      <div className="space-y-2">
        <input
          type="text"
          value={label}
          onChange={(e) => setLabel(e.target.value)}
          placeholder="Label (optional, e.g. Terminal)"
          autoComplete="off"
          autoCorrect="off"
          autoCapitalize="off"
          spellCheck={false}
          className="w-full px-3 py-2 text-xs rounded-lg border border-stone-300 dark:border-stone-600 bg-white dark:bg-stone-700 text-stone-900 dark:text-stone-100 placeholder-stone-400 dark:placeholder-stone-500 focus:outline-none focus:ring-2 focus:ring-stone-500"
        />
        <div className="flex gap-2">
          <input
            type="text"
            value={bundleId}
            onChange={(e) => setBundleId(e.target.value)}
            onKeyDown={(e) => { if (e.key === 'Enter') handleAdd(); }}
            placeholder="com.apple.Terminal"
            autoComplete="off"
            autoCorrect="off"
            autoCapitalize="off"
            spellCheck={false}
            className="flex-1 min-w-0 px-3 py-2 text-xs font-mono rounded-lg border border-stone-300 dark:border-stone-600 bg-white dark:bg-stone-700 text-stone-900 dark:text-stone-100 placeholder-stone-400 dark:placeholder-stone-500 focus:outline-none focus:ring-2 focus:ring-stone-500"
          />
          <button
            type="button"
            onClick={handleAdd}
            disabled={!bundleId.trim()}
            className="shrink-0 px-3 py-2 rounded-lg text-xs font-medium border border-stone-300 dark:border-stone-600 bg-white dark:bg-stone-700 text-stone-700 dark:text-stone-300 hover:bg-stone-50 dark:hover:bg-stone-600 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
          >
            Add
          </button>
        </div>
      </div>
    </div>
  );
}

// Custom voice commands: user-defined phrase -> replacement pairs applied after
// the built-in command set. Simple add/remove list.
function VoiceCommandsEditor({ commands, onChange }: {
  commands: VoiceCommand[];
  onChange: (next: VoiceCommand[]) => void;
}) {
  const [phrase, setPhrase] = useState('');
  const [replacement, setReplacement] = useState('');

  const handleAdd = () => {
    const trimmedPhrase = phrase.trim();
    if (!trimmedPhrase) return;
    if (commands.some((c) => c.phrase.toLowerCase() === trimmedPhrase.toLowerCase())) {
      setPhrase('');
      setReplacement('');
      return;
    }
    onChange([...commands, { phrase: trimmedPhrase, replacement }]);
    setPhrase('');
    setReplacement('');
  };

  const handleRemove = (p: string) => {
    onChange(commands.filter((c) => c.phrase !== p));
  };

  return (
    <div className="mt-3">
      <p className="mb-2 text-xs text-stone-500 dark:text-stone-400">
        Add your own spoken phrases. When you say the phrase it's replaced by the
        text (case-insensitive). Runs after the built-in commands.
      </p>

      {commands.length > 0 && (
        <ul className="mb-3 space-y-1.5">
          {commands.map((c) => (
            <li
              key={c.phrase}
              className="flex items-center gap-2 px-2.5 py-2 rounded-lg border border-stone-200 dark:border-stone-600 bg-stone-50 dark:bg-stone-700/40"
            >
              <div className="min-w-0 flex-1">
                <div className="text-xs font-medium text-stone-700 dark:text-stone-300 truncate">
                  “{c.phrase}”
                </div>
                <div className="text-xs font-mono text-stone-500 dark:text-stone-400 truncate">
                  → {c.replacement || '(empty)'}
                </div>
              </div>
              <button
                type="button"
                onClick={() => handleRemove(c.phrase)}
                aria-label={`Remove voice command ${c.phrase}`}
                className="shrink-0 p-1 rounded-md text-stone-400 hover:text-red-600 hover:bg-stone-100 dark:hover:bg-stone-600 transition-colors"
              >
                <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
                </svg>
              </button>
            </li>
          ))}
        </ul>
      )}

      <div className="space-y-2">
        <input
          type="text"
          value={phrase}
          onChange={(e) => setPhrase(e.target.value)}
          placeholder="Spoken phrase (e.g. my email)"
          autoComplete="off"
          autoCorrect="off"
          autoCapitalize="off"
          spellCheck={false}
          className="w-full px-3 py-2 text-xs rounded-lg border border-stone-300 dark:border-stone-600 bg-white dark:bg-stone-700 text-stone-900 dark:text-stone-100 placeholder-stone-400 dark:placeholder-stone-500 focus:outline-none focus:ring-2 focus:ring-stone-500"
        />
        <div className="flex gap-2">
          <input
            type="text"
            value={replacement}
            onChange={(e) => setReplacement(e.target.value)}
            onKeyDown={(e) => { if (e.key === 'Enter') handleAdd(); }}
            placeholder="Replacement (e.g. me@example.com)"
            autoComplete="off"
            autoCorrect="off"
            autoCapitalize="off"
            spellCheck={false}
            className="flex-1 min-w-0 px-3 py-2 text-xs rounded-lg border border-stone-300 dark:border-stone-600 bg-white dark:bg-stone-700 text-stone-900 dark:text-stone-100 placeholder-stone-400 dark:placeholder-stone-500 focus:outline-none focus:ring-2 focus:ring-stone-500"
          />
          <button
            type="button"
            onClick={handleAdd}
            disabled={!phrase.trim()}
            className="shrink-0 px-3 py-2 rounded-lg text-xs font-medium border border-stone-300 dark:border-stone-600 bg-white dark:bg-stone-700 text-stone-700 dark:text-stone-300 hover:bg-stone-50 dark:hover:bg-stone-600 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
          >
            Add
          </button>
        </div>
      </div>
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

const SETTINGS_CATEGORIES = [
  { id: 'transcription', label: 'Transcription' },
  { id: 'recording', label: 'Recording' },
  { id: 'output', label: 'Output & Paste' },
  { id: 'profiles', label: 'Per-App Profiles' },
  { id: 'vocab', label: 'Vocabulary' },
  { id: 'about', label: 'About' },
] as const;

export function SettingsPanel({ isOpen, onClose, settings, onUpdateSettings, status, onResetStats, onViewLogs, accessibilityGranted, onCheckForUpdate, updateStatus }: SettingsPanelProps) {
  const [confirmReset, setConfirmReset] = useState(false);
  const [activeCat, setActiveCat] = useState<string>('transcription');
  const [version, setVersion] = useState('');

  useEffect(() => { getVersion().then(setVersion); }, []);
  const confirmResetTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

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

  const handleRequestPermission = () => invoke('request_accessibility_permission');

  const handleChooseFolder = async () => {
    try {
      const selected = await open({ directory: true, multiple: false });
      if (typeof selected === 'string') {
        onUpdateSettings({ outputDir: selected });
      }
    } catch {
      // Dialog cancelled or unavailable — keep the current folder.
    }
  };

  // Code-vocab scan: live walker + ticking counts + done-state. Seeded from the
  // persisted last-scan summary so reopening settings shows the prior result.
  const vocabScan = useVocabScan(settings.codeVocabLastScan);
  // useVocabScan returns a fresh object literal each render, but scan/cancel are
  // stable useCallbacks — depend on the function, not the whole object, so
  // runVocabScan (and the closures built from it) keep a stable identity.
  const { scan: doScan } = vocabScan;

  // Run a scan against a folder and persist its summary so the done-state
  // survives a settings reopen. Persisting via onUpdateSettings keeps the hook
  // and localStorage in sync (the hook also resolves to the summary).
  const runVocabScan = useCallback(
    async (folder: string) => {
      if (!folder) return;
      const summary = await doScan(folder);
      if (summary) onUpdateSettings({ codeVocabLastScan: summary });
    },
    [doScan, onUpdateSettings],
  );

  const handleChooseCodeVocabFolder = async () => {
    try {
      const selected = await open({ directory: true, multiple: false });
      if (typeof selected === 'string') {
        onUpdateSettings({ codeVocabFolder: selected });
        // Auto-scan the freshly chosen folder.
        void runVocabScan(selected);
      }
    } catch {
      // Dialog cancelled or unavailable — keep the current folder.
    }
  };

  // Clear the folder: also drop the persisted scan and reset the strip to idle.
  const handleClearCodeVocabFolder = () => {
    vocabScan.cancel();
    onUpdateSettings({ codeVocabFolder: '', codeVocabLastScan: null });
  };

  const saveToFile = settings.saveTranscript || settings.saveAudio;

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
  const selectedModel = AVAILABLE_MODEL_OPTIONS.find(m => m.value === settings.model);
  const supportsCoreMl = AVAILABLE_MODEL_OPTIONS.some(m => m.backend === 'coreml');
  const useNeuralEngine = selectedModel?.backend === 'coreml';
  // The sherpa Parakeet bundle is English-only; FluidAudio Parakeet v3 is multilingual.
  const isEnglishOnlyModel = settings.model.endsWith('.en') || selectedModel?.backend === 'parakeet';
  const downloadProgressPercent =
    modelDownload.phase === 'downloading' && modelDownload.total > 0
      ? Math.round((modelDownload.received / modelDownload.total) * 100)
      : null;

  return (
    <div className="flex-1 flex overflow-hidden bg-white dark:bg-stone-900">
      {/* Left: category nav rail */}
      <nav className="w-48 shrink-0 flex flex-col border-r border-stone-200 dark:border-stone-700 bg-stone-50 dark:bg-stone-800/40 overflow-y-auto">
        <div className="flex items-center justify-between h-12 shrink-0 px-3">
          <h2 className="text-sm font-semibold text-stone-900 dark:text-stone-100">Settings</h2>
          <button
            onClick={onClose}
            aria-label="Close settings"
            className="p-1 rounded-md text-stone-500 hover:bg-stone-200 dark:hover:bg-stone-700 transition-colors"
          >
            <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>
        <div className="px-2 pb-3 space-y-0.5">
          {SETTINGS_CATEGORIES.map((c) => (
            <button
              key={c.id}
              type="button"
              onClick={() => setActiveCat(c.id)}
              className={`w-full text-left px-3 py-2 rounded-lg text-sm transition-colors ${
                activeCat === c.id
                  ? 'bg-stone-200 dark:bg-stone-700 text-stone-900 dark:text-stone-100 font-medium'
                  : 'text-stone-600 dark:text-stone-400 hover:bg-stone-100 dark:hover:bg-stone-700/50'
              }`}
            >
              {c.label}
            </button>
          ))}
        </div>
      </nav>

      {/* Right: active category content */}
      <div className="flex-1 overflow-y-auto">
        <div className="max-w-2xl px-6 py-5">
        <SettingsSection pageId="transcription" activePage={activeCat} title="Transcription" subtitle="Model, language, microphone">
        {supportsCoreMl && (
          <div className="flex items-center justify-between gap-6">
            <div>
              <label className="block text-sm font-medium text-stone-700 dark:text-stone-300">
                Apple Neural Engine
              </label>
              <p className="mt-1 text-xs text-stone-500 dark:text-stone-400">
                {useNeuralEngine
                  ? 'Parakeet v3 via Core ML (fastest)'
                  : 'Enable to switch to the Core ML transcription path'}
              </p>
            </div>
            <button
              type="button"
              role="switch"
              aria-checked={useNeuralEngine}
              aria-label="Use Apple Neural Engine"
              disabled={isRecording}
              onClick={() => onUpdateSettings({
                model: useNeuralEngine
                  ? 'parakeet-tdt-0.6b-v2-fp16'
                  : 'parakeet-tdt-0.6b-v3-coreml',
              })}
              className={`relative inline-flex shrink-0 h-6 w-11 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-stone-500 focus:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50 ${
                useNeuralEngine ? 'bg-stone-800 dark:bg-stone-300' : 'bg-stone-300 dark:bg-stone-500'
              }`}
            >
              <span
                className={`inline-block h-4 w-4 transform rounded-full bg-white shadow transition-transform ${
                  useNeuralEngine ? 'translate-x-6' : 'translate-x-1'
                }`}
              />
            </button>
          </div>
        )}

        {/* Model Selector */}
        <div>
          <label className="block text-sm font-medium text-stone-700 dark:text-stone-300 mb-2">
            Transcription Model
          </label>
          <Select
            value={settings.model}
            onChange={(value) => onUpdateSettings({ model: value })}
            disabled={isRecording}
            items={AVAILABLE_MODEL_OPTIONS.map((m) => ({ value: m.value, label: `${m.label} (${m.size})` }))}
          />
          <p className="mt-1 text-xs text-stone-500 dark:text-stone-400">
            Larger models are more accurate but slower.
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

        {/* Language Selector */}
        <div>
          <label className="block text-sm font-medium text-stone-700 dark:text-stone-300 mb-2">
            Language
          </label>
          <Select
            value={settings.language}
            onChange={(value) => onUpdateSettings({ language: value })}
            disabled={isRecording || isEnglishOnlyModel}
            items={LANGUAGE_OPTIONS}
          />
          {isEnglishOnlyModel ? (
            <p className="mt-1 text-xs text-stone-500 dark:text-stone-400">
              English only model — switch to Whisper Large Turbo for other languages.
            </p>
          ) : isRecording ? (
            <p className="mt-1 text-xs text-amber-600 dark:text-amber-400">
              Stop recording before changing language
            </p>
          ) : (
            <p className="mt-1 text-xs text-stone-500 dark:text-stone-400">
              Auto Detect lets Whisper identify the language each recording.
            </p>
          )}
        </div>

        {/* Smart Punctuation Toggle */}
        <div className="flex items-center justify-between">
          <div>
            <label className="block text-sm font-medium text-stone-700 dark:text-stone-300">
              Smart Punctuation
            </label>
            <p className="mt-1 text-xs text-stone-500 dark:text-stone-400">
              Add periods, commas, and capitalization to transcriptions.
            </p>
          </div>
          <button
            type="button"
            role="switch"
            aria-checked={settings.smartPunctuation}
            aria-label="Smart punctuation"
            onClick={() => onUpdateSettings({ smartPunctuation: !settings.smartPunctuation })}
            className={`relative inline-flex shrink-0 h-6 w-11 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-stone-500 focus:ring-offset-2 ${
              settings.smartPunctuation ? 'bg-stone-800 dark:bg-stone-300' : 'bg-stone-300 dark:bg-stone-500'
            }`}
          >
            <span
              className={`inline-block h-4 w-4 transform rounded-full bg-white shadow transition-transform ${
                settings.smartPunctuation ? 'translate-x-6' : 'translate-x-1'
              }`}
            />
          </button>
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

        {/* Idle Timeout */}
        <div>
          <label className="block text-sm font-medium text-stone-700 dark:text-stone-300 mb-2">
            Release Model After Inactivity
          </label>
          <Select
            value={String(settings.idleTimeoutMinutes)}
            onChange={(value) => onUpdateSettings({ idleTimeoutMinutes: Number(value) })}
            disabled={isRecording}
            items={IDLE_TIMEOUT_OPTIONS.map((o) => ({ value: String(o.value), label: o.label }))}
          />
          <p className="mt-1 text-xs text-stone-500 dark:text-stone-400">
            Free memory by unloading the model when idle. Set to Never to keep it loaded.
          </p>
        </div>

        {/* Transcript Cleanup Toggle */}
        <div className="flex items-center justify-between">
          <div>
            <label className="block text-sm font-medium text-stone-700 dark:text-stone-300">
              Transcript Cleanup
            </label>
            <p className="mt-1 text-xs text-stone-500 dark:text-stone-400">
              Strip filler words (um, uh) and tidy spacing before pasting.
            </p>
          </div>
          <button
            type="button"
            role="switch"
            aria-checked={settings.cleanupEnabled}
            aria-label="Transcript cleanup"
            onClick={() => onUpdateSettings({ cleanupEnabled: !settings.cleanupEnabled })}
            className={`relative inline-flex shrink-0 h-6 w-11 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-stone-500 focus:ring-offset-2 ${
              settings.cleanupEnabled ? 'bg-stone-800 dark:bg-stone-300' : 'bg-stone-300 dark:bg-stone-500'
            }`}
          >
            <span
              className={`inline-block h-4 w-4 transform rounded-full bg-white shadow transition-transform ${
                settings.cleanupEnabled ? 'translate-x-6' : 'translate-x-1'
              }`}
            />
          </button>
        </div>

        {/* Cleanup sub-options — only meaningful while cleanup is enabled. */}
        {settings.cleanupEnabled && (
          <div className="ml-3 pl-3 border-l border-stone-200 dark:border-stone-700 space-y-3">
            <div className="flex items-center justify-between">
              <label className="text-xs text-stone-600 dark:text-stone-400">
                Remove filler words (um, uh)
              </label>
              <button
                type="button"
                role="switch"
                aria-checked={settings.cleanupRemoveFiller}
                aria-label="Remove filler words"
                onClick={() => onUpdateSettings({ cleanupRemoveFiller: !settings.cleanupRemoveFiller })}
                className={`relative inline-flex shrink-0 h-5 w-9 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-stone-500 focus:ring-offset-2 ${
                  settings.cleanupRemoveFiller ? 'bg-stone-800 dark:bg-stone-300' : 'bg-stone-300 dark:bg-stone-500'
                }`}
              >
                <span
                  className={`inline-block h-3.5 w-3.5 transform rounded-full bg-white shadow transition-transform ${
                    settings.cleanupRemoveFiller ? 'translate-x-4' : 'translate-x-1'
                  }`}
                />
              </button>
            </div>
            <div className="flex items-center justify-between">
              <label className="text-xs text-stone-600 dark:text-stone-400">
                Capitalize sentences
              </label>
              <button
                type="button"
                role="switch"
                aria-checked={settings.cleanupCapitalize}
                aria-label="Capitalize sentences"
                onClick={() => onUpdateSettings({ cleanupCapitalize: !settings.cleanupCapitalize })}
                className={`relative inline-flex shrink-0 h-5 w-9 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-stone-500 focus:ring-offset-2 ${
                  settings.cleanupCapitalize ? 'bg-stone-800 dark:bg-stone-300' : 'bg-stone-300 dark:bg-stone-500'
                }`}
              >
                <span
                  className={`inline-block h-3.5 w-3.5 transform rounded-full bg-white shadow transition-transform ${
                    settings.cleanupCapitalize ? 'translate-x-4' : 'translate-x-1'
                  }`}
                />
              </button>
            </div>
          </div>
        )}
        </SettingsSection>

        <SettingsSection pageId="recording" activePage={activeCat} title="Recording" subtitle="Trigger mode, shortcut key">
        {/* Voice Detection */}
        <div>
          <label className="block text-sm font-medium text-stone-700 dark:text-stone-300 mb-2">
            Voice Detection
          </label>
          <VadSensitivitySlider
            value={settings.vadSensitivity}
            onCommit={(value) => onUpdateSettings({ vadSensitivity: value })}
          />
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
        </SettingsSection>

        <SettingsSection pageId="output" activePage={activeCat} title="Output" subtitle="Auto-paste, launch at login">
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
          {settings.autoPaste && saveToFile && (
            <div className="mt-2 flex items-center gap-2 px-3 py-2 text-xs bg-amber-50 dark:bg-amber-900/20 border border-amber-200 dark:border-amber-800 rounded-lg text-amber-700 dark:text-amber-400">
              <span className="w-2 h-2 rounded-full bg-amber-500 shrink-0" />
              <span>Paused while saving audio or transcripts to file</span>
            </div>
          )}
          {settings.autoPaste && (
            <PasteDelaySlider
              value={settings.autoPasteDelayMs}
              onCommit={(value) => onUpdateSettings({ autoPasteDelayMs: value })}
            />
          )}
        </div>

        {/* Save Transcript to File Toggle */}
        <div className="flex items-center justify-between">
          <div>
            <label className="block text-sm font-medium text-stone-700 dark:text-stone-300">
              Save Transcript to File
            </label>
            <p className="mt-1 text-xs text-stone-500 dark:text-stone-400">
              Write each transcription to a .txt file
            </p>
          </div>
          <button
            type="button"
            role="switch"
            aria-checked={settings.saveTranscript}
            aria-label="Save transcript to file"
            onClick={() => onUpdateSettings({ saveTranscript: !settings.saveTranscript })}
            className={`relative inline-flex shrink-0 h-6 w-11 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-stone-500 focus:ring-offset-2 ${
              settings.saveTranscript ? 'bg-stone-800 dark:bg-stone-300' : 'bg-stone-300 dark:bg-stone-500'
            }`}
          >
            <span
              className={`inline-block h-4 w-4 transform rounded-full bg-white shadow transition-transform ${
                settings.saveTranscript ? 'translate-x-6' : 'translate-x-1'
              }`}
            />
          </button>
        </div>

        {/* Save Audio to File Toggle */}
        <div>
          <div className="flex items-center justify-between">
            <div>
              <label className="block text-sm font-medium text-stone-700 dark:text-stone-300">
                Save Audio to File
              </label>
              <p className="mt-1 text-xs text-stone-500 dark:text-stone-400">
                Write each recording to a .wav file
              </p>
            </div>
            <button
              type="button"
              role="switch"
              aria-checked={settings.saveAudio}
              aria-label="Save audio to file"
              onClick={() => onUpdateSettings({ saveAudio: !settings.saveAudio })}
              className={`relative inline-flex shrink-0 h-6 w-11 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-stone-500 focus:ring-offset-2 ${
                settings.saveAudio ? 'bg-stone-800 dark:bg-stone-300' : 'bg-stone-300 dark:bg-stone-500'
              }`}
            >
              <span
                className={`inline-block h-4 w-4 transform rounded-full bg-white shadow transition-transform ${
                  settings.saveAudio ? 'translate-x-6' : 'translate-x-1'
                }`}
              />
            </button>
          </div>

          {saveToFile && (
            <div className="mt-3">
              <label className="block text-xs text-stone-600 dark:text-stone-400 mb-1">
                Output Folder
              </label>
              <div className="px-3 py-2 text-xs rounded-lg border border-stone-300 dark:border-stone-600 bg-stone-50 dark:bg-stone-700/50 text-stone-700 dark:text-stone-300 break-all">
                {settings.outputDir || 'Documents/Murmur (default)'}
              </div>
              <div className="mt-2 flex items-center gap-3">
                <button
                  onClick={handleChooseFolder}
                  className="text-xs font-medium text-stone-600 hover:text-stone-900 dark:text-stone-400 dark:hover:text-stone-200 underline hover:no-underline transition-colors"
                >
                  Choose Folder
                </button>
                {settings.outputDir && (
                  <button
                    onClick={() => onUpdateSettings({ outputDir: '' })}
                    className="text-xs font-medium text-stone-500 hover:text-stone-800 dark:text-stone-500 dark:hover:text-stone-300 underline hover:no-underline transition-colors"
                  >
                    Reset to default
                  </button>
                )}
              </div>
              <p className="mt-2 text-xs text-stone-500 dark:text-stone-400">
                Text is still copied to the clipboard, but auto-paste is paused while saving to file.
              </p>
            </div>
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

        {/* Voice Commands Toggle */}
        <div className="flex items-center justify-between">
          <div>
            <label className="block text-sm font-medium text-stone-700 dark:text-stone-300">
              Voice Commands
            </label>
            <p className="mt-1 text-xs text-stone-500 dark:text-stone-400">
              Spoken tokens like "new line", "period", or "scratch that" transform the text before pasting.
            </p>
          </div>
          <button
            type="button"
            role="switch"
            aria-checked={settings.voiceCommandsEnabled}
            aria-label="Voice commands"
            onClick={() => onUpdateSettings({ voiceCommandsEnabled: !settings.voiceCommandsEnabled })}
            className={`relative inline-flex shrink-0 h-6 w-11 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-stone-500 focus:ring-offset-2 ${
              settings.voiceCommandsEnabled ? 'bg-stone-800 dark:bg-stone-300' : 'bg-stone-300 dark:bg-stone-500'
            }`}
          >
            <span
              className={`inline-block h-4 w-4 transform rounded-full bg-white shadow transition-transform ${
                settings.voiceCommandsEnabled ? 'translate-x-6' : 'translate-x-1'
              }`}
            />
          </button>
        </div>

        {settings.voiceCommandsEnabled && (
          <VoiceCommandsEditor
            commands={settings.voiceCommands}
            onChange={(next) => onUpdateSettings({ voiceCommands: next })}
          />
        )}
        </SettingsSection>

        <SettingsSection pageId="profiles" activePage={activeCat} title="Per-App Profiles" subtitle="Auto-paste + cleanup per frontmost app">
          <AppProfilesEditor
            profiles={settings.appProfiles}
            onChange={(next) => onUpdateSettings({ appProfiles: next })}
          />
        </SettingsSection>

        <SettingsSection pageId="vocab" activePage={activeCat} title="Vocabulary" subtitle="Bias transcription toward your terms">
        {/* Manual custom vocabulary — feeds the same initial prompt as code-aware. */}
        <CustomVocabularyTextarea
          value={settings.customVocabulary}
          onCommit={(value) => onUpdateSettings({ customVocabulary: value })}
        />

        {/* Code-Aware Vocabulary Toggle */}
        <div>
          <div className="flex items-center justify-between">
            <div>
              <label className="block text-sm font-medium text-stone-700 dark:text-stone-300">
                Code-Aware Vocabulary
              </label>
              <p className="mt-1 text-xs text-stone-500 dark:text-stone-400">
                Bias transcription toward common dev terms (useEffect, kubectl, stderr) — works out of the box, no folder needed. Optionally add a project folder for your own identifiers. With Smart Correction on (below), these terms are also fixed in the transcript on every model, including Parakeet.
              </p>
            </div>
            <button
              type="button"
              role="switch"
              aria-checked={settings.codeVocabEnabled}
              aria-label="Code-aware vocabulary"
              onClick={() => onUpdateSettings({ codeVocabEnabled: !settings.codeVocabEnabled })}
              className={`relative inline-flex shrink-0 h-6 w-11 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-stone-500 focus:ring-offset-2 ${
                settings.codeVocabEnabled ? 'bg-stone-800 dark:bg-stone-300' : 'bg-stone-300 dark:bg-stone-500'
              }`}
            >
              <span
                className={`inline-block h-4 w-4 transform rounded-full bg-white shadow transition-transform ${
                  settings.codeVocabEnabled ? 'translate-x-6' : 'translate-x-1'
                }`}
              />
            </button>
          </div>

          {settings.codeVocabEnabled && (
            <div className="mt-3">
              <label className="block text-xs text-stone-600 dark:text-stone-400 mb-1">
                Project Folder (optional)
              </label>
              <div className="px-3 py-2 text-xs rounded-lg border border-stone-300 dark:border-stone-600 bg-stone-50 dark:bg-stone-700/50 text-stone-700 dark:text-stone-300 break-all">
                {settings.codeVocabFolder || 'No folder — built-in dev terms only'}
              </div>
              <div className="mt-2 flex items-center gap-3">
                <button
                  onClick={handleChooseCodeVocabFolder}
                  className="text-xs font-medium text-stone-600 hover:text-stone-900 dark:text-stone-400 dark:hover:text-stone-200 underline hover:no-underline transition-colors"
                >
                  Choose Folder
                </button>
                {settings.codeVocabFolder && (
                  <button
                    onClick={handleClearCodeVocabFolder}
                    className="text-xs font-medium text-stone-500 hover:text-stone-800 dark:text-stone-500 dark:hover:text-stone-300 underline hover:no-underline transition-colors"
                  >
                    Clear
                  </button>
                )}
              </div>

              {/* Live scan feedback strip: idle / scanning / done+empty. */}
              <VocabScanStrip
                status={vocabScan.status}
                walker={vocabScan.walker}
                stats={vocabScan.stats}
                folder={settings.codeVocabFolder}
                onScan={() => void runVocabScan(settings.codeVocabFolder)}
                onCancel={vocabScan.cancel}
              />

              <p className="mt-2 text-xs text-stone-500 dark:text-stone-400">
                Optional. When set, the folder is scanned once for your identifiers (dependency and build directories are skipped) and layered on top of the built-in terms. Re-select to rescan after big code changes.
              </p>
            </div>
          )}
        </div>

        {/* Smart Correction — post-model, applies vocab on every backend (Tiers 1-2). */}
        <div>
          <div className="flex items-center justify-between">
            <div>
              <label className="block text-sm font-medium text-stone-700 dark:text-stone-300">
                Smart Correction
              </label>
              <p className="mt-1 text-xs text-stone-500 dark:text-stone-400">
                Fix vocabulary in the transcript after recognition, on every model. Turns "use effect" into useEffect and, with Code-Aware on, "standard error" into stderr.
              </p>
            </div>
            <button
              type="button"
              role="switch"
              aria-checked={settings.correctionEnabled}
              aria-label="Smart correction"
              onClick={() => onUpdateSettings({ correctionEnabled: !settings.correctionEnabled })}
              className={`relative inline-flex shrink-0 h-6 w-11 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-stone-500 focus:ring-offset-2 ${
                settings.correctionEnabled ? 'bg-stone-800 dark:bg-stone-300' : 'bg-stone-300 dark:bg-stone-500'
              }`}
            >
              <span
                className={`inline-block h-4 w-4 transform rounded-full bg-white shadow transition-transform ${
                  settings.correctionEnabled ? 'translate-x-6' : 'translate-x-1'
                }`}
              />
            </button>
          </div>

          {settings.correctionEnabled && (
            <div className="mt-3 flex items-center justify-between gap-4">
              <div>
                <label className="block text-xs font-medium text-stone-700 dark:text-stone-300">
                  Sounds-like matching
                </label>
                <p className="mt-1 text-xs text-stone-500 dark:text-stone-400">
                  Also recover close mishearings near your vocabulary (e.g. "red pivot" → "rePivot"). Turn off if you see unwanted swaps.
                </p>
              </div>
              <button
                type="button"
                role="switch"
                aria-checked={settings.correctionFuzzy}
                aria-label="Sounds-like matching"
                onClick={() => onUpdateSettings({ correctionFuzzy: !settings.correctionFuzzy })}
                className={`relative inline-flex shrink-0 h-5 w-9 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-stone-500 focus:ring-offset-2 ${
                  settings.correctionFuzzy ? 'bg-stone-800 dark:bg-stone-300' : 'bg-stone-300 dark:bg-stone-500'
                }`}
              >
                <span
                  className={`inline-block h-3.5 w-3.5 transform rounded-full bg-white shadow transition-transform ${
                    settings.correctionFuzzy ? 'translate-x-4' : 'translate-x-1'
                  }`}
                />
              </button>
            </div>
          )}
        </div>
        </SettingsSection>

        <SettingsSection pageId="about" activePage={activeCat} title="About" subtitle="Stats, logs, updates">
        {/* Model Info */}
        <div>
          <div className="text-sm text-stone-600 dark:text-stone-400">
            <p><strong>Model:</strong> {selectedModel?.label}</p>
            <p><strong>Backend:</strong> {selectedModel?.backend === 'coreml' ? 'FluidAudio (Apple Neural Engine)' : selectedModel?.backend === 'parakeet' ? 'sherpa-onnx (CPU)' : 'Whisper (Metal GPU)'}</p>
            <p><strong>Size:</strong> {selectedModel?.size}</p>
          </div>
        </div>

        {/* Reset Stats */}
        <div>
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
        <div>
          <button
            onClick={onViewLogs}
            className="w-full px-3 py-2 rounded-lg text-xs font-medium border border-stone-300 dark:border-stone-600 bg-white dark:bg-stone-700 text-stone-700 dark:text-stone-300 hover:bg-stone-50 dark:hover:bg-stone-600 transition-colors"
          >
            View Logs
          </button>
        </div>

        {/* Updates */}
        <div>
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

        {/* Version */}
        {version && (
          <div className="text-center">
            <span className="text-xs text-stone-400 dark:text-stone-500">v{version}</span>
          </div>
        )}
        </SettingsSection>
        </div>
      </div>
    </div>
  );
}
