import { act, StrictMode } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { MeetingAudioPlayback, useMeetingAudio, type MeetingAudioController } from './useMeetingAudio';
import type { MeetingAudioChunk } from '../meetingAudio';

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listenerFailure: false,
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, listener: (event: { payload: unknown }) => void) => {
    if (mocks.listenerFailure && name === 'query-state-changed') throw new Error('Listener unavailable');
    mocks.listeners.set(name, listener);
    return () => mocks.listeners.delete(name);
  }),
}));

function deferred<T>() {
  let resolve: (value: T) => void = () => {};
  const promise = new Promise<T>(done => { resolve = done; });
  return { promise, resolve };
}
class FakeSource {
  buffer: unknown = null;
  onended: (() => void) | null = null;
  connect = vi.fn();
  disconnect = vi.fn();
  start = vi.fn();
  stop = vi.fn();
}
class FakeContext {
  static latest: FakeContext;
  currentTime = 0;
  state = 'suspended';
  destination = {};
  sources: FakeSource[] = [];
  constructor() { FakeContext.latest = this; }
  resume = vi.fn(async () => { this.state = 'running'; });
  close = vi.fn(async () => { this.state = 'closed'; });
  decodeAudioData = vi.fn(async (_bytes: ArrayBuffer) => ({ numberOfChannels: 1, duration: 10 }));
  createBufferSource() {
    const source = new FakeSource();
    this.sources.push(source);
    return source;
  }
}
let chunks: MeetingAudioChunk[];
let durationMs: number;
let playback: MeetingAudioPlayback;
let root: Root | null;
let controller: MeetingAudioController | null;
async function settle() { for (let step = 0; step < 30; step += 1) await Promise.resolve(); }
function reads() { return mocks.invoke.mock.calls.filter(call => call[0] === 'read_meeting_audio_range'); }
function Harness({ sessionId = 'meeting', busy = false }: { sessionId?: string; busy?: boolean }) {
  controller = useMeetingAudio({ sessionId, captureBusy: busy });
  return null;
}
function controls() {
  if (!controller) throw new Error('Hook not mounted');
  return controller;
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.stubGlobal('AudioContext', FakeContext);
  mocks.invoke.mockReset();
  mocks.listeners.clear();
  mocks.listenerFailure = false;
  root = null;
  controller = null;
  chunks = [
    { segmentId: 1, channel: 'me', startMs: 0, endMs: 10000 },
    { segmentId: 2, channel: 'them', startMs: 0, endMs: 10000 },
  ];
  durationMs = 10000;
  mocks.invoke.mockImplementation(async (command: string, request?: Record<string, unknown>) => {
    if (command === 'get_meeting_audio_capture_busy') return false;
    if (command === 'read_meeting_audio_range') return new ArrayBuffer(44);
    if (command === 'get_meeting_audio_manifest' && request) {
      const fromMs = typeof request.fromMs === 'number' ? request.fromMs : 0;
      const limit = typeof request.limit === 'number' ? request.limit : 128;
      const matching = chunks.filter(chunk => chunk.endMs > fromMs && (request.channel === 'all' || chunk.channel === request.channel));
      const page = matching.slice(0, limit);
      const last = page[page.length - 1];
      return {
        kind: 'available', sessionId: request.sessionId, durationMs, chunks: page,
        nextCursor: matching.length > limit && last ? { startMs: last.startMs, segmentId: last.segmentId } : null,
      };
    }
    throw new Error(`Unexpected command: ${command}`);
  });
  playback = new MeetingAudioPlayback('meeting', () => {});
});
afterEach(async () => {
  playback.dispose();
  if (root) await act(async () => root?.unmount());
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

describe('meeting audio clock and bounded scheduler', () => {
  it('resumes in the gesture and schedules overlapping channels on the same clock', async () => {
    await playback.load();
    expect(reads()).toHaveLength(0);
    playback.playAll();
    expect(FakeContext.latest.resume).toHaveBeenCalledOnce();
    expect(reads()).toHaveLength(0);
    await settle();
    expect(playback.state.status).toBe('playing');
    expect(FakeContext.latest.sources.map(source => source.start.mock.calls)).toEqual([[[0, 0, 10]], [[0, 0, 10]]]);
  });

  it('seeks inside chunks and keeps global time when changing canonical channel', async () => {
    await playback.load();
    playback.playSegment({ speaker: 'them', startMs: 4000 });
    await settle();
    const context = FakeContext.latest;
    expect(context.sources[0].start).toHaveBeenCalledWith(0, 4, 6);
    context.currentTime = 1;
    playback.setChannel('me');
    await settle();
    expect(context.sources[0].stop).toHaveBeenCalledOnce();
    expect(playback.state.positionMs).toBe(5000);
    expect(context.sources[1].start).toHaveBeenCalledWith(1, 5, 5);
    playback.pause();
    playback.seek(7000);
    const readCount = reads().length;
    await settle();
    expect(reads()).toHaveLength(readCount);
    playback.play();
    await settle();
    expect(context.sources[2].start).toHaveBeenCalledWith(1, 7, 3);
  });

  it('keeps silence on the timeline and waits until two seconds before a distant chunk', async () => {
    chunks = [{ segmentId: 1, channel: 'me', startMs: 30000, endMs: 40000 }];
    durationMs = 40000;
    await playback.load();
    playback.playAll();
    await settle();
    const context = FakeContext.latest;
    expect(playback.state.status).toBe('playing');
    expect(reads()).toHaveLength(0);
    context.currentTime = 27;
    await vi.advanceTimersByTimeAsync(100);
    expect(playback.state.positionMs).toBe(27000);
    expect(reads()).toHaveLength(0);
    context.currentTime = 28;
    await vi.advanceTimersByTimeAsync(100);
    expect(reads()).toHaveLength(1);
    expect(context.sources[0].start).toHaveBeenCalledWith(30, 0, 10);
  });

  it('does not decode a whole session or retain more than current and next per channel', async () => {
    chunks = Array.from({ length: 10 }, (_, index) => ({ segmentId: index + 1, channel: 'me', startMs: index * 1000, endMs: (index + 1) * 1000 }));
    await playback.load();
    playback.playAll();
    await settle();
    expect(reads()).toHaveLength(2);
    await vi.advanceTimersByTimeAsync(1000);
    expect(reads()).toHaveLength(2);
    const context = FakeContext.latest;
    context.currentTime = 1;
    context.sources[0].onended?.();
    await vi.advanceTimersByTimeAsync(100);
    expect(reads()).toHaveLength(3);
    expect(context.sources[0].buffer).toBeNull();
  });

  it('discards decode completion after pause and never auto resumes after capture', async () => {
    await playback.load();
    playback.playSegment({ speaker: 'me', startMs: 0 });
    const decoded = deferred<{ numberOfChannels: number; duration: number }>();
    FakeContext.latest.decodeAudioData.mockImplementation(() => decoded.promise);
    await settle();
    playback.setBusy(true);
    playback.setBusy(false);
    decoded.resolve({ numberOfChannels: 1, duration: 10 });
    await settle();
    expect(playback.state.status).toBe('paused');
    expect(FakeContext.latest.sources).toHaveLength(0);
  });

  it('discards a range that finishes after invalidation without decoding it', async () => {
    await playback.load();
    const original = mocks.invoke.getMockImplementation();
    const read = deferred<ArrayBuffer>();
    mocks.invoke.mockImplementation(async (...args) => args[0] === 'read_meeting_audio_range' ? read.promise : original?.(...args));
    playback.playAll();
    await settle();
    playback.invalidate();
    read.resolve(new ArrayBuffer(44));
    await settle();
    expect(FakeContext.latest.decodeAudioData).not.toHaveBeenCalled();
    expect(FakeContext.latest.sources).toHaveLength(0);
    expect(playback.state.status).toBe('unavailable');
  });

  it('rechecks capture ownership after decoding, before producing any output', async () => {
    await playback.load();
    const original = mocks.invoke.getMockImplementation();
    let probes = 0;
    mocks.invoke.mockImplementation(async (...args) => args[0] === 'get_meeting_audio_capture_busy' ? ++probes > 1 : original?.(...args));
    playback.playAll();
    await settle();
    expect(FakeContext.latest.decodeAudioData).toHaveBeenCalledTimes(2);
    expect(FakeContext.latest.sources).toHaveLength(0);
    expect(playback.state.status).toBe('paused');
  });

  it('freezes both channels at an underrun and resumes the late chunk from its beginning', async () => {
    chunks = [
      { segmentId: 1, channel: 'me', startMs: 0, endMs: 10000 },
      { segmentId: 2, channel: 'them', startMs: 0, endMs: 10000 },
      { segmentId: 3, channel: 'me', startMs: 10000, endMs: 20000 },
      { segmentId: 4, channel: 'them', startMs: 10000, endMs: 20000 },
    ];
    durationMs = 20000;
    await playback.load();
    playback.playAll();
    await settle();
    const context = FakeContext.latest;
    const delayed = deferred<{ numberOfChannels: number; duration: number }>();
    context.decodeAudioData.mockImplementationOnce(() => delayed.promise);
    context.currentTime = 8;
    await vi.advanceTimersByTimeAsync(100);
    expect(reads()).toHaveLength(4);
    context.sources[0].onended?.();
    context.sources[1].onended?.();
    context.currentTime = 10.1;
    await vi.advanceTimersByTimeAsync(100);
    expect(playback.state.status).toBe('buffering');
    expect(playback.state.positionMs).toBe(10000);
    expect(context.sources[2].stop).toHaveBeenCalledOnce();
    context.currentTime = 14;
    await vi.advanceTimersByTimeAsync(100);
    expect(playback.state.positionMs).toBe(10000);
    delayed.resolve({ numberOfChannels: 1, duration: 10 });
    await settle();
    expect(playback.state.status).toBe('playing');
    expect(context.sources.slice(-2).map(source => source.start.mock.calls)).toEqual([[[14, 0, 10]], [[14, 0, 10]]]);
    expect(reads()).toHaveLength(4);
  });

  it('fails visibly on range errors, suspended audio, and capture ownership', async () => {
    await playback.load();
    const original = mocks.invoke.getMockImplementation();
    mocks.invoke.mockImplementation(async (...args) => {
      if (args[0] === 'read_meeting_audio_range') throw new Error('missing');
      return original?.(...args);
    });
    playback.playAll();
    await settle();
    expect(playback.state.status).toBe('error');
    expect(FakeContext.latest.sources).toHaveLength(0);
    FakeContext.latest.resume.mockRejectedValueOnce(new Error('denied'));
    playback.playAll();
    await settle();
    expect(playback.state.status).toBe('error');
    mocks.invoke.mockImplementation(async (...args) => args[0] === 'get_meeting_audio_capture_busy' ? true : original?.(...args));
    playback.playAll();
    await settle();
    expect(playback.state.status).toBe('paused');
  });
});

describe('meeting playback lifetime', () => {
  it.each([false, true])('refreshes a completed selected meeting without autoplay (initial read pending: %s)', async pendingInitial => {
    const original = mocks.invoke.getMockImplementation();
    const initial = deferred<unknown>();
    let complete = false;
    let busy = true;
    mocks.invoke.mockImplementation(async (...args) => {
      if (args[0] === 'get_meeting_audio_capture_busy') return busy;
      if (args[0] === 'get_meeting_audio_manifest' && !complete) {
        return pendingInitial ? initial.promise : { kind: 'unavailable', reason: 'notFinished' };
      }
      return original?.(...args);
    });
    root = createRoot(document.createElement('div'));
    await act(async () => { root?.render(<Harness busy />); await settle(); });
    expect(controls().status).toBe(pendingInitial ? 'loading' : 'unavailable');
    complete = true;
    await act(async () => {
      mocks.listeners.get('meeting-status-changed')?.({ payload: { sessionId: null, phase: 'idle' } });
      await settle();
    });
    expect(controls().status).toBe(pendingInitial ? 'loading' : 'unavailable');
    busy = false;
    await act(async () => {
      root?.render(<Harness />);
      mocks.listeners.get('query-state-changed')?.({ payload: { state: 'idle' } });
      await settle();
    });
    expect(controls().status).toBe('ready');
    expect(controls().unavailableReason).toBeNull();
    initial.resolve({ kind: 'unavailable', reason: 'notFinished' });
    await act(async () => { await settle(); });
    expect(controls().status).toBe('ready');
    expect(reads()).toHaveLength(0);
    await act(async () => { controls().playAll(); await settle(); });
    expect(controls().status).toBe('playing');
  });

  it('invalidates playing audio on deletion', async () => {
    root = createRoot(document.createElement('div'));
    await act(async () => { root?.render(<Harness />); await settle(); });
    await act(async () => { controls().playAll(); await settle(); });
    const context = FakeContext.latest;
    await act(async () => { mocks.listeners.get('meeting-audio-invalidated')?.({ payload: { sessionId: 'other' } }); });
    expect(controls().status).toBe('playing');
    await act(async () => { mocks.listeners.get('meeting-audio-invalidated')?.({ payload: { sessionId: 'meeting' } }); });
    expect(controls().status).toBe('unavailable');
    expect(context.sources.every(source => source.stop.mock.calls.length === 1)).toBe(true);
    await act(async () => { controls().playAll(); await settle(); });
    expect(controls().status).toBe('unavailable');
  });

  it('pauses immediately on actual capture events and session changes', async () => {
    root = createRoot(document.createElement('div'));
    await act(async () => { root?.render(<Harness />); await settle(); });
    await act(async () => { controls().playAll(); await settle(); });
    const oldContext = FakeContext.latest;
    await act(async () => {
      mocks.listeners.get('query-state-changed')?.({ payload: { state: 'listening' } });
      await settle();
    });
    expect(controls().status).toBe('paused');
    await act(async () => { controls().playAll(); await settle(); });
    await act(async () => { root?.render(<Harness sessionId="other" />); await settle(); });
    expect(oldContext.close).toHaveBeenCalledOnce();
    expect(controls().status).toBe('ready');
    expect(controls().positionMs).toBe(0);
  });

  it('survives React StrictMode effect replay', async () => {
    root = createRoot(document.createElement('div'));
    await act(async () => { root?.render(<StrictMode><Harness /></StrictMode>); await settle(); });
    expect(controls().status).toBe('ready');
    await act(async () => { controls().playAll(); await settle(); });
    expect(controls().status).toBe('playing');
  });

  it.each(['microphone-startup-benchmark-progress', 'transform-capture-starting'])('pauses immediately on %s before capture can receive playback', async eventName => {
    root = createRoot(document.createElement('div'));
    await act(async () => { root?.render(<Harness />); await settle(); });
    await act(async () => { controls().playAll(); await settle(); });
    const context = FakeContext.latest;
    await act(async () => {
      mocks.listeners.get(eventName)?.({ payload: null });
      expect(context.sources.every(source => source.stop.mock.calls.length === 1)).toBe(true);
    });
    expect(controls().status).toBe('paused');
    expect(context.sources.every(source => source.stop.mock.calls.length === 1)).toBe(true);
  });

  it.each(['transform-review-hidden', 'transform-capture-failed'])('recovers after %s and delayed capture-owner release on explicit Play', async terminalEvent => {
    const original = mocks.invoke.getMockImplementation();
    let busy = false;
    mocks.invoke.mockImplementation(async (...args) => args[0] === 'get_meeting_audio_capture_busy' ? busy : original?.(...args));
    root = createRoot(document.createElement('div'));
    await act(async () => { root?.render(<Harness />); await settle(); });
    await act(async () => { controls().playAll(); await settle(); });
    busy = true;
    await act(async () => {
      mocks.listeners.get('transform-state-changed')?.({ payload: { state: 'capturing' } });
      await settle();
    });
    expect(controls().status).toBe('paused');
    await act(async () => {
      mocks.listeners.get(terminalEvent)?.({ payload: {} });
      await settle();
    });
    busy = false;
    expect(controls().status).toBe('paused');
    await act(async () => { controls().play(); await settle(); });
    expect(controls().status).toBe('playing');
  });

  it.each(['subscription', 'hydration'])('shows %s monitoring failures and recovers only through renewed checks', async failure => {
    if (failure === 'subscription') mocks.listenerFailure = true;
    const original = mocks.invoke.getMockImplementation();
    if (failure === 'hydration') mocks.invoke.mockImplementation(async (...args) => {
      if (args[0] === 'get_meeting_audio_capture_busy') throw new Error('Unavailable');
      return original?.(...args);
    });
    root = createRoot(document.createElement('div'));
    await act(async () => { root?.render(<Harness />); await settle(); });
    expect(controls().status).toBe('error');
    expect(controls().error).toContain('safety checks');
    await act(async () => { controls().playAll(); await settle(); });
    expect(reads()).toHaveLength(0);
    mocks.listenerFailure = false;
    mocks.invoke.mockImplementation(async (...args) => original?.(...args));
    if (failure === 'subscription') {
      await act(async () => {
        mocks.listeners.get('recording-status-changed')?.({ payload: 'idle' });
        await settle();
      });
      expect(controls().status).toBe('error');
    }
    await act(async () => { controls().retry(); await settle(); });
    expect(controls().error).toBeNull();
    expect(controls().status).toBe('ready');
    expect(reads()).toHaveLength(0);
    await act(async () => { controls().playAll(); await settle(); });
    expect(controls().status).toBe('playing');
  });
});
