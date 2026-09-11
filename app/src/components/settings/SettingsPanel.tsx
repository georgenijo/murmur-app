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
  DICTATION_KEY_OPTION_GROUPS,
  IDLE_TIMEOUT_OPTIONS,
  LANGUAGE_OPTIONS,
  PASTE_LAST_SHORTCUT_OPTIONS,
  RECORDING_MODE_OPTIONS,
  isFunctionDictationKey,
  pasteLastShortcutConflict,
  pasteLastShortcutLabel,
  type PasteLastShortcut,
  type RecordingMode,
  type Settings,
} from '../../lib/settings';
import { setPasteLastShortcut } from '../../lib/deliveryRecovery';
import { useVocabScan } from '../../lib/hooks/useVocabScan';
import { useAudioInputInventory } from '../../lib/hooks/useAudioInputInventory';
import { useModelRuntimeCatalog } from '../../lib/modelRuntime';
import {
  modelGuidanceSummary,
  modelMemoryWarning,
  useModelHardwareGuidance,
} from '../../lib/modelHardwareGuidance';
import {
  correlatedModelDownloadAttempt,
  modelDownloadLabel,
  modelDownloadPercent,
  type ModelDownloadProgress,
} from '../../lib/modelDownload';
import { useVoiceQuerySettings } from '../../lib/hooks/useVoiceQuerySettings';
import type { QuerySetupStatus } from '../../lib/hooks/useQueryFlow';
import { useTransformModelSettings } from '../../lib/hooks/useTransformModelSettings';
import { VoiceQuerySettings } from './VoiceQuerySettings';
import { TransformModelSettings } from './TransformModelSettings';
import { TranscriptionModelStorage } from './TranscriptionModelStorage';
import type { DictationStatus } from '../../lib/types';
import { UpdateIndicator } from '../UpdateIndicator';
import type { UpdateStatus } from '../../lib/updater';
import {
  downloadModel as downloadModelCommand,
  isNotchPillInstalled,
  requestAccessibilityPermission,
} from '../../lib/dictation';
import { beginCurrentUiTransition, useUiLatencyDestination } from '../../lib/uiLatency';
import { Select } from '../ui/Select';
import { INTERNAL_BENCHMARK_BUILD } from '../../lib/buildFlavor';
import { playSoundCue, type SoundCue } from '../../lib/soundCues';
import { AppOverridesEditor } from './AppOverridesEditor';
import { ModesManager } from './ModesManager';
import { AppearanceSettings } from './AppearanceSettings';
import { PerformanceLab } from './PerformanceLab';
import { MicrophoneInputTest } from './MicrophoneInputTest';
import { MeetingDiarizationSettings } from './MeetingDiarizationSettings';
import { OverlayCalibrationControl } from './OverlayCalibrationControl';
import { SettingsSection } from './SettingsSection';
import { SettingsBranch } from './SettingsBranch';
import { SettingsEditorsWindow, type SettingsEditorTab } from './SettingsEditorsWindow';
import { CustomizationHub, type CustomizationDestination } from './CustomizationHub';
import { useSettingsSurfaceActive } from './SettingsSurfaceContext';
import { SettingsCallout, SettingsDisclosure } from './SettingsLayout';
import {
  DiagnosticsWorkspace,
  type DiagnosticsTab,
} from '../log-viewer/DiagnosticsWorkspace';
import { SettingToggle } from './SettingToggle';
import {
  KeyboardShortcutsSettings,
  type GlobalShortcutId,
  type ShortcutOwnerDestination,
} from './KeyboardShortcutsSettings';

