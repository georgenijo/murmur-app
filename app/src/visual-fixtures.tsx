import { AppSurface } from './components/ui/AppSurface';
import React from 'react';
import ReactDOM from 'react-dom/client';
import { Channel } from '@tauri-apps/api/core';
import { useAutoUpdater } from './lib/hooks/useAutoUpdater';
import { mockIPC } from '@tauri-apps/api/mocks';
import { MainHeader } from './components/MainHeader';
import { ModelDownloader } from './components/ModelDownloader';
import { TransformDiagnosticsView } from './components/log-viewer/TransformDiagnosticsView';
import { DictationDiagnosticsView } from './components/log-viewer/DictationDiagnosticsView';
import { RunsView } from './components/log-viewer/RunsView';
import { OnboardingFlow } from './components/onboarding/OnboardingFlow';
import { CommandPalette } from './components/CommandPalette';
import { AboutModal } from './components/AboutModal';
import { DictationPreviewCard } from './components/dictation-preview/DictationPreviewApp';
import { HomeDashboard } from './components/home/HomeDashboard';
import { HomeSidebar } from './components/home/HomeSidebar';
import { InsightsView } from './components/home/InsightsView';
import { MeetingsPanel } from './components/history/MeetingsPanel';
import { QueryHistoryPanel } from './components/history/QueryHistoryPanel';
import { useQueryHistory } from './lib/hooks/useQueryHistory';
import { AssistantWorkspace } from './components/assistant/AssistantWorkspace';
import { QueryReviewView } from './components/query-review/QueryReviewApp';
import './components/assistant/assistant.css';
import { Button, Input } from './components/ui/FormControls';
import { Select } from './components/ui/Select';
import AnimatedSwitch from './components/ui/animated-switch/animated-switch';
import { SettingsSection } from './components/settings/SettingsSection';
import { SettingsRow, SettingsCard, SettingsUnavailableGroup } from './components/settings/SettingsRow';
import { SettingToggle } from './components/settings/SettingToggle';
import type { SettingsEditorTab } from './components/settings/SettingsEditorsWindow';
import { SettingsPanel } from './components/settings/SettingsPanel';
import { UpdateIndicator } from './components/UpdateIndicator';
import { UpdateModal } from './components/UpdateModal';
import { Copy, Folder } from 'lucide-react';
import { MiddleTruncate } from './components/ui/middle-truncate';
import { DashboardAction } from './components/ui/DashboardPrimitives';
import { AnimatedDropdown, AnimatedDropdownTrigger, AnimatedDropdownContent, AnimatedDropdownItem, AnimatedDropdownSeparator } from './components/ui/animated-dropdown/animated-dropdown';
import { FluidTooltip } from './components/ui/fluid-tooltip/fluid-tooltip';
import { WorkspacePageHeader } from './components/ui/DashboardPrimitives';
import { DiscoveryHintToast } from './components/DiscoveryHintToast';
import { DEFAULT_SETTINGS, type Settings } from './lib/settings';
import { AppearanceProvider } from './lib/hooks/useAppearance';
import type { DictationStatus } from './lib/types';
import type { MainDestination } from './lib/homeDashboard';
import { removeHistoryEntries, toggleHistoryEntryPinned, type HistoryEntry } from './lib/history';
import { dayKey, loadStats } from './lib/stats';
import { useMeetings } from './lib/hooks/useMeetings';
import { DEFAULT_THEME, OPENCODE_THEME, applyResolvedTheme, parseVsCodeThemeFile, resolveTheme, type ThemeConfigV1 } from './lib/appearance';
import { VoiceIslandFixture } from './visual-fixtures/VoiceIslandFixture';
import './styles.css';

const query = new URLSearchParams(window.location.search);
const requestedState = query.get('state') ?? 'idle';
const copyFixture = requestedState === 'settings-copy';
const copyKnowledge = [
  { id: 'copy-transform', payload: { kind: 'transform', name: 'Meeting notes', instruction: 'Summarize decisions and action items.' }, enabled: true, scope: { kind: 'global' }, provenance: 'manual', createdAtMs: 1, updatedAtMs: 2, revision: 1 },
  { id: 'copy-command', payload: { kind: 'replacement_rule', source: 'insert greeting', replacement: 'Hello, team' }, enabled: true, scope: { kind: 'global' }, provenance: 'manual', voiceCommand: { commandType: 'text_replacement', allowClipboardRead: false }, createdAtMs: 1, updatedAtMs: 2, revision: 1 },
];
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
  opencode: OPENCODE_THEME,
  'flat-dark': parseVsCodeThemeFile({
    name: 'Flat dark',
    type: 'dark',
    colors: {
      'editor.background': '#111111',
      'editor.foreground': '#c4c4cc',
      'sideBar.background': '#111111',
      'panel.background': '#111111',
      'menu.background': '#111111',
      'panel.border': '#111111',
      'focusBorder': '#999999',
    },
  }).theme,
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

const shortUpdateNotes = `## What's Changed

- Faster local transcription.
- More reliable microphone startup.
- Clearer recording controls.`;

const longUpdateNotes = `## New Features

- Smart Auto can select from approved microphones with recent signal.
- Voice Query can read an explicitly approved workspace for the current session.
- Meeting review can assign local names to remote speakers.

## Bug Fixes

- Microphone inventory excludes output-only devices.
- Recording diagnostics report post-stop delivery latency accurately.
- Private capture review keeps transcript text collapsed by default.
- The notch restores its active state after a webview reload.
- Markdown links follow CommonMark heading rules.

## Other Changes

- Settings use consistent row spacing and branch alignment.
- Long-session echo-cancellation checks support thirty-minute captures.`;

