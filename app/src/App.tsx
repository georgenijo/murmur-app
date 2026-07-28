import { useState, useEffect, lazy, Suspense, useCallback, useMemo, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { flog } from './lib/log';
import { SettingsPanel, SETTINGS_CATEGORIES } from './components/settings';
import { CommandPalette } from './components/CommandPalette';
import type { PaletteCommand } from './lib/commandPalette';
import { isEditableTarget, mainWindowShortcut } from './lib/keyboardShortcuts';
import { saveHistoryExport } from './lib/historyExport';
import { PermissionsBanner } from './components/PermissionsBanner';
import { AboutModal } from './components/AboutModal';
import { StatusHeader } from './components/StatusHeader';
import { RecordingControls } from './components/RecordingControls';
import { TranscriptionView } from './components/TranscriptionView';
import { FileTranscriptionPanel } from './components/FileTranscriptionPanel';
import { useInitialization } from './lib/hooks/useInitialization';
import { useSettings } from './lib/hooks/useSettings';
import { useHistoryManagement } from './lib/hooks/useHistoryManagement';
import { useRecordingState } from './lib/hooks/useRecordingState';
import { useHoldDownToggle } from './lib/hooks/useHoldDownToggle';
import { useDoubleTapToggle } from './lib/hooks/useDoubleTapToggle';
import { useTransformFlow } from './lib/hooks/useTransformFlow';
import { useCombinedToggle } from './lib/hooks/useCombinedToggle';
import { useShowAboutListener } from './lib/hooks/useShowAboutListener';
import { useOverlaySettingsSync } from './lib/hooks/useOverlaySettingsSync';
import { useOpenSettingsListener } from './lib/hooks/useOpenSettingsListener';
import { useEscapeCancel } from './lib/hooks/useEscapeCancel';
import { useSilenceAutoStop } from './lib/hooks/useSilenceAutoStop';
import { useAutoUpdater } from './lib/hooks/useAutoUpdater';
import { UpdateModal } from './components/UpdateModal';
import type { UpdateStatus } from './lib/updater';
import { StatsBar } from './components/StatsBar';
const ResourceMonitor = lazy(() => import('./components/ResourceMonitor').then(m => ({ default: m.ResourceMonitor })));
const UsageDashboard = lazy(() => import('./components/UsageDashboard').then(m => ({ default: m.UsageDashboard })));
import { resetStats } from './lib/stats';
import { ModelDownloader } from './components/ModelDownloader';
import { OnboardingFlow } from './components/onboarding/OnboardingFlow';
import { isOnboardingComplete, markOnboardingComplete, resetOnboarding } from './lib/onboarding';
import { checkAccessibilityPermission, checkMicrophonePermissionStatus, checkModelExists } from './lib/dictation';
import { getModelRuntimeCatalog } from './lib/modelRuntime';

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

  const completeOnboarding = useCallback((model: typeof settings.model) => {
    markOnboardingComplete();
    markModelReady(model);
    setOnboardingState('done');
  }, [markModelReady]);

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
  const { historyEntries, addEntry, updateEntry, clearHistory } = useHistoryManagement();
  const {
    status, recordingDuration, error: recordingError,
    handleStart, handleStop, toggleRecording, statsVersion,
  } = useRecordingState({ addEntry, microphone: settings.microphone });
  const [statsResetVersion, setStatsResetVersion] = useState(0);
  const combinedStatsVersion = statsVersion + statsResetVersion;
  const handleResetStats = () => { resetStats(); setStatsResetVersion(v => v + 1); };
  // Keep the global hotkeys disarmed until onboarding completes — accessibility
  // can be granted mid-wizard, and a hold/double-tap must not start a recording
  // behind the OnboardingFlow screen.
  const hotkeysArmed = onboardingState === 'done';
  useHoldDownToggle({ enabled: hotkeysArmed && settings.recordingMode === 'hold_down', initialized, accessibilityGranted, holdDownKey: settings.doubleTapKey, onStart: handleStart, onStop: handleStop });
  useDoubleTapToggle({ enabled: hotkeysArmed && settings.recordingMode === 'double_tap', initialized, accessibilityGranted, doubleTapKey: settings.doubleTapKey, status, onToggle: toggleRecording });
  useCombinedToggle({ enabled: hotkeysArmed && settings.recordingMode === 'both', initialized, accessibilityGranted, triggerKey: settings.doubleTapKey, status, onStart: handleStart, onStop: handleStop, onToggle: toggleRecording });
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
  const { showAbout, setShowAbout } = useShowAboutListener();
  const updater = useAutoUpdater();

  // DEV ONLY: cycle through mock update modal states for visual testing
  const devUpdateIndex = useRef(-1);
  const devMockStates: UpdateStatus[] = import.meta.env.DEV ? [
    { phase: 'available', version: '0.7.0', notes: '## What\'s New\n- OTA auto-updater\n- Bug fixes\n- Performance improvements', isForced: false },
    { phase: 'available', version: '0.7.0', notes: 'Critical security fix.', isForced: true },
    { phase: 'downloading', version: '0.7.0', progress: 65 },
    { phase: 'error', message: 'Network request failed: could not resolve host', isForced: false },
  ] : [];
  const [devUpdateStatus, setDevUpdateStatus] = useState<UpdateStatus | null>(null);

  const checkForUpdate = useCallback(async () => {
    if (import.meta.env.DEV) {
      devUpdateIndex.current = (devUpdateIndex.current + 1) % devMockStates.length;
      setDevUpdateStatus(devMockStates[devUpdateIndex.current]);
      return;
    }
    return updater.checkForUpdate();
  }, [updater.checkForUpdate]);

  const updateStatus = devUpdateStatus ?? updater.updateStatus;
  const dismissUpdate = useCallback(() => {
    if (devUpdateStatus) { setDevUpdateStatus(null); return; }
    updater.dismissUpdate();
  }, [devUpdateStatus, updater.dismissUpdate]);
  const skipVersion = useCallback(() => {
    if (devUpdateStatus) { setDevUpdateStatus(null); return; }
    updater.skipVersion();
  }, [devUpdateStatus, updater.skipVersion]);
  const startDownload = updater.startDownload;

  const [isSettingsOpen, setIsSettingsOpen] = useState(false);
  const [mainTab, setMainTab] = useState<'record' | 'file'>('record');
  // Bumped to move focus into the history search box (command palette action).
  const [historySearchToken, setHistorySearchToken] = useState<number | undefined>(undefined);
  const focusHistorySearch = useCallback(() => {
    setMainTab('record');
    setIsSettingsOpen(false);
    setHistorySearchToken((token) => (token ?? 0) + 1);
  }, []);

  // Overlay gear button asks the main window to open the Settings panel.
  const openSettings = useCallback(() => setIsSettingsOpen(true), []);
  useOpenSettingsListener(openSettings);

  // ---- Command palette (⌘K) ----------------------------------------------
  const [isPaletteOpen, setIsPaletteOpen] = useState(false);
  const [settingsPageRequest, setSettingsPageRequest] = useState<{ page: string; token: number } | null>(null);
  const openSettingsPage = useCallback((page: string) => {
    setSettingsPageRequest((previous) => ({ page, token: (previous?.token ?? 0) + 1 }));
    setIsSettingsOpen(true);
  }, []);

  const openLogViewer = useCallback(() => {
    invoke('open_log_viewer').catch((e: unknown) =>
      flog.warn('main', 'Failed to open log viewer', { error: String(e) }));
  }, []);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const shortcut = mainWindowShortcut(event, isEditableTarget(event.target));
      if (!shortcut) return;
      event.preventDefault();
      if (shortcut === 'palette') setIsPaletteOpen((open) => !open);
      else if (shortcut === 'search') { setIsPaletteOpen(false); focusHistorySearch(); }
      else if (shortcut === 'settings') { setIsPaletteOpen(false); setIsSettingsOpen(true); }
      else openLogViewer();
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [focusHistorySearch, openLogViewer]);

  const commands = useMemo<PaletteCommand[]>(() => {
    const isRecording = status === 'recording';
    const lastEntry = historyEntries[historyEntries.length - 1];
    const items: PaletteCommand[] = [
      {
        id: 'recording-toggle',
        title: isRecording ? 'Stop recording' : 'Start recording',
        section: 'Recording',
        keywords: ['dictate', 'microphone', 'transcribe'],
        run: () => { void (isRecording ? handleStop() : handleStart()); },
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
        id: 'tab-record',
        title: 'Go to Record',
        section: 'Navigation',
        keywords: ['history', 'main'],
        run: () => { setIsSettingsOpen(false); setMainTab('record'); },
      },
      {
        id: 'tab-file',
        title: 'Go to Transcribe File',
        section: 'Navigation',
        keywords: ['import', 'audio', 'wav'],
        run: () => { setIsSettingsOpen(false); setMainTab('file'); },
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
        title: 'Open log viewer',
        section: 'Diagnostics',
        keywords: ['events', 'debug', 'runs', 'performance'],
        run: openLogViewer,
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
    focusHistorySearch, openSettingsPage, openLogViewer, checkForUpdate, setShowAbout,
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
    <div className="h-screen bg-background text-on-surface flex flex-col font-[-apple-system,BlinkMacSystemFont,'Segoe_UI',Roboto,sans-serif]">
      {import.meta.env.DEV && (
        <div className="bg-warning/10 text-warning text-xs font-semibold text-center py-0.5 tracking-widest uppercase select-none">
          Dev
        </div>
      )}
      <StatusHeader
        status={status}
        initialized={initialized}
        recordingDuration={recordingDuration}
        onSettingsToggle={() => setIsSettingsOpen(o => !o)}
        isSettingsOpen={isSettingsOpen}
      />

      <PermissionsBanner />

      <StatsBar statsVersion={combinedStatsVersion} />

      <div className="flex-1 flex overflow-hidden">
        <main className={`flex-1 flex-col overflow-hidden p-4 gap-4 ${isSettingsOpen ? 'hidden' : 'flex'}`}>
          <div className="shrink-0 flex gap-1 p-1 self-start bg-surface-container rounded-xl">
            {(['record', 'file'] as const).map((tab) => (
              <button
                key={tab}
                onClick={() => setMainTab(tab)}
                className={`px-3 py-1 text-sm font-medium rounded-md transition-colors ${
                  mainTab === tab
                    ? 'bg-surface-container-lowest text-on-surface shadow-sm'
                    : 'text-on-surface-variant hover:text-on-surface'
                }`}
              >
                {tab === 'record' ? 'Record' : 'Transcribe File'}
              </button>
            ))}
          </div>

          {mainTab === 'record' ? (
            <>
              <TranscriptionView
                historyEntries={historyEntries}
                onClearHistory={clearHistory}
                onUpdateHistoryEntry={updateEntry}
                focusSearchToken={historySearchToken}
              />

              {error && (
                <div className="shrink-0 px-4 py-3 bg-error/10 border border-error/30 rounded-lg">
                  <p className="text-error text-sm">{error}</p>
                </div>
              )}

              <RecordingControls status={status} initialized={initialized} onStart={handleStart} onStop={handleStop} triggerKey={settings.doubleTapKey} />

              <Suspense fallback={null}><UsageDashboard statsVersion={combinedStatsVersion} /></Suspense>

              {import.meta.env.DEV && <Suspense fallback={null}><ResourceMonitor /></Suspense>}
            </>
          ) : (
            <FileTranscriptionPanel addEntry={addEntry} />
          )}
        </main>

        {isSettingsOpen && (
        <SettingsPanel
          isOpen={isSettingsOpen}
          onClose={() => setIsSettingsOpen(false)}
          settings={settings}
          onUpdateSettings={updateSettings}
          status={status}
          onResetStats={handleResetStats}
          onViewLogs={openLogViewer}
          onRerunSetup={() => {
            setIsSettingsOpen(false);
            resetOnboarding();
            setOnboardingState('needed');
          }}
          accessibilityGranted={accessibilityGranted}
          onCheckForUpdate={checkForUpdate}
          updateStatus={updateStatus}
          configureError={configureError}
          pageRequest={settingsPageRequest}
        />
        )}
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
      <UpdateModal
        status={updateStatus}
        onDownload={startDownload}
        onSkip={skipVersion}
        onDismiss={dismissUpdate}
      />
    </div>
  );
}

export default App;
