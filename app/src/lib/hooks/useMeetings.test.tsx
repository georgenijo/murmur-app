import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { DEFAULT_SETTINGS, type Settings } from '../settings';
import type { MeetingDetail } from '../meetings';
import { useMeetings } from './useMeetings';

const meetingMocks = vi.hoisted(() => ({
  getMeeting: vi.fn(),
  startMeeting: vi.fn(),
}));
const eventMocks = vi.hoisted(() => ({
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
}));

vi.mock('../log', () => ({ flog: { warn: vi.fn() } }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, listener: (event: { payload: unknown }) => void) => {
    eventMocks.listeners.set(name, listener);
    return () => eventMocks.listeners.delete(name);
  }),
}));
vi.mock('../meetings', async (importOriginal) => {
  const original = await importOriginal<typeof import('../meetings')>();
  return {
    ...original,
    getMeeting: meetingMocks.getMeeting,
    startMeeting: meetingMocks.startMeeting,
    getMeetingStatus: vi.fn(async () => original.IDLE_MEETING_STATUS),
    getMeetingSummaryStatus: vi.fn(async () => original.IDLE_MEETING_SUMMARY_STATUS),
    getSystemAudioPermissionStatus: vi.fn(async () => 'granted'),
    listMeetings: vi.fn(async () => ({ sessions: [], total: 0, offset: 0, limit: 50 })),
    cancelMeetingSummary: vi.fn(async () => false),
    copyMeeting: vi.fn(async () => {}),
    deleteAllMeetings: vi.fn(async () => {}),
    deleteMeeting: vi.fn(async () => {}),
    openSystemAudioPreferences: vi.fn(async () => {}),
    requestSystemAudioPermission: vi.fn(async () => ({
      permission: 'granted', captureReady: true, audioFlowing: false, needsRelaunch: false,
    })),
    restoreMeetingReviewFromGenerated: vi.fn(),
    saveMeetingExport: vi.fn(),
    saveMeetingReview: vi.fn(),
    startMeetingSummary: vi.fn(),
    stopMeeting: vi.fn(async () => {}),
  };
});

function detail(id: string, speakerLabel: string): MeetingDetail {
  return {
    session: {
      id, startedAtMs: 1, endedAtMs: 2, status: 'complete', modelName: 'base.en',
      language: 'en', smartPunctuation: true, retainAudio: false, durationMs: 1,
      segmentCount: 1, preview: 'Evidence', errorCode: null,
    },
    segments: [{
      id: id === 'first' ? 1 : 2,
      sessionId: id,
      speaker: 'them',
      remoteSpeakerId: 1,
      sequence: 0,
      startMs: 0,
      endMs: 1,
      status: 'final',
      text: 'Evidence',
      audioAvailable: false,
      errorCode: null,
    }],
    labels: { me: 'Me', them: 'Them' },
    remoteSpeakers: [{ speakerId: 1, label: speakerLabel }],
    generated: null,
    review: null,
    activeDocument: null,
    activeOrigin: null,
  };
}

function deferred<T>() {
  let resolve: (value: T) => void = () => {};
  const promise = new Promise<T>((next) => {
    resolve = next;
  });
  return { promise, resolve };
}

