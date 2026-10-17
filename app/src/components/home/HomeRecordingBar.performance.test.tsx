import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  handlers: new Map<string, Set<(event: { payload: unknown }) => void>>(),
  addEntry: vi.fn(),
}));
vi.mock('@tauri-apps/api/event', () => ({
  listen: async (name: string, handler: (event: { payload: unknown }) => void) => {
    const handlers = mocks.handlers.get(name) ?? new Set();
    mocks.handlers.set(name, handlers);
    handlers.add(handler);
    return () => { handlers.delete(handler); };
  },
}));
vi.mock('../../lib/dictation', () => ({
  startRecording: vi.fn(), stopRecording: vi.fn(), cancelRecording: vi.fn(),
}));
vi.mock('../../lib/stats', () => ({ updateStats: vi.fn() }));
vi.mock('../../lib/log', () => ({ flog: { info: vi.fn(), warn: vi.fn(), error: vi.fn() } }));

import { useRecordingState } from '../../lib/hooks/useRecordingState';
import { HomeRecordingBar } from './HomeRecordingBar';

describe('recording audio render ownership', () => {
  let container: HTMLDivElement;
  let root: Root;
  let ownerRenders: number;
  function RecordingOwner({ active = true }: { active?: boolean }) {
    ownerRenders += 1;
    const recording = useRecordingState({ addEntry: mocks.addEntry, microphone: 'system_default' });
    return <HomeRecordingBar
      active={active}
      status={recording.status}
      recordingDuration={recording.recordingDuration}
      initialized
      triggerKey="shift_l"
      recordingMode="both"
      meetingPhase="idle"
      onRecord={recording.handleStart}
      onStop={recording.handleStop}
    />;
  }
  const emit = async (name: string, payload: unknown) => {
    await act(async () => {
      mocks.handlers.get(name)?.forEach(handler => handler({ payload }));
    });
  };
  beforeEach(() => {
    vi.useFakeTimers();
    mocks.handlers.clear();
    ownerRenders = 0;
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    vi.useRealTimers();
    container.remove();
  });

  it('updates the waveform for every audio sample without rerendering the recording owner', async () => {
    await act(async () => root.render(<RecordingOwner />));
    await emit('recording-status-changed', 'recording');
    const quiet = container.querySelector<HTMLElement>('.home-record-waveform span')!.style.height;
    const initialRenders = ownerRenders;
    // Flush each event separately: one batched burst would conceal the baseline
    // App-level render cost. These are render counts, not timing measurements.
    for (let index = 0; index < 120; index += 1) {
      await emit('audio-level', index % 2 === 0 ? 0 : 0.06);
    }
    expect(ownerRenders).toBe(initialRenders);
    expect(container.querySelector<HTMLElement>('.home-record-waveform span')!.style.height).not.toBe(quiet);
    expect(mocks.handlers.get('audio-level')?.size).toBe(1);
  });

  it('unsubscribes the hidden waveform while recording lifecycle events remain active', async () => {
    await act(async () => root.render(<RecordingOwner />));
    expect(mocks.handlers.get('audio-level')?.size ?? 0).toBe(0);
    await emit('recording-status-changed', 'recording');
    expect(mocks.handlers.get('audio-level')?.size).toBe(1);
    await act(async () => root.render(<RecordingOwner active={false} />));
    expect(mocks.handlers.get('audio-level')?.size).toBe(0);
    await emit('recording-status-changed', 'processing');
    expect(container.textContent).toContain('Writing it out');
    await emit('recording-status-changed', 'recording');
    await act(async () => root.render(<RecordingOwner />));
    expect(mocks.handlers.get('audio-level')?.size).toBe(1);
  });
});
