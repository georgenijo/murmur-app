import { useState, useEffect, lazy, Suspense, useCallback, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { flog } from './lib/log';
import { SettingsPanel } from './components/settings';
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
import { useCombinedToggle } from './lib/hooks/useCombinedToggle';
import { useShowAboutListener } from './lib/hooks/useShowAboutListener';
import { useOverlaySettingsSync } from './lib/hooks/useOverlaySettingsSync';
import { useOpenSettingsListener } from './lib/hooks/useOpenSettingsListener';
import { useEscapeCancel } from './lib/hooks/useEscapeCancel';
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
import { checkMicrophonePermissionStatus } from './lib/dictation';

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

  const { settings, updateSettings, applyExternalSettings } = useSettings();
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
    invoke<boolean>('check_specific_model_exists', { modelName: settings.model })
      .then(setModelReady)
      .catch(() => setModelReady(true)); // fail open so main UI still loads
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Setup-assistant gate. Runs when the completion flag is absent, but
  // grandfathers existing installs: if both permissions and the model are
  // already in place, set the flag silently so upgrades never see the wizard.
  const [onboardingState, setOnboardingState] = useState<'unknown' | 'needed' | 'done'>('unknown');
  useEffect(() => {
    if (isOnboardingComplete()) {
      setOnboardingState('done');
      return;
    }
    (async () => {
      const [micStatus, axGranted, modelExists] = await Promise.all([
        checkMicrophonePermissionStatus().catch(() => 'unknown' as const),
        invoke<boolean>('check_accessibility_permission').catch(() => false),
        invoke<boolean>('check_specific_model_exists', { modelName: settings.model }).catch(() => false),
      ]);
      if (micStatus === 'granted' && axGranted && modelExists) {
        flog.info('main', 'Onboarding grandfathered: permissions and model already present');
        markOnboardingComplete();
        setOnboardingState('done');
      } else {
        flog.info('main', 'Onboarding needed', { micStatus, axGranted, modelExists });
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
  const { historyEntries, addEntry, clearHistory } = useHistoryManagement();
  const {
    status, recordingDuration, error: recordingError,
    handleStart, handleStop, toggleRecording, statsVersion,
  } = useRecordingState({ addEntry, microphone: settings.microphone });
  const [statsResetVersion, setStatsResetVersion] = useState(0);
  const combinedStatsVersion = statsVersion + statsResetVersion;
  const handleResetStats = () => { resetStats(); setStatsResetVersion(v => v + 1); };
  useHoldDownToggle({ enabled: settings.recordingMode === 'hold_down', initialized, accessibilityGranted, holdDownKey: settings.doubleTapKey, onStart: handleStart, onStop: handleStop });
  useDoubleTapToggle({ enabled: settings.recordingMode === 'double_tap', initialized, accessibilityGranted, doubleTapKey: settings.doubleTapKey, status, onToggle: toggleRecording });
  useCombinedToggle({ enabled: settings.recordingMode === 'both', initialized, accessibilityGranted, triggerKey: settings.doubleTapKey, status, onStart: handleStart, onStop: handleStop, onToggle: toggleRecording });
  useEscapeCancel({ status, enabled: initialized && accessibilityGranted === true });
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

  // Overlay gear button asks the main window to open the Settings panel.
  const openSettings = useCallback(() => setIsSettingsOpen(true), []);
  useOpenSettingsListener(openSettings);

  const error = initError || recordingError;

  if (onboardingState === 'unknown' || modelReady === null) {
    return <div className="h-screen bg-stone-50 dark:bg-stone-900" />;
  }
  if (onboardingState === 'needed') {
    return (
      <OnboardingFlow
        initialModel={settings.model}
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
    <div className="h-screen bg-stone-50 dark:bg-stone-900 flex flex-col font-[-apple-system,BlinkMacSystemFont,'Segoe_UI',Roboto,sans-serif]">
      {import.meta.env.DEV && (
        <div className="bg-amber-400 text-amber-900 text-xs font-semibold text-center py-0.5 tracking-widest uppercase select-none">
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
          <div className="shrink-0 flex gap-1 p-1 self-start bg-stone-100 dark:bg-stone-800 rounded-lg">
            {(['record', 'file'] as const).map((tab) => (
              <button
                key={tab}
                onClick={() => setMainTab(tab)}
                className={`px-3 py-1 text-sm font-medium rounded-md transition-colors ${
                  mainTab === tab
                    ? 'bg-white text-stone-900 shadow-sm dark:bg-stone-700 dark:text-stone-100'
                    : 'text-stone-500 hover:text-stone-700 dark:text-stone-400 dark:hover:text-stone-200'
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
              />

              {error && (
                <div className="shrink-0 px-4 py-3 bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 rounded-lg">
                  <p className="text-red-600 dark:text-red-400 text-sm">{error}</p>
                </div>
              )}

              <RecordingControls status={status} initialized={initialized} onStart={handleStart} onStop={handleStop} />

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
          onViewLogs={() => invoke('open_log_viewer').catch((e: unknown) => flog.warn('main', 'Failed to open log viewer', { error: String(e) }))}
          onRerunSetup={() => {
            setIsSettingsOpen(false);
            resetOnboarding();
            setOnboardingState('needed');
          }}
          accessibilityGranted={accessibilityGranted}
          onCheckForUpdate={checkForUpdate}
          updateStatus={updateStatus}
        />
        )}
      </div>

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