const meetingFixture = {
  session: {
    title: requestedState === 'meetings-review-audio' ? 'Synthetic retained-audio review' : null,
    titleSource: requestedState === 'meetings-review-audio' ? 'manual' : null,
    attendees: [],
    id: 'meeting-fixture', startedAtMs: Date.UTC(2026, 7, 31, 14, 30), endedAtMs: Date.UTC(2026, 7, 31, 14, 48),
    status: 'complete', modelName: 'base.en', language: 'en', smartPunctuation: true,
    retainAudio: requestedState === 'meetings-review-audio', durationMs: 1_080_000, segmentCount: 2,
    preview: 'We agreed to ship the local review workspace.', errorCode: null,
  },
  segments: [
    { id: 101, sessionId: 'meeting-fixture', speaker: 'me', remoteSpeakerId: null, sequence: 0, startMs: 12_000, endMs: 18_000, status: 'final', text: 'We agreed to ship the local review workspace.', audioAvailable: requestedState === 'meetings-review-audio', errorCode: null },
    { id: 102, sessionId: 'meeting-fixture', speaker: 'them', remoteSpeakerId: null, sequence: 0, startMs: 27_000, endMs: 35_000, status: 'final', text: 'I will verify the export formats and source links.', audioAvailable: requestedState === 'meetings-review-audio', errorCode: null },
  ],
  labels: { me: 'George', them: 'Alex' },
  remoteSpeakers: [],
  generated: requestedState === 'meetings-review-audio' ? null : { revision: 2, document: { schema: 'murmur.meeting-review.v1', summary: { key: 'summary', text: 'The team agreed to ship and verify the local review workspace.', sourceSegmentIds: [101, 102] }, decisions: [{ key: 'decision:0', text: 'Ship the review workspace locally.', sourceSegmentIds: [101] }], actionItems: [{ key: 'action:0', text: 'Verify export formats and source links.', owner: 'Alex', dueDate: null, sourceSegmentIds: [102] }], openQuestions: [] } },
  review: requestedState === 'meetings-review-audio' ? null : { revision: 1, basedOnGeneratedRevision: 1, document: { schema: 'murmur.meeting-review.v1', summary: { key: 'summary', text: 'The meeting review is ready for final verification.', sourceSegmentIds: [101, 102] }, decisions: [{ key: 'decision:0', text: 'Keep all review data local.', sourceSegmentIds: [101] }], actionItems: [{ key: 'action:0', text: 'Verify every export format.', owner: 'Alex', dueDate: null, sourceSegmentIds: [102] }], openQuestions: [] } },
  activeDocument: requestedState === 'meetings-review-audio' ? null : { schema: 'murmur.meeting-review.v1', summary: { key: 'summary', text: 'The meeting review is ready for final verification.', sourceSegmentIds: [101, 102] }, decisions: [{ key: 'decision:0', text: 'Keep all review data local.', sourceSegmentIds: [101] }], actionItems: [{ key: 'action:0', text: 'Verify every export format.', owner: 'Alex', dueDate: null, sourceSegmentIds: [102] }], openQuestions: [] },
  activeOrigin: requestedState === 'meetings-review-audio' ? null : 'reviewed',
};

// A realistic backlog: many sessions across weeks, mostly untitled, with a long
// retained-audio transcript and no review yet.
const manyMeetings = requestedState === 'meetings-many';
const manyMeetingLines = [
  'Thanks for jumping on. I wanted to walk through the launch checklist before Friday.',
  'Sounds good. I pulled the latest numbers this morning, so we can start there.',
  'The onboarding drop-off is still mostly at the microphone permission step.',
  'Right, and the new copy should help, but I want to see a week of data first.',
  'Can we also confirm the export formats work with the calendar names?',
  'I tested Markdown and plain text. Captions still need a pass.',
  'Okay. Let us put captions on the list for next sprint then.',
  'One more thing: the notetaker list gets hard to scan once there are a lot of meetings.',
  'Agreed. Grouping by day would already help a lot.',
  'I will draft something and share it by Thursday.',
  'Perfect. Anything else before we wrap?',
  'No, I think that covers it. Thanks everyone.',
];
const manyMeetingSessions = Array.from({ length: 14 }, (_, index) => {
  const startedAtMs = Date.UTC(2026, 8, 4, 18, 0) - index * 19 * 60 * 60 * 1000;
  const titles: Record<number, string> = { 1: 'Launch checklist sync', 4: 'Design review', 9: 'Weekly planning' };
  return {
    id: `meeting-${index}`,
    title: titles[index] ?? null,
    titleSource: titles[index] ? 'manual' : null,
    attendees: [],
    startedAtMs,
    endedAtMs: startedAtMs + (12 + index * 3) * 60_000,
    status: index === 6 ? 'interrupted' : 'complete',
    modelName: 'base.en', language: 'en', smartPunctuation: true,
    retainAudio: index % 3 === 0,
    durationMs: (12 + index * 3) * 60_000,
    segmentCount: 40 + index * 23,
    preview: manyMeetingLines[index % manyMeetingLines.length],
    errorCode: null,
  };
});
const manyMeetingDetail = (id: string) => {
  const session = manyMeetingSessions.find((candidate) => candidate.id === id) ?? manyMeetingSessions[0];
  return {
    session,
    segments: manyMeetingLines.map((text, index) => ({
      id: 500 + index, sessionId: session.id, speaker: index % 2 === 0 ? 'me' : 'them', remoteSpeakerId: null,
      sequence: index, startMs: 4_000 + index * 21_000, endMs: 4_000 + index * 21_000 + 15_000,
      status: 'final', text, audioAvailable: session.retainAudio, errorCode: null,
    })),
    labels: { me: 'Me', them: 'Them' },
    remoteSpeakers: [],
    generated: null,
    review: null,
    activeDocument: null,
    activeOrigin: null,
  };
};

