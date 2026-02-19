import { useState } from 'react';
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
import { useHotkeyToggle } from './lib/hooks/useHotkeyToggle';
import { useDoubleTapToggle } from './lib/hooks/useDoubleTapToggle';
import { useShowAboutListener } from './lib/hooks/useShowAboutListener';

function App() {
  const { settings, updateSettings } = useSettings();
  const { initialized, error: initError } = useInitialization(settings);
  const { historyEntries, addEntry, clearHistory } = useHistoryManagement();
  const {
    status, recordingDuration, error: recordingError,
    handleStart, handleStop, toggleRecording,
  } = useRecordingState({ addEntry });
  useHotkeyToggle({ enabled: settings.recordingMode === 'hotkey', initialized, hotkey: settings.hotkey, onToggle: toggleRecording });
  useDoubleTapToggle({ enabled: settings.recordingMode === 'double_tap', initialized, doubleTapKey: settings.doubleTapKey, status, onToggle: toggleRecording });
  const { showAbout, setShowAbout } = useShowAboutListener();

  const [isSettingsOpen, setIsSettingsOpen] = useState(false);

  const error = initError || recordingError;

  return (
    <div className="h-screen bg-stone-50 dark:bg-stone-900 flex flex-col font-[-apple-system,BlinkMacSystemFont,'Segoe_UI',Roboto,sans-serif]">
      <StatusHeader
        status={status}
        initialized={initialized}
        recordingDuration={recordingDuration}
        onSettingsToggle={() => setIsSettingsOpen(o => !o)}
        isSettingsOpen={isSettingsOpen}
      />

      <PermissionsBanner />

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
        </main>

        <SettingsPanel
          isOpen={isSettingsOpen}
          onClose={() => setIsSettingsOpen(false)}
          settings={settings}
          onUpdateSettings={updateSettings}
          status={status}
        />
      </div>

      <AboutModal
        isOpen={showAbout}
        onClose={() => setShowAbout(false)}
      />
    </div>
  );
}

export default App;
