import { useState } from 'react';
import { SettingsPanel } from './components/settings';
import { PermissionsBanner } from './components/PermissionsBanner';
import { AboutModal } from './components/AboutModal';
import { StatusHeader } from './components/StatusHeader';
import { RecordingControls } from './components/RecordingControls';
import { TabNavigation, TabType } from './components/TabNavigation';
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
    status, transcription, recordingDuration, error: recordingError,
    handleStart, handleStop, toggleRecording,
  } = useRecordingState({ addEntry });
  useHotkeyToggle({ enabled: settings.recordingMode === 'hotkey', initialized, hotkey: settings.hotkey, onToggle: toggleRecording });
  useDoubleTapToggle({ enabled: settings.recordingMode === 'double_tap', initialized, hotkey: settings.hotkey, status, onToggle: toggleRecording });
  const { showAbout, setShowAbout } = useShowAboutListener();

  const [isSettingsOpen, setIsSettingsOpen] = useState(false);
  const [activeTab, setActiveTab] = useState<TabType>('current');

  const error = initError || recordingError;

  return (
    <div className="h-screen bg-gray-50 dark:bg-gray-900 flex flex-col font-[-apple-system,BlinkMacSystemFont,'Segoe_UI',Roboto,sans-serif]">
      <StatusHeader status={status} initialized={initialized} recordingDuration={recordingDuration} />

      <PermissionsBanner />

      <main className="flex-1 flex flex-col overflow-hidden p-4 gap-4">
        <TabNavigation activeTab={activeTab} onTabChange={setActiveTab} historyCount={historyEntries.length} />

        <TranscriptionView
          activeTab={activeTab}
          transcription={transcription}
          status={status}
          historyEntries={historyEntries}
          onClearHistory={clearHistory}
        />

        {error && (
          <div className="flex-shrink-0 px-4 py-3 bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 rounded-lg">
            <p className="text-red-600 dark:text-red-400 text-sm">{error}</p>
          </div>
        )}

        <RecordingControls status={status} initialized={initialized} onStart={handleStart} onStop={handleStop} />
      </main>

      <footer className="flex-shrink-0 px-4 py-3 border-t border-gray-200 dark:border-gray-700 bg-white/80 dark:bg-gray-800/80 backdrop-blur-sm">
        <div className="flex justify-end">
          <button
            className="flex items-center gap-2 px-3 py-1.5 text-sm text-gray-600 dark:text-gray-400 hover:text-gray-900 dark:hover:text-gray-200 hover:bg-gray-100 dark:hover:bg-gray-700 rounded-lg transition-colors"
            onClick={() => setIsSettingsOpen(true)}
          >
            <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z" />
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
            </svg>
            Settings
          </button>
        </div>
      </footer>

      <SettingsPanel
        isOpen={isSettingsOpen}
        onClose={() => setIsSettingsOpen(false)}
        settings={settings}
        onUpdateSettings={updateSettings}
        status={status}
      />

      <AboutModal
        isOpen={showAbout}
        onClose={() => setShowAbout(false)}
      />
    </div>
  );
}

export default App;