function QueryHistoryFixture() {
  const history = useQueryHistory(true);
  return <QueryHistoryPanel history={history} retentionEnabled />;
}

const queryHistoryFixture = {
  schemaVersion: 1,
  entries: [
    {
      schemaVersion: 1, id: 'a'.repeat(32), timestampMs: Date.UTC(2026, 8, 4, 17, 12), provider: 'claude',
      question: 'What should I review before the launch sync?',
      answer: 'Three things are worth a look:\n\n1. **Onboarding drop-off** at the microphone step.\n2. Caption export, which still needs a pass.\n3. The meeting list once many sessions pile up.',
      tokens: null, durationMs: 6_400, errorCode: null,
    },
    {
      schemaVersion: 1, id: 'b'.repeat(32), timestampMs: Date.UTC(2026, 8, 3, 21, 40), provider: 'codex',
      question: 'Summarize yesterday in one line.',
      answer: 'Mostly planning: the launch checklist, export formats, and a design review.',
      tokens: null, durationMs: 3_100, errorCode: null,
    },
  ],
  total: 2, offset: 0, hasMore: false,
};

mockIPC((command, payload) => {
  // #850 regression states use synthetic captures and record only invoked IDs.
  if (requestedState === 'copy-diagnostics') {
    const replacement = document.documentElement.hasAttribute('data-copy-replace-captures');
    const capture = { captureId: replacement ? 'capture-two' : 'capture-one', recordingId: replacement ? 42 : 41,
      transformPassId: replacement ? 52 : 51, capturedAtMs: 1_788_552_000_000, expiresAtMs: 1_789_156_800_000, outcome: 'success', hasContent: true };
    if (command === 'get_dictation_diagnostic_capture_status') return { state: 'unarmed' };
    if (command === 'get_transform_diagnostic_capture_status') return { armed: false, expiresAtMs: null };
    if (command === 'list_transform_attempts') return { schemaVersion: 1, attempts: [] };
    if (command === 'list_transform_diagnostic_captures' || command === 'list_dictation_diagnostic_captures') {
      if (command === 'list_transform_diagnostic_captures' && document.documentElement.hasAttribute('data-copy-delay-refresh')) {
        document.documentElement.setAttribute('data-copy-refresh-waiting', 'true');
        return new Promise(resolve => window.addEventListener('copy-release-refresh', () => {
          document.documentElement.removeAttribute('data-copy-refresh-waiting');
          document.documentElement.setAttribute('data-copy-listed-capture', capture.captureId);
          resolve([capture]);
        }, { once: true }));
      }
      document.documentElement.setAttribute('data-copy-listed-capture', capture.captureId);
      return [capture];
    }
    if (command === 'get_transform_diagnostic_capture') return { ...capture, schemaVersion: 1, selection: 'Synthetic selection', instruction: 'Shorten', output: 'Synthetic output', phases: [] };
    if (command === 'get_dictation_diagnostic_capture') return { ...capture, schemaVersion: 1, result: { kind: 'noContent', outcome: 'cancelled', errorCode: 'cancelled' } };
    if (command === 'delete_transform_diagnostic_capture' || command === 'delete_dictation_diagnostic_capture') {
      document.documentElement.setAttribute('data-copy-deleted-capture', String((payload as { captureId: string }).captureId));
      return;
    }
  }
  if (requestedState === 'copy-onboarding') {
    const permission = query.get('permission') ?? 'unknown';
    if (command === 'check_microphone_permission_status') return permission;
    if (command === 'check_accessibility_permission') return permission === 'loading' ? new Promise(() => {}) : permission === 'granted';
    if (command === 'get_system_audio_permission_status') return permission === 'granted' || permission === 'denied' ? permission : 'unknown';
    if (command === 'get_calendar_permission_status') return permission === 'unknown' || permission === 'loading' ? 'notDetermined' : permission;
    if (command === 'request_system_audio_permission' || command === 'request_calendar_permission') return new Promise(() => {});
  }
  if (command === 'list_query_history') return queryHistoryFixture;
  if (manyMeetings) {
    if (command === 'get_system_audio_permission_status') return 'unknown';
    if (command === 'list_meetings') {
      return { sessions: manyMeetingSessions, searchMatches: {}, total: manyMeetingSessions.length, offset: 0, limit: 50 };
    }
    if (command === 'get_meeting') {
      const id = payload && typeof payload === 'object' && 'id' in payload ? String(payload.id) : 'meeting-0';
      return manyMeetingDetail(id);
    }
    if (command === 'get_meeting_audio_manifest') {
      return {
        kind: 'available', sessionId: 'meeting-0', durationMs: 12 * 60_000,
        chunks: [{ segmentId: 500, channel: 'me', startMs: 4_000, endMs: 19_000 }], nextCursor: null,
      };
    }
  }
  if (requestedState === 'update-flow') {
    if (command === 'plugin:app|version') return '0.44.1';
    if (command === 'get_update_install_environment') return { appTranslocated: false };
    if (command === 'plugin:updater|check') {
      return { rid: 1, currentVersion: '0.44.1', version: '0.44.2', body: shortUpdateNotes, rawJson: {} };
    }
    if (command === 'plugin:updater|download') {
      const channel = payload && 'onEvent' in payload ? payload.onEvent : null;
      if (!(channel instanceof Channel)) throw new Error('Missing updater progress channel');
      channel.onmessage({ event: 'Started', data: { contentLength: 100 } });
      channel.onmessage({ event: 'Progress', data: { chunkLength: 65 } });
      return new Promise<number>((resolve) => setTimeout(() => {
        channel.onmessage({ event: 'Finished' });
        resolve(2);
      }, 1500));
    }
    if (command === 'plugin:updater|install' || command === 'plugin:process|restart') {
      const key = command === 'plugin:updater|install' ? 'data-update-installs' : 'data-update-restarts';
      document.documentElement.setAttribute(key, String(Number(document.documentElement.getAttribute(key) ?? 0) + 1));
      return;
    }
  }
  if (command === 'configure_smart_auto_probe' || command === 'retry_smart_auto_probe') return 1;
  if (command === 'get_meeting_status') {
    return {
      phase: 'idle',
      sessionId: null,
      elapsedMs: 0,
      chunksCommitted: 0,
      microphoneActive: false,
      systemAudioActive: false,
      echoCancellation: { state: 'off' },
      errorCode: null,
    };
  }
  if (command === 'get_system_audio_permission_status') return 'granted';
  if (command === 'get_calendar_permission_status') return query.get('calendar') === 'denied' ? 'denied' : 'granted';
  if (command === 'get_meeting_calendar_events') return [
    { selectionToken: 'first-event', title: 'Orion planning', attendees: ['Alex Example', 'Casey Example'], startMs: Date.UTC(2026, 7, 31, 14), endMs: Date.UTC(2026, 7, 31, 15) },
    { selectionToken: 'second-event', title: 'Design review', attendees: ['Jordan Example'], startMs: Date.UTC(2026, 7, 31, 14, 30), endMs: Date.UTC(2026, 7, 31, 15, 30) },
  ];
  if (command === 'get_meeting_summary_status') {
    return { generation: 0, sessionId: null, phase: 'idle', completedChunks: 0, totalChunks: 0, elapsedMs: 0, peakRssMb: 0, errorCode: null };
  }
  if (command === 'list_meetings') {
    return requestedState === 'meetings-empty'
      ? { sessions: [], searchMatches: {}, total: 0, offset: 0, limit: 50 }
      : { sessions: [meetingFixture.session], searchMatches: {}, total: 1, offset: 0, limit: 50 };
  }
  if (command === 'get_meeting') return meetingFixture;
  if (command === 'get_meeting_audio_capture_busy') return false;
  if (command === 'get_meeting_audio_manifest') return requestedState === 'meetings-review-audio'
    ? {
      kind: 'available',
      sessionId: 'meeting-fixture',
      durationMs: 1_080_000,
      chunks: [{ segmentId: 101, channel: 'me', startMs: 12_000, endMs: 18_000 }],
      nextCursor: null,
    }
    : { kind: 'unavailable', reason: 'notRetained' };
  if (command === 'get_meeting_review_export') return '# Meeting review\n';
  if (command === 'save_meeting_review' || command === 'restore_meeting_review_from_generated') return meetingFixture;
  if (command === 'save_meeting_review_export') return 20;
  if (command === 'get_meeting_bundle_preview') return { audioSegments: 1, audioBytes: 192_044, unavailableSegments: 0, maxAudioBytes: 2_147_483_648 };
  if (command === 'save_meeting_bundle_export') return { bytes: 192_900, includedSegments: 1, omittedSegments: 0 };
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
  if (command === 'get_smart_auto_microphone_status') {
    if (requestedState === 'settings-smart-auto-empty') {
      return { state: 'blocked', message: 'No included microphone has recent signal.' };
    }
    const smartAuto = payload && 'smartAuto' in payload ? payload.smartAuto : null;
    const requireRecentSignal = smartAuto && typeof smartAuto === 'object' && 'requireRecentSignal' in smartAuto
      ? smartAuto.requireRecentSignal
      : null;
    if (requireRecentSignal === true) {
      return { state: 'ready', deviceId: 'fixture-built-in', reason: 'preferred_approved', validForMs: 90_000 };
    }
    if (requireRecentSignal === false) {
      return { state: 'ready', deviceId: 'fixture-built-in', reason: 'preferred_approved', validForMs: null };
    }
    return { state: 'blocked', message: 'Smart Auto fixture received an invalid request.' };
  }
  if (command === 'get_audio_input_inventory') {
    return {
      schemaVersion: 2,
      revision: 1,
      status: 'available',
      devices: [
        { id: 'fixture-built-in', name: 'MacBook Pro Microphone', kind: 'builtIn', connected: true, hasInput: true },
        { id: 'fixture-anker', name: 'Anker USB Microphone', kind: 'external', connected: true, hasInput: true },
        ...(requestedState.startsWith('settings-smart-auto')
          ? [{ id: 'fixture-desk', name: 'Desk Microphone', kind: 'external', connected: false, hasInput: true }]
          : []),
      ],
      defaultInputId: 'fixture-built-in',
      lidState: 'open',
      errorCode: null,
    };
  }
  if (command === 'get_knowledge_store_status') {
    return {
      availability: 'ready',
      schemaVersion: 1,
      recordCount: copyFixture ? copyKnowledge.length : 0,
      storeRevision: 0,
      recoveryAtMs: null,
      message: null,
    };
  }
  if (command === 'list_knowledge') {
    const request = (payload as Record<string, unknown> | undefined)?.request as { kind?: string; voiceCommand?: boolean } | undefined;
    const entries = copyFixture ? copyKnowledge.filter((entry) => request?.voiceCommand ? !!entry.voiceCommand : !request?.kind || entry.payload.kind === request.kind) : [];
    return { entries, total: entries.length, nextOffset: null, storeRevision: 0 };
  }
  if (command === 'upsert_knowledge') return { ...copyKnowledge[1], ...((payload as Record<string, unknown> | undefined)?.draft as object) };
  if (command === 'delete_knowledge') return 1;
  if (command === 'get_model_runtime_catalog' || command.startsWith('list_')) return [];
  if (command === 'transform_model_status') {
    return { state: 'ready', path: '/fixture/transform.gguf', sizeBytes: 986_000_000, sha256: 'fixture', runtimeDisabled: false };
  }
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

const shortcutFixtureEnabled = requestedState === 'settings-shortcuts';
const fixtureTransformHoldKey: Settings['transformHoldKey'] = shortcutFixtureEnabled ? 'alt_r' : null;
const fixtureQueryHotkey: Settings['queryHotkey'] = shortcutFixtureEnabled ? 'ctrl_l' : null;
const fixturePasteLastShortcut: Settings['pasteLastShortcut'] = shortcutFixtureEnabled ? 'command_option_v' : null;

const fixtureSettings = {
  ...DEFAULT_SETTINGS,
  smartAutoMicrophoneEnabled: requestedState.startsWith('settings-smart-auto'),
  smartAutoProbeEnabled: requestedState === 'settings-smart-auto',
  smartAutoApprovedDeviceIds: requestedState === 'settings-smart-auto' || requestedState === 'settings-smart-auto-no-probes'
    ? ['fixture-built-in', 'fixture-anker', 'fixture-desk']
    : [],
  smartAutoPreferredDeviceIds: requestedState === 'settings-smart-auto' || requestedState === 'settings-smart-auto-no-probes'
    ? ['fixture-built-in']
    : [],
  siteModeLookupEnabled: requestedState === 'settings-site-modes',
  transformHoldKey: requestedState === 'settings-transform-practice' ? 'alt_r' : fixtureTransformHoldKey,
  queryHotkey: fixtureQueryHotkey,
  pasteLastShortcut: fixturePasteLastShortcut,
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
  appProfiles: requestedState === 'settings-folding' && query.get('profile') === '1' ? [{
    bundleId: 'com.example.Fixture', label: 'Fixture editor', autoPasteOverride: null,
    cleanupOverride: null, smartFormattingOverride: null, cliFormattingOverride: null,
    writingStyle: null, ideContextEnabled: false, ideProjectRoots: ['/fixture/project'], queryContextExcluded: false,
  }] : [],
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
    failuresByErrorCode: {
      no_speech: 5,
      cancelled: 13,
      audio_recovering: 5,
      provider_not_authenticated: 1,
      audio_start_failed: 1,
      provider_error: 1,
    },
  },
  activity: {
    ...fixtureStats.activity,
    meetings: { count: 2, totalMinutes: 44, summariesGenerated: 1 },
    transforms: { runs: 4, approved: 3, undone: 0, byPresetName: {} },
    corrections: { proposed: 3, taught: 2, byScope: { global: 1, app: 1, project: 0 } },
  },
}));

