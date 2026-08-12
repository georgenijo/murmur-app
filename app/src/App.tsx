import { useState, useEffect, useCallback, useMemo, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { flog } from './lib/log';
import {
  SettingsPanel,
  SettingsSurfaceActiveContext,
  SETTINGS_CATEGORIES,
  settingsLatencyView,
} from './components/settings';
import { CommandPalette } from './components/CommandPalette';
import type { PaletteCommand } from './lib/commandPalette';
import { isEditableTarget, mainWindowShortcut } from './lib/keyboardShortcuts';
import { saveHistoryExport } from './lib/historyExport';
import { PermissionsBanner } from './components/PermissionsBanner';
import { AboutModal } from './components/AboutModal';
import { MainHeader } from './components/MainHeader';
import { TranscriptionView, type HistoryWorkspace } from './components/TranscriptionView';
import { FooterStats } from './components/FooterStats';
import { FileTranscriptionToasts } from './components/FileTranscriptionToasts';
import { useInitialization } from './lib/hooks/useInitialization';
import { useSettings } from './lib/hooks/useSettings';
import { useHistoryManagement } from './lib/hooks/useHistoryManagement';
import { useMeetings } from './lib/hooks/useMeetings';
import { useFileTranscription } from './lib/hooks/useFileTranscription';
import { useRecordingState } from './lib/hooks/useRecordingState';
import { useHoldDownToggle } from './lib/hooks/useHoldDownToggle';
import { useDoubleTapToggle } from './lib/hooks/useDoubleTapToggle';
import { useTransformFlow } from './lib/hooks/useTransformFlow';
import { useQueryFlow } from './lib/hooks/useQueryFlow';
import { useCombinedToggle } from './lib/hooks/useCombinedToggle';
import { useShowAboutListener } from './lib/hooks/useShowAboutListener';
import { useOverlaySettingsSync } from './lib/hooks/useOverlaySettingsSync';
import { useOpenSettingsListener } from './lib/hooks/useOpenSettingsListener';
import { useEscapeCancel } from './lib/hooks/useEscapeCancel';
import { useSilenceAutoStop } from './lib/hooks/useSilenceAutoStop';
import { useAutoUpdater } from './lib/hooks/useAutoUpdater';
import { UpdateModal } from './components/UpdateModal';
import { WhatsNewModal } from './components/WhatsNewModal';
import { UpdateIndicator } from './components/UpdateIndicator';
import type { CompletedUpdate, UpdateStatus } from './lib/updater';
import { resetStats } from './lib/stats';
import { ModelDownloader } from './components/ModelDownloader';
import { OnboardingFlow } from './components/onboarding/OnboardingFlow';
import { isOnboardingComplete, markOnboardingComplete, resetOnboarding } from './lib/onboarding';
import { checkAccessibilityPermission, checkMicrophonePermissionStatus, checkModelExists } from './lib/dictation';
import { getModelRuntimeCatalog } from './lib/modelRuntime';
import { open } from '@tauri-apps/plugin-dialog';
import { INTERNAL_BENCHMARK_BUILD } from './lib/buildFlavor';
import { cancelMicrophonePreview } from './lib/microphonePreview';
import {
  beginCurrentUiTransition,
  useUiLatencyDestination,
  type UiLatencyTrigger,
} from './lib/uiLatency';

const PERFORMANCE_BUILD_BADGE = import.meta.env.VITE_MURMUR_BUILD_ID?.startsWith('settings-phase2-')
  ? 'Use this · Phase 2 Perf'
  : undefined;

function App() {
  // --- Diagnostic: track when main window becomes visible/focused ---
  useEffect(() => {
    const onFocus = () => flog.info('main', 'FOCUS');
    const onBlur = () => flog.info('main', 'BLUR');
    const onVisibility = () => flog.info('main', 'VISIBILITY', { hidden: document.hidden });
    window.addEventListener('focus', onFocus);
    window.addEventListener('blur', onBlur);
    document.addEventListener('visibilitychange', onVisibility);
    flog.info('main', 'App mounted');
    return () => {
      window.removeEventListener('focus', onFocus);
      window.removeEventListener('blur', onBlur);
      document.removeEventListener('visibilitychange', onVisibility);
    };
  }, []);

  const [modelReady, setModelReady] = useState<boolean | null>(null);

  const { settings, updateSettings, applyExternalSettings, configureError } = useSettings();
  const meetings = useMeetings(settings);
  const markModelReady = useCallback((downloadedModel: typeof settings.model) => {
    if (downloadedModel !== settings.model) {
      updateSettings({ model: downloadedModel });
    }
    setModelReady(true);
  }, [settings.model, updateSettings]);
  const { initialized, error: initError } = useInitialization(settings);

  // First-launch gate: is the currently-selected model present? Checked once on
  // mount (not reactively) so changing models in Settings uses the inline
  // download flow there rather than re-showing this full-screen downloader.
  useEffect(() => {
    checkModelExists(settings.model)
      .then(setModelReady)
      .catch(() => setModelReady(true)); // fail open so main UI still loads
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Setup-assistant gate. Runs when the completion flag is absent, but
  // grandfathers existing installs: if both permissions and *any* model are
  // already in place, set the flag silently so upgrades never see the wizard.
  // Checking every model (not just the settings default) keeps a fresh webview
  // data store (e.g. tauri dev vs installed app) from re-running the wizard
  // when models are already on disk (#240).
  const [onboardingState, setOnboardingState] = useState<'unknown' | 'needed' | 'done'>('unknown');
  useEffect(() => {
    if (isOnboardingComplete()) {
      setOnboardingState('done');
      return;
    }
    (async () => {
      const [micStatus, axGranted, modelCatalog] = await Promise.all([
        checkMicrophonePermissionStatus().catch(() => 'unknown' as const),
        checkAccessibilityPermission().catch(() => false),
        getModelRuntimeCatalog().catch(() => []),
      ]);
      const anyModelExists = modelCatalog.some((model) => model.installState === 'installed');
      if (micStatus === 'granted' && axGranted && anyModelExists) {
        flog.info('main', 'Onboarding grandfathered: permissions and a model already present');
        markOnboardingComplete();
        setOnboardingState('done');
      } else {
        flog.info('main', 'Onboarding needed', { micStatus, axGranted, anyModelExists });
        setOnboardingState('needed');
      }
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const completeOnboarding = useCallback((
    model: typeof settings.model,
    recordingMode: typeof settings.recordingMode,
    doubleTapKey: typeof settings.doubleTapKey,
  ) => {
    markOnboardingComplete();
    updateSettings({ recordingMode, doubleTapKey });
    markModelReady(model);
    setOnboardingState('done');
  }, [markModelReady, updateSettings]);

  // Keep settings in sync when the overlay's quick controls change them.
  useOverlaySettingsSync(applyExternalSettings);

  // Track accessibility permission — when it transitions false→true the
  // double-tap listener restarts automatically (rdev silently does nothing
  // if started without permission).
  const [accessibilityGranted, setAccessibilityGranted] = useState<boolean | null>(null);
  useEffect(() => {
    const check = () => {
      invoke<boolean>('check_accessibility_permission')
        .then(setAccessibilityGranted)
        .catch(() => {});
    };
    check();
    window.addEventListener('focus', check);
    return () => window.removeEventListener('focus', check);
  }, []);
  const { historyEntries, addEntry, updateEntry, clearHistory } = useHistoryManagement(settings.retainHistory);
  const {
    status, recordingDuration, error: recordingError,
    handleStart, handleHoldStart, handleStop, toggleRecording, audioLevel, statsVersion,
  } = useRecordingState({ addEntry, microphone: settings.microphone });
  const [statsResetVersion, setStatsResetVersion] = useState(0);
  const combinedStatsVersion = statsVersion + statsResetVersion;
  const handleResetStats = useCallback(() => {
    resetStats();
    setStatsResetVersion(v => v + 1);
  }, []);
  // Keep the global hotkeys disarmed until onboarding completes — accessibility
  // can be granted mid-wizard, and a hold/double-tap must not start a recording
  // behind the OnboardingFlow screen.
  const hotkeysArmed = onboardingState === 'done';
  useHoldDownToggle({ enabled: hotkeysArmed && settings.recordingMode === 'hold_down', initialized, accessibilityGranted, holdDownKey: settings.doubleTapKey, onStart: handleHoldStart, onStop: handleStop });
  useDoubleTapToggle({ enabled: hotkeysArmed && settings.recordingMode === 'double_tap', initialized, accessibilityGranted, doubleTapKey: settings.doubleTapKey, status, onToggle: toggleRecording });
  useCombinedToggle({ enabled: hotkeysArmed && settings.recordingMode === 'both', initialized, accessibilityGranted, triggerKey: settings.doubleTapKey, status, onStart: handleHoldStart, onStop: handleStop, onToggle: toggleRecording });
  useEscapeCancel({ status, enabled: hotkeysArmed && initialized && accessibilityGranted === true });
  // Hands-free finish for any recording not started by holding the trigger
  // key (double-tap, button, overlay, locked mode). The hook tracks the
  // origin itself and ignores hold-started recordings, where the key release
  // owns the stop.
  useSilenceAutoStop({
    enabled: hotkeysArmed,
    status,
    silenceMs: settings.autoStopSilenceMs,
    onAutoStop: handleStop,
  });
  // Independent AX-selection transform hotkey (issue #312). Enabled only when
  // the user has configured a transform key; drives capture -> instruction ->
  // review via the transform-review popover window.
  useTransformFlow({
    enabled: hotkeysArmed && settings.transformHoldKey !== null,
    initialized,
    accessibilityGranted,
    transformHoldKey: settings.transformHoldKey,
    microphone: settings.microphone,
  });
  useQueryFlow({
    enabled: hotkeysArmed
      && settings.queryHotkey !== null
      && settings.queryExecutable.trim().length > 0,
    initialized,
    accessibilityGranted,
    queryHotkey: settings.queryHotkey,
    microphone: settings.microphone,
    command: {
      executable: settings.queryExecutable,
      arguments: settings.queryArguments,
      timeoutSeconds: settings.queryTimeoutSeconds,
    },
  });
  const { showAbout, setShowAbout } = useShowAboutListener();
  const updater = useAutoUpdater({ automaticChecksEnabled: !INTERNAL_BENCHMARK_BUILD });

  // DEV ONLY: cycle through updater and post-update modal states for visual testing
  const devUpdateIndex = useRef(-1);
  const devMockStates: UpdateStatus[] = import.meta.env.DEV ? [
    {
      phase: 'error',
      stage: 'install',
      message: 'macOS opened Murmur from a read-only security location. Quit Murmur, then use Finder to move or reinstall it in Applications before reopening it and trying the update again.',
      isForced: false,
      recovery: 'reinstall',
    },
    { phase: 'available', version: '0.7.0', notes: '## What\'s New\n- OTA auto-updater\n- Bug fixes\n- Performance improvements', isForced: false },
    { phase: 'available', version: '0.7.0', notes: 'Critical security fix.', isForced: true },
    { phase: 'preparing', version: '0.7.0' },
    { phase: 'downloading', version: '0.7.0', progress: 65 },
  ] : [];
  const [devUpdateStatus, setDevUpdateStatus] = useState<UpdateStatus | null>(null);
  const [devCompletedUpdate, setDevCompletedUpdate] = useState<CompletedUpdate | null>(null);
  const [devUpdateDialogOpen, setDevUpdateDialogOpen] = useState(false);

  const checkForUpdate = useCallback(async () => {
    if (import.meta.env.DEV) {
      devUpdateIndex.current = (devUpdateIndex.current + 1) % (devMockStates.length + 1);
      if (devUpdateIndex.current === 0) {
        setDevUpdateStatus(null);
        setDevUpdateDialogOpen(false);
        setDevCompletedUpdate({
          version: '0.22.0',
          notes: '## New Features\n\n- Faster local transcription\n- Selected-text transforms\n\n## Bug Fixes\n\n- More reliable microphone startup\n- Smoother overlay behavior',
        });
      } else {
        setDevCompletedUpdate(null);
        setDevUpdateStatus(devMockStates[devUpdateIndex.current - 1]);
        setDevUpdateDialogOpen(true);
      }
      return;
    }
    return updater.checkForUpdate();
  }, [updater.checkForUpdate]);

  const updateStatus = devUpdateStatus ?? updater.updateStatus;
  const isUpdateDialogOpen = devUpdateStatus
    ? devUpdateDialogOpen
    : updater.isUpdateDialogOpen;
  const showAvailableUpdate = useCallback(() => {
    if (
      devUpdateStatus?.phase === 'available' ||
      (devUpdateStatus?.phase === 'error' && devUpdateStatus.stage === 'install')
    ) {
      setDevUpdateDialogOpen(true);
      return;
    }
    updater.showAvailableUpdate();
  }, [devUpdateStatus, updater.showAvailableUpdate]);
  const dismissUpdate = useCallback(() => {
    if (devUpdateStatus) { setDevUpdateDialogOpen(false); return; }
    updater.dismissUpdate();
  }, [devUpdateStatus, updater.dismissUpdate]);
  const skipVersion = useCallback(() => {
    if (devUpdateStatus) {
      setDevUpdateDialogOpen(false);
      setDevUpdateStatus(null);
      return;
    }
    updater.skipVersion();
  }, [devUpdateStatus, updater.skipVersion]);
  const startDownload = updater.startDownload;
  const completedUpdate = devCompletedUpdate ?? updater.completedUpdate;
  const dismissCompletedUpdate = useCallback(() => {
    if (devCompletedUpdate) {
      setDevCompletedUpdate(null);
      return;
    }
    updater.dismissCompletedUpdate();
  }, [devCompletedUpdate, updater.dismissCompletedUpdate]);

  // The native menu-bar item brings the main window forward and asks the same
  // updater used by Settings and the command palette to perform a manual check.
  useEffect(() => {
    let disposed = false;
    let unlistenCheck: (() => void) | undefined;
    listen<unknown>('check-for-updates-requested', () => {
      void checkForUpdate();
    })
      .then((unlisten) => {
        if (disposed) unlisten();
        else unlistenCheck = unlisten;
      })
      .catch((err) => {
        flog.warn('updater', 'could not listen for menu-bar update checks', {
          error: String(err),
        });
      });
    return () => {
      disposed = true;
      unlistenCheck?.();
    };
  }, [checkForUpdate]);

  // Keep the native menu label in sync with the passive in-app indicator.
  useEffect(() => {
    const version = updateStatus.phase === 'available' ? updateStatus.version : null;
    invoke('set_tray_update_available', { version }).catch((err: unknown) => {
      flog.warn('updater', 'could not update menu-bar update item', {
        error: String(err),
      });
    });
  }, [updateStatus]);

  const [isSettingsOpen, setIsSettingsOpen] = useState(false);
  const [settingsPageRequest, setSettingsPageRequest] = useState<{ page: string; token: number } | null>(null);
  const settingsViewRef = useRef('settings.dictation');
  const settingsActiveRef = useRef(isSettingsOpen);
  settingsActiveRef.current = isSettingsOpen;
  useEffect(() => {
    if (isSettingsOpen) return;
    // Settings is warm-mounted behind the main surface. Explicitly cancel its
    // capture-only preview whenever that surface is hidden.
    void cancelMicrophonePreview().catch((error: unknown) => {
      flog.warn('audio', 'could not cancel hidden microphone preview', {
        error: String(error),
      });
    });
  }, [isSettingsOpen]);
  const trackSettingsView = useCallback((view: string) => {
    settingsViewRef.current = view;
  }, []);
  useUiLatencyDestination(
    onboardingState === 'done' && modelReady === true && !isSettingsOpen
      ? 'main.history'
      : null,
  );
  useUiLatencyDestination(isSettingsOpen ? settingsViewRef.current : null);

  const closeSettings = useCallback((trigger: UiLatencyTrigger = 'programmatic') => {
    if (!isSettingsOpen) return;
    beginCurrentUiTransition('main.history', trigger);
    setIsSettingsOpen(false);
  }, [isSettingsOpen]);

  const openSettings = useCallback((trigger: UiLatencyTrigger = 'programmatic') => {
    if (isSettingsOpen) return;
    beginCurrentUiTransition(settingsViewRef.current, trigger);
    setIsSettingsOpen(true);
  }, [isSettingsOpen]);

  // Bumped to move focus into the history search box (command palette action).
  const [historySearchToken, setHistorySearchToken] = useState<number | undefined>(undefined);
  const [historyWorkspace, setHistoryWorkspace] = useState<HistoryWorkspace>('transcripts');
  const focusHistorySearch = useCallback((trigger: UiLatencyTrigger = 'programmatic') => {
    closeSettings(trigger);
    setHistoryWorkspace('transcripts');
    setHistorySearchToken((token) => (token ?? 0) + 1);
  }, [closeSettings]);

  const fileTranscription = useFileTranscription({ addEntry });
  const pickAudioFiles = useCallback(async () => {
    try {
      const selected = await open({
        multiple: true,
        filters: [{ name: 'Audio', extensions: ['wav', 'mp3', 'm4a'] }],
      });
      const paths = Array.isArray(selected) ? selected : selected ? [selected] : [];
      if (paths.length > 0) fileTranscription.enqueue(paths);
    } catch (e) {
      flog.warn('file-transcribe', 'file dialog failed', { error: String(e) });
    }
  }, [fileTranscription.enqueue]);

  // Overlay gear button asks the main window to open the Settings panel.
  const openSettingsFromOverlay = useCallback(() => openSettings('programmatic'), [openSettings]);
  useOpenSettingsListener(openSettingsFromOverlay);

  const rerunSetup = useCallback(() => {
    setIsSettingsOpen(false);
    resetOnboarding();
    setOnboardingState('needed');
  }, []);

  // ---- Command palette (⌘K) ----------------------------------------------
  const [isPaletteOpen, setIsPaletteOpen] = useState(false);
  const openSettingsPage = useCallback((page: string) => {
    beginCurrentUiTransition(settingsLatencyView(page), 'programmatic');
    setSettingsPageRequest((previous) => ({ page, token: (previous?.token ?? 0) + 1 }));
    setIsSettingsOpen(true);
  }, []);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const shortcut = mainWindowShortcut(event, isEditableTarget(event.target));
      if (!shortcut) return;
      event.preventDefault();
      if (shortcut === 'palette') setIsPaletteOpen((open) => !open);
      else if (shortcut === 'search') { setIsPaletteOpen(false); focusHistorySearch('keyboard'); }
      else if (shortcut === 'settings') { setIsPaletteOpen(false); openSettings('keyboard'); }
      else openSettingsPage('performance');
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [focusHistorySearch, openSettings, openSettingsPage]);

  const commands = useMemo<PaletteCommand[]>(() => {
    const isRecording = status === 'recording' || status === 'starting';
    const meetingBusy = !['idle', 'failed'].includes(meetings.status.phase);
    const meetingCanStop = ['starting', 'recording'].includes(meetings.status.phase);
    const lastEntry = historyEntries[historyEntries.length - 1];
    const items: PaletteCommand[] = [
      ...(!meetingBusy ? [{
        id: 'recording-toggle',
        title: status === 'starting'
          ? 'Cancel microphone connection'
          : isRecording
            ? 'Stop recording'
            : 'Start recording',
        section: 'Recording',
        keywords: ['dictate', 'microphone', 'transcribe'],
        run: () => { void (isRecording ? handleStop() : handleStart()); },
      }] : []),
      {
        id: 'meeting-toggle',
        title: meetings.status.phase === 'processing' || meetings.status.phase === 'stopping'
          ? 'Show meeting transcript progress'
          : meetingCanStop
            ? 'Stop meeting capture'
            : 'Start meeting capture',
        section: 'Meeting',
        keywords: ['system audio', 'call', 'me', 'them', 'record'],
        run: () => {
          closeSettings('programmatic');
          setHistoryWorkspace('meetings');
          if (meetings.status.phase === 'processing' || meetings.status.phase === 'stopping') return;
          void (meetingCanStop ? meetings.stop() : meetings.start());
        },
      },
      {
        id: 'app-disable-toggle',
        title: settings.disabled ? 'Enable Murmur' : 'Disable Murmur',
        section: 'Recording',
        keywords: ['pause', 'mute', 'hotkey'],
        hint: settings.disabled ? 'currently off' : undefined,
        run: () => updateSettings({ disabled: !settings.disabled }),
      },
      {
        id: 'transcribe-file',
        title: 'Transcribe audio file…',
        section: 'Recording',
        keywords: ['file', 'import', 'audio', 'wav', 'mp3', 'm4a'],
        run: () => { void pickAudioFiles(); },
      },
      {
        id: 'history-search',
        title: 'Search transcripts',
        section: 'History',
        keywords: ['find', 'filter', 'history'],
        run: focusHistorySearch,
      },
      // Offered only when there is something to act on, so no row is a dead end.
      ...(lastEntry ? [{
        id: 'history-copy-last',
        title: 'Copy last transcript',
        section: 'History',
        keywords: ['clipboard', 'again'],
        run: async () => {
          await navigator.clipboard.writeText(lastEntry.text).catch((e: unknown) =>
            flog.warn('main', 'Palette copy failed', { error: String(e) }));
        },
      }, {
        id: 'history-export',
        title: 'Export history to a Markdown file',
        section: 'History',
        keywords: ['save', 'download', 'markdown'],
        run: async () => {
          await saveHistoryExport(historyEntries, 'markdown').catch((e: unknown) =>
            flog.warn('main', 'Palette export failed', { error: String(e) }));
        },
      }] : []),
      {
        id: 'show-history',
        title: 'Show transcription history',
        section: 'Navigation',
        keywords: ['record', 'main'],
        run: () => { closeSettings('programmatic'); setHistoryWorkspace('transcripts'); },
      },
      {
        id: 'show-meetings',
        title: 'Show meeting transcripts',
        section: 'Navigation',
        keywords: ['system audio', 'calls', 'me', 'them'],
        run: () => { closeSettings('programmatic'); setHistoryWorkspace('meetings'); },
      },
      ...SETTINGS_CATEGORIES.map((category) => ({
        id: `settings-${category.id}`,
        title: `Settings: ${category.label}`,
        section: 'Settings',
        keywords: ['preferences', 'options'],
        run: () => openSettingsPage(category.id),
      })),
      {
        id: 'logs',
        title: 'Open performance diagnostics',
        section: 'Diagnostics',
        keywords: ['events', 'debug', 'log', 'logs', 'runs', 'performance'],
        run: () => openSettingsPage('performance'),
      },
      {
        id: 'check-updates',
        title: 'Check for updates',
        section: 'App',
        keywords: ['version', 'upgrade'],
        run: () => { void checkForUpdate(); },
      },
      {
        id: 'about',
        title: 'About Murmur',
        section: 'App',
        keywords: ['version', 'credits'],
        run: () => setShowAbout(true),
      },
      {
        id: 'rerun-setup',
        title: 'Re-run setup assistant',
        section: 'App',
        keywords: ['onboarding', 'permissions', 'wizard'],
        run: () => { setIsSettingsOpen(false); resetOnboarding(); setOnboardingState('needed'); },
      },
    ];
    return items;
  }, [
    status, historyEntries, settings.disabled, updateSettings, handleStart, handleStop,
    focusHistorySearch, openSettingsPage, closeSettings, checkForUpdate, setShowAbout, pickAudioFiles,
    meetings,
  ]);

  const error = initError || recordingError;

  if (onboardingState === 'unknown' || modelReady === null) {
    return <div className="h-screen bg-background" />;
  }
  if (onboardingState === 'needed') {
    return (
      <OnboardingFlow
        initialModel={settings.model}
        recordingMode={settings.recordingMode}
        triggerKey={settings.doubleTapKey}
        onComplete={completeOnboarding}
      />
    );
  }
  if (modelReady === false) {
    return (
      <ModelDownloader
        initialModel={settings.model}
        onComplete={markModelReady}
      />
    );
  }

  return (
    <div className="relative flex h-screen flex-col overflow-hidden bg-background text-on-surface font-[-apple-system,BlinkMacSystemFont,'Segoe_UI',Roboto,sans-serif]">
      {import.meta.env.DEV && (
        <div className="absolute left-1/2 top-0 z-50 -translate-x-1/2 rounded-b-md bg-warning/15 px-2 py-0.5 text-[9px] font-bold uppercase tracking-widest text-warning select-none">
          Dev
        </div>
      )}
      <MainHeader
        status={status}
        initialized={initialized}
        recordingDuration={recordingDuration}
        audioLevel={audioLevel}
        triggerKey={settings.doubleTapKey}
        recordingMode={settings.recordingMode}
        onRecord={handleStart}
        onStop={handleStop}
        onOpenSettings={() => {
          if (isSettingsOpen) closeSettings('pointer');
          else openSettings('pointer');
        }}
        settingsOpen={isSettingsOpen}
        mode={isSettingsOpen ? 'settings' : 'main'}
        buildBadge={PERFORMANCE_BUILD_BADGE}
        meetingPhase={meetings.status.phase}
        meetingElapsedMs={meetings.status.elapsedMs}
        updateIndicator={!INTERNAL_BENCHMARK_BUILD ? (
          <UpdateIndicator
            status={updateStatus}
            onOpen={showAvailableUpdate}
            onRetryCheck={() => void checkForUpdate()}
          />
        ) : undefined}
      />

      <PermissionsBanner />

      <div className="relative min-h-0 flex-1 overflow-hidden">
        <main
          aria-hidden={isSettingsOpen}
          {...(isSettingsOpen ? { inert: '' } : {})}
          className={`ui-persistent-surface absolute inset-0 flex min-h-0 flex-col overflow-hidden ${isSettingsOpen ? 'pointer-events-none opacity-0' : 'opacity-100'}`}
        >
          <TranscriptionView
            historyEntries={historyEntries}
            onClearHistory={clearHistory}
            onUpdateHistoryEntry={updateEntry}
            focusSearchToken={historySearchToken}
            onTranscribeFile={pickAudioFiles}
            workspace={historyWorkspace}
            onWorkspaceChange={setHistoryWorkspace}
            meetings={meetings}
          />

          {error && (
            <div className="absolute bottom-4 left-4 right-4 z-20 rounded-xl border border-error/30 bg-surface-container-lowest px-4 py-3 shadow-xl">
              <p className="text-sm text-error">{error}</p>
            </div>
          )}

          {fileTranscription.isDragging && (
            <div className="pointer-events-none absolute inset-3 z-40 grid place-items-center rounded-2xl border-2 border-dashed border-primary bg-background/90 backdrop-blur-sm">
              <div className="text-center">
                <span className="mx-auto mb-3 grid h-12 w-12 place-items-center rounded-2xl bg-primary/10 text-2xl text-on-surface">↓</span>
                <p className="text-base font-semibold text-on-surface">Drop to transcribe</p>
                <p className="mt-1 text-xs text-on-surface-variant">WAV, MP3, and M4A files</p>
              </div>
            </div>
          )}

          <FileTranscriptionToasts
            queue={fileTranscription.queue}
            error={fileTranscription.error}
            onCancel={fileTranscription.cancel}
            onDismiss={fileTranscription.dismiss}
          />
        </main>

        <section
          aria-hidden={!isSettingsOpen}
          {...(!isSettingsOpen ? { inert: '' } : {})}
          className={`ui-persistent-surface absolute inset-0 flex min-h-0 overflow-hidden ${isSettingsOpen ? 'opacity-100' : 'pointer-events-none opacity-0'}`}
        >
          <SettingsSurfaceActiveContext.Provider value={isSettingsOpen}>
            <SettingsPanel
              settings={settings}
              onUpdateSettings={updateSettings}
              initialized={initialized}
              status={status}
              onResetStats={handleResetStats}
              onRerunSetup={rerunSetup}
              accessibilityGranted={accessibilityGranted}
              onCheckForUpdate={checkForUpdate}
              updateStatus={updateStatus}
              configureError={configureError}
              pageRequest={settingsPageRequest}
              onLatencyViewChange={trackSettingsView}
              activeRef={settingsActiveRef}
            />
          </SettingsSurfaceActiveContext.Provider>
        </section>
      </div>

      <div
        aria-hidden={isSettingsOpen}
        className={`shrink-0 ${isSettingsOpen ? 'pointer-events-none opacity-0' : 'opacity-100'}`}
      >
        <FooterStats statsVersion={combinedStatsVersion} />
      </div>

      <CommandPalette
        isOpen={isPaletteOpen}
        onClose={() => setIsPaletteOpen(false)}
        commands={commands}
      />

      <AboutModal
        isOpen={showAbout}
        onClose={() => setShowAbout(false)}
      />
      {!completedUpdate && (
        <UpdateModal
          status={isUpdateDialogOpen ? updateStatus : { phase: 'idle' }}
          onDownload={startDownload}
          onSkip={skipVersion}
          onDismiss={dismissUpdate}
        />
      )}
      <WhatsNewModal
        update={completedUpdate}
        onDismiss={dismissCompletedUpdate}
      />
    </div>
  );
}

export default App;
