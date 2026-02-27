import { useState, useEffect, lazy, Suspense, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { flog } from './lib/log';
import { SettingsPanel } from './components/settings';
import { PermissionsBanner } from './components/PermissionsBanner';
import { AboutModal } from './components/AboutModal';
import { StatusHeader } from './components/StatusHeader';
import { RecordingControls } from './components/RecordingControls';
import { TranscriptionView } from './components/TranscriptionView';
import { useInitialization } from './lib/hooks/useInitialization';
import { useSettings } from './lib/hooks/useSettings';
import { useHistoryManagement } from './lib/hooks/useHistoryManagement';
import { useRecordingState } from './lib/hooks/useRecordingState';
import { useHoldDownToggle } from './lib/hooks/useHoldDownToggle';
import { useDoubleTapToggle } from './lib/hooks/useDoubleTapToggle';
import { useShowAboutListener } from './lib/hooks/useShowAboutListener';
import { useAutoUpdater } from './lib/hooks/useAutoUpdater';
import { UpdateModal } from './components/UpdateModal';
import { StatsBar } from './components/StatsBar';
import { LogViewer } from './components/LogViewer';
const ResourceMonitor = lazy(() => import('./components/ResourceMonitor').then(m => ({ default: m.ResourceMonitor })));
import { resetStats } from './lib/stats';
import { ModelDownloader } from './components/ModelDownloader';

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
  const markModelReady = useCallback(() => setModelReady(true), []);

  useEffect(() => {
    invoke<boolean>('check_model_exists')
      .then(setModelReady)
      .catch(() => setModelReady(true)); // fail open so main UI still loads
  }, []);

  const { settings, updateSettings } = useSettings();
  const { initialized, error: initError } = useInitialization(settings);

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
  const { showAbout, setShowAbout } = useShowAboutListener();
  const { updateStatus, checkForUpdate, startDownload, skipVersion, dismissUpdate } = useAutoUpdater();

  const [isSettingsOpen, setIsSettingsOpen] = useState(false);
  const [isLogViewerOpen, setIsLogViewerOpen] = useState(false);

  const error = initError || recordingError;

  if (modelReady === null) return <div className="h-screen bg-stone-50 dark:bg-stone-900" />;
  if (modelReady === false) return <ModelDownloader onComplete={markModelReady} />;

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
        <main className="flex-1 flex flex-col overflow-hidden p-4 gap-4">
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

          {import.meta.env.DEV && <Suspense fallback={null}><ResourceMonitor /></Suspense>}
        </main>

        <SettingsPanel
          isOpen={isSettingsOpen}
          onClose={() => setIsSettingsOpen(false)}
          settings={settings}
          onUpdateSettings={updateSettings}
          status={status}
          onResetStats={handleResetStats}
          onViewLogs={() => setIsLogViewerOpen(true)}
          accessibilityGranted={accessibilityGranted}
          onCheckForUpdate={checkForUpdate}
          updateStatus={updateStatus}
        />
      </div>

      <AboutModal
        isOpen={showAbout}
        onClose={() => setShowAbout(false)}
      />
      <LogViewer
        isOpen={isLogViewerOpen}
        onClose={() => setIsLogViewerOpen(false)}
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
