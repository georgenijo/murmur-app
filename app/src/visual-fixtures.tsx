import React from 'react';
import ReactDOM from 'react-dom/client';
import { mockIPC } from '@tauri-apps/api/mocks';
import { MainHeader } from './components/MainHeader';
import { DictationPreviewCard } from './components/dictation-preview/DictationPreviewApp';
import { HomeDashboard } from './components/home/HomeDashboard';
import { HomeSidebar } from './components/home/HomeSidebar';
import { InsightsView } from './components/home/InsightsView';
import { SettingsPanel } from './components/settings/SettingsPanel';
import { UpdateIndicator } from './components/UpdateIndicator';
import { DEFAULT_SETTINGS } from './lib/settings';
import { AppearanceProvider } from './lib/hooks/useAppearance';
import type { DictationStatus } from './lib/types';
import type { MainDestination } from './lib/homeDashboard';
import { dayKey, loadStats } from './lib/stats';
import { useMeetings } from './lib/hooks/useMeetings';
import './styles.css';

const query = new URLSearchParams(window.location.search);
const requestedState = query.get('state') ?? 'idle';
const appearance = query.get('appearance') === 'dark' ? 'dark' : 'light';
const status: DictationStatus = requestedState === 'recording'
  ? 'recording'
  : requestedState === 'processing'
    ? 'processing'
    : requestedState === 'update-recovering'
      ? 'recovering'
      : 'idle';

document.documentElement.dataset.appearance = appearance;

mockIPC((command) => {
  if (command === 'get_meeting_status') {
    return {
      phase: 'idle',
      sessionId: null,
      elapsedMs: 0,
      chunksCommitted: 0,
      microphoneActive: false,
      systemAudioActive: false,
      errorCode: null,
    };
  }
  if (command === 'get_system_audio_permission_status') return 'granted';
  if (command === 'get_microphone_preview_status') {
    return {
      previewId: null,
      state: 'idle',
      stillConnecting: false,
      errorKind: null,
      message: null,
    };
  }
  if (command === 'start_microphone_preview') {
    return {
      previewId: 1,
      state: 'active',
      stillConnecting: false,
      errorKind: null,
      message: null,
    };
  }
  if (command === 'stop_microphone_preview') {
    return {
      previewId: null,
      state: 'idle',
      stillConnecting: false,
      errorKind: null,
      message: null,
    };
  }
  if (command === 'cancel_microphone_preview') return false;
  if (command === 'get_audio_input_inventory') {
    return {
      schemaVersion: 1,
      revision: 1,
      status: 'available',
      devices: [],
      defaultInputId: null,
      errorCode: null,
    };
  }
  if (command === 'get_model_runtime_catalog' || command.startsWith('list_')) return [];
  if (command.includes('version')) return '0.27.0';
  return null;
}, { shouldMockEvents: true });

const entries = [
  {
    id: 'older',
    text: 'History owns the window.',
    timestamp: Date.UTC(2026, 7, 6, 14, 37),
    duration: 2,
    source: 'recording' as const,
  },
  {
    id: 'file',
    text: 'Imported audio uses the same spacing rhythm without taking over the workspace.',
    timestamp: Date.UTC(2026, 7, 6, 14, 47),
    duration: 38,
    source: 'file' as const,
    sourceName: 'design-review.wav',
  },
  {
    id: 'newest',
    text: 'The compact transcript keeps its metadata aligned and its actions quiet.',
    timestamp: Date.UTC(2026, 7, 6, 15, 26),
    duration: 8,
    source: 'recording' as const,
  },
];

const fixtureSettings = {
  ...DEFAULT_SETTINGS,
  vocabularyEntries: [{
    id: 'fixture-term',
    written: 'Murmur',
    aliases: ['murmur app'],
    enabled: true,
    scope: { kind: 'global' as const },
  }],
  appProfiles: [],
};

