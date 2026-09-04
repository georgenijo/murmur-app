import React from 'react';
import ReactDOM from 'react-dom/client';
import { mockIPC } from '@tauri-apps/api/mocks';
import { MainHeader } from './components/MainHeader';
import { CommandPalette } from './components/CommandPalette';
import { AboutModal } from './components/AboutModal';
import { DictationPreviewCard } from './components/dictation-preview/DictationPreviewApp';
import { HomeDashboard } from './components/home/HomeDashboard';
import { HomeSidebar } from './components/home/HomeSidebar';
import { InsightsView } from './components/home/InsightsView';
import { MeetingsPanel } from './components/history/MeetingsPanel';
import { SettingsPanel } from './components/settings/SettingsPanel';
import { UpdateIndicator } from './components/UpdateIndicator';
import { WorkspacePageHeader } from './components/ui/DashboardPrimitives';
import { DEFAULT_SETTINGS } from './lib/settings';
import { AppearanceProvider } from './lib/hooks/useAppearance';
import type { DictationStatus } from './lib/types';
import type { MainDestination } from './lib/homeDashboard';
import type { HistoryEntry } from './lib/history';
import { dayKey, loadStats } from './lib/stats';
import { useMeetings } from './lib/hooks/useMeetings';
import { DEFAULT_THEME, applyResolvedTheme, resolveTheme, type ThemeConfigV1 } from './lib/appearance';
import './styles.css';

const query = new URLSearchParams(window.location.search);
const requestedState = query.get('state') ?? 'idle';
const appearance = query.get('appearance') === 'dark' ? 'dark' : 'light';
const importedThemeFixture = query.get('theme');
const status: DictationStatus = requestedState === 'recording'
  ? 'recording'
  : requestedState === 'processing'
    ? 'processing'
    : requestedState === 'update-recovering'
      ? 'recovering'
      : 'idle';

const importedThemeFixtures: Record<string, ThemeConfigV1> = {
  'open-vsx-low-contrast': {
    version: 1,
    presetId: 'custom',
    accent: '#b8b8b8',
    light: {
      background: '#ededed',
      surface: '#ebebeb',
      'on-surface': '#dedede',
      'on-surface-variant': '#d7d7d7',
      'outline-variant': '#e3e3e3',
    },
    dark: {
      background: '#25282b',
      surface: '#272a2d',
      'on-surface': '#303438',
      'on-surface-variant': '#34383c',
      'outline-variant': '#2d3135',
    },
  },
  'open-vsx-high-saturation': {
    version: 1,
    presetId: 'custom',
    accent: '#ff00d4',
    light: {
      background: '#fff500',
      surface: '#00f5ff',
      'on-surface': '#ff00d4',
      'on-surface-variant': '#002bff',
      'outline-variant': '#ff3b00',
    },
    dark: {
      background: '#19002f',
      surface: '#001b51',
      'on-surface': '#ff2bd6',
      'on-surface-variant': '#00f5ff',
      'outline-variant': '#ff4d00',
    },
  },
};

const importedTheme = importedThemeFixture
  ? importedThemeFixtures[importedThemeFixture]
  : undefined;
if (importedTheme) applyResolvedTheme(resolveTheme(importedTheme, appearance));
else if (appearance === 'dark') applyResolvedTheme(resolveTheme(DEFAULT_THEME, appearance));
else document.documentElement.dataset.appearance = appearance;