function VisualFixture() {
  React.useLayoutEffect(() => {
    document.documentElement.classList.add('murmur-main-window');
    return () => document.documentElement.classList.remove('murmur-main-window');
  }, []);
  const settingsOpen = requestedState === 'settings-folding' || copyFixture || requestedState === 'settings'
    || requestedState === 'settings-general'
    || requestedState === 'settings-appearance'
    || requestedState === 'settings-shortcuts'
    || requestedState === 'settings-transform-practice'
    || requestedState === 'settings-site-modes'
    || requestedState.startsWith('settings-smart-auto');
  const [homeSidebarWidth, setHomeSidebarWidth] = React.useState(256);
  const meetings = useMeetings(fixtureSettings);
  const [historyEntries, setHistoryEntries] = React.useState(entries);
  const [settings, setSettings] = React.useState<Settings>(fixtureSettings);
  const [copyRequestReady, setCopyRequestReady] = React.useState(false);
  React.useEffect(() => { if (copyFixture) setCopyRequestReady(true); }, []);
  const [destination, setDestination] = React.useState<MainDestination>(
    requestedState === 'insights' ? 'insights' : requestedState.startsWith('assistant') ? 'queries' : requestedState.startsWith('meetings-') ? 'meetings' : 'home',
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
    if (requestedState.startsWith('meetings-review')) void meetings.select('meeting-fixture');
    if (manyMeetings) void meetings.select('meeting-0');
  }, [meetings.select]);

  if (requestedState === 'copy-diagnostics') return <div data-visual-ready="true" className="h-full overflow-auto bg-background text-on-surface">
    {query.get('panel') === 'transform' ? <TransformDiagnosticsView />
      : query.get('panel') === 'runs' ? <RunsView runs={[]} loading={false} error={null} cleared={false} clearing={false} clearError={null} onRetry={() => {}}
        onClear={async () => { document.documentElement.setAttribute('data-copy-cleared-runs', 'true'); }} onShowEvents={() => {}} />
      : <DictationDiagnosticsView />}
  </div>;

  if (requestedState === 'first-run-model') return <div className="main-first-run main-first-run-model">
    <ModelDownloader initialModel={fixtureSettings.model} onComplete={() => {}} />
  </div>;
  if (requestedState === 'first-run-onboarding' || requestedState === 'copy-onboarding') return <div className="main-first-run main-first-run-onboarding">
    <OnboardingFlow initialModel={fixtureSettings.model} recordingMode="both" triggerKey="shift_l" onComplete={() => {}} />
  </div>;

  return (
    <div
      style={{ '--home-sidebar-width': `${homeSidebarWidth}px` } as React.CSSProperties}
      data-appearance={appearance}
      data-theme-fixture={importedThemeFixture ?? 'sonic'}
      data-visual-ready="true"
      className="main-app-shell flex h-full min-h-0 w-full flex-col overflow-hidden bg-background text-on-surface"
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
        showStatusChip={destination !== 'home'}
        updateIndicator={requestedState === 'home-update-check' ? (
          <UpdateIndicator status={{ phase: 'idle' }} onOpen={() => {}} onDownload={() => {}} onRestart={() => {}} onRetryCheck={() => {}} />
        ) : requestedState === 'update-recovering' ? (
          <UpdateIndicator
            status={{
              phase: 'available',
              version: 'v0.27.1',
              notes: '',
              isForced: false,
            }}
            onDownload={() => {}}
            onRestart={() => {}}
            onOpen={() => {}}
            onRetryCheck={() => {}}
          />
        ) : undefined}
      />
      {settingsOpen ? (
        <SettingsPanel
          settings={settings}
          onUpdateSettings={(updates) => setSettings((current) => ({ ...current, ...updates }))}
          initialized
          status="idle"
          onResetStats={() => {}}
          onRerunSetup={() => {}}
          accessibilityGranted
          onCheckForUpdate={async () => {}}
          onDownloadUpdate={() => {}}
          onRestartUpdate={() => {}}
          onOpenUpdate={() => {}}
          updateStatus={{ phase: 'idle' }}
          configureError={null}
          pageRequest={requestedState === 'settings-folding'
            ? { page: query.get('page') ?? 'recording', target: query.get('target') ?? undefined, token: 849 }
            : copyFixture
            ? (copyRequestReady ? { page: query.get('page') ?? 'general', editorTab: (query.get('editor') ?? undefined) as SettingsEditorTab | undefined, token: 850 } : null)
            : requestedState === 'settings-general'
              ? { page: 'general', token: 1 }
              : requestedState === 'settings-appearance'
                ? { page: 'appearance', token: 1 }
            : requestedState === 'settings-shortcuts'
            ? { page: 'shortcuts', token: 1 }
            : requestedState === 'settings-transform-practice'
              ? { page: 'ai-transform', target: 'transform-practice', token: 1 }
            : undefined}
        />
      ) : (
        <div className="flex min-h-0 flex-1 overflow-hidden">
          <HomeSidebar
            onWidthChange={setHomeSidebarWidth}
            initialCollapsed={requestedState === 'home-sidebar-collapsed'}
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
                  description="Local meeting transcripts and summaries"
                  back={{ label: 'Back to Home', onActivate: backToHome }}
                />
                <MeetingsPanel meetings={meetings} playbackBusy={false} />
              </section>
            ) : destination === 'queries' ? (
              <section className="main-secondary-view" aria-labelledby="queries-view-title">
                <WorkspacePageHeader
                  title="Assistant"
                  titleId="queries-view-title"
                  description="Talk, type, and pick up where you left off"
                  back={{ label: 'Back to Home', onActivate: backToHome }}
                />
                <div className="assistant-surface-tabs" role="group" aria-label="Assistant views">
                  <button type="button" aria-pressed={requestedState !== 'assistant-history'}>Conversations</button>
                  <button type="button" aria-pressed={requestedState === 'assistant-history'}>Previous queries</button>
                </div>
                {requestedState === 'assistant-history' ? <QueryHistoryFixture /> : <AssistantWorkspace
                  connected={requestedState !== 'assistant-connect'} configured loading={false} pending={false}
                  conversations={[{ id: 'fixture', title: 'A quiet evening at home', updatedAtMs: Date.UTC(2026, 8, 13) }]}
                  selectedId="fixture" confirmationOutcomeUnknownActionIds={[]} messages={requestedState === 'assistant-empty' ? [] : requestedState === 'assistant-long' ? Array.from({ length: 80 }, (_, index) => ({
                    id: `message-${index}`, role: index % 2 === 0 ? 'user' as const : 'assistant' as const,
                    text: `Message ${index + 1}. A synthetic conversation for checking scroll boundaries and readable text.`,
                    status: 'complete' as const, actions: [],
                  })) : [
                    { id: 'question', role: 'user', text: 'Is the reading lamp on?', status: 'complete', actions: [] },
                    { id: 'answer', role: 'assistant', text: 'The reading lamp is reported **on**. The latest report is recent.', status: 'complete', actions: [] },
                  ]} phase={requestedState === 'assistant-listening' ? 'listening' : 'idle'} error={null} voiceAvailable
                  dictation={requestedState === 'assistant-listening' ? { passId: 1, text: 'What needs my attention tomorrow?', status: 'partial' } : null}
                  onConnect={async () => {}} onDisconnect={async () => {}} onNew={async () => {}} onSelect={async () => {}}
                  onDelete={async () => {}} onSend={async () => true} onVoice={async () => {}} onFinishVoice={async () => {}} onStop={async () => {}}
                  onConfirmAction={async () => {}} onCancelAction={async () => {}} onRefreshAction={async () => {}} onSettings={() => {}}
                />}
              </section>
            ) : (
              <HomeDashboard
                historyEntries={historyEntries}
                onClearHistory={() => setHistoryEntries([])}
                onDeleteHistoryEntries={(targets) => setHistoryEntries((current) => removeHistoryEntries(current, targets))}
                onUpdateHistoryEntry={() => {}}
                onToggleHistoryPinned={(entry) => setHistoryEntries((current) => toggleHistoryEntryPinned(current, entry).entries)}
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
                discovery={query.get('checklist') === 'hidden' ? undefined : {
                  completed: ['transform', 'voice_query', 'meeting', 'correction'],
                  onAction: () => {},
                  onDismiss: () => {},
                }}
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
          { id: 'transcribe', title: 'Transcribe audio or video file…', section: 'Dictation', run: () => {} },
          { id: 'history', title: 'Search transcripts', section: 'History', hint: '⌘F', run: () => {} },
          { id: 'insights', title: 'Open Insights', section: 'Navigate', run: () => {} },
          { id: 'settings', title: 'Open Settings', section: 'Navigate', hint: '⌘,', run: () => {} },
          { id: 'logs', title: 'Open Performance workspace', section: 'Navigate', hint: '⌘L', run: () => {} },
        ]}
      />
      {requestedState === 'discovery-hint' && (
        <DiscoveryHintToast hint="browser_modes" onAction={() => {}} onDismiss={() => {}} />
      )}
      <AboutModal isOpen={requestedState === 'about'} onClose={() => {}} />
      <UpdateModal
        status={requestedState === 'update-dialog-short'
          ? { phase: 'available', version: '0.43.1', notes: shortUpdateNotes, isForced: false }
          : requestedState === 'update-dialog-long'
            ? { phase: 'available', version: '0.43.1', notes: longUpdateNotes, isForced: false }
            : requestedState === 'update-downloading'
              ? { phase: 'downloading', version: '0.44.2', progress: 65 }
              : requestedState === 'update-ready'
                ? { phase: 'ready', version: '0.44.2', isForced: false }
                : { phase: 'idle' }}
        onDownload={() => {}}
        onRetryCheck={() => {}}
        onRestart={() => {}}
        onDismiss={() => {}}
      />
    </div>
  );
}