const fixtureBuckets = Object.fromEntries(Array.from({ length: 7 }, (_, index) => {
  const date = new Date();
  date.setDate(date.getDate() - (6 - index));
  return [dayKey(date), {
    words: [420, 680, 310, 900, 560, 180, 760][index],
    recordings: [3, 5, 2, 7, 4, 1, 6][index],
    recordingSeconds: [180, 240, 160, 300, 220, 90, 260][index],
  }];
}));

localStorage.setItem('dictation-stats', JSON.stringify({
  ...loadStats(),
  totalWords: 5168,
  totalRecordings: 220,
  totalDurationSeconds: 1640,
  wpmSamples: [184, 192, 189, 191],
  dailyBuckets: fixtureBuckets,
}));

function VisualFixture() {
  const settingsOpen = requestedState === 'settings' || requestedState === 'settings-appearance';
  const meetings = useMeetings(fixtureSettings);
  const [destination, setDestination] = React.useState<MainDestination>(requestedState === 'insights' ? 'insights' : 'home');

  return (
    <div
      data-appearance={appearance}
      data-visual-ready="true"
      className="flex h-screen w-screen flex-col overflow-hidden bg-background text-on-surface"
    >
      <MainHeader
        status={status}
        initialized
        recordingDuration={12}
        audioLevel={status === 'recording' ? 0.045 : 0}
        recordingMode={requestedState === 'update-recovering' ? 'both' : 'hold_down'}
        onRecord={() => {}}
        onStop={() => {}}
        onOpenSettings={() => {}}
        settingsOpen={settingsOpen}
        triggerKey="shift_l"
        mode={settingsOpen ? 'settings' : 'main'}
        showRecordControls={false}
        updateIndicator={requestedState === 'update-recovering' ? (
          <UpdateIndicator
            status={{
              phase: 'available',
              version: 'v0.27.1',
              notes: '',
              isForced: false,
            }}
            onOpen={() => {}}
            onRetryCheck={() => {}}
          />
        ) : undefined}
      />
      {settingsOpen ? (
        <SettingsPanel
          settings={fixtureSettings}
          onUpdateSettings={() => {}}
          initialized
          status="idle"
          onResetStats={() => {}}
          onRerunSetup={() => {}}
          accessibilityGranted
          onCheckForUpdate={async () => {}}
          updateStatus={{ phase: 'idle' }}
          configureError={null}
        />
      ) : (
        <div className="flex min-h-0 flex-1 overflow-hidden">
          <HomeSidebar active={destination} onNavigate={setDestination} onOpenSettings={() => {}} />
          <div className="main-dashboard-workspace">
            {destination === 'insights' ? (
              <InsightsView
                statsVersion={0}
                settings={fixtureSettings}
                onOpenVocabulary={() => {}}
                onOpenStyles={() => {}}
              />
            ) : (
              <HomeDashboard
                historyEntries={entries}
                onClearHistory={() => {}}
                onUpdateHistoryEntry={() => {}}
                onTranscribeFile={() => {}}
                status={status}
                initialized
                recordingDuration={12}
                audioLevel={status === 'recording' ? 0.045 : 0}
                settings={fixtureSettings}
                meetings={meetings}
                statsVersion={0}
                onRecord={() => {}}
                onStop={() => {}}
                onOpenInsights={() => setDestination('insights')}
                onOpenSettings={() => {}}
              />
            )}
          </div>
        </div>
      )}
    </div>
  );
}

/** Live dictation preview card over a stand-in desktop, at the real window
 *  width (460pt) so the wrapping matches what ships. */
function DictationPreviewFixture() {
  const text = query.get('text')
    ?? 'Okay, um, one thing I want to clear up is why is it showing me a snippet of my prompt';
  return (
    <div className="flex min-h-screen items-start justify-center bg-[#6b6fae] pt-6">
      <div style={{ width: 460 }}>
        <DictationPreviewCard text={text} />
      </div>
    </div>
  );
}

ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <React.StrictMode>
    {requestedState === 'dictation-preview' ? (
      <DictationPreviewFixture />
    ) : requestedState === 'settings-appearance' ? (
      <AppearanceProvider>
        <VisualFixture />
      </AppearanceProvider>
    ) : (
      <VisualFixture />
    )}
  </React.StrictMode>,
);
