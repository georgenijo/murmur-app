import { useState, useEffect, useRef, useCallback } from 'react';
import { listen } from '@tauri-apps/api/event';
import { initDictation, startRecording, stopRecording, configure } from './lib/dictation';
import { SettingsPanel } from './components/settings';
import { HistoryPanel } from './components/history';
import { PermissionsBanner } from './components/PermissionsBanner';
import { AboutModal } from './components/AboutModal';
import { Settings, loadSettings, saveSettings, DEFAULT_SETTINGS } from './lib/settings';
import { HistoryEntry, loadHistory, saveHistory, addHistoryEntry } from './lib/history';
import { registerHotkey, unregisterHotkey, hotkeyToShortcut } from './lib/hotkey';

type TabType = 'current' | 'history';

function App() {
  const [status, setStatus] = useState<string>('idle');
  const [transcription, setTranscription] = useState<string>('');
  const [error, setError] = useState<string>('');
  const [initialized, setInitialized] = useState(false);
  const [isSettingsOpen, setIsSettingsOpen] = useState(false);
  const [showAbout, setShowAbout] = useState(false);
  const [settings, setSettings] = useState<Settings>(DEFAULT_SETTINGS);
  const [recordingStartTime, setRecordingStartTime] = useState<number | null>(null);
  const [recordingDuration, setRecordingDuration] = useState(0);
  const [activeTab, setActiveTab] = useState<TabType>('current');
  const [historyEntries, setHistoryEntries] = useState<HistoryEntry[]>([]);

  // Refs to access current state in hotkey callback
  const statusRef = useRef<string>(status);
  const initializedRef = useRef<boolean>(initialized);
  const recordingStartTimeRef = useRef<number | null>(recordingStartTime);
  const historyEntriesRef = useRef<HistoryEntry[]>(historyEntries);

  // Keep refs in sync with state
  useEffect(() => { statusRef.current = status; }, [status]);
  useEffect(() => { initializedRef.current = initialized; }, [initialized]);
  useEffect(() => { recordingStartTimeRef.current = recordingStartTime; }, [recordingStartTime]);
  useEffect(() => { historyEntriesRef.current = historyEntries; }, [historyEntries]);

  useEffect(() => {
    // Load saved settings
    const savedSettings = loadSettings();
    setSettings(savedSettings);

    // Load history
    const savedHistory = loadHistory();
    setHistoryEntries(savedHistory);

    // Initialize dictation
    initDictation()
      .then((res) => {
        setInitialized(true);
        if (res.state) setStatus(res.state);
        // Apply saved settings to bridge
        return configure({ model: savedSettings.model, language: savedSettings.language });
      })
      .catch((err) => setError(String(err)));
  }, []);

  useEffect(() => {
    let interval: ReturnType<typeof setInterval>;
    if (status === 'recording' && recordingStartTime) {
      interval = setInterval(() => {
        setRecordingDuration(Math.floor((Date.now() - recordingStartTime) / 1000));
      }, 100);
    } else {
      setRecordingDuration(0);
    }
    return () => clearInterval(interval);
  }, [status, recordingStartTime]);

  const handleUpdateSettings = async (updates: Partial<Settings>) => {
    const newSettings = { ...settings, ...updates };
    setSettings(newSettings);
    saveSettings(newSettings);

    // If model or language changed, update the bridge
    if (updates.model || updates.language) {
      try {
        await configure({ model: newSettings.model, language: newSettings.language });
      } catch (err) {
        console.error('Failed to configure:', err);
      }
    }
  };

  const handleStart = async () => {
    try {
      setRecordingStartTime(Date.now());
      setError('');
      const res = await startRecording();
      if (res.state) setStatus(res.state);
      if (res.type === 'error') setError(res.error || 'Unknown error');
    } catch (err) {
      setError(String(err));
    }
  };

  const handleStop = async () => {
    const duration = recordingStartTime
      ? Math.floor((Date.now() - recordingStartTime) / 1000)
      : 0;
    try {
      setStatus('processing');
      const res = await stopRecording();
      if (res.text) {
        setTranscription(res.text);
        // Add to history
        const newHistory = addHistoryEntry(historyEntries, res.text, duration);
        setHistoryEntries(newHistory);
        saveHistory(newHistory);
      }
      if (res.type === 'error') setError(res.error || 'Unknown error');
      // Always reset status to idle after processing completes
      setStatus(res.state || 'idle');
    } catch (err) {
      setError(String(err));
      setStatus('idle');
    }
  };

  const handleClearHistory = () => {
    setHistoryEntries([]);
  };

  // Handle hotkey toggle - uses refs to access current state
  const handleHotkeyToggle = useCallback(async () => {
    if (!initializedRef.current) return;

    const currentStatus = statusRef.current;
    if (currentStatus === 'processing') return; // Ignore during processing

    if (currentStatus === 'recording') {
      // Stop recording
      const duration = recordingStartTimeRef.current
        ? Math.floor((Date.now() - recordingStartTimeRef.current) / 1000)
        : 0;
      try {
        setStatus('processing');
        const res = await stopRecording();
        if (res.text) {
          setTranscription(res.text);
          // Add to history
          const currentHistory = historyEntriesRef.current;
          const newHistory = addHistoryEntry(currentHistory, res.text, duration);
          setHistoryEntries(newHistory);
          saveHistory(newHistory);
        }
        if (res.type === 'error') setError(res.error || 'Unknown error');
        // Always reset status to idle after processing completes
        setStatus(res.state || 'idle');
      } catch (err) {
        setError(String(err));
        setStatus('idle');
      }
    } else {
      // Start recording
      try {
        setRecordingStartTime(Date.now());
        setError('');
        const res = await startRecording();
        if (res.state) setStatus(res.state);
        if (res.type === 'error') setError(res.error || 'Unknown error');
      } catch (err) {
        setError(String(err));
      }
    }
  }, []);

  // Register global hotkey
  useEffect(() => {
    if (!initialized) return;

    const shortcut = hotkeyToShortcut(settings.hotkey);

    registerHotkey(shortcut, handleHotkeyToggle)
      .then(() => {
        console.log(`Registered hotkey: ${shortcut}`);
      })
      .catch((err) => {
        console.error('Failed to register hotkey:', err);
      });

    // Cleanup on unmount or when hotkey changes
    return () => {
      unregisterHotkey().catch((err) => {
        console.warn('Failed to unregister hotkey on cleanup:', err);
      });
    };
  }, [initialized, settings.hotkey, handleHotkeyToggle]);

  // Listen for 'show-about' event from tray menu
  useEffect(() => {
    const unlisten = listen('show-about', () => {
      setShowAbout(true);
    });

    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  const getStatusColor = (currentStatus: string) => {
    switch (currentStatus) {
      case 'recording':
        return 'text-red-500';
      case 'processing':
        return 'text-yellow-500';
      case 'idle':
        return initialized ? 'text-green-500' : 'text-gray-400';
      default:
        return 'text-gray-400';
    }
  };

  const getStatusText = () => {
    if (!initialized) return 'Initializing...';
    switch (status) {
      case 'recording':
        return 'Recording';
      case 'processing':
        return 'Processing...';
      case 'idle':
        return 'Ready';
      default:
        return status;
    }
  };

  return (
    <div className="h-screen bg-gray-50 dark:bg-gray-900 flex flex-col font-[-apple-system,BlinkMacSystemFont,'Segoe_UI',Roboto,sans-serif]">
      {/* Header */}
      <header className="flex-shrink-0 px-4 py-3 border-b border-gray-200 dark:border-gray-700 bg-white/80 dark:bg-gray-800/80 backdrop-blur-sm">
        <div className="flex items-center justify-between">
          <h1 className="text-lg font-semibold text-gray-900 dark:text-white tracking-tight">
            Local Dictation
          </h1>
          {/* Status Indicator */}
          <div className="flex items-center gap-2">
            {status === 'processing' ? (
              // Spinning loader
              <svg className="w-5 h-5 animate-spin text-yellow-500" xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24">
                <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4"></circle>
                <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"></path>
              </svg>
            ) : (
              // Mic icon
              <svg
                className={`w-5 h-5 ${status === 'recording' ? 'text-red-500 animate-pulse' : 'text-gray-400'}`}
                fill="currentColor"
                viewBox="0 0 24 24"
              >
                <path d="M12 14c1.66 0 3-1.34 3-3V5c0-1.66-1.34-3-3-3S9 3.34 9 5v6c0 1.66 1.34 3 3 3z"/>
                <path d="M17 11c0 2.76-2.24 5-5 5s-5-2.24-5-5H5c0 3.53 2.61 6.43 6 6.92V21h2v-3.08c3.39-.49 6-3.39 6-6.92h-2z"/>
              </svg>
            )}
            <span className={`text-sm font-medium ${getStatusColor(status)}`}>
              {status === 'recording' && recordingDuration > 0
                ? `${recordingDuration}s`
                : getStatusText()}
            </span>
          </div>
        </div>
      </header>

      {/* Permissions Banner */}
      <PermissionsBanner />

      {/* Main Content Area */}
      <main className="flex-1 flex flex-col overflow-hidden p-4 gap-4">
        {/* Tab Navigation */}
        <div className="flex-shrink-0 flex gap-1 bg-gray-100 dark:bg-gray-800 p-1 rounded-lg">
          <button
            onClick={() => setActiveTab('current')}
            className={`flex-1 px-4 py-2 text-sm font-medium rounded-md transition-colors ${
              activeTab === 'current'
                ? 'bg-white dark:bg-gray-700 text-gray-900 dark:text-white shadow-sm'
                : 'text-gray-600 dark:text-gray-400 hover:text-gray-900 dark:hover:text-white'
            }`}
          >
            Current
          </button>
          <button
            onClick={() => setActiveTab('history')}
            className={`flex-1 px-4 py-2 text-sm font-medium rounded-md transition-colors ${
              activeTab === 'history'
                ? 'bg-white dark:bg-gray-700 text-gray-900 dark:text-white shadow-sm'
                : 'text-gray-600 dark:text-gray-400 hover:text-gray-900 dark:hover:text-white'
            }`}
          >
            History
            {historyEntries.length > 0 && (
              <span className="ml-1.5 px-1.5 py-0.5 text-xs bg-gray-200 dark:bg-gray-600 rounded-full">
                {historyEntries.length}
              </span>
            )}
          </button>
        </div>

        {/* Tab Content */}
        <div className="flex-1 overflow-y-auto bg-white dark:bg-gray-800 rounded-xl shadow-sm border border-gray-200 dark:border-gray-700 p-4 flex flex-col">
          {activeTab === 'current' ? (
            // Current Transcription Display
            transcription ? (
              <p className="text-gray-900 dark:text-gray-100 text-sm leading-relaxed whitespace-pre-wrap">
                {transcription}
              </p>
            ) : (
              <p className="text-gray-400 dark:text-gray-500 text-sm italic">
                {status === 'recording'
                  ? 'Listening...'
                  : status === 'processing'
                  ? 'Transcribing audio...'
                  : 'Press the button below to start recording'}
              </p>
            )
          ) : (
            // History Panel
            <HistoryPanel
              entries={historyEntries}
              onClearHistory={handleClearHistory}
            />
          )}
        </div>

        {/* Error Display */}
        {error && (
          <div className="flex-shrink-0 px-4 py-3 bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 rounded-lg">
            <p className="text-red-600 dark:text-red-400 text-sm">{error}</p>
          </div>
        )}

        {/* Recording Controls */}
        <div className="flex-shrink-0 flex justify-center gap-3">
          {status === 'recording' ? (
            <button
              onClick={handleStop}
              disabled={!initialized}
              className="flex items-center gap-2 px-6 py-3 bg-red-500 hover:bg-red-600 active:bg-red-700 text-white font-medium rounded-xl shadow-sm transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
            >
              <svg
                className="w-5 h-5"
                fill="currentColor"
                viewBox="0 0 20 20"
              >
                <rect x="5" y="5" width="10" height="10" rx="1" />
              </svg>
              Stop Recording
            </button>
          ) : (
            <button
              onClick={handleStart}
              disabled={!initialized || status === 'processing'}
              className="flex items-center gap-2 px-6 py-3 bg-blue-500 hover:bg-blue-600 active:bg-blue-700 text-white font-medium rounded-xl shadow-sm transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
            >
              <svg
                className="w-5 h-5"
                fill="currentColor"
                viewBox="0 0 20 20"
              >
                <circle cx="10" cy="10" r="6" />
              </svg>
              {status === 'processing' ? 'Processing...' : 'Start Recording'}
            </button>
          )}
        </div>
      </main>

      {/* Footer */}
      <footer className="flex-shrink-0 px-4 py-3 border-t border-gray-200 dark:border-gray-700 bg-white/80 dark:bg-gray-800/80 backdrop-blur-sm">
        <div className="flex justify-end">
          <button
            className="flex items-center gap-2 px-3 py-1.5 text-sm text-gray-600 dark:text-gray-400 hover:text-gray-900 dark:hover:text-gray-200 hover:bg-gray-100 dark:hover:bg-gray-700 rounded-lg transition-colors"
            onClick={() => setIsSettingsOpen(true)}
          >
            <svg
              className="w-4 h-4"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
            >
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z"
              />
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"
              />
            </svg>
            Settings
          </button>
        </div>
      </footer>

      <SettingsPanel
        isOpen={isSettingsOpen}
        onClose={() => setIsSettingsOpen(false)}
        settings={settings}
        onUpdateSettings={handleUpdateSettings}
      />

      <AboutModal
        isOpen={showAbout}
        onClose={() => setShowAbout(false)}
      />
    </div>
  );
}

export default App;
