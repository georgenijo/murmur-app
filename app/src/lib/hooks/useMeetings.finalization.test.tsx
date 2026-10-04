import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { MeetingReviewWorkspace } from '../../components/history/MeetingReviewWorkspace';
import { DEFAULT_SETTINGS } from '../settings';
import { IDLE_MEETING_STATUS, type MeetingDetail, type MeetingRuntimeStatus } from '../meetings';
import { useMeetings } from './useMeetings';

const mocks = vi.hoisted(() => ({ get: vi.fn(), list: vi.fn(), metadata: vi.fn(), status: vi.fn() }));
const events = vi.hoisted(() => new Map<string, (event: { payload: unknown }) => void>());
vi.mock('../log', () => ({ flog: { warn: vi.fn() } }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, callback: (event: { payload: unknown }) => void) => {
    events.set(name, callback);
    return () => events.delete(name);
  }),
}));
vi.mock('../meetings', async (original) => ({
  ...await original<typeof import('../meetings')>(),
  getMeeting: mocks.get,
  listMeetings: mocks.list,
  saveMeetingMetadata: mocks.metadata,
  getMeetingStatus: mocks.status,
  getMeetingSummaryStatus: vi.fn(async () => ({ phase: 'idle', sessionId: null })),
  getSystemAudioPermissionStatus: vi.fn(async () => 'granted'),
}));