describe('useMeetings remote speaker refresh', () => {
  let container: HTMLDivElement;
  let root: Root;
  let current: ReturnType<typeof useMeetings> | null;
  let renderedSettings: Settings;

  const controller = () => {
    if (!current) throw new Error('Meeting controller has not rendered.');
    return current;
  };

  function Harness() {
    current = useMeetings(renderedSettings);
    return null;
  }

  async function renderSettings(settings: Settings) {
    renderedSettings = settings;
    await act(async () => root.render(<Harness />));
  }

  beforeEach(async () => {
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    current = null;
    renderedSettings = { ...DEFAULT_SETTINGS };
    eventMocks.listeners.clear();
    meetingMocks.getMeeting.mockReset();
    meetingMocks.startMeeting.mockReset();
    await act(async () => root.render(<Harness />));
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('drops a speaker event refresh when the selected meeting changes before it resolves', async () => {
    const staleRefresh = deferred<MeetingDetail>();
    let firstRequests = 0;
    meetingMocks.getMeeting.mockImplementation((id: string) => {
      if (id === 'second') return Promise.resolve(detail('second', 'Second speaker'));
      firstRequests += 1;
      return firstRequests === 1
        ? Promise.resolve(detail('first', 'Speaker 1'))
        : staleRefresh.promise;
    });
    await act(async () => controller().select('first'));

    await act(async () => {
      eventMocks.listeners.get('meeting-speakers-updated')?.({ payload: { sessionId: 'first' } });
    });
    await act(async () => controller().select('second'));
    await act(async () => staleRefresh.resolve(detail('first', 'Stale renamed speaker')));

    expect(controller().detail?.session.id).toBe('second');
    expect(controller().detail?.remoteSpeakers[0].label).toBe('Second speaker');
  });

  it('freezes the diarization opt-in in the meeting start request', async () => {
    meetingMocks.startMeeting.mockResolvedValue(detail('first', 'Speaker 1').session);
    meetingMocks.getMeeting.mockResolvedValue(detail('first', 'Speaker 1'));
    await act(async () => controller().start());

    expect(meetingMocks.startMeeting).toHaveBeenCalledWith(expect.objectContaining({
      diarization: false,
    }));
  });

  it('uses the latest Smart Auto policy after mount and drops it after a manual pin', async () => {
    meetingMocks.startMeeting.mockResolvedValue(detail('first', 'Speaker 1').session);
    meetingMocks.getMeeting.mockResolvedValue(detail('first', 'Speaker 1'));

    await renderSettings({
      ...renderedSettings,
      smartAutoMicrophoneEnabled: true,
      smartAutoApprovedDeviceIds: [],
      smartAutoPreferredDeviceIds: [],
      smartAutoAllowContinuity: false,
    });
    await act(async () => controller().start());
    expect(meetingMocks.startMeeting).toHaveBeenLastCalledWith(expect.objectContaining({
      microphone: 'system_default',
      smartAuto: {
        approvedDeviceIds: [],
        preferredDeviceIds: [],
        allowContinuity: false,
        requireRecentSignal: false,
      },
    }));

    await renderSettings({
      ...renderedSettings,
      smartAutoApprovedDeviceIds: ['usb', 'iphone'],
      smartAutoPreferredDeviceIds: ['iphone', 'usb'],
      smartAutoAllowContinuity: true,
    });
    await act(async () => controller().start());
    expect(meetingMocks.startMeeting).toHaveBeenLastCalledWith(expect.objectContaining({
      smartAuto: {
        approvedDeviceIds: ['usb', 'iphone'],
        preferredDeviceIds: ['iphone', 'usb'],
        allowContinuity: true,
        requireRecentSignal: false,
      },
    }));

    await renderSettings({
      ...renderedSettings,
      smartAutoProbeEnabled: true,
    });
    await act(async () => controller().start());
    expect(meetingMocks.startMeeting).toHaveBeenLastCalledWith(expect.objectContaining({
      smartAuto: {
        approvedDeviceIds: ['usb', 'iphone'],
        preferredDeviceIds: ['iphone', 'usb'],
        allowContinuity: true,
        requireRecentSignal: true,
      },
    }));

    await renderSettings({
      ...renderedSettings,
      smartAutoProbeEnabled: false,
    });
    await act(async () => controller().start());
    expect(meetingMocks.startMeeting).toHaveBeenLastCalledWith(expect.objectContaining({
      smartAuto: {
        approvedDeviceIds: ['usb', 'iphone'],
        preferredDeviceIds: ['iphone', 'usb'],
        allowContinuity: true,
        requireRecentSignal: false,
      },
    }));

    await renderSettings({
      ...renderedSettings,
      microphone: 'usb',
      smartAutoMicrophoneEnabled: false,
    });
    await act(async () => controller().start());
    expect(meetingMocks.startMeeting).toHaveBeenLastCalledWith(expect.objectContaining({
      microphone: 'usb',
      smartAuto: null,
    }));
  });
});
