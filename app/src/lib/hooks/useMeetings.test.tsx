import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { DEFAULT_SETTINGS, type Settings } from '../settings';
import type { MeetingPage, MeetingDetail } from '../meetings';
import { useMeetings } from './useMeetings';

const meetingMocks = vi.hoisted(() => ({
  getMeeting: vi.fn(),
  startMeeting: vi.fn(),
  saveMeetingMetadata: vi.fn(),
  listMeetings: vi.fn(),
  saveMeetingReview: vi.fn(),
  restoreMeetingReviewFromGenerated: vi.fn(),
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
    saveMeetingMetadata: meetingMocks.saveMeetingMetadata,
    getMeetingStatus: vi.fn(async () => original.IDLE_MEETING_STATUS),
    getMeetingSummaryStatus: vi.fn(async () => original.IDLE_MEETING_SUMMARY_STATUS),
    getSystemAudioPermissionStatus: vi.fn(async () => 'granted'),
    listMeetings: meetingMocks.listMeetings,
    cancelMeetingSummary: vi.fn(async () => false),
    copyMeeting: vi.fn(async () => {}),
    deleteAllMeetings: vi.fn(async () => {}),
    deleteMeeting: vi.fn(async () => {}),
    openSystemAudioPreferences: vi.fn(async () => {}),
    requestSystemAudioPermission: vi.fn(async () => ({
      permission: 'granted', captureReady: true, audioFlowing: false, needsRelaunch: false,
    })),
    restoreMeetingReviewFromGenerated: meetingMocks.restoreMeetingReviewFromGenerated,
    saveMeetingExport: vi.fn(),
    saveMeetingReview: meetingMocks.saveMeetingReview,
    startMeetingSummary: vi.fn(),
    stopMeeting: vi.fn(async () => {}),
  };
});