function workspace(id = 'first', status: MeetingDetail['session']['status'] = 'active'): MeetingDetail {
  const document = { schema: 'murmur.meeting-review.v1' as const, summary: { key: 'summary', text: 'Synthetic draft', sourceSegmentIds: [11] }, decisions: [], actionItems: [], openQuestions: [] };
  return {
    session: { id, title: null, titleSource: null, attendees: [], startedAtMs: 1, endedAtMs: status === 'active' ? null : 45_001, status, modelName: 'base.en', language: 'en', smartPunctuation: false, retainAudio: false, durationMs: status === 'active' ? 1_000 : 45_000, segmentCount: status === 'active' ? 0 : 1, preview: 'Synthetic evidence', errorCode: status === 'failed' ? 'worker_failed' : null },
    segments: [{ id: 11, sessionId: id, speaker: 'me', remoteSpeakerId: null, sequence: 0, startMs: 0, endMs: 1_000, status: status === 'active' ? 'pending' : 'final', text: status === 'active' ? '' : 'Synthetic evidence', audioAvailable: false, errorCode: null }],
    labels: { me: 'Me', them: 'Them' }, remoteSpeakers: [], generated: { revision: 1, document }, review: null, activeDocument: document, activeOrigin: 'generated',
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

describe('useMeetings selected meeting finalization', () => {
  let root: Root;
  let container: HTMLDivElement;
  let current: ReturnType<typeof useMeetings>;
  function Harness() {
    current = useMeetings(DEFAULT_SETTINGS);
    return current.detail ? <MeetingReviewWorkspace meetings={current} segments={current.detail.segments} captureBusy={false} onNotice={() => {}} /> : null;
  }
  async function status(phase: MeetingRuntimeStatus['phase'], sessionId: string | null = 'first', generation = 1) {
    const payload: MeetingRuntimeStatus = { ...IDLE_MEETING_STATUS, phase, sessionId, generation };
    await act(async () => events.get('meeting-status-changed')!({ payload }));
  }
  beforeEach(async () => {
    events.clear();
    mocks.get.mockReset().mockImplementation(async (id: string) => workspace(id));
    mocks.metadata.mockReset();
    mocks.status.mockReset().mockResolvedValue(IDLE_MEETING_STATUS);
    mocks.list.mockReset().mockResolvedValue({ sessions: [], searchMatches: {}, total: 0, offset: 0, limit: 50 });
    container = document.createElement('div');
    document.body.append(container);
    root = createRoot(container);
    await act(async () => root.render(<Harness />));
  });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); });

  it.each([['idle', 'complete'], ['failed', 'failed']] as const)('reloads persisted selected detail on %s without re-clicking', async (phase, storedStatus) => {
    await act(async () => current.select('first'));
    const format = container.querySelector<HTMLSelectElement>('select[aria-label="Meeting review export format"]')!;
    await act(async () => { format.value = 'srt'; format.dispatchEvent(new Event('change', { bubbles: true })); });
    const captions = () => [...container.querySelectorAll('button')].find((button) => button.textContent === 'Copy captions')!;
    expect(captions().disabled).toBe(true);
    await status('processing');
    mocks.get.mockResolvedValue(workspace('first', storedStatus));
    await status(phase, phase === 'idle' ? null : 'first');
    expect(current.detail?.session).toMatchObject({ status: storedStatus, endedAtMs: 45_001, durationMs: 45_000, segmentCount: 1 });
    expect(current.detail?.segments[0].status).toBe('final');
    expect(mocks.get).toHaveBeenCalledTimes(2);
    expect(mocks.list).toHaveBeenCalledTimes(2);
    expect(captions().disabled).toBe(false);
    await status(phase, phase === 'idle' ? null : 'first');
    expect(mocks.get).toHaveBeenCalledTimes(2);
  });

  it('uses an explicit terminal session id even without a prior active event', async () => {
    await act(async () => current.select('first'));
    mocks.get.mockResolvedValue(workspace('first', 'failed'));
    await status('failed');
    expect(current.detail?.session.status).toBe('failed');
  });

  it('does not refresh another selection or infer an unknown id from the selection', async () => {
    await act(async () => current.select('second'));
    const selected = current.detail;
    await status('processing');
    await status('idle', null);
    await status('idle', null, 2);
    expect(mocks.get).toHaveBeenCalledTimes(1);
    expect(current.detail).toBe(selected);
  });

  it('does not let delayed initialization replace a newer event in the same generation', async () => {
    await act(async () => root.unmount());
    root = createRoot(container);
    const initial = deferred<MeetingRuntimeStatus>();
    mocks.status.mockReturnValue(initial.promise);
    await act(async () => root.render(<Harness />));
    await act(async () => current.select('first'));
    await status('processing');
    await act(async () => initial.resolve({ ...IDLE_MEETING_STATUS, generation: 1 }));
    expect(current.status.phase).toBe('processing');
    mocks.get.mockResolvedValue(workspace('first', 'complete'));
    await status('idle', null);
    expect(current.detail?.session.status).toBe('complete');
  });

  it('invalidates an active snapshot still loading when finalization arrives', async () => {
    const loading = deferred<MeetingDetail>();
    mocks.get.mockReturnValueOnce(loading.promise).mockResolvedValue(workspace('first', 'complete'));
    let selecting!: Promise<void>;
    await act(async () => { selecting = current.select('first'); });
    await status('processing');
    await status('idle', null);
    await act(async () => { loading.resolve(workspace()); await selecting; });
    expect(current.detail?.session.status).toBe('complete');
  });

  it.each(['second', null, 'first'])('drops a terminal request after selection changes to %s', async (id) => {
    await act(async () => current.select('first'));
    await status('processing');
    const pending = deferred<MeetingDetail>();
    mocks.get.mockReturnValueOnce(pending.promise);
    await status('idle', null);
    if (id === 'first') await act(async () => current.select(null));
    mocks.get.mockResolvedValue(workspace(id ?? 'first', 'failed'));
    await act(async () => current.select(id));
    await act(async () => pending.resolve(workspace('first', 'complete')));
    expect(current.detail?.session.status ?? null).toBe(id ? 'failed' : null);
    expect(current.detail?.session.id ?? null).toBe(id);
  });

  it('ignores duplicate receipts and obsolete generations, including an older in-flight response', async () => {
    await act(async () => current.select('first'));
    await status('processing');
    const old = deferred<MeetingDetail>();
    mocks.get.mockReturnValueOnce(old.promise);
    await status('failed');
    await status('failed');
    expect(mocks.get).toHaveBeenCalledTimes(2);
    await status('processing', 'first', 2);
    await status('idle', null, 1);
    await act(async () => old.resolve(workspace('first', 'failed')));
    expect(current.status.phase).toBe('processing');
    expect(current.detail?.session.status).toBe('active');
    mocks.get.mockResolvedValue(workspace('first', 'complete'));
    await status('idle', null, 2);
    expect(current.detail?.session.status).toBe('complete');
  });

  it('keeps capture refresh independent of unsaved review drafts and concurrent metadata saves', async () => {
    await act(async () => current.select('first'));
    const reviewState = current.detail!;
    const edit = [...container.querySelectorAll('button')].find((button) => button.textContent === 'Edit review')!;
    await act(async () => edit.click());
    const textarea = container.querySelector<HTMLTextAreaElement>('textarea[aria-label="Review summary"]')!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!.call(textarea, 'Unsaved synthetic edit');
      textarea.dispatchEvent(new Event('input', { bubbles: true }));
    });
    await status('processing');
    const pending = deferred<MeetingDetail>();
    mocks.get.mockReturnValueOnce(pending.promise);
    const saved = { ...reviewState, session: { ...reviewState.session, title: 'New title', titleSource: 'manual' as const } };
    const metadata = deferred<MeetingDetail>();
    mocks.metadata.mockReturnValue(metadata.promise);
    let saving!: Promise<boolean>;
    await act(async () => { saving = current.saveMetadata({ sessionId: 'first', title: 'New title', attendees: [] }); });
    await status('idle', null);
    await act(async () => { metadata.resolve(saved); await saving; });
    await act(async () => pending.resolve({ ...workspace('first', 'complete'), generated: { ...reviewState.generated!, revision: 2 } }));
    expect(current.detail?.session).toMatchObject({ status: 'complete', title: 'New title' });
    expect(current.detail?.generated).toBe(reviewState.generated);
    expect(container.querySelector<HTMLTextAreaElement>('textarea[aria-label="Review summary"]')?.value).toBe('Unsaved synthetic edit');
  });

  it('reports a current failed refresh and leaves detail intact; obsolete failures do not affect a new selection', async () => {
    await act(async () => current.select('first'));
    await status('processing');
    const original = current.detail;
    mocks.get.mockRejectedValueOnce(new Error('Synthetic reload failure'));
    await status('idle', null);
    expect(current.detail).toBe(original);
    expect(current.error).toContain('Synthetic reload failure');
    await status('processing', 'first', 2);
    const pending = deferred<MeetingDetail>();
    mocks.get.mockReturnValueOnce(pending.promise);
    await status('failed', 'first', 2);
    await act(async () => current.select('second'));
    await act(async () => pending.reject(new Error('Obsolete failure')));
    expect(current.error).toBeNull();
    expect(current.detail?.session.id).toBe('second');
  });
});
