import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { getVersion } from '@tauri-apps/api/app';
import { invoke } from '@tauri-apps/api/core';
import {
  selectedDeviceExists,
} from '../../lib/audioDevices';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import {
  AUTO_STOP_SILENCE_OPTIONS,
  AVAILABLE_MODEL_OPTIONS,
  DEFAULT_SETTINGS,
  DOUBLE_TAP_KEY_OPTIONS,
  IDLE_TIMEOUT_OPTIONS,
  LANGUAGE_OPTIONS,
  RECORDING_MODE_OPTIONS,
  QUERY_CONTEXT_LEVEL_OPTIONS,
  QUERY_KEY_OPTIONS,
  TRANSFORM_KEY_OPTIONS,
  type QueryKey,
  type QueryProviderId,
  type RecordingMode,
  type Settings,
  type TransformKey,
} from '../../lib/settings';
import {
  CUSTOM_QUERY_PRESET,
  launchQueryProviderSignIn,
  listQueryProviderPresets,
  loadQueryEnvironment,
  saveQueryEnvironment,
  testQueryProvider,
  validateQueryCommand,
  type QueryCommandConfig,
  type QueryEnvironmentVariable,
  type QueryProviderPreset,
  type QueryProviderTestResult,
} from '../../lib/queryProviders';
import { useVocabScan } from '../../lib/hooks/useVocabScan';
import { useAudioInputInventory } from '../../lib/hooks/useAudioInputInventory';
import { useModelRuntimeCatalog } from '../../lib/modelRuntime';
import {
  modelDownloadLabel,
  modelDownloadPercent,
  type ModelDownloadProgress,
} from '../../lib/modelDownload';
import {
  downloadTransformModel,
  removeTransformModel,
  resetTransformRuntime,
  setTransformKey,
  startTransformListener,
  stopTransformListener,
  TRANSFORM_MODEL_SIZE_LABEL,
  transformModelStatus,
  type TransformModelStatus,
} from '../../lib/transformSettings';
import type { DictationStatus } from '../../lib/types';
import type { UpdateStatus } from '../../lib/updater';
import { isNotchPillInstalled } from '../../lib/dictation';
import { beginCurrentUiTransition, useUiLatencyDestination } from '../../lib/uiLatency';
import { Select } from '../ui/Select';
import { INTERNAL_BENCHMARK_BUILD } from '../../lib/buildFlavor';
import { playSoundCue, type SoundCue } from '../../lib/soundCues';
import { AppOverridesEditor } from './AppOverridesEditor';
import { AppearanceSettings } from './AppearanceSettings';
import { PerformanceLab } from './PerformanceLab';
import { MicrophoneInputTest } from './MicrophoneInputTest';
import { OverlayCalibrationControl } from './OverlayCalibrationControl';
import { SettingsSection } from './SettingsSection';
import { SettingsEditorsWindow, type SettingsEditorTab } from './SettingsEditorsWindow';
import { useSettingsSurfaceActive } from './SettingsSurfaceContext';
import {
  DiagnosticsWorkspace,
  type DiagnosticsTab,
} from '../log-viewer/DiagnosticsWorkspace';

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
      <span className={`inline-block h-4 w-4 rounded-full shadow transition-transform ${checked ? 'translate-x-6 bg-on-primary' : 'translate-x-1 bg-on-surface-variant'}`} />
    </button>
  );
}

function SettingToggle({ title, description, label = title, checked, onChange, disabled = false, targetId }: {
  title: string;
  description: string;
  label?: string;
  checked: boolean;
  onChange: () => void;
  disabled?: boolean;
  targetId?: string;
}) {
  return (
    <div
      data-setting-target={targetId}
      className="flex min-h-[52px] items-center justify-between gap-6 rounded-lg px-1 transition-shadow [&.settings-target-flash]:ring-2 [&.settings-target-flash]:ring-primary/40"
    >
      <div>
        <p className="text-sm font-medium text-on-surface">{title}</p>
        <p className="mt-0.5 text-xs leading-relaxed text-on-surface-variant">{description}</p>
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
        min={0}
        max={500}
        step={10}
        value={draft}
        onChange={(event) => setDraft(Number(event.target.value))}
        onPointerUp={() => onCommit(draft)}
        className="h-1.5 w-full cursor-pointer appearance-none rounded-full bg-surface-container-highest accent-primary"
      />
      <p className="mt-1 text-xs text-on-surface-variant">Increase this only if paste lands in the wrong window.</p>
    </div>
  );
}

function VadSensitivitySlider({
  value,
  onPreview,
  onCommit,
}: {
  value: number;
  onPreview: (value: number) => void;
  onCommit: (value: number) => void;
}) {
  const [draft, setDraft] = useState(value);
  useEffect(() => setDraft(value), [value]);
  return (
    <div>
      <div className="mb-1 flex items-center justify-between">
        <label className="text-xs text-on-surface-variant">Sensitivity</label>
        <span className="text-xs font-medium text-on-surface">{draft === 0 ? 'Off' : `${draft}%`}</span>
      </div>
      <input
        type="range"
        min={0}
        max={100}
        step={5}
        value={draft}
        onChange={(event) => {
          const next = Number(event.target.value);
          setDraft(next);
          onPreview(next);
        }}
        onPointerUp={(event) => onCommit(Number(event.currentTarget.value))}
        onKeyUp={(event) => onCommit(Number(event.currentTarget.value))}
        className="h-1.5 w-full cursor-pointer appearance-none rounded-full bg-surface-container-highest accent-primary"
      />
      <p className="mt-1 text-xs text-on-surface-variant">Off skips silence filtering for the lowest latency. Otherwise, higher keeps more audio.</p>
    </div>
  );
}

export interface SettingsPageRequest {
  page: string;
  token: number;
  editorTab?: SettingsEditorTab;
  target?: string;
}

interface SettingsPanelProps {
  settings: Settings;
  onUpdateSettings: (updates: Partial<Settings>) => void;
  initialized: boolean;
  status: DictationStatus;
  onResetStats: () => void;
  onRerunSetup: () => void;
  accessibilityGranted: boolean | null;
  onCheckForUpdate: () => Promise<void>;
  updateStatus: UpdateStatus;
  configureError: string | null;
  /** Page to show, from the command palette. The token makes a repeat request
   *  for the page you are already on still register. */
  pageRequest?: SettingsPageRequest | null;
  onLatencyViewChange?: (view: string) => void;
  /** Stable ref avoids re-rendering the warm Settings tree when its surface is hidden. */
  activeRef?: React.RefObject<boolean>;
}

export const SETTINGS_CATEGORIES = [
  { id: 'general', label: 'General', icon: 'general' },
  { id: 'recording', label: 'Recording', icon: 'recording' },
  { id: 'delivery', label: 'Delivery', icon: 'delivery' },
  { id: 'meetings', label: 'Meetings', icon: 'meetings' },
  { id: 'text', label: 'Text & Vocabulary', icon: 'text' },
  { id: 'ai', label: 'AI & Models', icon: 'ai' },
  { id: 'appearance', label: 'Appearance', icon: 'appearance' },
] as const;

export const SETTINGS_TOOLS = [
  { id: 'performance', label: 'Performance Lab', icon: 'performance' },
  { id: 'diagnostics', label: 'Diagnostics', icon: 'diagnostics' },
] as const;

const AI_DETAIL_PAGES = ['ai-query', 'ai-transform', 'ai-transcription'] as const;

/** Coerce a requested page id back to a real page — an unknown id opens the
 *  first page rather than rendering an empty pane. */
export function resolvePage(page: string | undefined): string {
  if (SETTINGS_CATEGORIES.some((category) => category.id === page)) return page as string;
  if (SETTINGS_TOOLS.some((tool) => tool.id === page)) return page as string;
  if (page === 'dictation') return 'recording';
  if (page === 'model' || page === 'transcription') return 'ai-transcription';
  if (page === 'benchmark') return 'performance';
  if (page === 'text-vocabulary') return 'text';
  if (page === 'transform') return 'ai-transform';
  if (page === 'voice-query' || page === 'query') return 'ai-query';
  if (page === 'app') return 'general';
  return SETTINGS_CATEGORIES[0].id;
}

export function settingsLatencyView(page: string | undefined): string {
  return `settings.${resolvePage(page)}`;
}

const SETTINGS_SEARCH_ITEMS = [
  { page: 'recording', target: 'microphone', title: 'Microphone', detail: 'Choose an input and check its live level.', keywords: 'audio input device test level gain' },
  { page: 'recording', target: 'voice-detection', title: 'Voice Detection', detail: 'Adjust silence filtering sensitivity.', keywords: 'vad sensitivity noise silence' },
  { page: 'recording', target: 'recording-trigger', title: 'Recording Trigger', detail: 'Hold, double-tap, or use both.', keywords: 'hotkey shortcut key timing feedback' },
  { page: 'recording', target: 'stop-on-silence', title: 'Stop on Silence', detail: 'Finish hands-free recordings after quiet.', keywords: 'automatic stop vad pause' },
  { page: 'delivery', target: 'auto-paste', title: 'Auto-Paste', detail: 'Paste clipboard results into the active app.', keywords: 'auto paste autopaste delivery clipboard' },
  { page: 'delivery', target: 'file-output', title: 'Save to File', detail: 'Save transcript or audio files locally.', keywords: 'delivery output folder wav txt' },
  { page: 'delivery', target: 'history', title: 'Transcription History', detail: 'Keep completed dictations on this Mac.', keywords: 'save retain local transcripts' },
  { page: 'delivery', target: 'app-overrides', title: 'App Overrides', detail: 'Customize delivery for individual apps.', keywords: 'profile bundle id per app' },
  { page: 'meetings', target: 'meeting-audio', title: 'Meeting Audio', detail: 'Choose whether source audio is retained.', keywords: 'capture wav keep delete' },
  { page: 'meetings', target: 'meeting-retention', title: 'Meeting Retention', detail: 'Set age and session limits.', keywords: 'history days sessions sqlite' },
  { page: 'ai-transcription', target: 'transcription-model', title: 'Speech-to-Text Model', detail: 'Select and manage the local recognition model.', keywords: 'whisper parakeet core ml download speech model' },
  { page: 'ai-transcription', target: 'language', title: 'Transcription Language', detail: 'Choose a fixed language or automatic detection.', keywords: 'multilingual auto detect' },
  { page: 'ai-query', target: 'voice-query-provider', title: 'Voice Query Provider', detail: 'Configure the CLI agent used for spoken questions.', keywords: 'agent command executable codex claude cloud answer hotkey provider' },
  { page: 'ai-query', target: 'voice-query-copy', title: 'Voice Query Clipboard', detail: 'Automatically copy successful answers to the clipboard.', keywords: 'clipboard copy automatic answers response' },
  { page: 'ai-transform', target: 'rewrite-model', title: 'Selected-Text Rewrite', detail: 'Configure on-device rewriting.', keywords: 'transform llm qwen rewrite shortcut model' },
  { page: 'text', target: 'punctuation', title: 'Smart Punctuation', detail: 'Add punctuation and sentence capitalization.', keywords: 'automatic punctuation' },
  { page: 'text', target: 'cleanup', title: 'Transcript Cleanup', detail: 'Remove filler words and tidy transcript spacing.', keywords: 'filler capitalization' },
  { page: 'text', target: 'text-editors', title: 'Vocabulary & Aliases', detail: 'Manage preferred words and spoken variants.', keywords: 'names spelling project scan developer terms knowledge voice commands replacement' },
  { page: 'appearance', target: 'appearance', title: 'Appearance', detail: 'Theme, accent, contrast, and color controls.', keywords: 'dark light colors palette community open vsx vscode import' },
  { page: 'general', target: 'launch-login', title: 'Launch at Login', detail: 'Start Murmur when you sign in.', keywords: 'startup autostart' },
  { page: 'general', target: 'setup', title: 'Setup Assistant', detail: 'Re-check permissions and model setup.', keywords: 'onboarding microphone accessibility' },
  { page: 'general', target: 'updates', title: 'Updates', detail: 'Check for a newer Murmur release.', keywords: 'version upgrade' },
  { page: 'performance', target: 'performance', title: 'Performance Lab', detail: 'Compare installed models on this Mac.', keywords: 'benchmark speed accuracy' },
  { page: 'diagnostics', target: 'diagnostics', title: 'Diagnostics', detail: 'Inspect events, runs, reports, and transforms.', keywords: 'logs performance compare debugger' },
] as const;

