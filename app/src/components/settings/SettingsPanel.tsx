import { useCallback, useEffect, useRef, useState } from 'react';
import { getVersion } from '@tauri-apps/api/app';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import {
  AVAILABLE_MODEL_OPTIONS,
  DEFAULT_SETTINGS,
  DOUBLE_TAP_KEY_OPTIONS,
  IDLE_TIMEOUT_OPTIONS,
  LANGUAGE_OPTIONS,
  RECORDING_MODE_OPTIONS,
  type RecordingMode,
  type Settings,
  vocabularyPrompt,
} from '../../lib/settings';
import { useVocabScan } from '../../lib/hooks/useVocabScan';
import { useModelRuntimeCatalog } from '../../lib/modelRuntime';
import {
  modelDownloadLabel,
  modelDownloadPercent,
  type ModelDownloadProgress,
} from '../../lib/modelDownload';
import type { DictationStatus } from '../../lib/types';
import type { UpdateStatus } from '../../lib/updater';
import { Select } from '../ui/Select';
import { AppOverridesEditor } from './AppOverridesEditor';
import { KnowledgeManager } from './KnowledgeManager';
import { PerformanceLab } from './PerformanceLab';
import { SettingsSection } from './SettingsSection';
import { VocabScanStrip } from './VocabScanStrip';
import { VocabularyAliasesEditor } from './VocabularyAliasesEditor';
import { VoiceCommandsManager } from './VoiceCommandsManager';

function Toggle({ label, checked, onChange, disabled = false }: {
  label: string;
  checked: boolean;
  onChange: () => void;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      onClick={onChange}
      className={`relative inline-flex h-6 w-11 shrink-0 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-primary focus:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50 ${checked ? 'bg-primary' : 'bg-surface-container-highest'}`}
    >
      <span className={`inline-block h-4 w-4 rounded-full bg-on-primary shadow transition-transform ${checked ? 'translate-x-6' : 'translate-x-1'}`} />
    </button>
  );
}

function SettingToggle({ title, description, label = title, checked, onChange, disabled = false }: {
  title: string;
  description: string;
  label?: string;
  checked: boolean;
  onChange: () => void;
  disabled?: boolean;
}) {
  return (
    <div className="flex items-start justify-between gap-6">
      <div>
        <p className="text-sm font-medium text-on-surface">{title}</p>
        <p className="mt-1 text-xs text-on-surface-variant">{description}</p>
      </div>
      <Toggle label={label} checked={checked} onChange={onChange} disabled={disabled} />
    </div>
  );
}

function PasteDelaySlider({ value, onCommit }: { value: number; onCommit: (value: number) => void }) {
  const [draft, setDraft] = useState(value);
  useEffect(() => setDraft(value), [value]);
  return (
    <div>
      <div className="mb-1 flex items-center justify-between">
        <label className="text-xs text-on-surface-variant">Paste Delay</label>
        <span className="text-xs font-medium text-on-surface">{draft}ms</span>
      </div>
      <input
        type="range"
        min={10}
        max={500}
        step={10}
        value={draft}
        onChange={(event) => setDraft(Number(event.target.value))}
        onPointerUp={() => onCommit(draft)}
        className="h-1.5 w-full cursor-pointer appearance-none rounded-full bg-surface-container-highest accent-primary"
      />
      <p className="mt-1 text-xs text-on-surface-variant">Increase this if paste lands in the wrong window.</p>
    </div>
  );
}

