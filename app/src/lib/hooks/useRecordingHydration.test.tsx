import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { beforeEach, afterEach, describe, expect, it, vi } from 'vitest';
import type { RecordingSnapshot } from '../recordingStatus';

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
  listen: vi.fn(),
  stop: vi.fn(),
  cancel: vi.fn(),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
vi.mock('../targetedEvents', () => ({ listen: mocks.listen }));
vi.mock('../dictation', () => ({ startRecording: vi.fn(), stopRecording: mocks.stop, cancelRecording: mocks.cancel }));
vi.mock('../log', () => ({ flog: { info: vi.fn(), warn: vi.fn(), error: vi.fn() } }));
import { useRecordingState } from './useRecordingState';

const recording: RecordingSnapshot = {
  state: 'recording', recordingId: 42, recordingStartedAtMs: Date.now() - 5000, locked: true,
};
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(done => { resolve = done; });
  return { promise, resolve };
}

describe('main renderer recording hydration', () => {
  let root: Root;
  let container: HTMLDivElement;
  let current: ReturnType<typeof useRecordingState>;
  let snapshot: ReturnType<typeof deferred<RecordingSnapshot>>;
  const emit = (event: string, payload: unknown) => mocks.listeners.get(event)?.({ payload });
  function Harness() {
    current = useRecordingState({ microphone: 'system_default' });
    return null;
  }
  beforeEach(() => {
    vi.resetAllMocks();
    mocks.listeners.clear();
    snapshot = deferred<RecordingSnapshot>();
    mocks.invoke.mockImplementation(async (command: string) => command === 'get_status' ? snapshot.promise : undefined);
    mocks.listen.mockImplementation(async (event: string, handler: (event: { payload: unknown }) => void) => {
      mocks.listeners.set(event, handler);
      return () => mocks.listeners.delete(event);
    });
    mocks.stop.mockResolvedValue({ state: 'idle' });
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });
  const mount = async () => { await act(async () => root.render(<Harness />)); };

  it('hydrates status, start time and native lock, then registers waveform interest and stops the live generation', async () => {
    await mount();
    expect(mocks.listeners.has('dictation-generation-started')).toBe(true);
    expect(mocks.invoke).not.toHaveBeenCalledWith('set_waveform_subscription', expect.anything());
    await act(async () => snapshot.resolve(recording));
    expect(current.status).toBe('recording');
    expect(current.lockedMode).toBe(true);
    expect(mocks.invoke).toHaveBeenCalledWith('set_waveform_subscription', { consumer: 'main-recording', active: true });
    await act(async () => emit('audio-level', 0.6));
    expect(current.audioLevel).toBe(0.6);
    await act(async () => current.handleStop());
    expect(mocks.stop).toHaveBeenCalledWith('other', 42);
  });

  it('cancels the hydrated starting generation', async () => {
    await mount();
    await act(async () => snapshot.resolve({ ...recording, state: 'starting', recordingStartedAtMs: null }));
    await act(async () => current.handleStop());
    expect(mocks.cancel).toHaveBeenCalledWith(42);
  });

  it('keeps later lifecycle and lock events after the snapshot', async () => {
    await mount();
    await act(async () => snapshot.resolve(recording));
    await act(async () => {
      emit('recording-status-changed', 'processing');
      emit('dictation-lock-changed', false);
      emit('recording-status-changed', 'idle');
    });
    expect(current.status).toBe('idle');
    expect(current.lockedMode).toBe(false);
    expect(mocks.invoke).toHaveBeenCalledWith('set_waveform_subscription', { consumer: 'main-recording', active: false });
  });

  it('rejects an older snapshot after a newer generation event', async () => {
    await mount();
    await act(async () => {
      emit('dictation-generation-started', { recordingId: 43 });
      emit('recording-status-changed', 'recording');
      snapshot.resolve({ ...recording, state: 'idle' });
    });
    expect(current.status).toBe('recording');
    await act(async () => current.handleStop());
    expect(mocks.stop).toHaveBeenCalledWith('other', 43);
  });

  it.each(['processing', 'idle', 'recovering'])('never resurrects recording if %s arrives before the snapshot returns', async state => {
    await mount();
    await act(async () => {
      emit('recording-status-changed', state);
      snapshot.resolve(recording);
    });
    expect(current.status).toBe(state);
    expect(mocks.invoke).not.toHaveBeenCalledWith('set_waveform_subscription', { consumer: 'main-recording', active: true });
  });

  it('waits for all lifecycle subscriptions before requesting the snapshot', async () => {
    const subscription = deferred<() => void>();
    mocks.listen.mockImplementation((event: string, handler: (event: { payload: unknown }) => void) => {
      mocks.listeners.set(event, handler);
      return event === 'dictation-generation-started' ? subscription.promise : Promise.resolve(() => {});
    });
    await mount();
    expect(mocks.invoke).not.toHaveBeenCalledWith('get_status');
    // This event occurs before subscription acknowledgement, and still fences the read.
    await act(async () => {
      emit('recording-status-changed', 'idle');
      subscription.resolve(() => {});
      snapshot.resolve(recording);
    });
    expect(current.status).toBe('idle');
  });

  it('restores the native elapsed duration', async () => {
    vi.useFakeTimers();
    try {
      await mount();
      await act(async () => snapshot.resolve({ ...recording, recordingStartedAtMs: Date.now() - 5000 }));
      await act(async () => vi.advanceTimersByTime(1000));
      expect(current.recordingDuration).toBe(6);
    } finally { vi.useRealTimers(); }
  });
});