/** Real updater hook and controls, with only native IPC mocked. */
function UpdateFlowFixture() {
  const updater = useAutoUpdater({ automaticChecksEnabled: false });
  const [settings, setSettings] = React.useState(DEFAULT_SETTINGS);
  return (
    <div data-visual-ready="true" className="flex h-screen flex-col bg-background text-on-surface">
      <header className="flex h-[42px] shrink-0 items-center px-4">
        <span className="text-sm font-semibold">Murmur</span>
        <UpdateIndicator status={updater.updateStatus} onOpen={updater.showAvailableUpdate} onDownload={updater.startDownload} onRestart={updater.restartUpdate} onRetryCheck={updater.checkForUpdate} />
      </header>
      <SettingsPanel settings={settings} onUpdateSettings={(updates) => setSettings((current) => ({ ...current, ...updates }))} initialized status="idle" onResetStats={() => {}} onRerunSetup={() => {}} accessibilityGranted
        onCheckForUpdate={updater.checkForUpdate} onDownloadUpdate={updater.startDownload} onRestartUpdate={updater.restartUpdate} onOpenUpdate={updater.showAvailableUpdate} updateStatus={updater.updateStatus} configureError={null}
        pageRequest={{ page: 'general', token: 1 }}
      />
      <UpdateModal status={updater.isUpdateDialogOpen ? updater.updateStatus : { phase: 'idle' }} onDownload={updater.startDownload} onRestart={updater.restartUpdate} onRetryCheck={updater.checkForUpdate} onDismiss={updater.dismissUpdate} />
    </div>
  );
}