const meetingFixture = {
  session: {
    id: 'meeting-fixture', startedAtMs: Date.UTC(2026, 7, 31, 14, 30), endedAtMs: Date.UTC(2026, 7, 31, 14, 48),
    status: 'complete', modelName: 'base.en', language: 'en', smartPunctuation: true,
    retainAudio: false, durationMs: 1_080_000, segmentCount: 2,
    preview: 'We agreed to ship the local review workspace.', errorCode: null,
  },
  segments: [
    { id: 101, sessionId: 'meeting-fixture', speaker: 'me', sequence: 0, startMs: 12_000, endMs: 18_000, status: 'final', text: 'We agreed to ship the local review workspace.', audioAvailable: false, errorCode: null },
    { id: 102, sessionId: 'meeting-fixture', speaker: 'them', sequence: 0, startMs: 27_000, endMs: 35_000, status: 'final', text: 'I will verify the export formats and source links.', audioAvailable: false, errorCode: null },
  ],
  labels: { me: 'George', them: 'Alex' },
  generated: { revision: 2, document: { schema: 'murmur.meeting-review.v1', summary: { key: 'summary', text: 'The team agreed to ship and verify the local review workspace.', sourceSegmentIds: [101, 102] }, decisions: [{ key: 'decision:0', text: 'Ship the review workspace locally.', sourceSegmentIds: [101] }], actionItems: [{ key: 'action:0', text: 'Verify export formats and source links.', owner: 'Alex', dueDate: null, sourceSegmentIds: [102] }], openQuestions: [] } },
  review: { revision: 1, basedOnGeneratedRevision: 1, document: { schema: 'murmur.meeting-review.v1', summary: { key: 'summary', text: 'The meeting review is ready for final verification.', sourceSegmentIds: [101, 102] }, decisions: [{ key: 'decision:0', text: 'Keep all review data local.', sourceSegmentIds: [101] }], actionItems: [{ key: 'action:0', text: 'Verify every export format.', owner: 'Alex', dueDate: null, sourceSegmentIds: [102] }], openQuestions: [] } },
  activeDocument: { schema: 'murmur.meeting-review.v1', summary: { key: 'summary', text: 'The meeting review is ready for final verification.', sourceSegmentIds: [101, 102] }, decisions: [{ key: 'decision:0', text: 'Keep all review data local.', sourceSegmentIds: [101] }], actionItems: [{ key: 'action:0', text: 'Verify every export format.', owner: 'Alex', dueDate: null, sourceSegmentIds: [102] }], openQuestions: [] },
  activeOrigin: 'reviewed',
};

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
  if (command === 'get_meeting_summary_status') {
    return { generation: 0, sessionId: null, phase: 'idle', completedChunks: 0, totalChunks: 0, elapsedMs: 0, peakRssMb: 0, errorCode: null };
  }
  if (command === 'list_meetings') {
    return requestedState === 'meetings-empty'
      ? { sessions: [], total: 0, offset: 0, limit: 50 }
      : { sessions: [meetingFixture.session], total: 1, offset: 0, limit: 50 };
  }
  if (command === 'get_meeting') return meetingFixture;
  if (command === 'get_meeting_review_export') return '# Meeting review\n';
  if (command === 'save_meeting_review' || command === 'restore_meeting_review_from_generated') return meetingFixture;
  if (command === 'save_meeting_review_export') return 20;
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
      devices: [
        { id: 'fixture-built-in', name: 'MacBook Pro Microphone' },
        { id: 'fixture-desk', name: 'Desk Microphone' },
      ],
      defaultInputId: 'fixture-built-in',
      errorCode: null,
    };
  }
  if (command === 'get_knowledge_store_status') {
    return {
      availability: 'ready',
      schemaVersion: 1,
      recordCount: 0,
      storeRevision: 0,
      recoveryAtMs: null,
      message: null,
    };
  }
  if (command === 'list_knowledge') {
    return { entries: [], total: 0, nextOffset: null, storeRevision: 0 };
  }
  if (command === 'get_model_runtime_catalog' || command.startsWith('list_')) return [];
  if (command.includes('version')) return '0.27.0';
  return null;
}, { shouldMockEvents: true });

const fixtureToday = new Date();

function fixtureTimestamp(dayOffset: number, hour: number, minute: number): number {
  return new Date(
    fixtureToday.getFullYear(),
    fixtureToday.getMonth(),
    fixtureToday.getDate() + dayOffset,
    hour,
    minute,
  ).getTime();
}

const entries: HistoryEntry[] = [
  ...Array.from({ length: 14 }, (_, index): HistoryEntry => {
    const dayOffset = index - 13;
    const daysAgo = Math.abs(dayOffset);
    const source = index % 5 === 0 ? 'file' : 'recording';
    return {
      schemaVersion: 2,
      id: `activity-${index}`,
      text: dayOffset === 0
        ? 'History owns the window, with the newest work close at hand and older notes one scroll away.'
        : `Local fixture dictation from ${daysAgo} ${daysAgo === 1 ? 'day' : 'days'} ago keeps the activity view tied to real transcript timestamps.`,
      timestamp: fixtureTimestamp(dayOffset, 9 + (index % 4), 5 + index),
      duration: 6 + index,
      source,
      ...(source === 'file' ? { sourceName: `local-note-${index + 1}.wav` } : {}),
    };
  }),
  {
    schemaVersion: 2,
    id: 'file',
    text: 'Imported audio uses the same spacing rhythm without taking over the workspace.',
    timestamp: fixtureTimestamp(0, 14, 47),
    duration: 38,
    source: 'file',
    sourceName: 'design-review.wav',
  },
  {
    schemaVersion: 2,
    id: 'newest',
    text: 'The compact transcript keeps its metadata aligned and its actions quiet.',
    timestamp: fixtureTimestamp(0, 15, 26),
    duration: 8,
    source: 'recording',
  },
];