function SettingsNavIcon({ icon }: { icon: string }) {
  const paths: Record<string, React.ReactNode> = {
    general: <><circle cx="12" cy="12" r="3" /><path d="M19 12a7 7 0 1 1-14 0 7 7 0 0 1 14 0Z" /></>,
    recording: <><rect x="9" y="3" width="6" height="11" rx="3" /><path d="M6 11a6 6 0 0 0 12 0M12 17v4M9 21h6" /></>,
    delivery: <><rect x="5" y="4" width="14" height="16" rx="2" /><path d="m9 12 2 2 4-5" /></>,
    meetings: <><rect x="3" y="5" width="18" height="16" rx="2" /><path d="M8 3v4M16 3v4M3 10h18" /></>,
    text: <><path d="M5 5h14M8 5v14M5 19h6M15 10h4M15 14h4" /></>,
    ai: <><circle cx="12" cy="12" r="3" /><path d="M12 2v4M12 18v4M2 12h4M18 12h4M5 5l3 3M16 16l3 3M19 5l-3 3M8 16l-3 3" /></>,
    appearance: <><path d="M12 3a9 9 0 1 0 9 9c0-1.1-.9-2-2-2h-1.5A2.5 2.5 0 0 1 15 7.5V5c0-1.1-.9-2-2-2Z" /><circle cx="8" cy="12" r="1" /><circle cx="10" cy="7" r="1" /></>,
    performance: <><path d="M4 18V9M10 18V5M16 18v-7M22 18V3" /></>,
    diagnostics: <><path d="M4 4h16v16H4zM8 9h8M8 13h5M8 17h3" /></>,
  };
  return (
    <svg className="h-4 w-4 shrink-0" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.7} aria-hidden="true">
      {paths[icon]}
    </svg>
  );
}

export function effectiveAutoPaste(settings: Pick<Settings, 'autoPaste' | 'saveTranscript' | 'saveAudio'>): boolean {
  return settings.autoPaste && !settings.saveTranscript && !settings.saveAudio;
}

export function autoPasteDeliveryDescription(settings: Pick<Settings, 'autoPaste' | 'saveTranscript' | 'saveAudio'>): string {
  if (!settings.saveTranscript && !settings.saveAudio) {
    return 'Paste the clipboard result into the active app (Accessibility permission required).';
  }
  return settings.autoPaste
    ? 'Paused while file output is on. Your saved preference will resume when file output is off.'
    : 'Unavailable while file output is on. Turn off file output to enable auto-paste.';
}

export function fileOutputDeliveryDescription(settings: Pick<Settings, 'autoPaste'>): string {
  return settings.autoPaste
    ? 'Clipboard copying stays on; only automatic paste is paused.'
    : 'Clipboard copying stays on; auto-paste remains off.';
}

function queryConfigurationMessage(error: unknown): string {
  const code = String(error);
  if (code.includes('invalid_executable')) return 'The CLI executable is missing, is not executable, or is not an absolute path.';
  if (code.includes('invalid_arguments')) return 'Fixed arguments exceed the Voice Query safety limits.';
  if (code.includes('invalid_timeout')) return 'Choose a timeout between 5 seconds and 5 minutes.';
  if (code.includes('invalid_environment')) return 'Declared environment values must be absolute config-directory paths.';
  if (code.includes('environment_unavailable')) return 'Murmur could not read the protected Voice Query environment file.';
  return 'Murmur could not validate this Voice Query configuration.';
}

function queryProviderTestMessage(
  provider: QueryProviderId,
  result: QueryProviderTestResult,
): string {
  if (isIncompleteCodexProbe(provider, result)) {
    return 'The Codex CLI installation is incomplete. Reinstall or update Codex, then choose Test again.';
  }
  if (result.authenticated === null) {
    return 'Executable validated. Custom providers do not have a built-in authentication probe.';
  }
  if (result.ok) return 'Authenticated and ready.';
  if (result.errorCode === 'provider_not_authenticated') {
    return result.signInFix ?? 'The provider is not authenticated.';
  }
  return 'The provider probe failed. Review its output below.';
}

function isIncompleteCodexProbe(
  provider: QueryProviderId,
  result: QueryProviderTestResult,
): boolean {
  if (provider !== 'codex' || result.errorCode !== 'probe_failed') return false;
  const detail = `${result.stdout}\n${result.stderr}`;
  return detail.includes('The Codex CLI installation is incomplete')
    || (
      /ENOENT/i.test(detail)
      && /@openai\/codex-darwin-/i.test(detail)
      && /\/vendor\//i.test(detail)
      && /\/codex\/codex/i.test(detail)
    );
}

function queryCommand(settings: Settings): QueryCommandConfig {
  return {
    provider: settings.queryProvider,
    executable: settings.queryExecutable,
    arguments: settings.queryArguments,
    timeoutSeconds: settings.queryTimeoutSeconds,
    contextLevel: settings.queryContextLevel,
    retainQueryHistory: settings.retainQueryHistory,
  };
}

const QUERY_SWITCH_INTERRUPTED_NOTICE =
  'Configuration changed during provider preflight. Voice Query remains off.';

function queryCommandFingerprintFor(
  command: QueryCommandConfig,
  transformHoldKey: TransformKey | null,
): string {
  return JSON.stringify([
    command.provider,
    command.executable,
    command.arguments,
    command.timeoutSeconds,
    command.contextLevel,
    transformHoldKey,
  ]);
}