function VadSensitivitySlider({ value, onCommit }: { value: number; onCommit: (value: number) => void }) {
  const [draft, setDraft] = useState(value);
  useEffect(() => setDraft(value), [value]);
  return (
    <div>
      <div className="mb-1 flex items-center justify-between">
        <label className="text-xs text-on-surface-variant">Sensitivity</label>
        <span className="text-xs font-medium text-on-surface">{draft}%</span>
      </div>
      <input
        type="range"
        min={0}
        max={100}
        step={5}
        value={draft}
        onChange={(event) => setDraft(Number(event.target.value))}
        onPointerUp={() => onCommit(draft)}
        className="h-1.5 w-full cursor-pointer appearance-none rounded-full bg-surface-container-highest accent-primary"
      />
      <p className="mt-1 text-xs text-on-surface-variant">Higher keeps more audio; lower trims silence more aggressively.</p>
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
  onRerunSetup: () => void;
  accessibilityGranted: boolean | null;
  onCheckForUpdate: () => Promise<void>;
  updateStatus: UpdateStatus;
  configureError: string | null;
}

export const SETTINGS_CATEGORIES = [
  { id: 'recording', label: 'Recording' },
  { id: 'transcription', label: 'Transcription' },
  { id: 'text-vocabulary', label: 'Text & Vocabulary' },
  { id: 'delivery', label: 'Delivery' },
  { id: 'performance', label: 'Performance' },
  { id: 'general', label: 'General' },
] as const;

export function effectiveAutoPaste(settings: Pick<Settings, 'autoPaste' | 'saveTranscript' | 'saveAudio'>): boolean {
  return settings.autoPaste && !settings.saveTranscript && !settings.saveAudio;
}

export function SettingsPanel({
  isOpen,
  onClose,
  settings,
  onUpdateSettings,
  status,
  onResetStats,
  onViewLogs,
  onRerunSetup,
  accessibilityGranted,
  onCheckForUpdate,
  updateStatus,
  configureError,
}: SettingsPanelProps) {
  const { byName: runtimeByName } = useModelRuntimeCatalog(isOpen);
  const [activeCat, setActiveCat] = useState<string>('recording');
  const [version, setVersion] = useState('');
  const [confirmReset, setConfirmReset] = useState(false);
  const contentRef = useRef<HTMLDivElement>(null);
  const confirmResetTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => { void getVersion().then(setVersion); }, []);
  useEffect(() => { contentRef.current?.scrollTo({ top: 0 }); }, [activeCat]);
  useEffect(() => () => {
    if (confirmResetTimeoutRef.current) clearTimeout(confirmResetTimeoutRef.current);
  }, []);

  const requestAccessibility = () => { void invoke('request_accessibility_permission'); };
  const chooseOutputFolder = async () => {
    try {
      const selected = await open({ directory: true, multiple: false });
      if (typeof selected === 'string') onUpdateSettings({ outputDir: selected });
    } catch {
      // Cancellation leaves the stored folder untouched.
    }
  };

  const vocabScan = useVocabScan(settings.codeVocabLastScan);
  const { scan: doScan } = vocabScan;
  const runVocabScan = useCallback(async (folder: string) => {
    if (!folder) return;
    const summary = await doScan(folder);
    if (summary?.adopted) onUpdateSettings({ codeVocabLastScan: summary });
    else if (summary) onUpdateSettings({ codeVocabLastScan: null });
  }, [doScan, onUpdateSettings]);
  const chooseCodeFolder = async () => {
    try {
      const selected = await open({ directory: true, multiple: false });
      if (typeof selected !== 'string') return;
      onUpdateSettings({ codeVocabFolder: selected, codeVocabLastScan: null });
      void runVocabScan(selected);
    } catch {
      // Cancellation leaves the stored folder untouched.
    }
  };
  const clearCodeFolder = () => {
    vocabScan.cancel();
    onUpdateSettings({ codeVocabFolder: '', codeVocabLastScan: null });
  };

  const selectedRuntime = runtimeByName.get(settings.model);
  const modelAvailable = selectedRuntime ? selectedRuntime.installState === 'installed' : null;
  const [modelDownload, setModelDownload] = useState<
    | { phase: 'idle' }
    | { phase: 'downloading'; progress: ModelDownloadProgress }
    | { phase: 'error'; message: string }
  >({ phase: 'idle' });
  const downloadUnlistenRef = useRef<(() => void) | null>(null);
  const downloadModelRef = useRef<string | null>(null);

  useEffect(() => {
    setModelDownload({ phase: 'idle' });
    downloadModelRef.current = null;
  }, [settings.model]);
  useEffect(() => () => {
    downloadUnlistenRef.current?.();
    downloadUnlistenRef.current = null;
  }, []);

  const downloadModel = useCallback(async () => {
    const modelName = settings.model;
    downloadModelRef.current = modelName;
    setModelDownload({ phase: 'downloading', progress: { received: 0, total: 0, phase: 'downloading' } });
    let unlisten: (() => void) | null = null;
    try {
      unlisten = await listen<ModelDownloadProgress>('download-progress', (event) => {
        if (downloadModelRef.current === modelName) setModelDownload({ phase: 'downloading', progress: event.payload });
      });
      downloadUnlistenRef.current = unlisten;
      await invoke('download_model', { modelName });
      unlisten();
      downloadUnlistenRef.current = null;
      if (downloadModelRef.current === modelName) setModelDownload({ phase: 'idle' });
    } catch (error) {
      unlisten?.();
      downloadUnlistenRef.current = null;
      if (downloadModelRef.current === modelName) setModelDownload({ phase: 'error', message: String(error) });
    } finally {
      if (downloadModelRef.current === modelName) downloadModelRef.current = null;
    }
  }, [settings.model]);

  const [audioDevices, setAudioDevices] = useState<string[]>([]);
  useEffect(() => {
    if (!isOpen) return;
    invoke<string[]>('list_audio_devices').then(setAudioDevices).catch(() => setAudioDevices([]));
  }, [isOpen]);

  const isRecording = status !== 'idle';
  const isDoubleTap = settings.recordingMode === 'double_tap';
  const isBoth = settings.recordingMode === 'both';
  const keyLabel = isBoth ? 'Trigger Key' : isDoubleTap ? 'Double-Tap Key' : 'Hold Key';
  const keyHelp = isBoth
    ? 'Hold to record, or double-tap to start and single-tap to stop.'
    : isDoubleTap ? 'Double-tap to start and single-tap to stop.' : 'Hold to start and release to stop.';
  const missingDevice = settings.microphone !== DEFAULT_SETTINGS.microphone
    && audioDevices.length > 0
    && !audioDevices.includes(settings.microphone);
  const englishOnly = selectedRuntime ? !selectedRuntime.capabilities.multilingual : true;
  const downloadProgress = modelDownload.phase === 'downloading'
    ? modelDownloadPercent(modelDownload.progress)
    : null;
  const saveToFile = settings.saveTranscript || settings.saveAudio;
  const autoPasteOn = effectiveAutoPaste(settings);

  const resetStats = () => {
    if (confirmReset) {
      if (confirmResetTimeoutRef.current) clearTimeout(confirmResetTimeoutRef.current);
      confirmResetTimeoutRef.current = null;
      setConfirmReset(false);
      onResetStats();
      return;
    }
    setConfirmReset(true);
    confirmResetTimeoutRef.current = setTimeout(() => {
      setConfirmReset(false);
      confirmResetTimeoutRef.current = null;
    }, 3000);
  };

  return (
    <div className="flex flex-1 overflow-hidden bg-surface text-on-surface">
      <nav aria-label="Settings pages" className="flex w-48 shrink-0 flex-col overflow-y-auto bg-surface-container-low">
        <div className="flex h-12 shrink-0 items-center justify-between px-3">
          <h2 className="text-sm font-semibold text-on-surface">Settings</h2>
          <button onClick={onClose} aria-label="Close settings" className="rounded-md p-1 text-on-surface-variant transition-colors hover:bg-surface-container-high hover:text-on-surface">
            <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" /></svg>
          </button>
        </div>
        <div className="space-y-0.5 px-2 pb-3">
          {SETTINGS_CATEGORIES.map((category) => (
            <button
              key={category.id}
              type="button"
              aria-current={activeCat === category.id ? 'page' : undefined}
              onClick={() => setActiveCat(category.id)}
              className={`w-full rounded-lg px-3 py-2 text-left text-sm transition-colors ${activeCat === category.id ? 'bg-surface-container-high font-medium text-primary' : 'text-on-surface-variant hover:bg-surface-container'}`}
            >
              {category.label}
            </button>
          ))}
        </div>
      </nav>

      <div ref={contentRef} data-testid="settings-content" className="flex-1 overflow-y-auto">
        <div className="max-w-2xl px-6 py-5">
          {configureError && <p role="alert" className="mb-4 rounded-lg bg-error/10 px-3 py-2 text-xs text-error">{configureError}</p>}

          <SettingsSection pageId="recording" activePage={activeCat} title="Recording" subtitle="Microphone, voice detection, and shortcuts">
            <div>
              <label className="mb-2 block text-sm font-medium text-on-surface">Microphone</label>
              <Select value={settings.microphone} onChange={(microphone) => onUpdateSettings({ microphone })} disabled={isRecording} items={[{ value: 'system_default', label: 'System Default' }, ...audioDevices.map((name) => ({ value: name, label: name }))]} />
              {missingDevice && <p className="mt-2 rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs text-amber-700 dark:text-amber-400">Selected device not found — Murmur will use System Default.</p>}
            </div>
            <div>
              <p className="mb-2 text-sm font-medium text-on-surface">Voice Detection</p>
              <VadSensitivitySlider value={settings.vadSensitivity} onCommit={(vadSensitivity) => onUpdateSettings({ vadSensitivity })} />
            </div>
            <div>
              <p className="mb-2 text-sm font-medium text-on-surface">Recording Trigger</p>
              <div className="flex gap-2">
                {RECORDING_MODE_OPTIONS.map((option) => (
                  <button key={option.value} type="button" disabled={isRecording} onClick={() => onUpdateSettings({ recordingMode: option.value as RecordingMode })} className={`flex-1 rounded-lg border px-3 py-2 text-xs font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-50 ${settings.recordingMode === option.value ? 'border-primary bg-primary text-on-primary' : 'border-outline-variant/30 bg-surface-container-lowest text-on-surface hover:bg-surface-container'}`}>{option.label}</button>
                ))}
              </div>
              {isRecording && <p className="mt-1 text-xs text-amber-600 dark:text-amber-400">Stop recording before changing mode.</p>}
            </div>
            {accessibilityGranted === false && (
              <div className="flex items-center gap-2 rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs text-amber-700 dark:text-amber-400">
                <span>Accessibility permission is required for keyboard detection.</span>
                <button type="button" onClick={requestAccessibility} className="ml-auto underline">Grant</button>
              </div>
            )}
            <div>
              <label className="mb-2 block text-sm font-medium text-on-surface">{keyLabel}</label>
              <Select value={settings.doubleTapKey} onChange={(doubleTapKey) => onUpdateSettings({ doubleTapKey })} disabled={isRecording} items={DOUBLE_TAP_KEY_OPTIONS} />
              <p className="mt-1 text-xs text-on-surface-variant">{keyHelp}</p>
            </div>
            {(isDoubleTap || isBoth) && <SettingToggle title="Hotkey Timing Feedback" description="Flash the overlay when a tap misses the double-tap window." checked={settings.hotkeyMissFeedback} onChange={() => onUpdateSettings({ hotkeyMissFeedback: !settings.hotkeyMissFeedback })} />}
          </SettingsSection>

          <SettingsSection pageId="transcription" activePage={activeCat} title="Transcription" subtitle="Model, language, and runtime lifecycle">
            <div>
              <label className="mb-2 block text-sm font-medium text-on-surface">Transcription Model</label>
              <Select
                value={settings.model}
                onChange={(model) => onUpdateSettings({ model })}
                disabled={isRecording}
                items={AVAILABLE_MODEL_OPTIONS.map((model) => ({ value: model.value, label: `${model.label}${model.backend === 'coreml' ? ' — Recommended' : ''} (${model.size})` }))}
              />
              <p className="mt-1 text-xs text-on-surface-variant">Parakeet Core ML is recommended on supported Macs. Larger models can be more accurate but use more storage and memory.</p>
              {selectedRuntime && <p className="mt-1 text-xs text-on-surface-variant" data-testid="model-runtime-status">{selectedRuntime.label}: {selectedRuntime.backend} / {selectedRuntime.accelerator} / {selectedRuntime.size} · {selectedRuntime.installState} · {selectedRuntime.lifecycleState}</p>}
              {isRecording && <p className="mt-1 text-xs text-amber-600 dark:text-amber-400">Stop recording before changing model.</p>}
              {modelAvailable === false && modelDownload.phase === 'idle' && (
                <div className="mt-2 flex items-center rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs text-amber-700 dark:text-amber-400">
                  <span>Model not downloaded</span><button type="button" onClick={() => void downloadModel()} className="ml-auto underline">Download</button>
                </div>
              )}
              {modelDownload.phase === 'downloading' && (
                <div className="mt-2">
                  <div className="mb-1 flex justify-between text-xs text-on-surface-variant"><span>{modelDownloadLabel(modelDownload.progress)}</span><span>{downloadProgress === null ? 'Working…' : `${downloadProgress}%`}</span></div>
                  <div className="h-1.5 overflow-hidden rounded-full bg-surface-container-highest"><div role="progressbar" aria-valuenow={downloadProgress ?? undefined} aria-valuemin={0} aria-valuemax={100} aria-valuetext={downloadProgress === null ? 'Model installation in progress' : `Download progress: ${downloadProgress} percent`} className={`h-full rounded-full bg-primary ${downloadProgress === null ? 'model-download-indeterminate' : 'transition-all duration-200'}`} style={downloadProgress === null ? undefined : { width: `${downloadProgress}%` }} /></div>
                </div>
              )}
              {modelDownload.phase === 'error' && <div className="mt-2 flex items-center rounded-lg border border-error/30 bg-error/10 px-3 py-2 text-xs text-error"><span>{modelDownload.message}</span><button type="button" onClick={() => void downloadModel()} className="ml-auto underline">Retry</button></div>}
            </div>
            <div>
              <label className="mb-2 block text-sm font-medium text-on-surface">Language</label>
              <Select value={settings.language} onChange={(language) => onUpdateSettings({ language })} disabled={isRecording || englishOnly} items={LANGUAGE_OPTIONS} />
              <p className="mt-1 text-xs text-on-surface-variant">{englishOnly ? 'This model is English-only. Choose Whisper Large Turbo for other languages.' : 'Auto Detect lets Whisper identify the language for each recording.'}</p>
            </div>
            <div>
              <label className="mb-2 block text-sm font-medium text-on-surface">Release Model After Inactivity</label>
              <Select value={String(settings.idleTimeoutMinutes)} onChange={(value) => onUpdateSettings({ idleTimeoutMinutes: Number(value) })} disabled={isRecording} items={IDLE_TIMEOUT_OPTIONS.map((option) => ({ value: String(option.value), label: option.label }))} />
              <p className="mt-1 text-xs text-on-surface-variant">Free memory by unloading an idle model; choose Never to keep it ready.</p>
            </div>
          </SettingsSection>

          <SettingsSection pageId="text-vocabulary" activePage={activeCat} title="Text & Vocabulary" subtitle="Cleanup, preferred terms, structured writing, and knowledge">
            <SettingToggle title="Automatic Punctuation" label="Smart punctuation" description="Add periods, commas, and capitalization to transcriptions." checked={settings.smartPunctuation} onChange={() => onUpdateSettings({ smartPunctuation: !settings.smartPunctuation })} />
            <SettingToggle title="Transcript Cleanup" description="Remove filler and tidy spacing before delivery." checked={settings.cleanupEnabled} onChange={() => onUpdateSettings({ cleanupEnabled: !settings.cleanupEnabled })} />
            {settings.cleanupEnabled && (
              <div className="ml-3 space-y-3 border-l border-outline-variant/30 pl-3">
                <SettingToggle title="Remove filler words" description="Remove filler tokens such as um and uh." checked={settings.cleanupRemoveFiller} onChange={() => onUpdateSettings({ cleanupRemoveFiller: !settings.cleanupRemoveFiller })} />
                <SettingToggle title="Capitalize sentences" description="Capitalize detected sentence starts." checked={settings.cleanupCapitalize} onChange={() => onUpdateSettings({ cleanupCapitalize: !settings.cleanupCapitalize })} />
              </div>
            )}
            <div className="border-t border-outline-variant/20 pt-4">
              <h2 className="text-sm font-medium text-on-surface">Names & Terms</h2>
              <p className="mt-1 mb-3 text-xs text-on-surface-variant">Teach Murmur preferred spellings and exact spoken variants.</p>
              <VocabularyAliasesEditor entries={settings.vocabularyEntries} voiceCommands={settings.voiceCommands} onChange={(vocabularyEntries) => onUpdateSettings({ vocabularyEntries, customVocabulary: vocabularyPrompt(vocabularyEntries) })} />
            </div>
            <SettingToggle title="Developer Terms" description="Bias recognition toward built-in development terms and, optionally, identifiers from one project folder." checked={settings.codeVocabEnabled} onChange={() => onUpdateSettings({ codeVocabEnabled: !settings.codeVocabEnabled })} />
            {settings.codeVocabEnabled && (
              <div className="ml-3 space-y-2 border-l border-outline-variant/30 pl-3">
                <p className="break-all rounded-lg border border-outline-variant/30 bg-surface-container-lowest px-3 py-2 text-xs text-on-surface">{settings.codeVocabFolder || 'No folder — built-in developer terms only'}</p>
                <div className="flex gap-3">
                  <button type="button" onClick={() => void chooseCodeFolder()} className="text-xs font-medium text-on-surface-variant underline hover:text-primary">Choose Folder</button>
                  {settings.codeVocabFolder && <button type="button" onClick={clearCodeFolder} className="text-xs font-medium text-on-surface-variant underline hover:text-primary">Clear</button>}
                </div>
                <VocabScanStrip status={vocabScan.status} walker={vocabScan.walker} stats={vocabScan.stats} folder={settings.codeVocabFolder} onScan={() => void runVocabScan(settings.codeVocabFolder)} onCancel={vocabScan.cancel} />
                <p className="text-xs text-on-surface-variant">The selected folder is scanned locally; dependency and build folders are skipped.</p>
              </div>
            )}
            <SettingToggle title="Apply Preferred Spellings" label="Smart correction" description="Apply names, terms, and developer vocabulary after recognition on every model." checked={settings.correctionEnabled} onChange={() => onUpdateSettings({ correctionEnabled: !settings.correctionEnabled })} />
            {settings.correctionEnabled && <div className="ml-3 border-l border-outline-variant/30 pl-3"><SettingToggle title="Correct Close Mishearings" label="Sounds-like matching" description="Recover close mishearings near your vocabulary; disable if you see unwanted swaps." checked={settings.correctionFuzzy} onChange={() => onUpdateSettings({ correctionFuzzy: !settings.correctionFuzzy })} /></div>}
            <SettingToggle title="Structured Writing" label="Smart formatting" description="Apply explicitly spoken lists, symbols, punctuation, and same-utterance corrections locally." checked={settings.smartFormattingEnabled} onChange={() => onUpdateSettings({ smartFormattingEnabled: !settings.smartFormattingEnabled })} />
            <SettingToggle title="Spoken Formatting" label="Voice commands" description="Use spoken tokens such as “new line,” “period,” or “scratch that” before delivery." checked={settings.voiceCommandsEnabled} onChange={() => onUpdateSettings({ voiceCommandsEnabled: !settings.voiceCommandsEnabled })} />
            <div className="border-t border-outline-variant/20 pt-4">
              <h2 className="text-sm font-medium text-on-surface">Phrase Replacements & Snippets</h2>
              <p className="mt-1 mb-3 text-xs text-on-surface-variant">Create exact spoken phrases that insert replacement text or a multiline snippet.</p>
              <VoiceCommandsManager active={isOpen && activeCat === 'text-vocabulary'} globallyEnabled={settings.voiceCommandsEnabled} profiles={settings.appProfiles} />
            </div>
            <div className="border-t border-outline-variant/20 pt-4">
              <h2 className="mb-3 text-sm font-medium text-on-surface">Knowledge</h2>
              <KnowledgeManager active={isOpen && activeCat === 'text-vocabulary'} profiles={settings.appProfiles} />
            </div>
          </SettingsSection>

          <SettingsSection pageId="delivery" activePage={activeCat} title="Delivery" subtitle="Clipboard, paste, file output, and app-specific overrides">
            <div className="rounded-xl border border-primary/20 bg-primary/5 p-3">
              <h2 className="text-sm font-medium text-on-surface">Always copied to clipboard</h2>
              <p className="mt-1 text-xs text-on-surface-variant">Every completed transcription is copied first. Auto-paste and file output only change what happens next.</p>
            </div>
            <SettingToggle title="Auto-Paste" label="Auto paste" description={saveToFile ? 'Paused while file output is on. Your saved preference will resume when file output is off.' : 'Paste the clipboard result into the active app (Accessibility permission required).'} checked={autoPasteOn} disabled={saveToFile} onChange={() => onUpdateSettings({ autoPaste: !settings.autoPaste })} />
            {settings.autoPaste && saveToFile && <p role="status" className="rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs text-amber-700 dark:text-amber-400">Auto-paste is paused; the stored preference remains on.</p>}
            {autoPasteOn && accessibilityGranted !== null && <div className={`flex items-center gap-2 text-xs ${accessibilityGranted ? 'text-emerald-600 dark:text-emerald-400' : 'text-amber-600 dark:text-amber-400'}`}><span>{accessibilityGranted ? 'Accessibility permission granted' : 'Accessibility permission required'}</span>{accessibilityGranted === false && <button type="button" onClick={requestAccessibility} className="underline">Grant</button>}</div>}
            {autoPasteOn && <PasteDelaySlider value={settings.autoPasteDelayMs} onCommit={(autoPasteDelayMs) => onUpdateSettings({ autoPasteDelayMs })} />}
            <SettingToggle title="Save Transcript to File" description="Write each completed transcription to a .txt file." checked={settings.saveTranscript} onChange={() => onUpdateSettings({ saveTranscript: !settings.saveTranscript })} />
            <SettingToggle title="Save Audio to File" description="Write each recording to a .wav file." checked={settings.saveAudio} onChange={() => onUpdateSettings({ saveAudio: !settings.saveAudio })} />
            {saveToFile && (
              <div>
                <p className="mb-1 text-xs text-on-surface-variant">Output Folder</p>
                <p className="break-all rounded-lg border border-outline-variant/30 bg-surface-container-lowest px-3 py-2 text-xs text-on-surface">{settings.outputDir || 'Documents/Murmur (default)'}</p>
                <div className="mt-2 flex gap-3"><button type="button" onClick={() => void chooseOutputFolder()} className="text-xs font-medium text-on-surface-variant underline hover:text-primary">Choose Folder</button>{settings.outputDir && <button type="button" onClick={() => onUpdateSettings({ outputDir: '' })} className="text-xs font-medium text-on-surface-variant underline hover:text-primary">Reset to default</button>}</div>
                <p className="mt-2 text-xs text-on-surface-variant">Clipboard copying stays on; only automatic paste is paused.</p>
              </div>
            )}
            <div className="border-t border-outline-variant/20 pt-4">
              <h2 className="text-sm font-medium text-on-surface">App Overrides</h2>
              <p className="mt-1 mb-3 text-xs text-on-surface-variant">Override delivery and writing behavior for the frontmost macOS app.</p>
              <AppOverridesEditor profiles={settings.appProfiles} onChange={(appProfiles) => onUpdateSettings({ appProfiles })} />
            </div>
          </SettingsSection>

          <SettingsSection pageId="performance" activePage={activeCat} title="Performance" subtitle="Directional local model comparisons">
            <PerformanceLab status={status} />
          </SettingsSection>

          <SettingsSection pageId="general" activePage={activeCat} title="General" subtitle="Startup, support, updates, and app information">
            <SettingToggle title="Launch at Login" description="Start Murmur automatically when you log in." checked={settings.launchAtLogin} onChange={() => onUpdateSettings({ launchAtLogin: !settings.launchAtLogin })} />
            <button type="button" onClick={onRerunSetup} className="w-full rounded-lg border border-outline-variant/30 bg-surface-container-lowest px-3 py-2 text-xs font-medium text-on-surface-variant transition-colors hover:bg-surface-container hover:text-primary">Run Setup Assistant</button>
            <p className="-mt-3 text-xs text-on-surface-variant">Re-check permissions and model setup after a permission is revoked or stops working.</p>
            <button type="button" onClick={onViewLogs} className="w-full rounded-lg border border-outline-variant/30 bg-surface-container-lowest px-3 py-2 text-xs font-medium text-on-surface-variant transition-colors hover:bg-surface-container hover:text-primary">View Logs</button>
            <button type="button" aria-label={confirmReset ? 'Confirm reset statistics' : 'Reset statistics'} onClick={resetStats} className={`w-full rounded-lg border px-3 py-2 text-xs font-medium transition-colors ${confirmReset ? 'border-error/40 bg-error/10 text-error' : 'border-outline-variant/30 bg-surface-container-lowest text-on-surface-variant hover:bg-surface-container hover:text-primary'}`}>{confirmReset ? 'Confirm Reset' : 'Reset Stats'}</button>
            <div>
              <button type="button" onClick={() => void onCheckForUpdate()} disabled={updateStatus.phase === 'checking' || updateStatus.phase === 'downloading'} className="w-full rounded-lg border border-outline-variant/30 bg-surface-container-lowest px-3 py-2 text-xs font-medium text-on-surface-variant transition-colors hover:bg-surface-container hover:text-primary disabled:cursor-not-allowed disabled:opacity-50">{updateStatus.phase === 'checking' ? 'Checking…' : 'Check for Updates'}</button>
              {updateStatus.phase === 'up-to-date' && <p className="mt-1.5 text-xs text-emerald-600 dark:text-emerald-400">You’re up to date.</p>}
              {updateStatus.phase === 'available' && <p className="mt-1.5 text-xs text-primary">v{updateStatus.version} available</p>}
              {updateStatus.phase === 'error' && <p className="mt-1.5 text-xs text-error">Update check failed.</p>}
            </div>
            {version && <p className="text-center text-xs text-on-surface-variant">Murmur v{version}</p>}
          </SettingsSection>
        </div>
      </div>
    </div>
  );
}