const fixtureSettings = {
  ...DEFAULT_SETTINGS,
  siteModeLookupEnabled: requestedState === 'settings-site-modes',
  browserSiteRules: requestedState === 'settings-site-modes' ? [{
    id: 'fixture-github',
    browserBundleId: 'com.apple.Safari',
    host: 'github.com',
    modeId: 'builtin.technical',
    enabled: true,
  }] : [],
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

const fixtureStats = loadStats();
localStorage.setItem('dictation-stats', JSON.stringify({
  ...fixtureStats,
  totalWords: 5168,
  totalRecordings: 220,
  totalDurationSeconds: 1640,
  wpmSamples: [184, 192, 189, 191],
  dailyBuckets: fixtureBuckets,
  query: {
    ...fixtureStats.query,
    queriesRun: 11,
    successfulQueries: 9,
    failedQueries: 2,
    inputTokens: 4_280,
    outputTokens: 1_360,
    reportedCostUsd: 0.084,
    byProvider: {
      ...fixtureStats.query.byProvider,
      claude: {
        queriesRun: 7,
        successfulQueries: 6,
        failedQueries: 1,
        inputTokens: 2_960,
        outputTokens: 940,
        reportedCostUsd: 0.057,
      },
      codex: {
        queriesRun: 4,
        successfulQueries: 3,
        failedQueries: 1,
        inputTokens: 1_320,
        outputTokens: 420,
        reportedCostUsd: 0.027,
      },
    },
    failuresByErrorCode: { timed_out: 1, auth_failed: 1 },
  },
}));

function VisualFixture() {
  const settingsOpen = requestedState === 'settings'
    || requestedState === 'settings-appearance'
    || requestedState === 'settings-site-modes';
  const meetings = useMeetings(fixtureSettings);
  const [destination, setDestination] = React.useState<MainDestination>(
    requestedState === 'insights' ? 'insights' : requestedState.startsWith('meetings-') ? 'meetings' : 'home',
  );
  const homeNavigationRef = React.useRef<HTMLButtonElement>(null);
  const restoreHomeNavigationFocusRef = React.useRef(false);

  const backToHome = () => {
    restoreHomeNavigationFocusRef.current = true;
    setDestination('home');
  };

  React.useLayoutEffect(() => {
    if (destination !== 'home' || !restoreHomeNavigationFocusRef.current) return;
    restoreHomeNavigationFocusRef.current = false;
    homeNavigationRef.current?.focus();
  }, [destination]);

  React.useEffect(() => {
    if (requestedState === 'meetings-review') void meetings.select('meeting-fixture');
  }, [meetings.select]);

  return (
    <div
      data-appearance={appearance}
      data-theme-fixture={importedThemeFixture ?? 'sonic'}
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
          <HomeSidebar
            active={destination}
            homeButtonRef={homeNavigationRef}
            onNavigate={setDestination}
          />
          <div className="main-dashboard-workspace">
            {destination === 'insights' ? (
              <InsightsView
                statsVersion={0}
                onBackToHome={backToHome}
              />
            ) : destination === 'meetings' ? (
              <section className="main-secondary-view" aria-labelledby="meetings-view-title">
                <WorkspacePageHeader
                  title="Notetaker"
                  titleId="meetings-view-title"
                  description="Local meeting transcripts and summaries."
                  back={{ label: 'Back to Home', onActivate: backToHome }}
                />
                <MeetingsPanel meetings={meetings} />
              </section>
            ) : destination === 'queries' ? (
              <section className="main-secondary-view" aria-labelledby="queries-view-title">
                <WorkspacePageHeader
                  title="Queries"
                  titleId="queries-view-title"
                  description="Questions and answers retained explicitly on this Mac."
                  back={{ label: 'Back to Home', onActivate: backToHome }}
                />
              </section>
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
      <CommandPalette
        isOpen={requestedState === 'palette'}
        onClose={() => {}}
        commands={[
          { id: 'record', title: 'Start recording', section: 'Dictation', hint: '⇧ hold', run: () => {} },
          { id: 'transcribe', title: 'Transcribe audio file…', section: 'Dictation', run: () => {} },
          { id: 'history', title: 'Search transcripts', section: 'History', hint: '⌘F', run: () => {} },
          { id: 'insights', title: 'Open Insights', section: 'Navigate', run: () => {} },
          { id: 'settings', title: 'Open Settings', section: 'Navigate', hint: '⌘,', run: () => {} },
          { id: 'logs', title: 'Open Performance workspace', section: 'Navigate', hint: '⌘L', run: () => {} },
        ]}
      />
      <AboutModal isOpen={requestedState === 'about'} onClose={() => {}} />
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