function PasteDelaySlider({ value, onCommit }: { value: number; onCommit: (value: number) => void }) {
  const [draft, setDraft] = useState(value);
  useEffect(() => setDraft(value), [value]);
  return (
    <div className="settings-field">
      <div className="flex items-center justify-between">
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
      <p className="text-xs text-on-surface-variant">Increase this only if paste lands in the wrong window.</p>
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
    <div className="settings-field">
      <div className="flex items-center justify-between">
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
      <p className="text-xs text-on-surface-variant">Off skips silence filtering for the lowest latency. Otherwise, higher keeps more audio.</p>
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
  onDownloadUpdate: () => void;
  onRestartUpdate: () => void;
  onOpenUpdate: () => void;
  updateStatus: UpdateStatus;
  configureError: string | null;
  querySetupStatus?: QuerySetupStatus | null;
  /** External navigation request. The token makes a repeat request for the
   *  page you are already on still register. */
  pageRequest?: SettingsPageRequest | null;
  onLatencyViewChange?: (view: string) => void;
  /** Stable ref avoids re-rendering the warm Settings tree when its surface is hidden. */
  activeRef?: React.RefObject<boolean>;
}

export const SETTINGS_CATEGORIES = [
  { id: 'customize', label: 'Customize', icon: 'customize' },
  { id: 'modes', label: 'Modes', icon: 'modes' },
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
const GENERAL_DETAIL_PAGES = ['shortcuts'] as const;

interface SettingTargetRequest {
  id: string;
}

function settingTargetRequest(id: string | undefined): SettingTargetRequest | null {
  return id === undefined ? null : { id };
}

function customizationDestinationForRequest(
  request: Pick<SettingsPageRequest, 'page' | 'editorTab' | 'target'> | null | undefined,
): CustomizationDestination | null {
  if (!request) return null;
  if (request.page === 'text' && request.editorTab === 'commands') return 'commands';
  if (request.page === 'text' && request.editorTab === 'aliases') return 'text';
  if (request.page === 'text' && !request.editorTab) return 'text';
  if (request.page === 'delivery' && request.target === 'app-overrides') return 'styles';
  if (request.page === 'modes') return 'modes';
  if (resolvePage(request.page) === 'ai-transform') return 'transforms';
  return null;
}

function customizationRoute(destination: CustomizationDestination): {
  page: string;
  editorTab?: SettingsEditorTab;
  target?: string;
} {
  switch (destination) {
    case 'text': return { page: 'text' };
    case 'commands': return { page: 'text', editorTab: 'commands' };
    case 'styles': return { page: 'delivery', target: 'app-overrides' };
    case 'modes': return { page: 'modes' };
    case 'transforms': return { page: 'ai-transform' };
  }
}

/** Coerce a requested page id back to a real page — an unknown id opens the
 *  first page rather than rendering an empty pane. */
export function resolvePage(page: string | undefined): string {
  if (SETTINGS_CATEGORIES.some((category) => category.id === page)) return page as string;
  if (SETTINGS_TOOLS.some((tool) => tool.id === page)) return page as string;
  if ((AI_DETAIL_PAGES as readonly string[]).includes(page ?? '')) return page as string;
  if ((GENERAL_DETAIL_PAGES as readonly string[]).includes(page ?? '')) return page as string;
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
  { page: 'modes', target: 'modes', title: 'Modes', detail: 'Manage reusable behavior for apps and browser sites.', keywords: 'style profiles browser host site rules' },
  { page: 'meetings', target: 'meeting-audio', title: 'Meeting Audio', detail: 'Choose whether source audio is retained.', keywords: 'capture wav keep delete' },
  { page: 'meetings', target: 'meeting-speakers', title: 'Remote Speaker Labels', detail: 'Install and enable local per-speaker meeting labels.', keywords: 'diarization speaker names model local system audio' },
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
  { page: 'shortcuts', target: 'shortcuts', title: 'Keyboard Shortcuts', detail: 'See every global and main-window shortcut.', keywords: 'hotkey key binding command control recording transform query paste correction' },
  { page: 'general', target: 'setup', title: 'Setup Assistant', detail: 'Re-check permissions and model setup.', keywords: 'onboarding microphone accessibility' },
  { page: 'general', target: 'updates', title: 'Updates', detail: 'Check for a newer Murmur release.', keywords: 'version upgrade' },
  { page: 'performance', target: 'performance', title: 'Performance Lab', detail: 'Compare installed models on this Mac.', keywords: 'benchmark speed accuracy' },
  { page: 'diagnostics', target: 'diagnostics', title: 'Diagnostics', detail: 'Inspect events, runs, reports, and transforms.', keywords: 'logs performance compare debugger' },
] as const;

function SettingsNavIcon({ icon }: { icon: string }) {
  const paths: Record<string, React.ReactNode> = {
    customize: <><path d="M4 7h10M18 7h2M4 17h2M10 17h10" /><circle cx="16" cy="7" r="2" /><circle cx="8" cy="17" r="2" /></>,
    general: <><circle cx="12" cy="12" r="3" /><path d="M19 12a7 7 0 1 1-14 0 7 7 0 0 1 14 0Z" /></>,
    recording: <><rect x="9" y="3" width="6" height="11" rx="3" /><path d="M6 11a6 6 0 0 0 12 0M12 17v4M9 21h6" /></>,
    delivery: <><rect x="5" y="4" width="14" height="16" rx="2" /><path d="m9 12 2 2 4-5" /></>,
    modes: <><path d="M5 7h8M17 7h2M5 17h2M11 17h8" /><circle cx="15" cy="7" r="2" /><circle cx="9" cy="17" r="2" /></>,
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

export const SettingsPanel = memo(function SettingsPanel({
  settings,
  onUpdateSettings,
  initialized,
  status,
  onResetStats,
  onRerunSetup,
  accessibilityGranted,
  onCheckForUpdate,
  onDownloadUpdate,
  onRestartUpdate,
  onOpenUpdate,
  updateStatus,
  configureError,
  querySetupStatus = null,
  pageRequest = null,
  onLatencyViewChange,
  activeRef,
}: SettingsPanelProps) {
  const { models: runtimeModels, byName: runtimeByName } = useModelRuntimeCatalog();
  const [activeCat, setActiveCat] = useState<string>(() => resolvePage(pageRequest?.page));
  const [diagnosticsWindowError, setDiagnosticsWindowError] = useState<string | null>(null);
  const [searchQuery, setSearchQuery] = useState('');
  const [editorTab, setEditorTab] = useState<SettingsEditorTab | null>(null);
  const [targetRequest, setTargetRequest] = useState<SettingTargetRequest | null>(() => (
    settingTargetRequest(pageRequest?.target)
  ));
  const [customizationDetail, setCustomizationDetail] = useState<CustomizationDestination | null>(() => (
    customizationDestinationForRequest(pageRequest)
  ));
  const [editorBackToCustomization, setEditorBackToCustomization] = useState(() => (
    Boolean(pageRequest?.editorTab && customizationDestinationForRequest(pageRequest))
  ));
  const [customizationReturnFocus, setCustomizationReturnFocus] = useState<CustomizationDestination | null>(null);
  const [shortcutReturnFocus, setShortcutReturnFocus] = useState<GlobalShortcutId | null>(null);
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
    setTargetRequest(settingTargetRequest(pageRequest.target));
    const destination = customizationDestinationForRequest(pageRequest);
    setCustomizationDetail(destination);
    setCustomizationReturnFocus(destination);
    setEditorBackToCustomization(Boolean(destination && pageRequest.editorTab));
    setShortcutReturnFocus(null);
  }, [pageRequest]);
  const [version, setVersion] = useState('');
  const [confirmReset, setConfirmReset] = useState(false);
  const contentRef = useRef<HTMLDivElement>(null);
  const generalShortcutsButtonRef = useRef<HTMLButtonElement>(null);
  const restoreGeneralShortcutsFocusRef = useRef(false);
  const confirmResetTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => { void getVersion().then(setVersion); }, []);
  useLayoutEffect(() => {
    const content = contentRef.current;
    if (!content) return;
    if (!targetRequest) {
      content.scrollTo({ top: 0 });
      return;
    }
    const target = content.querySelector<HTMLElement>(`[data-setting-target="${targetRequest.id}"]`);
    if (!target) return;
    if (target instanceof HTMLDetailsElement) target.open = true;
    target.scrollIntoView({ block: 'center' });
    target.classList.add('settings-target-flash');
    const timeout = window.setTimeout(() => target.classList.remove('settings-target-flash'), 1800);
    return () => {
      window.clearTimeout(timeout);
      target.classList.remove('settings-target-flash');
    };
  }, [activeCat, editorTab, targetRequest]);
  useLayoutEffect(() => {
    if (activeCat !== 'shortcuts' || shortcutReturnFocus === null) return;
    contentRef.current
      ?.querySelector<HTMLButtonElement>(`[data-shortcut-id="${shortcutReturnFocus}"] button`)
      ?.focus();
  }, [activeCat, shortcutReturnFocus]);
  useLayoutEffect(() => {
    if (activeCat !== 'general' || !restoreGeneralShortcutsFocusRef.current) return;
    restoreGeneralShortcutsFocusRef.current = false;
    generalShortcutsButtonRef.current?.focus();
  }, [activeCat]);
  useEffect(() => () => {
    if (confirmResetTimeoutRef.current) clearTimeout(confirmResetTimeoutRef.current);
  }, []);

  const requestAccessibility = () => { void requestAccessibilityPermission(); };
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
    setTargetRequest(null);
    setEditorTab(tab);
    setEditorBackToCustomization(false);
  }, []);
  const closeEditor = useCallback(() => {
    beginCurrentUiTransition('settings.text', 'programmatic');
    setTargetRequest(null);
    setEditorTab(null);
    setEditorBackToCustomization(false);
  }, []);

  const openCustomizationDestination = useCallback((destination: CustomizationDestination) => {
    const route = customizationRoute(destination);
    beginCurrentUiTransition(route.editorTab
      ? `settings.text.editor.${route.editorTab}`
      : `settings.${route.page}`, 'pointer');
    setActiveCat(route.page);
    setEditorTab(route.editorTab ?? null);
    setTargetRequest(settingTargetRequest(route.target));
    setSearchQuery('');
    setCustomizationDetail(destination);
    setCustomizationReturnFocus(destination);
    setEditorBackToCustomization(Boolean(route.editorTab));
  }, []);

  const returnToCustomization = useCallback(() => {
    beginCurrentUiTransition('settings.customize', 'programmatic');
    setCustomizationReturnFocus(customizationDetail);
    setCustomizationDetail(null);
    setActiveCat('customize');
    setEditorTab(null);
    setEditorBackToCustomization(false);
    setTargetRequest(null);
    setSearchQuery('');
  }, [customizationDetail]);
  const popOutDiagnostics = useCallback(async (tab: DiagnosticsTab) => {
    setDiagnosticsWindowError(null);
    try {
      await invoke('show_diagnostics_window', { tab });
    } catch {
      setDiagnosticsWindowError('Diagnostics could not be opened in a separate window.');
    }
  }, []);

  const selectedRuntime = runtimeByName.get(settings.model);
  const modelHardwareGuidance = useModelHardwareGuidance();
  const selectedModelMemoryWarning = modelHardwareGuidance
    ? modelMemoryWarning(modelHardwareGuidance, settings.model)
    : null;
  const modelAvailable = selectedRuntime ? selectedRuntime.installState === 'installed' : null;
  const [modelDownload, setModelDownload] = useState<
    | { phase: 'idle' }
    | { phase: 'downloading'; progress: ModelDownloadProgress }
    | { phase: 'error'; message: string }
  >({ phase: 'idle' });
  const downloadUnlistenRef = useRef<(() => void) | null>(null);
  const downloadModelRef = useRef<string | null>(null);
  const downloadAttemptRef = useRef<number | null>(null);

  useEffect(() => {
    setModelDownload({ phase: 'idle' });
    downloadModelRef.current = null;
    downloadAttemptRef.current = null;
  }, [settings.model]);
  useEffect(() => () => {
    downloadUnlistenRef.current?.();
    downloadUnlistenRef.current = null;
  }, []);

  const downloadModel = useCallback(async () => {
    const modelName = settings.model;
    downloadModelRef.current = modelName;
    downloadAttemptRef.current = null;
    setModelDownload({ phase: 'downloading', progress: { modelName, received: 0, total: 0, phase: 'downloading' } });
    let unlisten: (() => void) | null = null;
    try {
      unlisten = await listen<ModelDownloadProgress>('download-progress', (event) => {
        const progress = event.payload;
        if (downloadModelRef.current !== modelName) return;
        const attemptId = correlatedModelDownloadAttempt(
          progress,
          modelName,
          downloadAttemptRef.current,
        );
        if (attemptId === undefined) return;
        downloadAttemptRef.current = attemptId;
        setModelDownload({ phase: 'downloading', progress });
      });
      downloadUnlistenRef.current = unlisten;
      await downloadModelCommand(modelName);
      unlisten();
      downloadUnlistenRef.current = null;
      if (downloadModelRef.current === modelName) setModelDownload({ phase: 'idle' });
    } catch (error) {
      unlisten?.();
      downloadUnlistenRef.current = null;
      if (downloadModelRef.current === modelName) setModelDownload({ phase: 'error', message: String(error) });
    } finally {
      if (downloadModelRef.current === modelName) {
        downloadModelRef.current = null;
        downloadAttemptRef.current = null;
      }
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
    const onFocus = () => {
      if (activeRef && !activeRef.current) return;
      refresh();
    };
    window.addEventListener('focus', onFocus);
    return () => {
      cancelled = true;
      window.removeEventListener('focus', onFocus);
    };
  }, [activeRef]);

  // ---- Transform model & Voice Query settings (extracted; see
  // lib/hooks/useTransformModelSettings.ts and lib/hooks/useVoiceQuerySettings.ts) ----
  const transformVm = useTransformModelSettings({
    settings,
    onUpdateSettings,
    activePage: activeCat === 'ai' || activeCat === 'ai-transform' ? activeCat : null,
  });

  const [pasteLastShortcutError, setPasteLastShortcutError] = useState<string | null>(null);
  const [pasteLastShortcutBusy, setPasteLastShortcutBusy] = useState(false);

  const updatePasteLastShortcut = async (next: PasteLastShortcut | null) => {
    setPasteLastShortcutError(null);
    const conflict = pasteLastShortcutConflict(next, settings);
    if (conflict) {
      setPasteLastShortcutError(conflict);
      return;
    }
    setPasteLastShortcutBusy(true);
    try {
      // The native command changes the active chord atomically. Persist only
      // after it succeeds, so a registration/permission failure leaves both
      // the listener and controlled picker on the last working value.
      await setPasteLastShortcut(next);
      onUpdateSettings({ pasteLastShortcut: next });
    } catch (error) {
      setPasteLastShortcutError(String(error));
    } finally {
      setPasteLastShortcutBusy(false);
    }
  };

  const voiceQueryVm = useVoiceQuerySettings({
    settings,
    onUpdateSettings,
    active: activeCat === 'ai-query',
  });

  const isRecording = status !== 'idle';
  const isDoubleTap = settings.recordingMode === 'double_tap';
  const isBoth = settings.recordingMode === 'both';
  const keyLabel = isBoth ? 'Trigger Key' : isDoubleTap ? 'Double-Tap Key' : 'Hold Key';
  const keyHelp = isBoth
    ? 'Hold to record, or double-tap to start and single-tap to stop.'
    : isDoubleTap ? 'Double-tap to start and single-tap to stop.' : 'Hold to start and release to stop.';

  const missingDevice = !settings.smartAutoMicrophoneEnabled
    && settings.microphone !== DEFAULT_SETTINGS.microphone
    && audioInventory?.status === 'available'
    && !selectedDeviceExists(settings.microphone, audioDevices);
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

  const openPage = (page: string, trigger: 'pointer' | 'programmatic' = 'pointer') => {
    beginCurrentUiTransition(`settings.${page}`, trigger);
    setActiveCat(page);
    setEditorTab(null);
    setSearchQuery('');
    setTargetRequest(null);
    setCustomizationDetail(null);
    setCustomizationReturnFocus(null);
    setEditorBackToCustomization(false);
    setShortcutReturnFocus(null);
  };

  const openShortcutOwner = (destination: ShortcutOwnerDestination) => {
    beginCurrentUiTransition(`settings.${destination.page}`, 'pointer');
    setActiveCat(destination.page);
    setEditorTab(null);
    setSearchQuery('');
    setTargetRequest(settingTargetRequest(destination.target));
    setCustomizationDetail(null);
    setCustomizationReturnFocus(null);
    setEditorBackToCustomization(false);
    setShortcutReturnFocus(destination.shortcutId);
  };

  const returnToShortcuts = () => {
    beginCurrentUiTransition('settings.shortcuts', 'programmatic');
    setActiveCat('shortcuts');
    setEditorTab(null);
    setSearchQuery('');
    setTargetRequest(null);
    setCustomizationDetail(null);
    setCustomizationReturnFocus(null);
    setEditorBackToCustomization(false);
  };

  const returnToGeneral = () => {
    restoreGeneralShortcutsFocusRef.current = true;
    openPage('general', 'programmatic');
  };

  const navPageIsActive = (page: string) => (
    page === 'ai'
      ? activeCat === 'ai' || activeCat.startsWith('ai-')
      : page === 'general' ? activeCat === 'general' || activeCat === 'shortcuts' : activeCat === page
  );

  const searchPageLabel = (page: string) => {
    if (page.startsWith('ai-')) return 'AI & Models';
    return SETTINGS_CATEGORIES.find((category) => category.id === page)?.label
      ?? SETTINGS_TOOLS.find((tool) => tool.id === page)?.label
      ?? page;
  };
  return (
    <div className="settings-workspace flex min-h-0 flex-1 overflow-hidden bg-background text-on-surface">
      <aside className="settings-sidebar flex min-h-0 w-[210px] shrink-0 flex-col overflow-hidden px-3 pb-3 pt-2 max-[760px]:w-[184px]">
        <label className="settings-search relative mb-3 block w-full min-w-0 shrink-0">
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
            className="h-9 w-full pl-9 pr-8 text-[13px] text-on-surface outline-none placeholder:text-on-surface-variant"
          />
          {searchQuery && (
            <button type="button" onClick={() => setSearchQuery('')} aria-label="Clear settings search" className="absolute right-2 top-1/2 grid h-6 w-6 -translate-y-1/2 place-items-center rounded-full text-on-surface-variant hover:bg-surface-container">×</button>
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
                className={`settings-nav-item flex min-h-8 w-full items-center gap-2.5 px-3 text-left text-[13px] transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-primary ${selected ? 'text-on-surface' : 'text-on-surface-variant hover:bg-surface-container hover:text-on-surface'}`}
              >
                <SettingsNavIcon icon={category.icon} />
                <span className="min-w-0 truncate">{category.label}</span>
              </button>
            );
          })}
        </nav>
        <div className="mt-auto border-t border-outline-variant/20 pt-3">
          <p className="settings-tools-eyebrow mb-1 px-3">Tools</p>
          {SETTINGS_TOOLS.map((tool) => {
            const selected = activeCat === tool.id;
            return (
              <button
                key={tool.id}
                type="button"
                aria-current={selected ? 'page' : undefined}
                onClick={() => openPage(tool.id)}
                className={`settings-nav-item flex min-h-8 w-full items-center gap-2.5 px-3 text-left text-[13px] transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-primary ${selected ? 'text-on-surface' : 'text-on-surface-variant hover:bg-surface-container hover:text-on-surface'}`}
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
              onBack={editorBackToCustomization ? returnToCustomization : closeEditor}
              backLabel={editorBackToCustomization ? 'Back to Customize' : undefined}
            />
          ) : searchQuery ? (
            <section aria-label="Settings search results">
              <h1 className="settings-page-title">Search</h1>
              <p className="settings-page-subtitle mb-4">Jump directly to a setting or tool.</p>
              <p className="mb-3 text-[11px] font-bold uppercase tracking-[0.12em] text-on-surface-variant">
                {searchResults.length} {searchResults.length === 1 ? 'result' : 'results'}
              </p>
              {searchResults.length === 0 ? (
                <div className="settings-card px-4 py-10 text-center text-sm text-on-surface-variant">
                  No settings match “{searchQuery}”.
                </div>
              ) : (
                <div className="settings-card overflow-hidden">
                  {searchResults.map((result) => (
                    <button
                      key={`${result.page}-${result.title}`}
                      type="button"
                      onClick={() => {
                        beginCurrentUiTransition(`settings.${result.page}`, 'pointer');
                        setActiveCat(result.page);
                        setEditorTab(null);
                        setTargetRequest(settingTargetRequest(result.target));
                        setSearchQuery('');
                        setCustomizationDetail(null);
                        setCustomizationReturnFocus(null);
                        setEditorBackToCustomization(false);
                        setShortcutReturnFocus(null);
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
          {customizationDetail && !editorTab && (
            <button
              type="button"
              onClick={returnToCustomization}
              className="settings-back-btn mb-4 focus:outline-none focus-visible:ring-2 focus-visible:ring-primary"
            >
              <span aria-hidden="true">‹</span> Back to Customize
            </button>
          )}
          {activeCat === 'shortcuts' && (
            <button
              type="button"
              onClick={returnToGeneral}
              className="settings-back-btn mb-4"
            >
              <span aria-hidden="true">‹</span> General
            </button>
          )}
          {shortcutReturnFocus !== null && activeCat !== 'shortcuts' && (
            <button
              type="button"
              onClick={returnToShortcuts}
              className="settings-back-btn mb-4"
            >
              <span aria-hidden="true">‹</span> Keyboard Shortcuts
            </button>
          )}
          {(AI_DETAIL_PAGES as readonly string[]).includes(activeCat) && !customizationDetail && shortcutReturnFocus === null && (
            <button
              type="button"
              onClick={() => openPage('ai', 'programmatic')}
              className="settings-back-btn mb-4"
            >
              <span aria-hidden="true">‹</span> AI &amp; Models
            </button>
          )}
          {activeCat === 'customize' && (
            <CustomizationHub
              focusDestination={customizationReturnFocus}
              onOpen={openCustomizationDestination}
            />
          )}
          <KeyboardShortcutsSettings
            settings={settings}
            activePage={activeCat}
            onOpenOwner={openShortcutOwner}
          />
          <SettingsSection pageId="recording" activePage={activeCat} title="Recording" subtitle="Microphone, voice detection, shortcuts, and automatic stopping">
            <div data-setting-target="microphone" className="settings-stack rounded-lg transition-shadow [&.settings-target-flash]:ring-2 [&.settings-target-flash]:ring-primary/40">
              <MicrophoneInputTest
                microphone={settings.microphone}
                devices={audioDevices}
                defaultInputId={audioInventory?.defaultInputId ?? null}
                active={activeCat === 'recording'}
                ready={initialized}
                vadSensitivity={previewVadSensitivity}
                dictationBusy={isRecording}
                missingDevice={missingDevice}
                inventoryAvailable={audioInventory?.status === 'available'}
                inventoryLoading={audioInventoryState.loading}
                onChange={(microphone) => onUpdateSettings({ microphone })}
                smartAuto={settings}
                lidState={audioInventory?.lidState ?? 'unknown'}
                onSmartAutoChange={onUpdateSettings}
              />
              {audioInventoryState.error && (
                <p role="alert" className="text-xs text-primary">{audioInventoryState.error} Close and reopen Settings if it does not refresh.</p>
              )}
            </div>
            <div data-setting-target="voice-detection" className="settings-field rounded-lg transition-shadow [&.settings-target-flash]:ring-2 [&.settings-target-flash]:ring-primary/40">
              <p className="text-sm font-medium text-on-surface">Voice Detection</p>
              <VadSensitivitySlider
                value={settings.vadSensitivity}
                onPreview={setPreviewVadSensitivity}
                onCommit={(vadSensitivity) => onUpdateSettings({ vadSensitivity })}
              />
            </div>
            <div data-setting-target="recording-trigger" className="settings-field rounded-lg transition-shadow [&.settings-target-flash]:ring-2 [&.settings-target-flash]:ring-primary/40">
              <p className="text-sm font-medium text-on-surface">Recording Trigger</p>
              <div className="settings-segmented">
                {RECORDING_MODE_OPTIONS.map((option) => (
                  <button key={option.value} type="button" disabled={isRecording} data-selected={settings.recordingMode === option.value} onClick={() => onUpdateSettings({ recordingMode: option.value as RecordingMode })} className="settings-segmented-btn">{option.label}</button>
                ))}
              </div>
              {isRecording && <p className="text-xs text-primary">Stop recording before changing mode.</p>}
            </div>
            {accessibilityGranted === false && (
              <div>
                <div className="flex items-center gap-2 rounded-lg border border-primary/30 bg-primary/10 px-3 py-2 text-xs text-on-surface">
                  <span>Accessibility permission is required for keyboard detection.</span>
                  <button type="button" onClick={requestAccessibility} className="ml-auto underline">Grant</button>
                </div>
              </div>
            )}
            <div data-setting-target="trigger-key" className="settings-field rounded-lg transition-shadow [&.settings-target-flash]:ring-2 [&.settings-target-flash]:ring-primary/40">
              <label className="block text-sm font-medium text-on-surface">{keyLabel}</label>
              <Select
                value={settings.doubleTapKey}
                onChange={(doubleTapKey) => onUpdateSettings({ doubleTapKey })}
                disabled={isRecording}
                items={DICTATION_KEY_OPTION_GROUPS}
                aria-label="Dictation trigger key"
              />
              <p className="text-xs text-on-surface-variant">{keyHelp}</p>
              {isFunctionDictationKey(settings.doubleTapKey) && (
                <p className="text-xs text-on-surface-variant">
                  On Apple keyboards, F1–F12 may require Fn or the macOS setting that uses function keys as standard keys. Murmur observes the key without suppressing its normal system or app action.
                </p>
              )}
            </div>
            {(isDoubleTap || isBoth) && <SettingToggle targetId="hotkey-feedback" title="Hotkey Timing Feedback" description="Flash the overlay when a tap misses the double-tap window." checked={settings.hotkeyMissFeedback} onChange={() => onUpdateSettings({ hotkeyMissFeedback: !settings.hotkeyMissFeedback })} />}
            <div data-setting-target="sound-cues" className="settings-stack rounded-lg transition-shadow [&.settings-target-flash]:ring-2 [&.settings-target-flash]:ring-primary/40">
              <SettingToggle
                title="Sound Cues"
                description="Play local feedback when recording starts, stops, succeeds, or fails."
                checked={settings.soundCuesEnabled}
                onChange={() => onUpdateSettings({ soundCuesEnabled: !settings.soundCuesEnabled })}
              />
              <SettingsBranch open={settings.soundCuesEnabled}>
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
              </SettingsBranch>
            </div>
            <div data-setting-target="stop-on-silence" className="settings-field rounded-lg transition-shadow [&.settings-target-flash]:ring-2 [&.settings-target-flash]:ring-primary/40">
              <label className="block text-sm font-medium text-on-surface">Stop on Silence</label>
              <div className="settings-segmented">
                {AUTO_STOP_SILENCE_OPTIONS.map((option) => (
                  <button
                    key={option.value}
                    type="button"
                    // Locked mid-recording like the sibling trigger controls:
                    // the detector reads this value live, so a change now
                    // would retune the recording already in flight.
                    disabled={isRecording}
                    aria-pressed={settings.autoStopSilenceMs === option.value}
                    data-selected={settings.autoStopSilenceMs === option.value}
                    onClick={() => onUpdateSettings({ autoStopSilenceMs: option.value })}
                    className="settings-segmented-btn"
                  >
                    {option.label}
                  </button>
                ))}
              </div>
              <p className="text-xs text-on-surface-variant">
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
                <span className="mt-0.5 block truncate text-xs text-on-surface-variant">Qwen2.5 1.5B · {transformVm.transformModel?.state === 'ready' ? 'Ready on-device' : 'Model setup required'}</span>
              </span>
              <span className="text-xs font-semibold text-on-surface-variant">Configure <span aria-hidden="true">›</span></span>
            </button>
          </SettingsSection>

          <VoiceQuerySettings
            settings={settings}
            onUpdateSettings={onUpdateSettings}
            activePage={activeCat}
            accessibilityGranted={accessibilityGranted}
            onRequestAccessibility={requestAccessibility}
            vm={voiceQueryVm}
            setupStatus={querySetupStatus}
          />

          <TransformModelSettings
            settings={settings}
            onUpdateSettings={onUpdateSettings}
            activePage={activeCat}
            accessibilityGranted={accessibilityGranted}
            onRequestAccessibility={requestAccessibility}
            onOpenEditor={openEditor}
            vm={transformVm}
          />

          <SettingsSection pageId="ai-transcription" activePage={activeCat} title="Speech-to-Text" subtitle="Recognition model, language, and memory lifecycle">
            <div data-setting-target="transcription-model" className="settings-field rounded-lg transition-shadow [&.settings-target-flash]:ring-2 [&.settings-target-flash]:ring-primary/40">
              <label className="block text-sm font-medium text-on-surface">Transcription Model</label>
              <Select
                value={settings.model}
                onChange={(model) => onUpdateSettings({ model })}
                disabled={isRecording}
                items={AVAILABLE_MODEL_OPTIONS.map((model) => ({
                  value: model.value,
                  label: `${model.label} (${model.size})`,
                  badge: modelHardwareGuidance?.warnedModels.includes(model.value)
                    ? 'Higher memory'
                    : modelHardwareGuidance?.recommendedModel === model.value
                      ? 'Recommended'
                      : undefined,
                  badgeTone: modelHardwareGuidance?.warnedModels.includes(model.value)
                    ? 'warning' as const
                    : 'accent' as const,
                }))}
              />
              <p className="text-xs text-on-surface-variant">
                {modelHardwareGuidance
                  ? modelGuidanceSummary(modelHardwareGuidance)
                  : 'Larger models can be more accurate but use more storage and memory.'}
              </p>
              {selectedModelMemoryWarning && (
                <div role="status" className="rounded-lg border border-warning/30 bg-warning/10 px-3 py-2 text-xs text-on-surface">
                  {selectedModelMemoryWarning}
                </div>
              )}
              {selectedRuntime && <p className="text-xs text-on-surface-variant" data-testid="model-runtime-status">{selectedRuntime.label}: {selectedRuntime.backend} / {selectedRuntime.accelerator} / {selectedRuntime.size} · {selectedRuntime.installState} · {selectedRuntime.lifecycleState}</p>}
              {isRecording && <p className="text-xs text-primary">Stop recording before changing model.</p>}
              {modelAvailable === false && modelDownload.phase === 'idle' && (
                <div className="flex items-center rounded-lg border border-primary/30 bg-primary/10 px-3 py-2 text-xs text-on-surface">
                  <span>Model not downloaded</span><button type="button" onClick={() => void downloadModel()} className="ml-auto underline">Download</button>
                </div>
              )}
              {modelDownload.phase === 'downloading' && (
                <div>
                  <div className="mb-1 flex justify-between text-xs text-on-surface-variant"><span>{modelDownloadLabel(modelDownload.progress)}</span><span>{downloadProgress === null ? 'Working…' : `${downloadProgress}%`}</span></div>
                  <div className="h-1.5 overflow-hidden rounded-full bg-surface-container-highest"><div role="progressbar" aria-valuenow={downloadProgress ?? undefined} aria-valuemin={0} aria-valuemax={100} aria-valuetext={downloadProgress === null ? 'Model installation in progress' : `Download progress: ${downloadProgress} percent`} className={`h-full rounded-full bg-primary ${downloadProgress === null ? 'model-download-indeterminate' : 'transition-all duration-200'}`} style={downloadProgress === null ? undefined : { width: `${downloadProgress}%` }} /></div>
                </div>
              )}
              {modelDownload.phase === 'error' && <div className="flex items-center rounded-lg border border-error/30 bg-error/10 px-3 py-2 text-xs text-error"><span>{modelDownload.message}</span><button type="button" onClick={() => void downloadModel()} className="ml-auto underline">Retry</button></div>}
            </div>
            <TranscriptionModelStorage models={runtimeModels} selectedModel={settings.model} busy={isRecording} />
            <div data-setting-target="language" className="settings-field rounded-lg transition-shadow [&.settings-target-flash]:ring-2 [&.settings-target-flash]:ring-primary/40">
              <label className="block text-sm font-medium text-on-surface">Language</label>
              <Select value={settings.language} onChange={(language) => onUpdateSettings({ language })} disabled={isRecording || englishOnly} items={LANGUAGE_OPTIONS} />
              <p className="text-xs text-on-surface-variant">{englishOnly ? 'This model is English-only. Choose Whisper Large Turbo for other languages.' : 'Auto Detect lets Whisper identify the language for each recording.'}</p>
            </div>
            <div className="settings-field">
              <label className="block text-sm font-medium text-on-surface">Release Model After Inactivity</label>
              <Select value={String(settings.idleTimeoutMinutes)} onChange={(value) => onUpdateSettings({ idleTimeoutMinutes: Number(value) })} disabled={isRecording} items={IDLE_TIMEOUT_OPTIONS.map((option) => ({ value: String(option.value), label: option.label }))} />
              <p className="text-xs text-on-surface-variant">Free memory by unloading an idle model; choose Never to keep it ready.</p>
            </div>
          </SettingsSection>

          <SettingsSection pageId="text" activePage={activeCat} title="Text & Vocabulary" subtitle="Cleanup, preferred terms, structured writing, and knowledge">
            <SettingToggle targetId="punctuation" title="Automatic Punctuation" label="Smart punctuation" description="Add periods, commas, and capitalization to transcriptions." checked={settings.smartPunctuation} onChange={() => onUpdateSettings({ smartPunctuation: !settings.smartPunctuation })} />
            <SettingToggle targetId="cleanup" title="Transcript Cleanup" description="Remove filler and tidy spacing before delivery." checked={settings.cleanupEnabled} onChange={() => onUpdateSettings({ cleanupEnabled: !settings.cleanupEnabled })} />
            <SettingsBranch open={settings.cleanupEnabled}>
              <SettingToggle title="Remove filler words" description="Remove filler tokens such as um and uh." checked={settings.cleanupRemoveFiller} onChange={() => onUpdateSettings({ cleanupRemoveFiller: !settings.cleanupRemoveFiller })} />
              <SettingToggle title="Capitalize sentences" description="Capitalize detected sentence starts." checked={settings.cleanupCapitalize} onChange={() => onUpdateSettings({ cleanupCapitalize: !settings.cleanupCapitalize })} />
            </SettingsBranch>
            <div data-setting-target="text-editors" className="grid gap-2 rounded-lg transition-shadow sm:grid-cols-2 [&.settings-target-flash]:ring-2 [&.settings-target-flash]:ring-primary/40">
              {([
                ['vocabulary', 'Vocabulary', 'Review identifiers retained from project scans.'],
                ['aliases', 'Aliases', 'Map spoken variants to canonical spellings.'],
                ['knowledge', 'Knowledge', 'Manage corrections, terms, snippets, and transforms.'],
                ['commands', 'Voice Commands', 'Create exact spoken replacements and snippets.'],
              ] as const).map(([tab, title, detail]) => (
                <button key={tab} type="button" onClick={() => openEditor(tab)} className="settings-card flex items-center gap-3 px-3 py-3 text-left transition-shadow hover:shadow-[var(--settings-shadow-2)]">
                  <span className="min-w-0 flex-1">
                    <span className="block text-sm font-semibold text-on-surface">{title}</span>
                    <span className="mt-0.5 block text-[11px] leading-relaxed text-on-surface-variant">{detail}</span>
                  </span>
                  <span className="text-on-surface-variant" aria-hidden="true">›</span>
                </button>
              ))}
            </div>
            <SettingsDisclosure title="Advanced" layout="rows">
              <SettingToggle title="Developer Terms" description="Make built-in development terms and an optional project scan available only to apps configured as Code / technical or with Local IDE project context." checked={settings.codeVocabEnabled} onChange={() => onUpdateSettings({ codeVocabEnabled: !settings.codeVocabEnabled })} />
              <SettingsBranch open={settings.codeVocabEnabled}>
                <div className="settings-field">
                  <p className="break-all rounded-lg border border-outline-variant/30 bg-surface-container-lowest px-3 py-2 text-xs text-on-surface">{settings.codeVocabFolder || 'No folder — built-in developer terms only'}</p>
                  <button type="button" onClick={() => openEditor('scan')} className="rounded-lg bg-surface-container-high px-3 py-2 text-xs font-semibold text-on-surface hover:text-primary">Manage Project Scan</button>
                  <p className="text-xs text-on-surface-variant">The selected folder is scanned locally; dependency and build folders are skipped. Unconfigured apps keep ordinary prose vocabulary.</p>
                </div>
              </SettingsBranch>
              <SettingToggle title="Apply Preferred Spellings" label="Smart correction" description="Apply names, terms, and developer vocabulary after recognition on every model." checked={settings.correctionEnabled} onChange={() => onUpdateSettings({ correctionEnabled: !settings.correctionEnabled })} />
              <SettingsBranch open={settings.correctionEnabled}>
                <SettingToggle title="Correct Close Mishearings" label="Sounds-like matching" description="Recover close mishearings near your vocabulary; disable if you see unwanted swaps." checked={settings.correctionFuzzy} onChange={() => onUpdateSettings({ correctionFuzzy: !settings.correctionFuzzy })} />
              </SettingsBranch>
              <SettingToggle title="Structured Writing" label="Smart formatting" description="Apply explicitly spoken lists, symbols, punctuation, and same-utterance corrections locally." checked={settings.smartFormattingEnabled} onChange={() => onUpdateSettings({ smartFormattingEnabled: !settings.smartFormattingEnabled })} />
              <SettingToggle title="Spoken Formatting" label="Voice commands" description="Use spoken tokens such as “new line,” “period,” or “scratch that” before delivery." checked={settings.voiceCommandsEnabled} onChange={() => onUpdateSettings({ voiceCommandsEnabled: !settings.voiceCommandsEnabled })} />
            </SettingsDisclosure>
          </SettingsSection>

          <SettingsSection pageId="delivery" activePage={activeCat} title="Delivery" subtitle="Choose what happens after transcription finishes">
            <SettingsCallout title="Always copied to clipboard">
              Auto-paste and file output happen afterward, so the finished text remains recoverable.
            </SettingsCallout>
            <SettingToggle targetId="auto-paste" title="Auto-Paste" label="Auto paste" description={autoPasteDeliveryDescription(settings)} checked={autoPasteOn} disabled={saveToFile} onChange={() => onUpdateSettings({ autoPaste: !settings.autoPaste })} />
            <SettingsBranch open={settings.autoPaste}>
              {saveToFile && <p role="status" data-tone="accent" className="settings-callout text-xs text-on-surface">Auto-paste is paused; the stored preference remains on.</p>}
              {!saveToFile && accessibilityGranted === false && (
                <div className="flex items-center gap-2 text-xs text-primary">
                  <span>Accessibility permission is required to paste into the active app.</span>
                  <button type="button" onClick={requestAccessibility} className="ml-auto underline">Grant</button>
                </div>
              )}
              <div
                aria-hidden={settings.autoPaste && !autoPasteOn}
                className={settings.autoPaste && !autoPasteOn ? 'hidden' : undefined}
              >
                <PasteDelaySlider value={settings.autoPasteDelayMs} onCommit={(autoPasteDelayMs) => onUpdateSettings({ autoPasteDelayMs })} />
              </div>
            </SettingsBranch>
            <div data-setting-target="paste-last-shortcut" className="settings-field rounded-lg transition-shadow [&.settings-target-flash]:ring-2 [&.settings-target-flash]:ring-primary/40">
              <p className="block text-sm font-medium text-on-surface">
                Paste Last Shortcut
              </p>
              <Select
                aria-label="Paste Last Shortcut"
                value={settings.pasteLastShortcut ?? 'disabled'}
                disabled={pasteLastShortcutBusy}
                onChange={(value) => {
                  void updatePasteLastShortcut(
                    value === 'disabled' ? null : value as PasteLastShortcut,
                  );
                }}
                items={PASTE_LAST_SHORTCUT_OPTIONS}
              />
              <p className="text-xs leading-relaxed text-on-surface-variant">
                {settings.pasteLastShortcut === null
                  ? 'Disabled. Choose a shortcut to retry the latest completed delivery from anywhere.'
                  : `Active: ${pasteLastShortcutLabel(settings.pasteLastShortcut)}. Retried text uses the same secure target checks.`}
              </p>
              {pasteLastShortcutError && <p role="alert" className="text-xs text-error">{pasteLastShortcutError}</p>}
              {accessibilityGranted === false && settings.pasteLastShortcut !== null && (
                <div className="flex items-center gap-2 text-xs text-primary">
                  <span>Accessibility permission is required for the global Paste Last shortcut.</span>
                  <button type="button" onClick={requestAccessibility} className="underline">Grant</button>
                </div>
              )}
            </div>
            <div data-setting-target="file-output" className="settings-stack rounded-lg transition-shadow [&.settings-target-flash]:ring-2 [&.settings-target-flash]:ring-primary/40">
              <SettingToggle title="Save Transcript to File" description="Write each completed transcription to a .txt file." checked={settings.saveTranscript} onChange={() => onUpdateSettings({ saveTranscript: !settings.saveTranscript })} />
              <SettingToggle title="Save Audio to File" description="Write each recording to a .wav file." checked={settings.saveAudio} onChange={() => onUpdateSettings({ saveAudio: !settings.saveAudio })} />
              <SettingsBranch open={saveToFile}>
                <div className="settings-field">
                  <p className="text-xs text-on-surface-variant">Shared Output Folder</p>
                  <p className="break-all rounded-lg border border-outline-variant/30 bg-surface-container-lowest px-3 py-2 text-xs text-on-surface">{settings.outputDir || 'Documents/Murmur (default)'}</p>
                  <div className="flex gap-3"><button type="button" onClick={() => void chooseOutputFolder()} className="text-xs font-medium text-on-surface-variant underline hover:text-primary">Choose Folder</button>{settings.outputDir && <button type="button" onClick={() => onUpdateSettings({ outputDir: '' })} className="text-xs font-medium text-on-surface-variant underline hover:text-primary">Reset to default</button>}</div>
                  <p className="text-xs text-on-surface-variant">Used by saved transcripts and audio. {fileOutputDeliveryDescription(settings)}</p>
                </div>
              </SettingsBranch>
            </div>
            <SettingToggle
              targetId="history"
              title="Save Transcription History"
              description="Keep completed microphone and file transcripts in Murmur on this Mac. Turning this off affects new transcripts; existing history remains until you clear it."
              checked={settings.retainHistory}
              onChange={() => onUpdateSettings({ retainHistory: !settings.retainHistory })}
            />
            {notchPillInstalled && <SettingToggle title="Mirror Captions to NotchPill" description="Show your latest dictation in the NotchPill notch overlay. Stays on this Mac — only the final text is written locally." checked={settings.mirrorToNotchPill} onChange={() => onUpdateSettings({ mirrorToNotchPill: !settings.mirrorToNotchPill })} />}
            <SettingsDisclosure title="Advanced" description="Override delivery and writing behavior for the frontmost macOS app." layout="stack" targetId="app-overrides">
              <AppOverridesEditor profiles={settings.appProfiles} onChange={(appProfiles) => onUpdateSettings({ appProfiles })} />
            </SettingsDisclosure>
          </SettingsSection>

          <SettingsSection card={false} pageId="modes" activePage={activeCat} title="Modes" subtitle="Reusable behavior for apps and browser sites">
            <div data-setting-target="modes">
              <ModesManager
                modes={settings.modes}
                profiles={settings.appProfiles}
                siteLookupEnabled={settings.siteModeLookupEnabled}
                siteRules={settings.browserSiteRules}
                onChange={onUpdateSettings}
              />
            </div>
          </SettingsSection>

          <SettingsSection pageId="meetings" activePage={activeCat} title="Meetings" subtitle="Local meeting transcript and audio retention">
            <SettingsCallout title="Stored separately from dictation">
              Meeting transcripts use the crash-safe local store and never appear in dictation history.
            </SettingsCallout>
            <SettingToggle
              targetId="meeting-audio"
              title="Keep Meeting Audio"
              description="Off by default. When off, each private chunk WAV is deleted after its transcript commits."
              checked={settings.meetingRetainAudio}
              onChange={() => onUpdateSettings({ meetingRetainAudio: !settings.meetingRetainAudio })}
            />
            <SettingToggle
              targetId="meeting-echo-cancellation"
              title="Reduce Speaker Echo"
              description="Experimental and off by default. Removes speaker playback from the Me channel. If processing fails, Murmur keeps the original microphone audio."
              checked={settings.meetingEchoCancellationEnabled}
              onChange={() => onUpdateSettings({ meetingEchoCancellationEnabled: !settings.meetingEchoCancellationEnabled })}
            />
            <MeetingDiarizationSettings
              enabled={settings.meetingDiarization}
              onEnabledChange={(meetingDiarization) => onUpdateSettings({ meetingDiarization })}
            />
            <div data-setting-target="meeting-retention" className="grid gap-4 rounded-lg transition-shadow sm:grid-cols-2 [&.settings-target-flash]:ring-2 [&.settings-target-flash]:ring-primary/40">
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

          <SettingsSection card={false} pageId="performance" activePage={activeCat} title="Performance Lab" subtitle="Compare installed speech models on this Mac">
            <div data-setting-target="performance">
              <PerformanceLab status={status} settings={settings} onUpdateSettings={onUpdateSettings} audioInventory={audioInventory} />
            </div>
          </SettingsSection>

          <SettingsSection card={false} pageId="diagnostics" activePage={activeCat} title="Diagnostics" subtitle="Events, run history, performance, reports, and transform traces">
            <div data-setting-target="diagnostics" className="settings-card h-[520px] min-h-0 overflow-hidden">
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
            <button
              ref={generalShortcutsButtonRef}
              type="button"
              data-setting-target="shortcuts"
              onClick={() => openPage('shortcuts')}
              className="settings-setting-row flex w-full items-center justify-between gap-6 rounded-lg text-left transition-colors hover:bg-surface-container-low focus:outline-none focus-visible:ring-2 focus-visible:ring-primary"
            >
              <span className="settings-title-description">
                <span className="block text-sm font-medium text-on-surface">Keyboard Shortcuts</span>
                <span className="mt-0.5 block text-xs leading-relaxed text-on-surface-variant">See global and main-window shortcuts in one place.</span>
              </span>
              <span aria-hidden="true" className="text-on-surface-variant">›</span>
            </button>
            <div data-setting-target="setup" className="settings-field rounded-lg transition-shadow [&.settings-target-flash]:ring-2 [&.settings-target-flash]:ring-primary/40">
              <button type="button" onClick={onRerunSetup} className="w-full rounded-lg border border-outline-variant/30 bg-surface-container-lowest px-3 py-2 text-xs font-medium text-on-surface-variant transition-colors hover:bg-surface-container hover:text-primary">Run Setup Assistant</button>
              <p className="text-xs text-on-surface-variant">Re-check permissions and model setup after a permission is revoked or stops working.</p>
            </div>
            <SettingsDisclosure title="Advanced" layout="stack">
              <OverlayCalibrationControl
                offset={settings.overlayVerticalOffset}
                onCommit={(overlayVerticalOffset) => onUpdateSettings({ overlayVerticalOffset })}
              />
              <button type="button" aria-label={confirmReset ? 'Confirm reset statistics' : 'Reset statistics'} onClick={resetStats} className={`w-full rounded-lg border px-3 py-2 text-xs font-medium transition-colors ${confirmReset ? 'border-error/40 bg-error/10 text-error' : 'border-outline-variant/30 bg-surface-container-lowest text-on-surface-variant hover:bg-surface-container hover:text-primary'}`}>{confirmReset ? 'Confirm Reset' : 'Reset Stats'}</button>
            </SettingsDisclosure>
            {!INTERNAL_BENCHMARK_BUILD && <div data-setting-target="updates" className="settings-field rounded-lg transition-shadow [&.settings-target-flash]:ring-2 [&.settings-target-flash]:ring-primary/40">
              <UpdateIndicator
                variant="settings"
                status={updateStatus}
                onRetryCheck={() => void onCheckForUpdate()}
                onDownload={onDownloadUpdate}
                onRestart={onRestartUpdate}
                onOpen={onOpenUpdate}
              />
            </div>}
            {INTERNAL_BENCHMARK_BUILD && (
              <SettingsCallout tone="accent" title="Internal benchmark build">
                Automatic updates, launch at login, and diagnostic log shipping are disabled.
              </SettingsCallout>
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