/** Synthetic live words at the native 560pt window width, not dictation proof. */
function DictationPreviewFixture() {
  const text = query.get('text')
    ?? 'Okay, um, one thing I want to clear up is why is it showing me a snippet of my prompt';
  return (
    <div className="flex min-h-screen items-start justify-center bg-[#6b6fae] pt-6">
      <div style={{ width: 460, height: 104 }}>
        <DictationPreviewCard text={text} />
      </div>
    </div>
  );
}

function MeetingSuggestionOverlayFixture() {
  return <VoiceIslandFixture meetingOnly />;
}

/** Render production primitives for browser-computed measurements, never
 * substitutes for native WKWebView evidence. */
/** Actual production primitives; only native IPC is mocked by this entry. */
function SettingsGeometryFixture() {
  const [changed, setChanged] = React.useState(true);
  return <main data-visual-ready="true" className="settings-workspace" style={{ padding: 24, display: 'block', overflow: 'auto', height: '100%' }}>
    <SettingsSection pageId="geometry" title="Delivery geometry">
      <SettingToggle title="Clipboard delivery" description="Copy the complete transcript locally. A long description remains readable at every width without clamping." checked={changed} defaultChecked={false} onChange={() => setChanged(!changed)} />
      <SettingsRow title="Language" description="Choose the language for recognition." control={<Select value="auto" onChange={() => {}} items={[{ value: 'auto', label: 'Automatic' }]} />} />
      <SettingsRow title="Controls" control={<><Button>Default</Button><Button size="sm">Small</Button><Button size="xs">Extra small</Button></>} />
      <SettingsRow title="Input" control={<Input aria-label="Example input" defaultValue="Local text" />} />
      <SettingToggle title="Disabled" description="Unavailable while recording." checked disabled onChange={() => {}} />
      <SettingsUnavailableGroup message="This feature requires a local model."><SettingToggle title="Unavailable" description="Install a model to enable this feature." checked={false} disabled onChange={() => {}} /></SettingsUnavailableGroup>
    </SettingsSection>
    <SettingsCard variant="plain"><Button>Plain one</Button><Button>Plain two</Button></SettingsCard>
    <div className="settings-row-controls" style={{ marginTop: 16 }}>
      <AnimatedSwitch aria-label="Small switch" size="sm" defaultChecked />
      <Select aria-label="Small select" size="sm" value="a" onChange={() => {}} items={[{ value: 'a', label: 'Small' }]} />
      <Select aria-label="Extra small select" size="xs" value="a" onChange={() => {}} items={[{ value: 'a', label: 'Extra small' }]} />
    </div>
    <div className="settings-segmented" style={{ marginTop: 16 }}><button className="settings-segmented-btn" data-selected="true">Selected</button><button className="settings-segmented-btn">Other</button></div>
  </main>;
}

