import { MeetingsPanel } from './components/history';
import { useHomeSidebar } from './lib/hooks/useHomeSidebar';
import { AppSurface } from './components/ui/AppSurface';
import { useId, useState, useEffect, useCallback, useLayoutEffect, useMemo, useRef, type CSSProperties } from 'react';
import { defaultButtonKeyDown } from './lib/defaultButton';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { flog } from './lib/log';
import {
  SettingsPanel,
  SettingsSurfaceActiveContext,
  SETTINGS_CATEGORIES,
  SETTINGS_TOOLS,
  settingsLatencyView,
  type SettingsPageRequest,
} from './components/settings';
import { CommandPalette } from './components/CommandPalette';
import type { PaletteCommand } from './lib/commandPalette';
import { nextRecordingCommands } from './lib/nextRecordingCommands';
import { useModeRuntime } from './lib/hooks/useModeRuntime';
import { isEditableTarget, mainWindowShortcut } from './lib/keyboardShortcuts';
import { saveHistoryExport } from './lib/historyExport';
import { PermissionsBanner } from './components/PermissionsBanner';
import { AboutModal } from './components/AboutModal';
import { MainHeader } from './components/MainHeader';
import { HomeDashboard } from './components/home/HomeDashboard';
import { HomeSidebar } from './components/home/HomeSidebar';
import { InsightsView } from './components/home/InsightsView';
import { WorkspacePageHeader } from './components/ui/DashboardPrimitives';
import type { MainDestination } from './lib/homeDashboard';
import { FileTranscriptionToasts } from './components/FileTranscriptionToasts';
import { MainErrorBanner } from './components/MainErrorBanner';
import { useInitialization } from './lib/hooks/useInitialization';
import { useSettings } from './lib/hooks/useSettings';
import { useHistoryManagement } from './lib/hooks/useHistoryManagement';
import { useLocalStats } from './lib/hooks/useLocalStats';
import { useMeetings } from './lib/hooks/useMeetings';
import { useMeetingSuggestionAcceptance } from './lib/hooks/useMeetingSuggestionAcceptance';
import { useFileTranscription } from './lib/hooks/useFileTranscription';
import { useRecordingState } from './lib/hooks/useRecordingState';
import { useShowAboutListener } from './lib/hooks/useShowAboutListener';
import { useOverlaySettingsSync } from './lib/hooks/useOverlaySettingsSync';
import { useOpenSettingsListener } from './lib/hooks/useOpenSettingsListener';
import { useSoundCues } from './lib/hooks/useSoundCues';
import { useAutoUpdater } from './lib/hooks/useAutoUpdater';
import { useDevUpdaterMock } from './lib/hooks/useDevUpdaterMock';
import {
  useDeliveryRecoveryListeners,
  useDeliveryRecoveryNotice,
} from './lib/hooks/useDeliveryRecoveryListeners';
import { UpdateModal } from './components/UpdateModal';
import { WhatsNewModal } from './components/WhatsNewModal';
import { UpdateIndicator } from './components/UpdateIndicator';
import { setTrayUpdateAvailable } from './lib/updater';
import { loadStats, resetStats } from './lib/stats';
import { ModelDownloader } from './components/ModelDownloader';
import { OnboardingFlow } from './components/onboarding/OnboardingFlow';
import { isOnboardingComplete, markOnboardingComplete, resetOnboarding } from './lib/onboarding';
import {
  completeChecklistItem,
  checklistDestination,
  discoveryEvidence,
  hintDestination,
  initializeDiscoveryAfterSetup,
  initializeGrandfatheredDiscovery,
  type ChecklistItemId,
  type DiscoveryHintId,
} from './lib/discovery';
import { useDiscovery } from './lib/hooks/useDiscovery';
import { DiscoveryHintToast } from './components/DiscoveryHintToast';
import {
  checkAccessibilityPermission,
  checkMicrophonePermissionStatus,
  checkModelExists,
} from './lib/dictation';
import { getModelRuntimeCatalog } from './lib/modelRuntime';
import { open } from '@tauri-apps/plugin-dialog';
import { INTERNAL_BENCHMARK_BUILD } from './lib/buildFlavor';
import { cancelMicrophonePreview } from './lib/microphonePreview';
import { getMeetingAudioCaptureBusy } from './lib/meetingAudio';
import {
  canRetryDelivery,
  retryLastDelivery,
  setPasteLastShortcut,
} from './lib/deliveryRecovery';
import { SETTINGS_VERSION, pasteLastShortcutLabel, smartAutoMicrophoneRequest } from './lib/settings';
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
  const [onboardingState, setOnboardingState] = useState<'unknown' | 'needed' | 'done'>('unknown');
  const mainAppReady = onboardingState === 'done' && modelReady === true;
  const mainAppReadyRef = useRef(mainAppReady);
  mainAppReadyRef.current = mainAppReady;
  const setupAdmittedRef = useRef(false);
  const canStartRecording = useCallback(() => mainAppReadyRef.current && !setupAdmittedRef.current, []);
  const [isSettingsOpen, setIsSettingsOpen] = useState(false);
  const homeSidebar = useHomeSidebar(false, isSettingsOpen);
  const homeNavId = useId();
  const sidebarToggleRef = useRef<HTMLButtonElement>(null);
  const separatorFocused = useRef(false);
  useLayoutEffect(() => {
    if (homeSidebar.collapsed && separatorFocused.current) {
      separatorFocused.current = false;
      sidebarToggleRef.current?.focus();
    }
  }, [homeSidebar.collapsed]);

  const { settings, getCurrentSettings, getConfirmedSettings, updateSettings, configureOnboarding, applyExternalSettings, configureError } = useSettings();
  const {
    notice: deliveryRecoveryNotice,
    presentNotice: presentDeliveryRecoveryNotice,
    clearNotice: clearDeliveryRecoveryNotice,
  } = useDeliveryRecoveryNotice();
  const pasteLastShortcutGenerationRef = useRef(0);
  const lastWorkingPasteLastShortcutRef = useRef<typeof settings.pasteLastShortcut>(null);
  const meetings = useMeetings(settings);
  useMeetingSuggestionAcceptance(meetings.start, (message) => {
    presentDeliveryRecoveryNotice({ message, retryable: false });
  });
  useSoundCues(settings, meetings.status.phase);
  const markModelReady = useCallback((downloadedModel: typeof settings.model) => {
    if (downloadedModel !== settings.model) {
      updateSettings({ model: downloadedModel });
    }
    setModelReady(true);
  }, [settings.model, updateSettings]);
  const { initialized, error: initError } = useInitialization(getConfirmedSettings, getCurrentSettings);
  const [mainDestination, setMainDestination] = useState<MainDestination>('home');
  const homeNavigationRef = useRef<HTMLButtonElement>(null);
  const restoreHomeNavigationFocusRef = useRef(false);
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
  useEffect(() => {
    if (isOnboardingComplete()) {
      markOnboardingComplete(); // Upgrade the durable native launch gate.
      initializeGrandfatheredDiscovery(discoveryEvidence(settings, loadStats()));
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
        initializeGrandfatheredDiscovery(discoveryEvidence(settings, loadStats()));
        setOnboardingState('done');
      } else {
        flog.info('main', 'Onboarding needed', { micStatus, axGranted, anyModelExists });
        setOnboardingState('needed');
      }
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const completeOnboarding = useCallback(async (
    model: typeof settings.model,
    recordingMode: typeof settings.recordingMode,
    doubleTapKey: typeof settings.doubleTapKey,
  ) => {
    await configureOnboarding({ model, recordingMode, doubleTapKey });
    await invoke('resume_dictation_after_setup', { settings: {
      ...getCurrentSettings(), model, recordingMode, doubleTapKey,
      settingsVersion: SETTINGS_VERSION, launchOnboardingComplete: true,
    } });
    setupAdmittedRef.current = false;
    markOnboardingComplete();
    initializeDiscoveryAfterSetup(discoveryEvidence(settings, loadStats()));
    setModelReady(true);
    setOnboardingState('done');
  }, [
    configureOnboarding,
    settings.appProfiles,
  ]);

  // Keep settings in sync when the overlay's quick controls change them.
  useOverlaySettingsSync(applyExternalSettings);

  useEffect(() => {
    const requested = settings.pasteLastShortcut;
    const generation = ++pasteLastShortcutGenerationRef.current;
    void setPasteLastShortcut(requested)
      .then(() => {
        if (pasteLastShortcutGenerationRef.current === generation) {
          lastWorkingPasteLastShortcutRef.current = requested;
        }
      })
      .catch((error: unknown) => {
        if (pasteLastShortcutGenerationRef.current !== generation) return;
        presentDeliveryRecoveryNotice({ message: String(error), retryable: false });
        const fallback = lastWorkingPasteLastShortcutRef.current;
        if (requested !== fallback) updateSettings({ pasteLastShortcut: fallback });
      });
  }, [presentDeliveryRecoveryNotice, settings.pasteLastShortcut, updateSettings]);

  useDeliveryRecoveryListeners(presentDeliveryRecoveryNotice);

  // Track accessibility permission — when it transitions false→true the
  // double-tap listener restarts automatically (rdev silently does nothing
  // if started without permission).
  const [accessibilityGranted, setAccessibilityGranted] = useState<boolean | null>(null);
  useEffect(() => {
    const check = () => {
      checkAccessibilityPermission()
        .then(setAccessibilityGranted)
        .catch(() => {});
    };
    check();
    window.addEventListener('focus', check);
    return () => window.removeEventListener('focus', check);
  }, []);
  const { historyEntries, addEntry, updateEntry, togglePinned, deleteEntries, clearHistory } = useHistoryManagement(settings.retainHistory);
  const {
    status, isCaptureBusy, recordingDuration, error: recordingError,
    canRetryDelivery: recordingCanRetryDelivery,
    dismissError: dismissRecordingError,
    handleStart, handleStop, audioLevel,
    smartDestinationStatus, smartDestinationAppName,
  } = useRecordingState({
    addEntry,
    microphone: settings.microphone,
    smartAuto: smartAutoMicrophoneRequest(settings),
    canStart: canStartRecording,
  });
  const combinedStatsVersion = useLocalStats();
  const discovery = useDiscovery({
    enabled: onboardingState === 'done' && modelReady === true,
    settings,
    statsVersion: combinedStatsVersion,
    historyEntries,
  });
  const handleResetStats = useCallback(() => {
    resetStats();
  }, []);
  // React supplies validated settings after first-run migration and reflects
  // native state. Unmounting this client never disarms the native controller.
  useEffect(() => {
    if (!initialized || accessibilityGranted === null || onboardingState === 'unknown' || modelReady === null) return;
    void invoke('configure_dictation_control', { settings: {
      ...settings,
      settingsVersion: SETTINGS_VERSION,
      launchOnboardingComplete: mainAppReady,
    } }).catch(() => {});
  }, [initialized, accessibilityGranted, mainAppReady, settings, onboardingState, modelReady]);
  const { showAbout, setShowAbout } = useShowAboutListener();
  const updater = useAutoUpdater({
    automaticChecksEnabled: !INTERNAL_BENCHMARK_BUILD,
    setupComplete: onboardingState === 'done' && modelReady === true,
  });

  const {
    checkForUpdate,
    updateStatus,
    isUpdateDialogOpen,
    showAvailableUpdate,
    dismissUpdate,
    restartUpdate,
    startDownload,
    completedUpdate,
    dismissCompletedUpdate,
  } = useDevUpdaterMock(updater);

  // The native menu-bar item brings the main window forward and asks the same
  // updater used by Settings and the command palette to perform a manual check.
  useEffect(() => {
    let disposed = false;
    let unlistenCheck: (() => void) | undefined;
    listen<unknown>('check-for-updates-requested', () => {
      if (updateStatus.phase === 'idle' || updateStatus.phase === 'up-to-date' ||
          (updateStatus.phase === 'error' && updateStatus.stage === 'check')) {
        void checkForUpdate();
      } else {
        showAvailableUpdate();
      }
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
  }, [checkForUpdate, showAvailableUpdate, updateStatus]);

  // Keep the native menu label in sync with the passive in-app indicator.
  useEffect(() => {
    const version = 'version' in updateStatus ? updateStatus.version : null;
    setTrayUpdateAvailable(version).catch((err: unknown) => {
      flog.warn('updater', 'could not update menu-bar update item', {
        error: String(err),
      });
    });
  }, [updateStatus]);

  const [settingsPageRequest, setSettingsPageRequest] = useState<SettingsPageRequest | null>(null);
  const settingsViewRef = useRef('settings.general');
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
      ? `main.${mainDestination}`
      : null,
  );
  useUiLatencyDestination(isSettingsOpen ? settingsViewRef.current : null);

  const closeSettings = useCallback((trigger: UiLatencyTrigger = 'programmatic') => {
    if (!isSettingsOpen) return;
    beginCurrentUiTransition(`main.${mainDestination}`, trigger);
    setIsSettingsOpen(false);
  }, [isSettingsOpen, mainDestination]);

  const openSettings = useCallback((trigger: UiLatencyTrigger = 'programmatic') => {
    if (!mainAppReady || isSettingsOpen) return;
    beginCurrentUiTransition('settings.general', trigger);
    setSettingsPageRequest((previous) => ({
      page: 'general',
      token: (previous?.token ?? 0) + 1,
    }));
    setIsSettingsOpen(true);
  }, [mainAppReady, isSettingsOpen]);

  const navigateMain = useCallback((destination: MainDestination, trigger: UiLatencyTrigger = 'programmatic') => {
    closeSettings(trigger);
    beginCurrentUiTransition(`main.${destination}`, trigger);
    setMainDestination(destination);
  }, [closeSettings]);

  const backToHome = useCallback(() => {
    restoreHomeNavigationFocusRef.current = true;
    navigateMain('home', 'programmatic');
  }, [navigateMain]);

  useLayoutEffect(() => {
    if (mainDestination !== 'home' || !restoreHomeNavigationFocusRef.current) return;
    restoreHomeNavigationFocusRef.current = false;
    homeNavigationRef.current?.focus();
  }, [mainDestination]);

  // Bumped to move focus into the history search box (command palette action).
  const [historySearchToken, setHistorySearchToken] = useState<number | undefined>(undefined);
  const [teachLatestToken, setTeachLatestToken] = useState<number | undefined>(undefined);
  const handleTeachLatestHandled = useCallback(() => setTeachLatestToken(undefined), []);
  const focusHistorySearch = useCallback((trigger: UiLatencyTrigger = 'programmatic') => {
    closeSettings(trigger);
    setMainDestination('home');
    setHistorySearchToken((token) => (token ?? 0) + 1);
  }, [closeSettings]);

  const fileTranscription = useFileTranscription({ addEntry });
  const pickMediaFiles = useCallback(async () => {
    try {
      const selected = await open({
        multiple: true,
        filters: [{ name: 'Audio and video', extensions: ['wav', 'mp3', 'm4a', 'mp4', 'mov'] }],
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

  const captureBusyRef = useRef(false);
  captureBusyRef.current = status !== 'idle' || !['idle', 'failed'].includes(meetings.status.phase);
  const setupRequestPendingRef = useRef(false);
  const rerunSetup = useCallback(async () => {
    if (setupRequestPendingRef.current) return;
    setupRequestPendingRef.current = true;
    try {
      // This native ownership check covers all active capture
      // and startup before the main window receives its lifecycle event.
      const nativeBusy = await getMeetingAudioCaptureBusy();
      if (nativeBusy || captureBusyRef.current || isCaptureBusy()) {
        presentDeliveryRecoveryNotice({ message: 'Stop recording first before running setup.', retryable: false });
        return;
      }
      if (!await invoke<boolean>('suspend_dictation_for_setup')) {
        presentDeliveryRecoveryNotice({ message: 'Stop recording first before running setup.', retryable: false });
        return;
      }
    } catch {
      presentDeliveryRecoveryNotice({ message: 'Could not check recording state. Try running setup again.', retryable: false });
      return;
    } finally {
      setupRequestPendingRef.current = false;
    }
    // Native admission is closed under the same fence as recording starts.
    // The local guard also blocks pending main-window button callbacks.
    setupAdmittedRef.current = true;
    setIsSettingsOpen(false);
    setIsPaletteOpen(false);
    resetOnboarding();
    setOnboardingState('needed');
  }, [presentDeliveryRecoveryNotice, isCaptureBusy]);

  // ---- Command palette (⌘K) ----------------------------------------------
  const [isPaletteOpen, setIsPaletteOpen] = useState(false);
  const modeRuntime = useModeRuntime();
  const setNextRecordingMode = useCallback((modeId: string | null) => {
    void modeRuntime.setNext(modeId).catch((error: unknown) => presentDeliveryRecoveryNotice({
      message: String(error),
      retryable: false,
    }));
  }, [modeRuntime.setNext, presentDeliveryRecoveryNotice]);
  const openSettingsTarget = useCallback((target: Omit<SettingsPageRequest, 'token'>) => {
    if (!mainAppReady) return;
    beginCurrentUiTransition(settingsLatencyView(target.page), 'programmatic');
    setSettingsPageRequest((previous) => ({
      ...target,
      token: (previous?.token ?? 0) + 1,
    }));
    setIsSettingsOpen(true);
  }, [mainAppReady]);
  const openSettingsPage = useCallback((page: string) => {
    openSettingsTarget({ page });
  }, [openSettingsTarget]);

  const openLatestTeaching = useCallback(() => {
    closeSettings('programmatic');
    setMainDestination('home');
    setTeachLatestToken((token) => (token ?? 0) + 1);
  }, [closeSettings]);

  const runDiscoveryChecklistAction = useCallback((id: ChecklistItemId) => {
    const destination = checklistDestination(id);
    switch (destination.kind) {
      case 'palette': setIsPaletteOpen(true); break;
      case 'settings': openSettingsTarget(destination); break;
      case 'main': navigateMain(destination.page, 'programmatic'); break;
      case 'teach': openLatestTeaching(); break;
      default: {
        const exhaustive: never = destination;
        return exhaustive;
      }
    }
  }, [navigateMain, openLatestTeaching, openSettingsTarget]);

  const runDiscoveryHintAction = useCallback((id: DiscoveryHintId) => {
    discovery.dismissHint(id);
    const destination = hintDestination(id);
    switch (destination.kind) {
      case 'palette': setIsPaletteOpen(true); break;
      case 'settings': openSettingsTarget(destination); break;
      case 'main': navigateMain(destination.page, 'programmatic'); break;
      case 'teach': openLatestTeaching(); break;
      default: {
        const exhaustive: never = destination;
        return exhaustive;
      }
    }
  }, [discovery.dismissHint, navigateMain, openLatestTeaching, openSettingsTarget]);

  const handleSettingsPageOpened = useCallback((page: string) => {
    if (page === 'shortcuts') completeChecklistItem('shortcuts');
  }, []);

  const handlePaletteOpened = useCallback(() => {
    completeChecklistItem('command_palette');
  }, []);

  const handleModeBound = useCallback(() => {
    completeChecklistItem('mode_binding');
  }, []);

  useEffect(() => {
    if (!mainAppReady) return;
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
  }, [mainAppReady, focusHistorySearch, openSettings, openSettingsPage]);

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
      ...nextRecordingCommands(modeRuntime.status, setNextRecordingMode),
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
          setMainDestination('meetings');
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
        title: 'Transcribe audio or video file…',
        section: 'Recording',
        keywords: ['file', 'import', 'audio', 'video', 'wav', 'mp3', 'm4a', 'mp4', 'mov'],
        run: () => { void pickMediaFiles(); },
      },
      {
        id: 'paste-last',
        title: 'Paste last or retry delivery',
        section: 'History',
        keywords: ['clipboard', 'again', 'recover', 'retry', 'delivery'],
        hint: pasteLastShortcutLabel(settings.pasteLastShortcut),
        run: () => { void retryLastDelivery(); },
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
        title: 'Export history to a Markdown file…',
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
        run: () => { closeSettings('programmatic'); setMainDestination('home'); },
      },
      {
        id: 'show-meetings',
        title: 'Show meeting transcripts',
        section: 'Navigation',
        keywords: ['system audio', 'calls', 'me', 'them'],
        run: () => { closeSettings('programmatic'); setMainDestination('meetings'); },
      },
      {
        id: 'settings-shortcuts',
        title: 'Settings: Keyboard shortcuts',
        section: 'Settings',
        keywords: ['hotkeys', 'bindings', 'keys'],
        run: () => openSettingsPage('shortcuts'),
      },
      ...SETTINGS_CATEGORIES.map((category) => ({
        id: `settings-${category.id}`,
        title: `Settings: ${category.label}`,
        section: 'Settings',
        keywords: ['preferences', 'options'],
        run: () => openSettingsPage(category.id),
      })),
      ...SETTINGS_TOOLS.map((tool) => ({
        id: `settings-${tool.id}`,
        title: `Open ${tool.label}`,
        section: 'Tools',
        keywords: tool.id === 'diagnostics'
          ? ['events', 'debug', 'log', 'logs', 'runs', 'performance']
          : ['benchmark', 'models', 'speed', 'accuracy'],
        run: () => openSettingsPage(tool.id),
      })),
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
        run: rerunSetup,
      },
    ];
    return items;
  }, [
    status, historyEntries, settings.disabled, settings.pasteLastShortcut, settings.microphone,
    settings.smartAutoMicrophoneEnabled, settings.smartAutoApprovedDeviceIds,
    settings.smartAutoPreferredDeviceIds, settings.smartAutoAllowContinuity,
    updateSettings, handleStart, handleStop,
    focusHistorySearch, openSettingsPage, closeSettings, checkForUpdate, setShowAbout, pickMediaFiles,
    meetings, presentDeliveryRecoveryNotice, rerunSetup,
    modeRuntime.status, setNextRecordingMode,
  ]);

  const [dismissedExternalErrorKey, setDismissedExternalErrorKey] = useState('');
  const errorPresentation = [
    initError ? {
      key: `initialization:${initError}`,
      message: initError,
      source: 'initialization' as const,
      retryable: false,
    } : null,
    recordingError ? {
      key: `recording:${recordingError}`,
      message: recordingError,
      source: 'recording' as const,
      retryable: recordingCanRetryDelivery,
    } : null,
    deliveryRecoveryNotice ? {
      key: `delivery:${deliveryRecoveryNotice.message}`,
      message: deliveryRecoveryNotice.message,
      source: 'delivery' as const,
      retryable: deliveryRecoveryNotice.retryable,
    } : null,
  ].find((candidate) => candidate !== null && candidate.key !== dismissedExternalErrorKey) ?? null;
  const [deliveryRetryBusy, setDeliveryRetryBusy] = useState(false);

  const dismissMainError = useCallback(() => {
    if (!errorPresentation) return;
    if (errorPresentation.source === 'recording') {
      dismissRecordingError();
      return;
    }
    if (errorPresentation.source === 'delivery') {
      clearDeliveryRecoveryNotice();
      return;
    }
    setDismissedExternalErrorKey(errorPresentation.key);
  }, [clearDeliveryRecoveryNotice, dismissRecordingError, errorPresentation]);

  const retryDeliveryFromBanner = useCallback(() => {
    if (deliveryRetryBusy) return;
    if (errorPresentation?.source === 'recording') dismissRecordingError();
    setDeliveryRetryBusy(true);
    presentDeliveryRecoveryNotice({ message: 'Trying delivery again…', retryable: true });
    void retryLastDelivery()
      .then((result) => presentDeliveryRecoveryNotice({
        message: result.message,
        retryable: canRetryDelivery(result),
      }, 5000))
      .catch(() => presentDeliveryRecoveryNotice({
        message: 'Paste Last did not finish. Try again.',
        retryable: true,
      }, 5000))
      .finally(() => setDeliveryRetryBusy(false));
  }, [
    deliveryRetryBusy,
    dismissRecordingError,
    errorPresentation,
    presentDeliveryRecoveryNotice,
  ]);

  if (onboardingState === 'unknown' || modelReady === null) {
    return <div className="h-full min-h-0 bg-background" />;
  }
  if (onboardingState === 'needed') {
    return (
      <div className="main-first-run main-first-run-onboarding">
        <OnboardingFlow
          initialModel={settings.model}
          recordingMode={settings.recordingMode}
          triggerKey={settings.doubleTapKey}
          onComplete={completeOnboarding}
        />
      </div>
    );
  }
  if (modelReady === false) {
    return (
      <div className="main-first-run main-first-run-model">
        <ModelDownloader
          initialModel={settings.model}
          onComplete={markModelReady}
        />
      </div>
    );
  }

  return (
    <AppSurface data-home-sidebar-collapsed={homeSidebar.collapsed} onKeyDown={defaultButtonKeyDown} surface="main" style={{ '--home-sidebar-width': `${homeSidebar.width}px` } as CSSProperties} className="main-app-shell relative flex h-full min-h-0 flex-col overflow-hidden bg-background text-on-surface">
      {import.meta.env.DEV && (
        <div className="absolute left-1/2 top-0 z-50 -translate-x-1/2 rounded-b-md bg-warning/15 px-2 py-0.5 text-[9px] font-semibold uppercase tracking-widest text-warning select-none">
          Dev
        </div>
      )}
      <MainHeader
        sidebar={{ collapsed: homeSidebar.collapsed, navId: homeNavId, toggle: homeSidebar.toggle, toggleRef: sidebarToggleRef }}
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
        showRecordControls={false}
        showStatusChip={mainDestination !== 'home'}
        updateIndicator={!INTERNAL_BENCHMARK_BUILD ? (
          <UpdateIndicator
            status={updateStatus}
            onDownload={startDownload}
            onRestart={restartUpdate}
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
          <div className="flex min-h-0 flex-1 overflow-hidden">
            <HomeSidebar
              sidebar={homeSidebar}
              navId={homeNavId}
              onSeparatorFocusChange={(focused) => { separatorFocused.current = focused; }}
              suspended={isSettingsOpen}
              active={mainDestination}
              homeButtonRef={homeNavigationRef}
              onNavigate={(destination) => navigateMain(destination, 'pointer')}
            />
            <div className="main-dashboard-workspace">
              {mainDestination === 'home' ? (
                <HomeDashboard
                  historyEntries={historyEntries}
                  onClearHistory={clearHistory}
                  onDeleteHistoryEntries={deleteEntries}
                  onUpdateHistoryEntry={updateEntry}
                  onToggleHistoryPinned={togglePinned}
                  focusSearchToken={historySearchToken}
                  teachLatestToken={teachLatestToken}
                  onTeachLatestHandled={handleTeachLatestHandled}
                  onTranscribeFile={pickMediaFiles}
                  status={status}
                  initialized={initialized}
                  recordingDuration={recordingDuration}
                  audioLevel={audioLevel}
                  smartDestinationStatus={smartDestinationStatus}
                  smartDestinationAppName={smartDestinationAppName}
                  settings={settings}
                  meetings={meetings}
                  statsVersion={combinedStatsVersion}
                  onRecord={handleStart}
                  onStop={handleStop}
                  onOpenInsights={() => navigateMain('insights', 'pointer')}
                  discovery={discovery.state && !discovery.state.checklistDismissed ? {
                    completed: discovery.state.completed,
                    onAction: runDiscoveryChecklistAction,
                    onDismiss: discovery.dismissChecklist,
                  } : undefined}
                />
              ) : mainDestination === 'meetings' ? (
                <section className="main-secondary-view" aria-labelledby="meetings-view-title">
                  <WorkspacePageHeader
                    title="Notetaker"
                    titleId="meetings-view-title"
                    description="Local meeting transcripts with speaker separation"
                    back={{ label: 'Back to Home', onActivate: backToHome }}
                  />
                  <MeetingsPanel meetings={meetings} playbackBusy={status !== 'idle'} />
                </section>
              ) : (
                <InsightsView
                  modes={settings.modes}
                  statsVersion={combinedStatsVersion}
                  onBackToHome={backToHome}
                />
              )}
            </div>
          </div>

          {errorPresentation && (
            <MainErrorBanner
              message={errorPresentation.message}
              onDismiss={dismissMainError}
              actionLabel={errorPresentation.retryable ? 'Try again' : undefined}
              actionBusy={deliveryRetryBusy}
              onAction={errorPresentation.retryable ? retryDeliveryFromBanner : undefined}
            />
          )}

          {fileTranscription.isDragging && (
            <div className="pointer-events-none absolute inset-3 z-40 grid place-items-center rounded-2xl border-2 border-dashed border-primary bg-background/90 backdrop-blur-sm">
              <div className="text-center">
                <span className="mx-auto mb-3 grid h-12 w-12 place-items-center rounded-2xl bg-primary/10 text-2xl text-on-surface">↓</span>
                <p className="text-base font-semibold text-on-surface">Drop to transcribe</p>
                <p className="mt-1 text-xs text-on-surface-variant">WAV, MP3, M4A, MP4, and MOV files</p>
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
              onDiscoveryChecklistReopen={() => {
                discovery.reopenChecklist();
                navigateMain('home', 'programmatic');
              }}
              onPageOpened={handleSettingsPageOpened}
              onModeBound={handleModeBound}
              accessibilityGranted={accessibilityGranted}
              onCheckForUpdate={checkForUpdate}
              onDownloadUpdate={startDownload}
              onRestartUpdate={restartUpdate}
              onOpenUpdate={showAvailableUpdate}
              updateStatus={updateStatus}
              configureError={configureError}
              pageRequest={settingsPageRequest}
              onLatencyViewChange={trackSettingsView}
              activeRef={settingsActiveRef}
            />
          </SettingsSurfaceActiveContext.Provider>
        </section>
      </div>

      <CommandPalette
        isOpen={isPaletteOpen}
        onClose={() => setIsPaletteOpen(false)}
        commands={commands}
        onOpened={handlePaletteOpened}
      />

      {discovery.state?.pendingHints[0] && (
        <DiscoveryHintToast
          hint={discovery.state.pendingHints[0]}
          onAction={runDiscoveryHintAction}
          onDismiss={discovery.dismissHint}
        />
      )}

      <AboutModal
        isOpen={showAbout}
        onClose={() => setShowAbout(false)}
      />
      {!completedUpdate && (
        <UpdateModal
          status={isUpdateDialogOpen ? updateStatus : { phase: 'idle' }}
          onDownload={startDownload}
          onRetryCheck={() => void checkForUpdate()}
          onRestart={restartUpdate}
          onDismiss={dismissUpdate}
        />
      )}
      <WhatsNewModal
        update={completedUpdate}
        onDismiss={dismissCompletedUpdate}
      />
    </AppSurface>
  );
}

export default App;