function detail(id: string, speakerLabel: string): MeetingDetail {
  return {
    session: {
      id, title: null, titleSource: null, attendees: [], startedAtMs: 1, endedAtMs: 2, status: 'complete', modelName: 'base.en',
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
    meetingMocks.saveMeetingMetadata.mockReset();
    meetingMocks.listMeetings.mockReset().mockResolvedValue({ sessions: [], searchMatches: {}, total: 0, offset: 0, limit: 50 });
    meetingMocks.saveMeetingReview.mockReset();
    meetingMocks.restoreMeetingReviewFromGenerated.mockReset();
    await act(async () => root.render(<Harness />));
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('keeps the latest search when an older query resolves afterward', async () => {
    const older = deferred<MeetingPage>();
    const latest = { sessions: [detail('second', 'Team') .session], searchMatches: { second: ['decision' as const] }, total: 1, offset: 0, limit: 50 };
    meetingMocks.listMeetings.mockImplementation((query: string) => query === 'old' ? older.promise : Promise.resolve(latest));
    let pending: Promise<void>;
    await act(async () => { pending = controller().refresh('old'); });
    await act(async () => controller().refresh('new'));
    await act(async () => {
      older.resolve({ sessions: [], searchMatches: {}, total: 0, offset: 0, limit: 50 });
      await pending;
    });
    expect(controller().page).toEqual(latest);
    expect(controller().appliedQuery).toBe('new');
    expect(controller().loading).toBe(false);
  });

  it('refreshes an active search after saving or restoring a review', async () => {
    meetingMocks.saveMeetingReview.mockResolvedValue(detail('first', 'Team'));
    meetingMocks.restoreMeetingReviewFromGenerated.mockResolvedValue(detail('first', 'Team'));
    await act(async () => controller().refresh('reviewword'));
    meetingMocks.listMeetings.mockClear();
    await act(async () => controller().saveReview({ sessionId: 'first', expectedReviewRevision: null, base: { kind: 'labels_only' }, labels: { me: 'Me', them: 'Team' }, document: null }));
    await act(async () => controller().restoreReview('first', 1, null));
    expect(meetingMocks.listMeetings.mock.calls).toEqual([['reviewword'], ['reviewword']]);
  });

  it('loads older meetings for the applied search, deduplicates overlap, and blocks double requests', async () => {
    const first = detail('first', 'Team').session;
    const second = detail('second', 'Team').session;
    const third = detail('third', 'Team').session;
    meetingMocks.listMeetings.mockResolvedValueOnce({ sessions: [first, second], searchMatches: {}, total: 4, offset: 0, limit: 2 });
    await act(async () => controller().refresh('search'));
    const pending = deferred<MeetingPage>();
    meetingMocks.listMeetings.mockReturnValueOnce(pending.promise);
    let loading: Promise<void>;
    await act(async () => {
      loading = controller().loadMore();
      void controller().loadMore();
    });
    expect(meetingMocks.listMeetings).toHaveBeenLastCalledWith('search', 2, 2);
    expect(controller().loadingMore).toBe(true);
    await act(async () => {
      pending.resolve({ sessions: [second, third], searchMatches: { third: ['title'] }, total: 4, offset: 2, limit: 2 });
      await loading;
    });
    expect(controller().page.sessions.map((s) => s.id)).toEqual(['first', 'second', 'third']);
    expect(controller().page.searchMatches.third).toEqual(['title']);
    expect(controller().hasMore).toBe(false);
    expect(controller().appliedQuery).toBe('search');
  });

  it.each(['search', 'refresh', 'delete', 'clear'])('rejects an old load-more reply after %s', async (operation) => {
    meetingMocks.listMeetings.mockResolvedValueOnce({ sessions: [detail('first', 'Team').session], searchMatches: {}, total: 2, offset: 0, limit: 1 });
    await act(async () => controller().refresh('old'));
    const pending = deferred<MeetingPage>();
    meetingMocks.listMeetings.mockReturnValueOnce(pending.promise);
    let loading: Promise<void>;
    await act(async () => { loading = controller().loadMore(); });
    const next = { sessions: operation === 'clear' ? [] : [detail('second', 'Team').session], searchMatches: {}, total: 1, offset: 0, limit: operation === 'search' ? 50 : 1 };
    meetingMocks.listMeetings.mockResolvedValueOnce(next);
    await act(async () => {
      if (operation === 'delete') await controller().remove('first');
      else if (operation === 'clear') await controller().clear();
      else await controller().refresh(operation === 'search' ? 'new' : undefined);
    });
    await act(async () => {
      pending.resolve({ sessions: [detail('first', 'Team').session], searchMatches: {}, total: 2, offset: 1, limit: 1 });
      await loading;
    });
    expect(controller().page).toEqual(next);
    expect(controller().loadingMore).toBe(false);
    expect(controller().appliedQuery).toBe(operation === 'search' ? 'new' : 'old');
  });

  it('retries a failed next page at the same offset without losing the current results', async () => {
    meetingMocks.listMeetings.mockResolvedValueOnce({ sessions: [detail('first', 'Team').session], searchMatches: {}, total: 2, offset: 0, limit: 1 });
    await act(async () => controller().refresh('search'));
    meetingMocks.listMeetings.mockRejectedValueOnce(new Error('Synthetic page failure'));
    await act(async () => controller().loadMore());
    expect(controller().page.sessions.map((s) => s.id)).toEqual(['first']);
    expect(controller().hasMore).toBe(true);
    meetingMocks.listMeetings.mockResolvedValueOnce({ sessions: [detail('second', 'Team').session], searchMatches: {}, total: 2, offset: 1, limit: 1 });
    await act(async () => controller().loadMore());
    expect(meetingMocks.listMeetings).toHaveBeenLastCalledWith('search', 1, 1);
    expect(controller().page.sessions.map((s) => s.id)).toEqual(['first', 'second']);
    expect(controller().error).toBeNull();
  });

  function mockPagedMeetings(count = 180) {
    const sessions = Array.from({ length: count }, (_, index) => detail(`meeting-${index}`, 'Team').session);
    meetingMocks.listMeetings.mockImplementation(async (_query: string, offset = 0, limit = 50) => {
      const bounded = Math.min(100, limit);
      return { sessions: sessions.slice(offset, offset + bounded), searchMatches: {}, total: sessions.length, offset, limit: bounded };
    });
    return sessions;
  }

  it.each(['refresh', 'summary', 'capture-idle', 'metadata', 'delete'])('preserves three loaded pages and selection after %s, respecting the 100-row maximum', async (source) => {
    const sessions = mockPagedMeetings();
    meetingMocks.getMeeting.mockResolvedValue(detail('meeting-120', 'Team'));
    meetingMocks.saveMeetingMetadata.mockResolvedValue(detail('meeting-120', 'Team'));
    await act(async () => controller().refresh('active-search'));
    await act(async () => controller().loadMore());
    await act(async () => controller().loadMore());
    await act(async () => controller().select('meeting-120'));
    expect(controller().page.sessions).toHaveLength(150);
    meetingMocks.listMeetings.mockClear();
    if (source === 'delete') sessions.splice(20, 1);
    await act(async () => {
      if (source === 'summary') eventMocks.listeners.get('meeting-summary-status-changed')?.({ payload: { phase: 'complete', sessionId: 'other' } });
      else if (source === 'capture-idle') eventMocks.listeners.get('meeting-status-changed')?.({ payload: { generation: 1, phase: 'idle', sessionId: null } });
      else if (source === 'metadata') await controller().saveMetadata({ sessionId: 'meeting-120', title: 'Synthetic title', attendees: [] });
      else if (source === 'delete') await controller().remove('meeting-20');
      else await controller().refresh();
    });
    expect(controller().page.sessions.map((s) => s.id)).toEqual(sessions.slice(0, 150).map((s) => s.id));
    expect(controller().page.sessions.some((s) => s.id === 'meeting-120')).toBe(true);
    expect(controller().detail?.session.id).toBe('meeting-120');
    expect(controller().appliedQuery).toBe('active-search');
    expect(meetingMocks.listMeetings.mock.calls.every(([query, , limit = 50]) => query === 'active-search' && limit <= 100)).toBe(true);
    await act(async () => controller().loadMore());
    expect(controller().page.sessions.map((s) => s.id)).toEqual(sessions.map((s) => s.id));
    expect(new Set(controller().page.sessions.map((s) => s.id)).size).toBe(sessions.length);
    expect(controller().hasMore).toBe(false);
  });

  it('expands the refreshed prefix when insertions move a visible selection past its boundary', async () => {
    const sessions = mockPagedMeetings(200);
    meetingMocks.getMeeting.mockResolvedValue(detail('meeting-145', 'Team'));
    await act(async () => controller().refresh());
    await act(async () => controller().loadMore());
    await act(async () => controller().loadMore());
    await act(async () => controller().select('meeting-145'));
    sessions.unshift(...Array.from({ length: 20 }, (_, i) => detail(`new-${i}`, 'Team').session));
    await act(async () => controller().refresh());
    expect(controller().page.sessions.length).toBeGreaterThanOrEqual(166);
    expect(controller().page.sessions.some((s) => s.id === 'meeting-145')).toBe(true);
    expect(controller().page.sessions.map((s) => s.id)).toEqual(sessions.slice(0, controller().page.sessions.length).map((s) => s.id));
  });

  it.each(['same', 'lower'])('bounds refresh and keeps the saved page count when a selection disappears from 1000 results with %s total', async (change) => {
    const sessions = mockPagedMeetings(1000);
    meetingMocks.getMeeting.mockResolvedValue(detail('meeting-20', 'Team'));
    await act(async () => controller().refresh('search'));
    await act(async () => controller().loadMore());
    await act(async () => controller().loadMore());
    await act(async () => controller().select('meeting-20'));
    sessions.splice(20, 1);
    if (change === 'same') sessions.push(detail('replacement', 'Team').session);
    for (let attempt = 0; attempt < 2; attempt += 1) {
      meetingMocks.listMeetings.mockClear();
      await act(async () => controller().refresh());
      expect(meetingMocks.listMeetings.mock.calls).toEqual([['search', 0, 100], ['search', 100, 50]]);
      expect(controller().page.sessions.map((s) => s.id)).toEqual(sessions.slice(0, 150).map((s) => s.id));
      expect(controller().page.sessions.some((s) => s.id === 'meeting-20')).toBe(false);
      expect(controller().hasMore).toBe(true);
    }
  });

  it('caps a missing-selection extension at first-page net growth rounded up to a page', async () => {
    const sessions = mockPagedMeetings(1000);
    meetingMocks.getMeeting.mockResolvedValue(detail('meeting-20', 'Team'));
    await act(async () => controller().refresh('search'));
    await act(async () => controller().loadMore());
    await act(async () => controller().loadMore());
    await act(async () => controller().select('meeting-20'));
    sessions.splice(20, 1);
    sessions.push(detail('new-1', 'Team').session, detail('new-2', 'Team').session);
    const original = meetingMocks.listMeetings.getMockImplementation()!;
    // Later pages observe much larger growth; only the first page sets the budget.
    meetingMocks.listMeetings.mockImplementation(async (query, offset = 0, limit = 50) => ({
      ...await original(query, offset, limit), total: offset === 0 ? 1001 : 2000,
    }));
    meetingMocks.listMeetings.mockClear();
    await act(async () => controller().refresh());
    expect(meetingMocks.listMeetings.mock.calls).toEqual([['search', 0, 100], ['search', 100, 50], ['search', 150, 50]]);
    expect(controller().page.sessions).toHaveLength(200);
    expect(controller().hasMore).toBe(true);
  });

  it('resets the loaded window for a new search filter', async () => {
    mockPagedMeetings();
    await act(async () => controller().refresh('old'));
    await act(async () => controller().loadMore());
    await act(async () => controller().loadMore());
    await act(async () => controller().refresh('new'));
    expect(controller().page.sessions).toHaveLength(50);
    expect(controller().appliedQuery).toBe('new');
    await act(async () => controller().loadMore());
    expect(meetingMocks.listMeetings).toHaveBeenLastCalledWith('new', 50, 50);
  });

  it('discards a partially rebuilt refresh after the search changes without issuing its remaining pages', async () => {
    mockPagedMeetings();
    await act(async () => controller().refresh('old'));
    await act(async () => controller().loadMore());
    await act(async () => controller().loadMore());
    const blocked = deferred<MeetingPage>();
    const original = meetingMocks.listMeetings.getMockImplementation()!;
    meetingMocks.listMeetings.mockImplementation((query, offset = 0, limit = 50) => query === 'old' ? blocked.promise : original(query, offset, limit));
    meetingMocks.listMeetings.mockClear();
    let pending: Promise<void>;
    await act(async () => { pending = controller().refresh(); });
    await act(async () => controller().refresh('new'));
    await act(async () => {
      blocked.resolve({ sessions: [], searchMatches: {}, total: 180, offset: 0, limit: 100 });
      await pending;
    });
    expect(controller().page.sessions).toHaveLength(50);
    expect(controller().appliedQuery).toBe('new');
    expect(meetingMocks.listMeetings.mock.calls.filter(([query]) => query === 'old')).toHaveLength(1);
    expect(controller().loading).toBe(false);
    expect(controller().hasMore).toBe(true);
  });

  it('keeps the old window until all refreshed pages finish and drops a stale second page', async () => {
    const sessions = mockPagedMeetings();
    await act(async () => controller().refresh('old'));
    await act(async () => controller().loadMore());
    await act(async () => controller().loadMore());
    const blocked = deferred<MeetingPage>();
    const original = meetingMocks.listMeetings.getMockImplementation()!;
    meetingMocks.listMeetings.mockImplementation((query, offset = 0, limit = 50) => query === 'old' && offset > 0 ? blocked.promise : original(query, offset, limit));
    let pending: Promise<void>;
    await act(async () => { pending = controller().refresh(); });
    expect(controller().page.sessions).toHaveLength(150);
    expect(controller().loading).toBe(true);
    expect(meetingMocks.listMeetings).toHaveBeenLastCalledWith('old', 100, 50);
    const calls = meetingMocks.listMeetings.mock.calls.length;
    await act(async () => controller().loadMore());
    expect(meetingMocks.listMeetings).toHaveBeenCalledTimes(calls);
    await act(async () => controller().refresh('new'));
    await act(async () => {
      blocked.resolve({ sessions: sessions.slice(100, 150), searchMatches: {}, total: 180, offset: 100, limit: 50 });
      await pending;
    });
    expect(controller().page.sessions).toHaveLength(50);
    expect(controller().appliedQuery).toBe('new');
    expect(controller().loading).toBe(false);
  });

  it('keeps the complete old window if a later refresh page fails and retries from zero', async () => {
    const sessions = mockPagedMeetings();
    await act(async () => controller().refresh('search'));
    await act(async () => controller().loadMore());
    await act(async () => controller().loadMore());
    sessions.splice(10, 2);
    const original = meetingMocks.listMeetings.getMockImplementation()!;
    meetingMocks.listMeetings.mockImplementation((query, offset = 0, limit = 50) => offset > 0 ? Promise.reject(new Error('Synthetic refresh failure')) : original(query, offset, limit));
    await act(async () => controller().refresh());
    expect(controller().page.sessions).toHaveLength(150);
    expect(controller().page.sessions.some((s) => s.id === 'meeting-10')).toBe(true);
    expect(controller().error).toContain('Synthetic refresh failure');
    meetingMocks.listMeetings.mockImplementation(original).mockClear();
    await act(async () => controller().refresh());
    expect(meetingMocks.listMeetings.mock.calls).toEqual([['search', 0, 100], ['search', 100, 50]]);
    expect(controller().page.sessions.map((s) => s.id)).toEqual(sessions.slice(0, 150).map((s) => s.id));
    expect(controller().error).toBeNull();
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

  it('does not replace a newly selected meeting when an earlier metadata save completes', async () => {
    const pending = deferred<MeetingDetail>();
    meetingMocks.getMeeting.mockImplementation((id: string) => Promise.resolve(detail(id, 'Speaker')));
    meetingMocks.saveMeetingMetadata.mockReturnValue(pending.promise);
    await act(async () => controller().select('first'));
    let saving: Promise<boolean> | undefined;
    await act(async () => { saving = controller().saveMetadata({ sessionId: 'first', title: 'Saved', attendees: [] }); });
    await act(async () => controller().select('second'));
    await act(async () => { pending.resolve({ ...detail('first', 'Speaker'), session: { ...detail('first', 'Speaker').session, title: 'Saved', titleSource: 'manual' } }); await saving; });
    expect(controller().detail?.session.id).toBe('second');
  });

  it('freezes the diarization opt-in in the meeting start request', async () => {
    meetingMocks.startMeeting.mockResolvedValue(detail('first', 'Speaker 1').session);
    meetingMocks.getMeeting.mockResolvedValue(detail('first', 'Speaker 1'));
    await act(async () => controller().start());

    expect(meetingMocks.startMeeting).toHaveBeenCalledWith(expect.objectContaining({
      diarization: false,
    }));
  });

  it('passes only the accepted suggestion token into the normal meeting start path', async () => {
    meetingMocks.startMeeting.mockResolvedValue(detail('first', 'Speaker 1').session);
    meetingMocks.getMeeting.mockResolvedValue(detail('first', 'Speaker 1'));

    await act(async () => controller().start('66666666-6666-4666-8666-666666666666'));

    expect(meetingMocks.startMeeting).toHaveBeenCalledWith(expect.objectContaining({
      suggestionToken: '66666666-6666-4666-8666-666666666666',
    }));
    expect(JSON.stringify(meetingMocks.startMeeting.mock.calls)).not.toContain('Product review');
  });

  it('propagates suggested-start failure with a stable manual recovery message', async () => {
    meetingMocks.startMeeting.mockRejectedValue(new Error('private Calendar event title'));

    await act(async () => {
      await expect(controller().start('77777777-7777-4777-8777-777777777777'))
        .rejects.toThrow('Notetaker could not start from that Calendar suggestion');
    });

    expect(controller().error).toContain('Start it manually');
    expect(controller().error).not.toContain('private Calendar event title');
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