function TypographyFixture() {
  return (
    <main data-visual-ready="true" className="space-y-4 p-6">
      <header className="ui-workspace-page-copy"><h1>Typography</h1><p>Local paths and controls</p></header>
      <div className="ui-window-wordmark">Murmur</div>
      <p className="text-sm">Body text</p>
      <p className="text-xs">Caption text</p>
      <p className="text-[11px]">Small text</p>
      <div className="ui-dashboard-stat"><span className="ui-dashboard-stat-value">1,234</span></div>
      <div className="w-64" data-testid="path-container">
        <MiddleTruncate className="font-mono text-xs" value={query.get('path') ?? '/Users/example/workspaces/murmur-app/docs/features/transcription.md'} />
      </div>
      <DashboardAction variant="primary" onActivate={() => {}}>Primary action</DashboardAction>
      <button type="button" className="ui-icon-button" aria-label="Copy"><Copy /></button>
      <AnimatedDropdown>
        <AnimatedDropdownTrigger aria-label="Fixture menu">Open menu</AnimatedDropdownTrigger>
        <AnimatedDropdownContent>
          <AnimatedDropdownItem icon={<Folder />}>Open folder <span className="ui-menu-shortcut">⌘O</span></AnimatedDropdownItem>
          <AnimatedDropdownSeparator />
          <AnimatedDropdownItem icon={<Copy />} disabled>Unavailable</AnimatedDropdownItem>
        </AnimatedDropdownContent>
      </AnimatedDropdown>
      <FluidTooltip.Group openDelay={0}>
        <FluidTooltip.Root id="measurement-tooltip">
          <FluidTooltip.Trigger><button type="button" className="ui-dashboard-action">Tooltip trigger</button></FluidTooltip.Trigger>
          <FluidTooltip.Content>Copy the full transcript</FluidTooltip.Content>
        </FluidTooltip.Root>
      </FluidTooltip.Group>
    </main>
  );
}

ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <React.StrictMode>
    {requestedState === 'settings-geometry' ? (
      <SettingsGeometryFixture />
    ) : requestedState === 'production-font-roots' ? (
      <main data-visual-ready="true">
        {(['main', 'onboarding', 'diagnostics'] as const).map(surface => (
          <AppSurface key={surface} surface={surface} className="font-serif"><p>{surface} production text</p></AppSurface>
        ))}
      </main>
    ) : requestedState === 'typography' ? (
      <TypographyFixture />
    ) : requestedState === 'query-review' ? (
      <div className="h-screen w-full">
      <QueryReviewView driver={{
        state: 'ready', errorCode: 'auto_copy_disabled', question: 'Is the living-room light on?',
        answer: query.has('long') ? 'This is a long answer.\n\n'.repeat(80) : 'Yes—the living-room light is on.',
        partial: '', errorDetail: null, usage: null, signInFix: null, signInStatus: null,
        signInBusy: false, contextSummary: 'Context: T3 Code — window title · no readable selection',
        capabilitySummary: 'Restricted · no trusted workspace · CLI inference may use network',
        canOpenInAssistant: true, assistantBusy: false, assistantOpenError: null,
        followUpBusy: false, followUpError: null, cancel: () => {}, copy: () => {},
        followUp: async () => {}, signIn: async () => {}, openInAssistant: async () => {},
      }} />
      </div>
    ) : requestedState === 'update-flow' ? (
      <UpdateFlowFixture />
    ) : requestedState === 'voice-island' ? (
      <VoiceIslandFixture />
    ) : requestedState === 'dictation-preview' ? (
      <DictationPreviewFixture />
    ) : requestedState === 'overlay-meeting-suggestion' ? (
      <MeetingSuggestionOverlayFixture />
    ) : (requestedState === 'settings-appearance' || (requestedState === 'settings-folding' && query.get('page') === 'appearance') || (copyFixture && query.get('page') === 'appearance')) ? (
      <AppearanceProvider>
        <VisualFixture />
      </AppearanceProvider>
    ) : (
      <VisualFixture />
    )}
  </React.StrictMode>,
);