export const SettingsPanel = memo(function SettingsPanel({
  settings,
  onUpdateSettings,
  initialized,
  status,
  onResetStats,
  onRerunSetup,
  accessibilityGranted,
  onCheckForUpdate,
  updateStatus,
  configureError,
  pageRequest = null,
  onLatencyViewChange,
  activeRef,
}: SettingsPanelProps) {
  const { byName: runtimeByName } = useModelRuntimeCatalog();
  const [activeCat, setActiveCat] = useState<string>(() => resolvePage(pageRequest?.page));
  const [diagnosticsWindowError, setDiagnosticsWindowError] = useState<string | null>(null);
  const [searchQuery, setSearchQuery] = useState('');
  const [editorTab, setEditorTab] = useState<SettingsEditorTab | null>(null);
  const [targetRequest, setTargetRequest] = useState<string | null>(null);
  const latencyView = editorTab
    ? `settings.text.editor.${editorTab}`
    : `settings.${activeCat}`;
  useUiLatencyDestination(activeRef?.current === false ? null : latencyView);
  useLayoutEffect(() => {
    onLatencyViewChange?.(latencyView);
  }, [latencyView, onLatencyViewChange]);
  const searchResults = useMemo(() => {
    const normalize = (value: string) => value.toLowerCase().replace(/[^a-z0-9]+/g, ' ').trim();
    const query = normalize(searchQuery);
    if (!query) return [];
    return SETTINGS_SEARCH_ITEMS.filter((item) =>
      normalize(`${item.title} ${item.detail} ${item.keywords}`).includes(query));
  }, [searchQuery]);
  const requestTokenRef = useRef(pageRequest?.token);
  useEffect(() => {
    if (!pageRequest || pageRequest.token === requestTokenRef.current) return;
    requestTokenRef.current = pageRequest.token;
    setActiveCat(resolvePage(pageRequest.page));
    setEditorTab(pageRequest.editorTab ?? null);
    setSearchQuery('');
    setTargetRequest(pageRequest.target ?? null);
  }, [pageRequest]);
  const [version, setVersion] = useState('');
  const [confirmReset, setConfirmReset] = useState(false);
  const contentRef = useRef<HTMLDivElement>(null);
  const confirmResetTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => { void getVersion().then(setVersion); }, []);
  useLayoutEffect(() => {
    const content = contentRef.current;
    if (!content) return;
    if (!targetRequest) {
      content.scrollTo({ top: 0 });
      return;
    }
    const target = content.querySelector<HTMLElement>(`[data-setting-target="${targetRequest}"]`);
    if (!target) return;
    target.scrollIntoView({ block: 'center' });
    target.classList.add('settings-target-flash');
    const timeout = window.setTimeout(() => target.classList.remove('settings-target-flash'), 1800);
    setTargetRequest(null);
    return () => window.clearTimeout(timeout);
  }, [activeCat, editorTab, targetRequest]);
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
  const openEditor = useCallback((tab: SettingsEditorTab) => {
    beginCurrentUiTransition(`settings.text.editor.${tab}`, 'pointer');
    setActiveCat('text');
    setSearchQuery('');
    setEditorTab(tab);
  }, []);
  const closeEditor = useCallback(() => {
    beginCurrentUiTransition('settings.text', 'programmatic');
    setEditorTab(null);
  }, []);
  const popOutDiagnostics = useCallback(async (tab: DiagnosticsTab) => {
    setDiagnosticsWindowError(null);
    try {
      await invoke('show_diagnostics_window', { tab });
    } catch {
      setDiagnosticsWindowError('Diagnostics could not be opened in a separate window.');
    }
  }, []);

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

  const settingsSurfaceActive = useSettingsSurfaceActive();
  const audioInventoryState = useAudioInputInventory(settingsSurfaceActive);
  const audioInventory = audioInventoryState.inventory;
  const audioDevices = audioInventory?.status === 'available' ? audioInventory.devices : [];
  const [previewVadSensitivity, setPreviewVadSensitivity] = useState(settings.vadSensitivity);
  useEffect(() => setPreviewVadSensitivity(settings.vadSensitivity), [settings.vadSensitivity]);

  const [notchPillInstalled, setNotchPillInstalled] = useState(false);
  useEffect(() => {
    let cancelled = false;
    const refresh = () => {
      isNotchPillInstalled()
        .then((installed) => {
          if (!cancelled) setNotchPillInstalled(installed);
        })
        .catch(() => {
          if (!cancelled) setNotchPillInstalled(false);
        });
    };
    refresh();
    return () => { cancelled = true; };
  }, []);
  useEffect(() => {
    let cancelled = false;
    const refresh = () => {
      if (activeRef && !activeRef.current) return;
      isNotchPillInstalled()
        .then((installed) => {
          if (!cancelled) setNotchPillInstalled(installed);
        })
        .catch(() => {
          if (!cancelled) setNotchPillInstalled(false);
        });
    };
    window.addEventListener('focus', refresh);
    return () => {
      cancelled = true;
      window.removeEventListener('focus', refresh);
    };
  }, [activeRef]);

  // ---- Transform model block (#312 D1) ------------------------------------
  const [transformModel, setTransformModel] = useState<TransformModelStatus | null>(null);
  const [transformModelBusy, setTransformModelBusy] = useState(false);
  const [transformModelError, setTransformModelError] = useState<string | null>(null);
  // Shortcut-picker failures get their own error line, separate from the model
  // block's error slot (#312 D1 round-2 finding 8).
  const [transformKeyError, setTransformKeyError] = useState<string | null>(null);
  const [transformDownloadPct, setTransformDownloadPct] = useState<number | null>(null);
  const [queryConfigError, setQueryConfigError] = useState<string | null>(null);
  const [queryConfigNotice, setQueryConfigNotice] = useState<string | null>(null);
  const [queryPresets, setQueryPresets] = useState<QueryProviderPreset[]>([CUSTOM_QUERY_PRESET]);
  const [queryEnvironment, setQueryEnvironment] = useState<QueryEnvironmentVariable[]>([]);
  const [configuredQueryEnvironment, setConfiguredQueryEnvironment] = useState<string[]>([]);
  const [queryEnvironmentStatus, setQueryEnvironmentStatus] = useState<string | null>(null);
  const [queryEnvironmentNeedsRepair, setQueryEnvironmentNeedsRepair] = useState(false);
  const [queryConfigBusy, setQueryConfigBusy] = useState(false);
  const [queryTestResult, setQueryTestResult] = useState<QueryProviderTestResult | null>(null);
  const [queryTestBusy, setQueryTestBusy] = useState(false);
  const [querySignInStatus, setQuerySignInStatus] = useState<string | null>(null);
  const signInPollRef = useRef(0);
  const queryConfigGenerationRef = useRef(0);
  const queryProviderSwitchRef = useRef<{ generation: number; hotkey: QueryKey } | null>(null);
  const queryCommandFingerprint = queryCommandFingerprintFor(
    queryCommand(settings),
    settings.transformHoldKey,
  );
  const queryCommandFingerprintRef = useRef(queryCommandFingerprint);

  const invalidateQueryRequests = () => {
    const interruptedProviderSwitch = queryProviderSwitchRef.current !== null;
    queryConfigGenerationRef.current += 1;
    signInPollRef.current += 1;
    queryProviderSwitchRef.current = null;
    setQueryConfigBusy(false);
    setQueryTestBusy(false);
    setQueryTestResult(null);
    setQuerySignInStatus(null);
    if (interruptedProviderSwitch) {
      setQueryConfigNotice(QUERY_SWITCH_INTERRUPTED_NOTICE);
    }
    return queryConfigGenerationRef.current;
  };

  const queryRequestIsCurrent = (generation: number) => (
    queryConfigGenerationRef.current === generation
  );

  useEffect(() => () => {
    signInPollRef.current += 1;
    queryConfigGenerationRef.current += 1;
    queryProviderSwitchRef.current = null;
  }, []);

  useLayoutEffect(() => {
    if (queryCommandFingerprintRef.current === queryCommandFingerprint) return;
    const interruptedProviderSwitch = queryProviderSwitchRef.current !== null;
    queryCommandFingerprintRef.current = queryCommandFingerprint;
    queryConfigGenerationRef.current += 1;
    signInPollRef.current += 1;
    queryProviderSwitchRef.current = null;
    setQueryConfigBusy(false);
    setQueryTestBusy(false);
    setQueryTestResult(null);
    setQuerySignInStatus(null);
    if (interruptedProviderSwitch) {
      setQueryConfigError(null);
      setQueryConfigNotice(QUERY_SWITCH_INTERRUPTED_NOTICE);
    }
  }, [queryCommandFingerprint]);

  useEffect(() => {
    if (activeCat !== 'ai-query') return;
    let cancelled = false;
    void listQueryProviderPresets()
      .then((presets) => {
        if (!cancelled) setQueryPresets(presets);
      })
      .catch(() => {
        if (!cancelled) setQueryPresets([CUSTOM_QUERY_PRESET]);
      });
    return () => { cancelled = true; };
  }, [activeCat]);

  useEffect(() => {
    if (activeCat !== 'ai-query') return;
    let cancelled = false;
    setQueryEnvironmentStatus(null);
    setQueryEnvironmentNeedsRepair(false);
    void loadQueryEnvironment(settings.queryProvider)
      .then((names) => {
        if (!cancelled) {
          setConfiguredQueryEnvironment(Array.isArray(names) ? names : []);
          setQueryEnvironment([]);
          setQueryEnvironmentNeedsRepair(false);
        }
      })
      .catch(() => {
        if (!cancelled) {
          setQueryEnvironment([]);
          setConfiguredQueryEnvironment([]);
          setQueryEnvironmentNeedsRepair(true);
          setQueryEnvironmentStatus('Could not load the protected environment file. Clear saved values to repair it.');
        }
      });
    return () => { cancelled = true; };
  }, [activeCat, settings.queryProvider]);

  const refreshTransformModel = useCallback(async () => {
    try {
      setTransformModel(await transformModelStatus());
    } catch {
      setTransformModel(null);
    }
  }, []);

  useEffect(() => {
    if (activeCat !== 'ai' && activeCat !== 'ai-transform') return;
    void refreshTransformModel();
  }, [activeCat, refreshTransformModel]);

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    listen<{ received?: number; total?: number }>(
      'transform-model-download-progress',
      (event) => {
        const { received = 0, total = 0 } = event.payload;
        if (total > 0) setTransformDownloadPct(Math.min(100, Math.round((received / total) * 100)));
        else setTransformDownloadPct(null);
      },
    )
      .then((fn) => {
        unlisten = fn;
      })
      .catch(() => {});
    return () => {
      unlisten?.();
    };
  }, []);

  const updateTransformHoldKey = async (next: TransformKey | null) => {
    setTransformKeyError(null);
    if (next !== null && next === settings.queryHotkey) {
      setTransformKeyError('That key is already assigned to Voice Query.');
      return;
    }
    try {
      if (next === null) {
        await stopTransformListener();
        onUpdateSettings({ transformHoldKey: null });
        return;
      }
      await setTransformKey(next);
      await startTransformListener(next);
      onUpdateSettings({ transformHoldKey: next });
    } catch (e) {
      setTransformKeyError(String(e));
    }
  };

  const downloadTransform = async () => {
    setTransformModelBusy(true);
    setTransformModelError(null);
    setTransformDownloadPct(0);
    try {
      await downloadTransformModel();
      await refreshTransformModel();
    } catch (e) {
      setTransformModelError(String(e));
    } finally {
      setTransformModelBusy(false);
      setTransformDownloadPct(null);
    }
  };

  const removeTransform = async () => {
    if (!window.confirm('Remove the on-device transform model (~1.1 GB)? You can re-download it later.')) {
      return;
    }
    setTransformModelBusy(true);
    setTransformModelError(null);
    try {
      await removeTransformModel();
      await refreshTransformModel();
    } catch (e) {
      setTransformModelError(String(e));
    } finally {
      setTransformModelBusy(false);
    }
  };

  const resetTransform = async () => {
    setTransformModelBusy(true);
    setTransformModelError(null);
    try {
      await resetTransformRuntime();
      await refreshTransformModel();
    } catch (e) {
      setTransformModelError(String(e));
    } finally {
      setTransformModelBusy(false);
    }
  };

  const isRecording = status !== 'idle';
  const isDoubleTap = settings.recordingMode === 'double_tap';
  const isBoth = settings.recordingMode === 'both';
  const keyLabel = isBoth ? 'Trigger Key' : isDoubleTap ? 'Double-Tap Key' : 'Hold Key';
  const keyHelp = isBoth
    ? 'Hold to record, or double-tap to start and single-tap to stop.'
    : isDoubleTap ? 'Double-tap to start and single-tap to stop.' : 'Hold to start and release to stop.';

  const toggleVoiceQuery = async () => {
    setQueryConfigError(null);
    if (settings.queryHotkey !== null) {
      invalidateQueryRequests();
      setQueryConfigNotice(null);
      onUpdateSettings({ queryHotkey: null });
      return;
    }
    if (!settings.queryExecutable.trim()) {
      setQueryConfigError('Choose the absolute path to a CLI executable before enabling Voice Query.');
      return;
    }
    const key = QUERY_KEY_OPTIONS.find((option) => option.value !== settings.transformHoldKey)?.value;
    if (!key) {
      setQueryConfigError('No dedicated shortcut is available.');
      return;
    }
    const command = queryCommand(settings);
    const generation = invalidateQueryRequests();
    setQueryConfigBusy(true);
    try {
      await validateQueryCommand(command);
      if (queryRequestIsCurrent(generation)) {
        setQueryConfigNotice(null);
        onUpdateSettings({ queryHotkey: key });
      }
    } catch (error) {
      if (queryRequestIsCurrent(generation)) {
        setQueryConfigError(queryConfigurationMessage(error));
      }
    } finally {
      if (queryRequestIsCurrent(generation)) {
        setQueryConfigBusy(false);
      }
    }
  };

  const selectQueryProvider = async (provider: QueryProviderId) => {
    const selected = queryPresets.find((preset) => preset.id === provider)
      ?? (provider === 'custom' ? CUSTOM_QUERY_PRESET : null);
    if (!selected) return;
    // A rapid second selection sees the hotkey carried by the still-current
    // switch transaction even though Settings has already persisted the
    // fail-closed temporary `null`. No completed or failed transaction keeps
    // this ref alive.
    const hotkeyToPreserve = settings.queryHotkey
      ?? queryProviderSwitchRef.current?.hotkey
      ?? null;
    const command: QueryCommandConfig = {
      provider,
      executable: selected.discoveredExecutable ?? '',
      arguments: [...selected.recommendedArguments],
      timeoutSeconds: settings.queryTimeoutSeconds,
      contextLevel: settings.queryContextLevel,
      retainQueryHistory: settings.retainQueryHistory,
    };
    const generation = invalidateQueryRequests();
    if (hotkeyToPreserve !== null) {
      queryProviderSwitchRef.current = { generation, hotkey: hotkeyToPreserve };
    }
    // This is the exact command being committed below. Priming the ref keeps
    // the controlled-settings layout effect from invalidating its own switch;
    // any different edit still changes the fingerprint and wins the race.
    queryCommandFingerprintRef.current = queryCommandFingerprintFor(
      command,
      settings.transformHoldKey,
    );
    setQueryConfigError(null);
    setQueryConfigNotice(hotkeyToPreserve !== null
      ? `Checking ${selected.label} before keeping Voice Query enabled…`
      : null);
    setQueryTestResult(null);
    setQuerySignInStatus(null);
    setQueryEnvironmentStatus(null);
    setQueryEnvironmentNeedsRepair(false);
    setQueryEnvironment([]);
    setConfiguredQueryEnvironment([]);
    onUpdateSettings({
      queryProvider: provider,
      queryExecutable: command.executable,
      queryArguments: command.arguments,
      queryHotkey: null,
    });
    if (hotkeyToPreserve === null) return;

    setQueryConfigBusy(true);
    setQueryTestBusy(true);
    try {
      await validateQueryCommand(command);
      if (!queryRequestIsCurrent(generation)) return;
      const result = await testQueryProvider(command);
      if (!queryRequestIsCurrent(generation)) return;
      setQueryTestResult(result);
      if (!result.ok) {
        queryProviderSwitchRef.current = null;
        setQueryConfigNotice(null);
        setQueryConfigError(
          `Voice Query remains off. ${queryProviderTestMessage(provider, result)}`,
        );
        return;
      }
      const pending = queryProviderSwitchRef.current;
      if (!pending || pending.generation !== generation || pending.hotkey !== hotkeyToPreserve) {
        return;
      }
      queryProviderSwitchRef.current = null;
      setQueryConfigNotice(
        `Provider changed to ${selected.label}. Voice Query stayed enabled after validation and preflight.`,
      );
      onUpdateSettings({ queryHotkey: hotkeyToPreserve });
    } catch (error) {
      if (queryRequestIsCurrent(generation)) {
        queryProviderSwitchRef.current = null;
        setQueryConfigNotice(null);
        setQueryConfigError(`Voice Query remains off. ${queryConfigurationMessage(error)}`);
      }
    } finally {
      if (queryRequestIsCurrent(generation)) {
        setQueryConfigBusy(false);
        setQueryTestBusy(false);
      }
    }
  };

  const saveDeclaredEnvironment = async () => {
    setQueryEnvironmentStatus(null);
    const entered = queryEnvironment.filter((variable) => variable.value.length > 0);
    if (entered.length === 0) {
      setQueryEnvironmentStatus('Enter an absolute config-directory path to save.');
      return;
    }
    const provider = settings.queryProvider;
    const generation = invalidateQueryRequests();
    try {
      await saveQueryEnvironment(provider, entered);
      if (!queryRequestIsCurrent(generation)) return;
      setConfiguredQueryEnvironment((current) => [
        ...new Set([...current, ...entered.map((variable) => variable.name)]),
      ]);
      setQueryEnvironment([]);
      setQueryEnvironmentStatus('Saved in Murmur’s protected app-data directory.');
      setQueryEnvironmentNeedsRepair(false);
      setQueryTestResult(null);
      setQuerySignInStatus(null);
    } catch (error) {
      if (queryRequestIsCurrent(generation)) {
        setQueryEnvironmentStatus(queryConfigurationMessage(error));
      }
    }
  };

  const clearDeclaredEnvironment = async () => {
    setQueryEnvironmentStatus(null);
    const provider = settings.queryProvider;
    const generation = invalidateQueryRequests();
    try {
      await saveQueryEnvironment(provider, []);
      if (!queryRequestIsCurrent(generation)) return;
      setQueryEnvironment([]);
      setConfiguredQueryEnvironment([]);
      setQueryEnvironmentStatus('Saved config-directory values cleared.');
      setQueryEnvironmentNeedsRepair(false);
      setQueryTestResult(null);
      setQuerySignInStatus(null);
    } catch (error) {
      if (queryRequestIsCurrent(generation)) {
        setQueryEnvironmentStatus(queryConfigurationMessage(error));
      }
    }
  };

  const runQueryTest = async (): Promise<QueryProviderTestResult | null> => {
    setQueryConfigError(null);
    setQuerySignInStatus(null);
    const command = queryCommand(settings);
    const generation = invalidateQueryRequests();
    setQueryTestBusy(true);
    try {
      const result = await testQueryProvider(command);
      if (!queryRequestIsCurrent(generation)) return null;
      setQueryTestResult(result);
      return result;
    } catch (error) {
      if (queryRequestIsCurrent(generation)) {
        setQueryConfigError(queryConfigurationMessage(error));
        setQueryTestResult(null);
      }
      return null;
    } finally {
      if (queryRequestIsCurrent(generation)) {
        setQueryTestBusy(false);
      }
    }
  };

  const signInQueryProvider = async () => {
    const poll = signInPollRef.current + 1;
    signInPollRef.current = poll;
    const command = queryCommand(settings);
    const generation = queryConfigGenerationRef.current;
    const ownsRequest = () => (
      signInPollRef.current === poll && queryRequestIsCurrent(generation)
    );
    setQueryConfigError(null);
    setQuerySignInStatus('Opening Terminal…');
    try {
      await launchQueryProviderSignIn(command);
      if (!ownsRequest()) return;
      setQuerySignInStatus('Terminal opened. Waiting for sign-in…');
      const deadline = Date.now() + 60_000;
      while (ownsRequest() && Date.now() < deadline) {
        await new Promise((resolve) => window.setTimeout(resolve, 2000));
        if (!ownsRequest()) return;
        const result = await testQueryProvider(command);
        if (!ownsRequest()) return;
        setQueryTestResult(result);
        if (result.ok) {
          setQuerySignInStatus('Signed in and ready.');
          return;
        }
      }
      if (ownsRequest()) {
        setQuerySignInStatus('Sign-in is still pending. Finish in Terminal, then choose Test.');
      }
    } catch (error) {
      if (ownsRequest()) {
        setQuerySignInStatus(null);
        setQueryConfigError(String(error).includes('sign_in')
          ? 'Murmur could not open the provider sign-in in Terminal.'
          : queryConfigurationMessage(error));
      }
    }
  };

  const chooseQueryExecutable = async () => {
    const generation = queryConfigGenerationRef.current;
    try {
      const selected = await open({ directory: false, multiple: false });
      if (typeof selected === 'string' && queryRequestIsCurrent(generation)) {
        setQueryConfigError(null);
        setQueryConfigNotice(settings.queryHotkey !== null
          ? 'Command changed. Voice Query was turned off so the new command can be tested before use.'
          : null);
        setQueryTestResult(null);
        setQuerySignInStatus(null);
        invalidateQueryRequests();
        onUpdateSettings({ queryExecutable: selected, queryHotkey: null });
      }
    } catch {
      // Cancellation leaves the configured executable untouched.
    }
  };
  const missingDevice = settings.microphone !== DEFAULT_SETTINGS.microphone
    && audioInventory?.status === 'available'
    && !selectedDeviceExists(settings.microphone, audioDevices);
  const englishOnly = selectedRuntime ? !selectedRuntime.capabilities.multilingual : true;
  const downloadProgress = modelDownload.phase === 'downloading'
    ? modelDownloadPercent(modelDownload.progress)
    : null;
  const saveToFile = settings.saveTranscript || settings.saveAudio;
  const autoPasteOn = effectiveAutoPaste(settings);
  const selectedQueryPreset = queryPresets.find((preset) => preset.id === settings.queryProvider)
    ?? (settings.queryProvider === 'custom' ? CUSTOM_QUERY_PRESET : null);
  const queryProviderItems = queryPresets.map((preset) => ({
    value: preset.id,
    label: preset.discoveredExecutable || preset.id === 'custom'
      ? preset.label
      : `${preset.label} — not found`,
  }));

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

  const openPage = (page: string, trigger: 'pointer' | 'programmatic' = 'pointer') => {
    beginCurrentUiTransition(`settings.${page}`, trigger);
    setActiveCat(page);
    setEditorTab(null);
    setSearchQuery('');
    setTargetRequest(null);
  };

  const navPageIsActive = (page: string) => (
    page === 'ai' ? activeCat === 'ai' || activeCat.startsWith('ai-') : activeCat === page
  );

  const searchPageLabel = (page: string) => {
    if (page.startsWith('ai-')) return 'AI & Models';
    return SETTINGS_CATEGORIES.find((category) => category.id === page)?.label
      ?? SETTINGS_TOOLS.find((tool) => tool.id === page)?.label
      ?? page;
  };
  return (
    <div className="flex min-h-0 flex-1 overflow-hidden bg-background text-on-surface">
      <aside className="flex min-h-0 w-[210px] shrink-0 flex-col overflow-hidden bg-surface-container-low px-3 pb-3 pt-2 max-[760px]:w-[184px]">
        <label className="relative mb-3 block w-full min-w-0 shrink-0">
          <span className="sr-only">Search all settings</span>
          <svg className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-on-surface-variant" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M21 21l-4.35-4.35M17 11a6 6 0 11-12 0 6 6 0 0112 0z" />
          </svg>
          <input
            type="search"
            value={searchQuery}
            onChange={(event) => {
              setEditorTab(null);
              setSearchQuery(event.target.value);
            }}
            placeholder="Search Settings"
            className="h-9 w-full rounded-lg border border-outline-variant bg-surface-container-lowest pl-9 pr-8 text-[13px] text-on-surface outline-none placeholder:text-on-surface-variant focus:border-primary"
          />
          {searchQuery && (
            <button type="button" onClick={() => setSearchQuery('')} aria-label="Clear settings search" className="absolute right-2 top-1/2 grid h-6 w-6 -translate-y-1/2 place-items-center rounded text-on-surface-variant hover:bg-surface-container">×</button>
          )}
        </label>
        <nav aria-label="Settings pages" className="min-h-0 space-y-0.5 overflow-y-auto">
          {SETTINGS_CATEGORIES.map((category) => {
            const selected = navPageIsActive(category.id);
            return (
              <button
                key={category.id}
                type="button"
                aria-current={selected ? 'page' : undefined}
                onClick={() => openPage(category.id)}
                className={`flex min-h-9 w-full items-center gap-2.5 rounded-lg px-3 text-left text-[13px] font-medium transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-primary ${selected ? 'bg-surface-container-high text-on-surface' : 'text-on-surface-variant hover:bg-surface-container hover:text-on-surface'}`}
              >
                <SettingsNavIcon icon={category.icon} />
                <span className="min-w-0 truncate">{category.label}</span>
              </button>
            );
          })}
        </nav>
        <div className="mt-auto border-t border-outline-variant/20 pt-3">
          <p className="mb-1 px-3 text-[10px] font-bold uppercase tracking-[0.14em] text-on-surface-variant">Tools</p>
          {SETTINGS_TOOLS.map((tool) => {
            const selected = activeCat === tool.id;
            return (
              <button
                key={tool.id}
                type="button"
                aria-current={selected ? 'page' : undefined}
                onClick={() => openPage(tool.id)}
                className={`flex min-h-9 w-full items-center gap-2.5 rounded-lg px-3 text-left text-[13px] font-medium transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-primary ${selected ? 'bg-surface-container-high text-on-surface' : 'text-on-surface-variant hover:bg-surface-container hover:text-on-surface'}`}
              >
                <SettingsNavIcon icon={tool.icon} />
                <span className="truncate">{tool.label}</span>
              </button>
            );
          })}
          <p className="mt-3 flex items-center gap-2 px-3 text-[11px] text-on-surface-variant"><span className="h-1.5 w-1.5 rounded-full bg-success" />Processing locally</p>
        </div>
      </aside>

      <main className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
        <div
          ref={contentRef}
          data-testid="settings-content"
          className="min-h-0 min-w-0 flex-1 overflow-y-auto"
        >
          <div className="mx-auto w-full max-w-3xl px-7 pb-10 pt-5 max-[760px]:px-5">
          {configureError && <p role="alert" className="mb-4 rounded-lg bg-error/10 px-3 py-2 text-xs text-error">{configureError}</p>}
          {editorTab ? (
            <SettingsEditorsWindow
              initialTab={editorTab}
              settings={settings}
              onUpdateSettings={onUpdateSettings}
              scanStatus={vocabScan.status}
              scanWalker={vocabScan.walker}
              scanStats={vocabScan.stats}
              onChooseCodeFolder={() => void chooseCodeFolder()}
              onClearCodeFolder={clearCodeFolder}
              onScan={() => void runVocabScan(settings.codeVocabFolder)}
              onCancelScan={vocabScan.cancel}
              onBack={closeEditor}
            />
          ) : searchQuery ? (
            <section aria-label="Settings search results">
              <h1 className="text-2xl font-semibold tracking-tight text-on-surface">Search</h1>
              <p className="mb-4 mt-1 text-sm text-on-surface-variant">Jump directly to a setting or tool.</p>
              <p className="mb-3 text-[11px] font-bold uppercase tracking-[0.12em] text-on-surface-variant">
                {searchResults.length} {searchResults.length === 1 ? 'result' : 'results'}
              </p>
              {searchResults.length === 0 ? (
                <div className="rounded-xl border border-dashed border-outline-variant/30 px-4 py-10 text-center text-sm text-on-surface-variant">
                  No settings match “{searchQuery}”.
                </div>
              ) : (
                <div className="overflow-hidden rounded-xl border border-outline-variant/25 bg-surface-container-lowest">
                  {searchResults.map((result) => (
                    <button
                      key={`${result.page}-${result.title}`}
                      type="button"
                      onClick={() => {
                        beginCurrentUiTransition(`settings.${result.page}`, 'pointer');
                        setActiveCat(result.page);
                        setEditorTab(null);
                        setTargetRequest(result.target);
                        setSearchQuery('');
                      }}
                      className="flex w-full items-center gap-4 border-b border-outline-variant/15 px-4 py-3 text-left last:border-b-0 hover:bg-surface-container-low"
                    >
                      <span className="min-w-0 flex-1">
                        <span className="block text-sm font-semibold text-on-surface">{result.title}</span>
                        <span className="mt-0.5 block text-xs text-on-surface-variant">{result.detail}</span>
                      </span>
                      <span className="rounded-full bg-surface-container-high px-2.5 py-1 text-[10px] font-bold uppercase tracking-wide text-on-surface-variant">
                        {searchPageLabel(result.page)}
                      </span>
                      <span aria-hidden="true" className="text-on-surface-variant">›</span>
                    </button>
                  ))}
                </div>
              )}
            </section>
          ) : (
          <div className="settings-page">
          {(AI_DETAIL_PAGES as readonly string[]).includes(activeCat) && (
            <button
              type="button"
              onClick={() => openPage('ai', 'programmatic')}
              className="mb-4 inline-flex h-8 items-center gap-1.5 rounded-lg border border-outline-variant/30 px-3 text-xs font-semibold text-on-surface-variant hover:bg-surface-container hover:text-on-surface"
            >
              <span aria-hidden="true">‹</span> AI &amp; Models
            </button>
          )}
          <SettingsSection pageId="recording" activePage={activeCat} title="Recording" subtitle="Microphone, voice detection, shortcuts, and automatic stopping">
            <div data-setting-target="microphone" className="rounded-lg transition-shadow [&.settings-target-flash]:ring-2 [&.settings-target-flash]:ring-primary/40">
              <MicrophoneInputTest
                microphone={settings.microphone}
                devices={audioDevices}
                active={activeCat === 'recording'}
                ready={initialized}
                vadSensitivity={previewVadSensitivity}
                dictationBusy={isRecording}
                missingDevice={missingDevice}
                inventoryAvailable={audioInventory?.status === 'available'}
                inventoryLoading={audioInventoryState.loading}
                onChange={(microphone) => onUpdateSettings({ microphone })}
              />
              {audioInventoryState.error && (
                <p role="alert" className="mt-2 text-xs text-primary">{audioInventoryState.error} Close and reopen Settings if it does not refresh.</p>
              )}
            </div>
            <div data-setting-target="voice-detection" className="rounded-lg px-1 transition-shadow [&.settings-target-flash]:ring-2 [&.settings-target-flash]:ring-primary/40">
              <p className="mb-2 text-sm font-medium text-on-surface">Voice Detection</p>
              <VadSensitivitySlider
                value={settings.vadSensitivity}
                onPreview={setPreviewVadSensitivity}
                onCommit={(vadSensitivity) => onUpdateSettings({ vadSensitivity })}
              />
            </div>
            <div data-setting-target="recording-trigger" className="rounded-lg px-1 transition-shadow [&.settings-target-flash]:ring-2 [&.settings-target-flash]:ring-primary/40">
              <p className="mb-2 text-sm font-medium text-on-surface">Recording Trigger</p>
              <div className="flex gap-2">
                {RECORDING_MODE_OPTIONS.map((option) => (
                  <button key={option.value} type="button" disabled={isRecording} onClick={() => onUpdateSettings({ recordingMode: option.value as RecordingMode })} className={`h-8 flex-1 rounded-lg border px-3 text-[length:var(--ui-font-label)] font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-50 ${settings.recordingMode === option.value ? 'border-primary bg-primary text-on-primary' : 'border-outline-variant/30 bg-surface-container-lowest text-on-surface hover:bg-surface-container'}`}>{option.label}</button>
                ))}
              </div>
              {isRecording && <p className="mt-1 text-xs text-primary">Stop recording before changing mode.</p>}
            </div>
            {accessibilityGranted === false && (
              <div className="flex items-center gap-2 rounded-lg border border-primary/30 bg-primary/10 px-3 py-2 text-xs text-on-surface">
                <span>Accessibility permission is required for keyboard detection.</span>
                <button type="button" onClick={requestAccessibility} className="ml-auto underline">Grant</button>
              </div>
            )}
            <div data-setting-target="trigger-key" className="rounded-lg px-1 transition-shadow [&.settings-target-flash]:ring-2 [&.settings-target-flash]:ring-primary/40">
              <label className="mb-2 block text-sm font-medium text-on-surface">{keyLabel}</label>
              <Select value={settings.doubleTapKey} onChange={(doubleTapKey) => onUpdateSettings({ doubleTapKey })} disabled={isRecording} items={DOUBLE_TAP_KEY_OPTIONS} />
              <p className="mt-1 text-xs text-on-surface-variant">{keyHelp}</p>
            </div>
            {(isDoubleTap || isBoth) && <SettingToggle targetId="hotkey-feedback" title="Hotkey Timing Feedback" description="Flash the overlay when a tap misses the double-tap window." checked={settings.hotkeyMissFeedback} onChange={() => onUpdateSettings({ hotkeyMissFeedback: !settings.hotkeyMissFeedback })} />}
            <div data-setting-target="sound-cues" className="rounded-lg px-1 transition-shadow [&.settings-target-flash]:ring-2 [&.settings-target-flash]:ring-primary/40">
              <SettingToggle
                title="Sound Cues"
                description="Play local feedback when recording starts, stops, succeeds, or fails."
                checked={settings.soundCuesEnabled}
                onChange={() => onUpdateSettings({ soundCuesEnabled: !settings.soundCuesEnabled })}
              />
              {settings.soundCuesEnabled && (
                <div className="mt-2 space-y-3 pl-1">
                  <label className="block text-xs font-medium text-on-surface-variant">
                    Volume · {settings.soundCueVolume}%
                    <input
                      className="mt-2 block w-full accent-primary"
                      type="range"
                      min="0"
                      max="100"
                      step="5"
                      value={settings.soundCueVolume}
                      onChange={(event) => onUpdateSettings({ soundCueVolume: Number(event.target.value) })}
                    />
                  </label>
                  <div className="flex flex-wrap gap-2" aria-label="Preview sound cues">
                    {(['start', 'stop', 'success', 'failure'] as const).map((cue: SoundCue) => (
                      <button
                        key={cue}
                        type="button"
                        onClick={() => playSoundCue(cue, settings.soundCueVolume)}
                        className="h-8 rounded-lg border border-outline-variant/30 bg-surface-container-lowest px-3 text-xs font-medium capitalize text-on-surface hover:bg-surface-container"
                      >
                        {cue}
                      </button>
                    ))}
                  </div>
                  <SettingToggle
                    title="Meeting Cues"
                    description="Also play cues during meeting capture. Off by default."
                    checked={settings.meetingSoundCuesEnabled}
                    onChange={() => onUpdateSettings({ meetingSoundCuesEnabled: !settings.meetingSoundCuesEnabled })}
                  />
                </div>
              )}
            </div>
            <div data-setting-target="stop-on-silence" className="rounded-lg px-1 transition-shadow [&.settings-target-flash]:ring-2 [&.settings-target-flash]:ring-primary/40">
              <label className="mb-2 block text-sm font-medium text-on-surface">Stop on Silence</label>
              <div className="flex gap-2">
                {AUTO_STOP_SILENCE_OPTIONS.map((option) => (
                  <button
                    key={option.value}
                    type="button"
                    // Locked mid-recording like the sibling trigger controls:
                    // the detector reads this value live, so a change now
                    // would retune the recording already in flight.
                    disabled={isRecording}
                    aria-pressed={settings.autoStopSilenceMs === option.value}
                    onClick={() => onUpdateSettings({ autoStopSilenceMs: option.value })}
                    className={`h-8 flex-1 rounded-lg border px-3 text-[length:var(--ui-font-label)] font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-50 ${settings.autoStopSilenceMs === option.value ? 'border-primary bg-primary text-on-primary' : 'border-outline-variant/30 bg-surface-container-lowest text-on-surface hover:bg-surface-container'}`}
                  >
                    {option.label}
                  </button>
                ))}
              </div>
              <p className="mt-1 text-xs text-on-surface-variant">
                Finish a recording automatically after this much quiet. Applies when you didn't
                start by holding the key — a held recording ends when you let go. It only arms
                once Murmur has heard you speak, so a silent start never stops itself, and you
                can still stop manually at any time.
              </p>
            </div>
          </SettingsSection>

          <SettingsSection pageId="ai" activePage={activeCat} title="AI & Models" subtitle="Choose the engine that powers each Murmur feature">
            <button
              type="button"
              onClick={() => openPage('ai-transcription')}
              className="flex w-full items-center gap-4 px-1 py-1 text-left hover:text-primary"
            >
              <span className="grid h-9 w-9 shrink-0 place-items-center rounded-lg bg-surface-container-high text-primary"><SettingsNavIcon icon="recording" /></span>
              <span className="min-w-0 flex-1">
                <span className="block text-sm font-semibold text-on-surface">Speech-to-Text</span>
                <span className="mt-0.5 block truncate text-xs text-on-surface-variant">{selectedRuntime?.label ?? AVAILABLE_MODEL_OPTIONS.find((option) => option.value === settings.model)?.label ?? settings.model} · {selectedRuntime?.installState ?? 'Local model'}</span>
              </span>
              <span className="text-xs font-semibold text-on-surface-variant">Configure <span aria-hidden="true">›</span></span>
            </button>
            <button
              type="button"
              onClick={() => openPage('ai-query')}
              className="flex w-full items-center gap-4 px-1 py-1 text-left hover:text-primary"
            >
              <span className="grid h-9 w-9 shrink-0 place-items-center rounded-lg bg-surface-container-high text-primary"><SettingsNavIcon icon="ai" /></span>
              <span className="min-w-0 flex-1">
                <span className="block text-sm font-semibold text-on-surface">Voice Query</span>
                <span className="mt-0.5 block truncate text-xs text-on-surface-variant">{settings.queryProvider === 'custom' ? 'Custom CLI' : settings.queryProvider} · {settings.queryHotkey === null ? 'Shortcut off' : 'Shortcut on'}</span>
              </span>
              <span className="text-xs font-semibold text-on-surface-variant">Configure <span aria-hidden="true">›</span></span>
            </button>
            <button
              type="button"
              onClick={() => openPage('ai-transform')}
              className="flex w-full items-center gap-4 px-1 py-1 text-left hover:text-primary"
            >
              <span className="grid h-9 w-9 shrink-0 place-items-center rounded-lg bg-surface-container-high text-primary"><SettingsNavIcon icon="text" /></span>
              <span className="min-w-0 flex-1">
                <span className="block text-sm font-semibold text-on-surface">Selected-Text Rewrite</span>
                <span className="mt-0.5 block truncate text-xs text-on-surface-variant">Qwen2.5 1.5B · {transformModel?.state === 'ready' ? 'Ready on-device' : 'Model setup required'}</span>
              </span>
              <span className="text-xs font-semibold text-on-surface-variant">Configure <span aria-hidden="true">›</span></span>
            </button>
          </SettingsSection>

          <SettingsSection pageId="ai-query" activePage={activeCat} title="Voice Query" subtitle="Provider, privacy, shortcut, and response behavior">
            <div className="rounded-xl border border-warning bg-warning/10 p-3">
              <p className="text-sm font-medium text-on-surface">You control where the question goes</p>
              <p className="mt-1 text-xs leading-relaxed text-on-surface">
                Murmur transcribes your question locally, then gives it to the exact CLI executable below.
                That CLI may send the question or answer to cloud services according to its own configuration, and may also send any optional app context you enable.
                Murmur cannot verify or prevent that network egress.
              </p>
            </div>

            <div data-setting-target="voice-query-provider" className="rounded-lg px-1 transition-shadow [&.settings-target-flash]:ring-2 [&.settings-target-flash]:ring-primary/40">
              <label className="mb-1.5 block text-sm font-medium text-on-surface">Provider</label>
              <Select
                value={settings.queryProvider}
                onChange={(value) => void selectQueryProvider(value as QueryProviderId)}
                items={queryProviderItems.length > 0
                  ? queryProviderItems
                  : [{ value: 'custom', label: 'Custom' }]}
              />
              {selectedQueryPreset && settings.queryProvider !== 'custom' && (
                <p className="mt-1 text-xs text-on-surface-variant">
                  {selectedQueryPreset.discoveredExecutable
                    ? `Found ${selectedQueryPreset.discoveredExecutable}`
                    : `Not found in ${selectedQueryPreset.discoveryPaths.join(', ')}`}
                </p>
              )}
              {settings.queryProvider === 'custom' && (
                <p className="mt-1 text-xs text-on-surface-variant">
                  Choose an absolute executable and its fixed arguments below. For a local smoke test,
                  use <code>/usr/bin/printf</code> with one fixed argument: <code>%s</code>.
                </p>
              )}
            </div>

            <SettingToggle
              title="Enable Voice Query"
              description="Double-tap a dedicated key to record; tap once to finish. No spoken keyword is used."
              checked={settings.queryHotkey !== null}
              disabled={queryConfigBusy}
              onChange={() => void toggleVoiceQuery()}
            />
            <SettingToggle
              targetId="voice-query-copy"
              title="Automatically copy answers"
              description="Copy successful final answers to the clipboard. Voice Query never auto-pastes."
              checked={settings.queryAutomaticallyCopyAnswers}
              onChange={() => onUpdateSettings({
                queryAutomaticallyCopyAnswers: !settings.queryAutomaticallyCopyAnswers,
              })}
            />
            {queryConfigError && <p role="alert" className="text-xs text-error">{queryConfigError}</p>}
            {queryConfigNotice && (
              <p role="status" className="text-xs text-on-surface-variant">{queryConfigNotice}</p>
            )}

            <div className="space-y-4">
              <div>
                <label htmlFor="query-executable" className="mb-1.5 block text-sm font-medium text-on-surface">CLI executable</label>
                <div className="flex gap-2">
                  <input
                    id="query-executable"
                    type="text"
                    value={settings.queryExecutable}
                    onChange={(event) => {
                      setQueryConfigError(null);
                      setQueryConfigNotice(settings.queryHotkey !== null
                        ? 'Command changed. Voice Query was turned off so the new command can be tested before use.'
                        : null);
                      setQueryTestResult(null);
                      setQuerySignInStatus(null);
                      invalidateQueryRequests();
                      onUpdateSettings({ queryExecutable: event.target.value, queryHotkey: null });
                    }}
                    placeholder="/absolute/path/to/agent"
                    spellCheck={false}
                    className="min-w-0 flex-1 rounded-lg border border-outline-variant bg-surface-container-lowest px-3 py-2 font-mono text-xs text-on-surface outline-none focus:border-primary"
                  />
                  <button type="button" onClick={() => void chooseQueryExecutable()} className="rounded-lg border border-outline-variant/30 px-3 py-2 text-xs font-semibold text-on-surface hover:bg-surface-container">
                    Browse…
                  </button>
                </div>
                <p className="mt-1 text-xs text-on-surface-variant">
                  Must be an absolute path to an executable file. No shell is ever invoked.
                </p>
              </div>

              <div>
                <label htmlFor="query-arguments" className="mb-1.5 block text-sm font-medium text-on-surface">Fixed arguments</label>
                <textarea
                  id="query-arguments"
                  rows={3}
                  value={settings.queryArguments.join('\n')}
                  onChange={(event) => {
                    setQueryConfigError(null);
                    setQueryConfigNotice(settings.queryHotkey !== null
                      ? 'Command changed. Voice Query was turned off so the new command can be tested before use.'
                      : null);
                    setQueryTestResult(null);
                    setQuerySignInStatus(null);
                    invalidateQueryRequests();
                    onUpdateSettings({
                      queryArguments: event.target.value.split('\n').filter((argument) => argument.length > 0),
                      queryHotkey: null,
                    });
                  }}
                  placeholder={'One argument per line\n--print'}
                  spellCheck={false}
                  className="w-full resize-y rounded-lg border border-outline-variant bg-surface-container-lowest px-3 py-2 font-mono text-xs leading-relaxed text-on-surface outline-none focus:border-primary"
                />
                <p className="mt-1 text-xs text-on-surface-variant">
                  Each line stays one argument. The transcript is appended as exactly one final argument, including spaces and punctuation.
                </p>
              </div>

              <div className="rounded-xl border border-outline-variant/30 bg-surface-container-low p-3">
                <div className="flex items-center gap-3">
                  <div className="min-w-0 flex-1">
                    <p className="text-sm font-medium text-on-surface">Provider preflight</p>
                    <p className="mt-1 text-xs text-on-surface-variant">
                      Runs the preset’s bounded authentication probe through the same direct-spawn and cleared-environment path as a query.
                    </p>
                  </div>
                  <button
                    type="button"
                    disabled={queryTestBusy || !settings.queryExecutable.trim()}
                    onClick={() => void runQueryTest()}
                    className="rounded-lg bg-primary px-3 py-1.5 text-xs font-semibold text-on-primary hover:bg-primary-dim disabled:cursor-not-allowed disabled:opacity-50"
                  >
                    {queryTestBusy ? 'Testing…' : 'Test'}
                  </button>
                </div>
                {queryTestResult && (
                  <div className="mt-3 space-y-2 text-xs">
                    <p className={queryTestResult.ok ? 'text-primary' : 'text-error'}>
                      {queryProviderTestMessage(settings.queryProvider, queryTestResult)}
                    </p>
                    {queryTestResult.stdout && !isIncompleteCodexProbe(settings.queryProvider, queryTestResult) && (
                      <div>
                        <p className="font-semibold text-on-surface-variant">stdout{queryTestResult.stdoutTruncated ? ' · tail only' : ''}</p>
                        <pre className="mt-1 max-h-32 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-surface-container-lowest p-2 font-mono text-[11px] text-on-surface">
                          {queryTestResult.stdout}
                        </pre>
                      </div>
                    )}
                    {queryTestResult.stderr && !isIncompleteCodexProbe(settings.queryProvider, queryTestResult) && (
                      <div>
                        <p className="font-semibold text-on-surface-variant">stderr{queryTestResult.stderrTruncated ? ' · tail only' : ''}</p>
                        <pre className="mt-1 max-h-32 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-surface-container-lowest p-2 font-mono text-[11px] text-on-surface">
                          {queryTestResult.stderr}
                        </pre>
                      </div>
                    )}
                    {!queryTestResult.ok && queryTestResult.signInFix && (
                      <button
                        type="button"
                        onClick={() => void signInQueryProvider()}
                        className="rounded-lg border border-outline-variant/30 px-3 py-1.5 font-semibold text-on-surface hover:bg-surface-container"
                      >
                        Sign in…
                      </button>
                    )}
                  </div>
                )}
                {querySignInStatus && (
                  <p aria-live="polite" className="mt-2 text-xs text-on-surface-variant">
                    {querySignInStatus}
                  </p>
                )}
              </div>

              {selectedQueryPreset && (
                selectedQueryPreset.permittedEnvironmentVariables.length > 0
                || queryEnvironmentNeedsRepair
                || queryEnvironmentStatus !== null
              ) && (
                <div className="rounded-xl border border-outline-variant/30 bg-surface-container-low p-3">
                  <p className="text-sm font-medium text-on-surface">
                    {selectedQueryPreset.permittedEnvironmentVariables.length > 0
                      ? 'Declared config directories'
                      : 'Voice Query environment'}
                  </p>
                  {selectedQueryPreset.permittedEnvironmentVariables.length > 0 && (
                    <p className="mt-1 text-xs leading-relaxed text-on-surface-variant">
                      Optional absolute directory paths are added to the cleared child environment.
                      HOME and the base allowlist cannot be overridden. API keys, tokens, and other
                      secret variables are not accepted. Values live only in Rust-owned app data,
                      never localStorage.
                    </p>
                  )}
                  {selectedQueryPreset.permittedEnvironmentVariables.length > 0 && (
                    <div className="mt-3 space-y-3">
                      {selectedQueryPreset.permittedEnvironmentVariables.map((name) => (
                        <div key={name}>
                          <label htmlFor={`query-env-${name}`} className="mb-1 block font-mono text-xs font-medium text-on-surface">
                            {name}{configuredQueryEnvironment.includes(name) ? ' · configured' : ''}
                          </label>
                          <input
                            id={`query-env-${name}`}
                            type="text"
                            value={queryEnvironment.find((variable) => variable.name === name)?.value ?? ''}
                            onChange={(event) => {
                              const value = event.target.value;
                              invalidateQueryRequests();
                              setQueryEnvironment((current) => [
                                ...current.filter((variable) => variable.name !== name),
                                ...(value ? [{ name, value }] : []),
                              ]);
                              setQueryEnvironmentStatus(null);
                            }}
                            placeholder={configuredQueryEnvironment.includes(name)
                              ? 'Enter a replacement path'
                              : '/absolute/path/to/config'}
                            spellCheck={false}
                            className="w-full rounded-lg border border-outline-variant bg-surface-container-lowest px-3 py-2 font-mono text-xs text-on-surface outline-none focus:border-primary"
                          />
                        </div>
                      ))}
                    </div>
                  )}
                  <div className="mt-3 flex items-center gap-3">
                    {selectedQueryPreset.permittedEnvironmentVariables.length > 0 && (
                      <button
                        type="button"
                        onClick={() => void saveDeclaredEnvironment()}
                        className="rounded-lg border border-outline-variant/30 px-3 py-1.5 text-xs font-semibold text-on-surface hover:bg-surface-container"
                      >
                        Save environment
                      </button>
                    )}
                    {(configuredQueryEnvironment.length > 0 || queryEnvironmentNeedsRepair) && (
                      <button
                        type="button"
                        onClick={() => void clearDeclaredEnvironment()}
                        className="rounded-lg border border-outline-variant/30 px-3 py-1.5 text-xs font-semibold text-on-surface hover:bg-surface-container"
                      >
                        Clear saved values
                      </button>
                    )}
                    {queryEnvironmentStatus && (
                      <span className="text-xs text-on-surface-variant">{queryEnvironmentStatus}</span>
                    )}
                  </div>
                </div>
              )}

              <div>
                <label className="mb-1.5 block text-sm font-medium text-on-surface">Context shared with the CLI</label>
                <Select
                  value={settings.queryContextLevel}
                  onChange={(queryContextLevel) => {
                    invalidateQueryRequests();
                    onUpdateSettings({ queryContextLevel });
                  }}
                  items={QUERY_CONTEXT_LEVEL_OPTIONS}
                />
                <p className="mt-1 text-xs text-on-surface-variant">
                  Off by default. App &amp; window adds only the frontmost app name and window title.
                  Choose App, window &amp; selection (the third option) to also add up to 8 KiB of
                  selected text after secure-field checks. The popover always shows what kind of
                  context was included, and per-app exclusions take precedence.
                </p>
              </div>

              <SettingToggle
                title="Keep Voice Query history on this Mac"
                description="Off by default. When on, Murmur keeps up to 200 recognized questions, answers, provider IDs, token counts, durations, and stable errors in a separate Rust-owned local store. This includes queries that shared app context: context is not stored as a separate field, but a saved answer may quote it. Turning history off affects new queries; existing entries remain until you delete them from History → Queries."
                checked={settings.retainQueryHistory}
                onChange={() => onUpdateSettings({ retainQueryHistory: !settings.retainQueryHistory })}
              />
              <p className="text-xs text-on-surface-variant">
                Voice Query counters and token usage appear under Insights in the main-window footer.
              </p>

              <div className="grid grid-cols-2 gap-4">
                <div>
                  <label className="mb-1.5 block text-sm font-medium text-on-surface">Query shortcut</label>
                  <Select
                    value={settings.queryHotkey ?? QUERY_KEY_OPTIONS.find((option) => option.value !== settings.transformHoldKey)?.value ?? 'shift_r'}
                    disabled={settings.queryHotkey === null}
                    onChange={(value) => {
                      const queryHotkey = value as QueryKey;
                      if (queryHotkey === settings.transformHoldKey) {
                        setQueryConfigError('That key is already assigned to Selected-text Transform.');
                        return;
                      }
                      setQueryConfigError(null);
                      onUpdateSettings({ queryHotkey });
                    }}
                    items={QUERY_KEY_OPTIONS}
                  />
                </div>
                <div>
                  <label className="mb-1.5 block text-sm font-medium text-on-surface">Timeout</label>
                  <Select
                    value={String(settings.queryTimeoutSeconds)}
                    onChange={(value) => {
                      invalidateQueryRequests();
                      onUpdateSettings({ queryTimeoutSeconds: Number(value) });
                    }}
                    items={[
                      { value: '30', label: '30 seconds' },
                      { value: '60', label: '1 minute' },
                      { value: '120', label: '2 minutes' },
                      { value: '300', label: '5 minutes' },
                    ]}
                  />
                </div>
              </div>

              {accessibilityGranted === false && settings.queryHotkey !== null && (
                <div className="flex items-center gap-2 rounded-lg border border-primary/30 bg-primary/10 px-3 py-2 text-xs text-on-surface">
                  <span>Accessibility permission is required for the global query shortcut.</span>
                  <button type="button" onClick={requestAccessibility} className="ml-auto underline">Grant</button>
                </div>
              )}
            </div>

            <div className="border-t border-outline-variant/20 pt-4 text-xs leading-relaxed text-on-surface-variant">
              Answers stream into a popover. Successful final answers are copied to the clipboard when automatic
              copying is enabled; otherwise, use Copy in the popover. They are never auto-pasted. Question and answer content enters only the separate local query store when you explicitly enable it.
              Context is never stored as a separate history field, but a saved answer may quote context shared with the CLI.
              Context content never enters saved files, usage stats, diagnostics, logs, or telemetry.
            </div>
          </SettingsSection>

          <SettingsSection pageId="ai-transform" activePage={activeCat} title="Selected-Text Rewrite" subtitle="On-device rewriting, shortcut, and saved instructions">
            <div data-setting-target="rewrite-model" className="rounded-xl border border-primary/20 bg-primary/5 p-3 transition-shadow [&.settings-target-flash]:ring-2 [&.settings-target-flash]:ring-primary/40">
              <p className="text-sm font-medium text-on-surface">Local only · Apple Silicon</p>
              <p className="mt-1 text-xs text-on-surface">
                Hold a dedicated shortcut, speak an instruction, and review a proposed rewrite before
                anything is written. The model stays on-device ({TRANSFORM_MODEL_SIZE_LABEL} download).
                Never auto-applies.
              </p>
            </div>
            <SettingToggle
              title="Enable Transform Shortcut"
              description="Hold the transform key while text is selected to capture a rewrite instruction."
              checked={settings.transformHoldKey !== null}
              onChange={() => {
                void updateTransformHoldKey(
                  settings.transformHoldKey === null ? 'alt_r' : null,
                );
              }}
            />
            {settings.transformHoldKey !== null && (
              <div className="ml-3 space-y-2 border-l border-outline-variant/30 pl-3">
                <label className="mb-1 block text-sm font-medium text-on-surface">Hold key</label>
                <Select
                  value={settings.transformHoldKey}
                  onChange={(value) => {
                    void updateTransformHoldKey(value as TransformKey);
                  }}
                  items={TRANSFORM_KEY_OPTIONS}
                />
                <p className="text-xs text-on-surface-variant">
                  Dictation hold keys are rejected. Right Option / Left Control / Right Shift only.
                </p>
                {transformKeyError && (
                  <p className="text-xs text-error">{transformKeyError}</p>
                )}
                {accessibilityGranted === false && (
                  <div className="flex items-center gap-2 rounded-lg border border-primary/30 bg-primary/10 px-3 py-2 text-xs text-on-surface">
                    <span>Accessibility permission is required for transform capture and apply.</span>
                    <button type="button" onClick={requestAccessibility} className="ml-auto underline">Grant</button>
                  </div>
                )}
              </div>
            )}
            <div className="border-t border-outline-variant/20 pt-4">
              <h2 className="text-sm font-medium text-on-surface">On-device model</h2>
              <p className="mt-1 mb-3 text-xs text-on-surface-variant">
                Qwen2.5-1.5B Instruct (Q4_K_M), {TRANSFORM_MODEL_SIZE_LABEL}. Downloaded to
                Application Support; verified by size and SHA-256. Apple Silicon only.
              </p>
              {transformModel && (
                <p className="mb-2 text-xs text-on-surface-variant" data-testid="transform-model-status">
                  Status:{' '}
                  {transformModel.state === 'ready'
                    ? 'Ready'
                    : transformModel.state === 'downloading'
                      ? 'Downloading…'
                      : 'Not downloaded'}
                </p>
              )}
              {transformDownloadPct !== null && (
                <div className="mb-2">
                  <div className="mb-1 flex justify-between text-xs text-on-surface-variant">
                    <span>Downloading transform model</span>
                    <span>{transformDownloadPct}%</span>
                  </div>
                  <div className="h-1.5 overflow-hidden rounded-full bg-surface-container-highest">
                    <div
                      className="h-full rounded-full bg-primary transition-all duration-200"
                      style={{ width: `${transformDownloadPct}%` }}
                    />
                  </div>
                </div>
              )}
              {transformModelError && (
                <p className="mb-2 text-xs text-error">{transformModelError}</p>
              )}
              <div className="flex flex-wrap gap-2">
                {transformModel?.state !== 'ready' && (
                  <button
                    type="button"
                    disabled={transformModelBusy || transformModel?.state === 'downloading'}
                    onClick={() => void downloadTransform()}
                    className="rounded-lg bg-primary px-3 py-1.5 text-xs font-medium text-on-primary disabled:opacity-50"
                  >
                    {transformModelBusy || transformModel?.state === 'downloading' ? 'Working…' : 'Download'}
                  </button>
                )}
                {transformModel?.state === 'ready' && (
                  <button
                    type="button"
                    disabled={transformModelBusy}
                    onClick={() => void removeTransform()}
                    className="rounded-lg border border-outline-variant/30 px-3 py-1.5 text-xs font-medium text-on-surface-variant disabled:opacity-50"
                  >
                    Remove
                  </button>
                )}
                {transformModel?.runtimeDisabled && (
                  <button
                    type="button"
                    disabled={transformModelBusy}
                    onClick={() => void resetTransform()}
                    className="rounded-lg border border-outline-variant/30 px-3 py-1.5 text-xs font-medium text-on-surface-variant disabled:opacity-50"
                    title="Clear the circuit breaker if the transform runtime was disabled after repeated faults"
                  >
                    Reset runtime
                  </button>
                )}
              </div>
              {transformModel?.runtimeDisabled && (
                <p className="mt-2 text-xs text-primary">
                  The transform runtime was disabled after repeated faults. Reset it to try again.
                </p>
              )}
            </div>
            <div className="border-t border-outline-variant/20 pt-4">
              <div className="flex items-center justify-between gap-4">
                <div>
                  <h2 className="text-sm font-medium text-on-surface">Saved transforms</h2>
                  <p className="mt-1 text-xs text-on-surface-variant">Create reusable spoken rewrite instructions.</p>
                </div>
                <button type="button" onClick={() => openEditor('transforms')} className="rounded-lg bg-surface-container-high px-3 py-2 text-xs font-semibold text-on-surface hover:text-primary">Manage</button>
              </div>
            </div>
          </SettingsSection>

          <SettingsSection pageId="ai-transcription" activePage={activeCat} title="Speech-to-Text" subtitle="Recognition model, language, and memory lifecycle">
            <div data-setting-target="transcription-model" className="rounded-lg px-1 transition-shadow [&.settings-target-flash]:ring-2 [&.settings-target-flash]:ring-primary/40">
              <label className="mb-2 block text-sm font-medium text-on-surface">Transcription Model</label>
              <Select
                value={settings.model}
                onChange={(model) => onUpdateSettings({ model })}
                disabled={isRecording}
                items={AVAILABLE_MODEL_OPTIONS.map((model) => ({ value: model.value, label: `${model.label}${model.backend === 'coreml' ? ' — Recommended' : ''} (${model.size})` }))}
              />
              <p className="mt-1 text-xs text-on-surface-variant">Parakeet Core ML is recommended on supported Macs. Larger models can be more accurate but use more storage and memory.</p>
              {selectedRuntime && <p className="mt-1 text-xs text-on-surface-variant" data-testid="model-runtime-status">{selectedRuntime.label}: {selectedRuntime.backend} / {selectedRuntime.accelerator} / {selectedRuntime.size} · {selectedRuntime.installState} · {selectedRuntime.lifecycleState}</p>}
              {isRecording && <p className="mt-1 text-xs text-primary">Stop recording before changing model.</p>}
              {modelAvailable === false && modelDownload.phase === 'idle' && (
                <div className="mt-2 flex items-center rounded-lg border border-primary/30 bg-primary/10 px-3 py-2 text-xs text-on-surface">
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
            <div data-setting-target="language" className="rounded-lg px-1 transition-shadow [&.settings-target-flash]:ring-2 [&.settings-target-flash]:ring-primary/40">
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

          <SettingsSection pageId="text" activePage={activeCat} title="Text & Vocabulary" subtitle="Cleanup, preferred terms, structured writing, and knowledge">
            <SettingToggle targetId="punctuation" title="Automatic Punctuation" label="Smart punctuation" description="Add periods, commas, and capitalization to transcriptions." checked={settings.smartPunctuation} onChange={() => onUpdateSettings({ smartPunctuation: !settings.smartPunctuation })} />
            <SettingToggle targetId="cleanup" title="Transcript Cleanup" description="Remove filler and tidy spacing before delivery." checked={settings.cleanupEnabled} onChange={() => onUpdateSettings({ cleanupEnabled: !settings.cleanupEnabled })} />
            {settings.cleanupEnabled && (
              <div className="ml-3 space-y-3 border-l border-outline-variant/30 pl-3">
                <SettingToggle title="Remove filler words" description="Remove filler tokens such as um and uh." checked={settings.cleanupRemoveFiller} onChange={() => onUpdateSettings({ cleanupRemoveFiller: !settings.cleanupRemoveFiller })} />
                <SettingToggle title="Capitalize sentences" description="Capitalize detected sentence starts." checked={settings.cleanupCapitalize} onChange={() => onUpdateSettings({ cleanupCapitalize: !settings.cleanupCapitalize })} />
              </div>
            )}
            <div data-setting-target="text-editors" className="grid gap-2 rounded-lg transition-shadow sm:grid-cols-2 [&.settings-target-flash]:ring-2 [&.settings-target-flash]:ring-primary/40">
              {([
                ['vocabulary', 'Vocabulary', 'Review identifiers retained from project scans.'],
                ['aliases', 'Aliases', 'Map spoken variants to canonical spellings.'],
                ['knowledge', 'Knowledge', 'Manage corrections, terms, snippets, and transforms.'],
                ['commands', 'Voice Commands', 'Create exact spoken replacements and snippets.'],
              ] as const).map(([tab, title, detail]) => (
                <button key={tab} type="button" onClick={() => openEditor(tab)} className="flex items-center gap-3 rounded-xl border border-outline-variant/20 bg-surface-container-low px-3 py-3 text-left hover:border-primary/35 hover:bg-surface-container">
                  <span className="min-w-0 flex-1">
                    <span className="block text-sm font-semibold text-on-surface">{title}</span>
                    <span className="mt-0.5 block text-[11px] leading-relaxed text-on-surface-variant">{detail}</span>
                  </span>
                  <span className="text-on-surface-variant" aria-hidden="true">›</span>
                </button>
              ))}
            </div>
            <details className="group">
              <summary className="flex cursor-pointer list-none items-center justify-between rounded-lg py-1 text-sm font-semibold text-on-surface focus:outline-none focus-visible:ring-2 focus-visible:ring-primary">
                Advanced
                <span aria-hidden="true" className="text-on-surface-variant transition-transform group-open:rotate-180">⌄</span>
              </summary>
              <div className="mt-3 space-y-4 border-t border-outline-variant/20 pt-3">
                <SettingToggle title="Developer Terms" description="Make built-in development terms and an optional project scan available only to apps configured as Code / technical or with Local IDE project context." checked={settings.codeVocabEnabled} onChange={() => onUpdateSettings({ codeVocabEnabled: !settings.codeVocabEnabled })} />
                {settings.codeVocabEnabled && (
                  <div className="ml-3 space-y-2 border-l border-outline-variant/30 pl-3">
                    <p className="break-all rounded-lg border border-outline-variant/30 bg-surface-container-lowest px-3 py-2 text-xs text-on-surface">{settings.codeVocabFolder || 'No folder — built-in developer terms only'}</p>
                    <button type="button" onClick={() => openEditor('scan')} className="rounded-lg bg-surface-container-high px-3 py-2 text-xs font-semibold text-on-surface hover:text-primary">Manage Project Scan</button>
                    <p className="text-xs text-on-surface-variant">The selected folder is scanned locally; dependency and build folders are skipped. Unconfigured apps keep ordinary prose vocabulary.</p>
                  </div>
                )}
                <SettingToggle title="Apply Preferred Spellings" label="Smart correction" description="Apply names, terms, and developer vocabulary after recognition on every model." checked={settings.correctionEnabled} onChange={() => onUpdateSettings({ correctionEnabled: !settings.correctionEnabled })} />
                {settings.correctionEnabled && <div className="ml-3 border-l border-outline-variant/30 pl-3"><SettingToggle title="Correct Close Mishearings" label="Sounds-like matching" description="Recover close mishearings near your vocabulary; disable if you see unwanted swaps." checked={settings.correctionFuzzy} onChange={() => onUpdateSettings({ correctionFuzzy: !settings.correctionFuzzy })} /></div>}
                <SettingToggle title="Structured Writing" label="Smart formatting" description="Apply explicitly spoken lists, symbols, punctuation, and same-utterance corrections locally." checked={settings.smartFormattingEnabled} onChange={() => onUpdateSettings({ smartFormattingEnabled: !settings.smartFormattingEnabled })} />
                <SettingToggle title="Spoken Formatting" label="Voice commands" description="Use spoken tokens such as “new line,” “period,” or “scratch that” before delivery." checked={settings.voiceCommandsEnabled} onChange={() => onUpdateSettings({ voiceCommandsEnabled: !settings.voiceCommandsEnabled })} />
              </div>
            </details>
          </SettingsSection>

          <SettingsSection pageId="delivery" activePage={activeCat} title="Delivery" subtitle="Choose what happens after transcription finishes">
            <div className="border-y border-outline-variant/20 px-1 py-3">
              <h2 className="text-sm font-medium text-on-surface">Always copied to clipboard</h2>
              <p className="mt-1 text-xs text-on-surface-variant">Auto-paste and file output happen afterward, so the finished text remains recoverable.</p>
            </div>
            <SettingToggle targetId="auto-paste" title="Auto-Paste" label="Auto paste" description={autoPasteDeliveryDescription(settings)} checked={autoPasteOn} disabled={saveToFile} onChange={() => onUpdateSettings({ autoPaste: !settings.autoPaste })} />
            {settings.autoPaste && saveToFile && <p role="status" className="rounded-lg border border-primary/30 bg-primary/10 px-3 py-2 text-xs text-on-surface">Auto-paste is paused; the stored preference remains on.</p>}
            {autoPasteOn && accessibilityGranted !== null && <div className={`flex items-center gap-2 text-xs ${accessibilityGranted ? 'text-success ' : 'text-primary '}`}><span>{accessibilityGranted ? 'Accessibility permission granted' : 'Accessibility permission required'}</span>{accessibilityGranted === false && <button type="button" onClick={requestAccessibility} className="underline">Grant</button>}</div>}
            {autoPasteOn && <PasteDelaySlider value={settings.autoPasteDelayMs} onCommit={(autoPasteDelayMs) => onUpdateSettings({ autoPasteDelayMs })} />}
            <div data-setting-target="file-output" className="space-y-3 rounded-lg transition-shadow [&.settings-target-flash]:ring-2 [&.settings-target-flash]:ring-primary/40">
              <SettingToggle title="Save Transcript to File" description="Write each completed transcription to a .txt file." checked={settings.saveTranscript} onChange={() => onUpdateSettings({ saveTranscript: !settings.saveTranscript })} />
              <SettingToggle title="Save Audio to File" description="Write each recording to a .wav file." checked={settings.saveAudio} onChange={() => onUpdateSettings({ saveAudio: !settings.saveAudio })} />
              {saveToFile && (
                <div>
                  <p className="mb-1 text-xs text-on-surface-variant">Output Folder</p>
                  <p className="break-all rounded-lg border border-outline-variant/30 bg-surface-container-lowest px-3 py-2 text-xs text-on-surface">{settings.outputDir || 'Documents/Murmur (default)'}</p>
                  <div className="mt-2 flex gap-3"><button type="button" onClick={() => void chooseOutputFolder()} className="text-xs font-medium text-on-surface-variant underline hover:text-primary">Choose Folder</button>{settings.outputDir && <button type="button" onClick={() => onUpdateSettings({ outputDir: '' })} className="text-xs font-medium text-on-surface-variant underline hover:text-primary">Reset to default</button>}</div>
                  <p className="mt-2 text-xs text-on-surface-variant">{fileOutputDeliveryDescription(settings)}</p>
                </div>
              )}
            </div>
            <SettingToggle
              targetId="history"
              title="Save Transcription History"
              description="Keep completed microphone and file transcripts in Murmur on this Mac. Turning this off affects new transcripts; existing history remains until you clear it."
              checked={settings.retainHistory}
              onChange={() => onUpdateSettings({ retainHistory: !settings.retainHistory })}
            />
            {notchPillInstalled && <SettingToggle title="Mirror Captions to NotchPill" description="Show your latest dictation in the NotchPill notch overlay. Stays on this Mac — only the final text is written locally." checked={settings.mirrorToNotchPill} onChange={() => onUpdateSettings({ mirrorToNotchPill: !settings.mirrorToNotchPill })} />}
            <details data-setting-target="app-overrides" className="group rounded-lg border-t border-outline-variant/20 pt-4 transition-shadow [&.settings-target-flash]:ring-2 [&.settings-target-flash]:ring-primary/40">
              <summary className="flex cursor-pointer list-none items-center justify-between rounded-lg py-1 text-sm font-semibold text-on-surface focus:outline-none focus-visible:ring-2 focus-visible:ring-primary">
                Advanced
                <span aria-hidden="true" className="text-on-surface-variant transition-transform group-open:rotate-180">⌄</span>
              </summary>
              <p className="mt-1 mb-3 text-xs text-on-surface-variant">Override delivery and writing behavior for the frontmost macOS app.</p>
              <AppOverridesEditor profiles={settings.appProfiles} onChange={(appProfiles) => onUpdateSettings({ appProfiles })} />
            </details>
          </SettingsSection>

          <SettingsSection pageId="meetings" activePage={activeCat} title="Meetings" subtitle="Local meeting transcript and audio retention">
            <div className="border-y border-outline-variant/20 px-1 py-3">
              <h2 className="text-sm font-medium text-on-surface">Stored separately from dictation</h2>
              <p className="mt-1 text-xs text-on-surface-variant">Meeting transcripts use the crash-safe local store and never appear in dictation history.</p>
            </div>
            <SettingToggle
              targetId="meeting-audio"
              title="Keep Meeting Audio"
              description="Off by default. When off, each private chunk WAV is deleted after its transcript commits."
              checked={settings.meetingRetainAudio}
              onChange={() => onUpdateSettings({ meetingRetainAudio: !settings.meetingRetainAudio })}
            />
            <div data-setting-target="meeting-retention" className="grid gap-4 rounded-lg px-1 transition-shadow sm:grid-cols-2 [&.settings-target-flash]:ring-2 [&.settings-target-flash]:ring-primary/40">
              <label className="text-sm font-medium text-on-surface">
                Keep transcripts by age
                <select
                  value={settings.meetingRetentionDays}
                  onChange={(event) => onUpdateSettings({ meetingRetentionDays: Number(event.target.value) })}
                  className="mt-2 h-9 w-full rounded-lg border border-outline-variant bg-surface-container-lowest px-2 text-sm text-on-surface"
                >
                  <option value={0}>No age limit</option>
                  <option value={30}>30 days</option>
                  <option value={90}>90 days</option>
                  <option value={365}>1 year</option>
                </select>
              </label>
              <label className="text-sm font-medium text-on-surface">
                Session limit
                <input
                  type="number"
                  min={1}
                  max={10000}
                  value={settings.meetingMaxSessions}
                  onChange={(event) => onUpdateSettings({ meetingMaxSessions: Math.max(1, Math.min(10000, Number(event.target.value) || 1)) })}
                  className="mt-2 h-9 w-full rounded-lg border border-outline-variant bg-surface-container-lowest px-2 text-sm text-on-surface"
                />
              </label>
            </div>
          </SettingsSection>

          <SettingsSection pageId="performance" activePage={activeCat} title="Performance Lab" subtitle="Compare installed speech models on this Mac">
            <div data-setting-target="performance">
              <PerformanceLab status={status} settings={settings} onUpdateSettings={onUpdateSettings} audioInventory={audioInventory} />
            </div>
          </SettingsSection>

          <SettingsSection pageId="diagnostics" activePage={activeCat} title="Diagnostics" subtitle="Events, run history, performance, reports, and transform traces">
            <div data-setting-target="diagnostics" className="h-[520px] min-h-0 overflow-hidden rounded-xl border border-outline-variant/25 bg-surface-container-lowest">
              <DiagnosticsWorkspace
                active={activeCat === 'diagnostics'}
                storeHealthEnabled
                onPopOut={(tab) => { void popOutDiagnostics(tab); }}
              />
            </div>
            {diagnosticsWindowError && (
              <p role="alert" className="text-xs text-error">
                {diagnosticsWindowError}
              </p>
            )}
          </SettingsSection>

          <SettingsSection plain pageId="appearance" activePage={activeCat} title="Appearance">
            <div data-setting-target="appearance"><AppearanceSettings /></div>
          </SettingsSection>

          <SettingsSection pageId="general" activePage={activeCat} title="General" subtitle="Startup, support, updates, and app information">
            {!INTERNAL_BENCHMARK_BUILD && <SettingToggle targetId="launch-login" title="Launch at Login" description="Start Murmur automatically when you log in." checked={settings.launchAtLogin} onChange={() => onUpdateSettings({ launchAtLogin: !settings.launchAtLogin })} />}
            <button data-setting-target="setup" type="button" onClick={onRerunSetup} className="w-full rounded-lg border border-outline-variant/30 bg-surface-container-lowest px-3 py-2 text-xs font-medium text-on-surface-variant transition-colors hover:bg-surface-container hover:text-primary [&.settings-target-flash]:ring-2 [&.settings-target-flash]:ring-primary/40">Run Setup Assistant</button>
            <p className="-mt-3 text-xs text-on-surface-variant">Re-check permissions and model setup after a permission is revoked or stops working.</p>
            <details className="group">
              <summary className="flex cursor-pointer list-none items-center justify-between rounded-lg py-1 text-sm font-semibold text-on-surface focus:outline-none focus-visible:ring-2 focus-visible:ring-primary">
                Advanced
                <span aria-hidden="true" className="text-on-surface-variant transition-transform group-open:rotate-180">⌄</span>
              </summary>
              <div className="mt-3 space-y-2 border-t border-outline-variant/20 pt-3">
                <OverlayCalibrationControl
                  offset={settings.overlayVerticalOffset}
                  onCommit={(overlayVerticalOffset) => onUpdateSettings({ overlayVerticalOffset })}
                />
                <button type="button" aria-label={confirmReset ? 'Confirm reset statistics' : 'Reset statistics'} onClick={resetStats} className={`w-full rounded-lg border px-3 py-2 text-xs font-medium transition-colors ${confirmReset ? 'border-error/40 bg-error/10 text-error' : 'border-outline-variant/30 bg-surface-container-lowest text-on-surface-variant hover:bg-surface-container hover:text-primary'}`}>{confirmReset ? 'Confirm Reset' : 'Reset Stats'}</button>
              </div>
            </details>
            {!INTERNAL_BENCHMARK_BUILD && <div data-setting-target="updates" className="rounded-lg transition-shadow [&.settings-target-flash]:ring-2 [&.settings-target-flash]:ring-primary/40">
              <button type="button" onClick={() => void onCheckForUpdate()} disabled={updateStatus.phase === 'checking' || updateStatus.phase === 'preparing' || updateStatus.phase === 'downloading' || updateStatus.phase === 'ready'} className="w-full rounded-lg border border-outline-variant/30 bg-surface-container-lowest px-3 py-2 text-xs font-medium text-on-surface-variant transition-colors hover:bg-surface-container hover:text-primary disabled:cursor-not-allowed disabled:opacity-50">{updateStatus.phase === 'checking' ? 'Checking…' : 'Check for Updates'}</button>
              {updateStatus.phase === 'up-to-date' && <p className="mt-1.5 text-xs text-success">You’re up to date.</p>}
              {updateStatus.phase === 'available' && <p className="mt-1.5 text-xs text-primary">v{updateStatus.version} available</p>}
              {updateStatus.phase === 'error' && (
                <p className="mt-1.5 text-xs text-error">
                  {updateStatus.stage === 'check'
                    ? 'Couldn\u2019t check for updates. Check your connection and try again.'
                    : 'Update installation needs attention.'}
                </p>
              )}
            </div>}
            {INTERNAL_BENCHMARK_BUILD && (
              <p className="rounded-lg border border-primary/25 bg-primary/10 px-3 py-2 text-xs text-on-surface">
                Internal benchmark build. Automatic updates, launch at login, and diagnostic log shipping are disabled.
              </p>
            )}
            {version && <p className="text-center text-xs text-on-surface-variant">Murmur v{version}</p>}
          </SettingsSection>
        </div>
        )}
      </div>
      </div>
      </main>
    </div>
  );
});
