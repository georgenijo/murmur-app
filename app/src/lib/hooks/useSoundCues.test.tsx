import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
  listen: vi.fn(),
  play: vi.fn(),
  report: vi.fn(),
}));

vi.mock('@tauri-apps/api/event', () => ({ listen: mocks.listen }));
vi.mock('../soundCues', () => ({ playSoundCue: mocks.play }));
vi.mock('../dictation', () => ({ reportStartCueFinished: mocks.report }));

import { useSoundCues } from './useSoundCues';
import type { MeetingRuntimePhase } from '../meetings';

describe('useSoundCues', () => {
  let container: HTMLDivElement;
  let root: Root;
  let phase: MeetingRuntimePhase;
  let settings = { soundCuesEnabled: true, soundCueVolume: 45, meetingSoundCuesEnabled: false, muteSpeakersWhileDictating: false };

  function Harness() {
    useSoundCues(settings, phase);
    return null;
  }

  beforeEach(async () => {
    mocks.listeners.clear();
    mocks.play.mockReset();
    mocks.report.mockReset();
    settings = { soundCuesEnabled: true, soundCueVolume: 45, meetingSoundCuesEnabled: false, muteSpeakersWhileDictating: false };
    mocks.listen.mockReset();
    mocks.listen.mockImplementation(async (name: string, handler: (event: { payload: unknown }) => void) => {
      mocks.listeners.set(name, handler);
      return () => mocks.listeners.delete(name);
    });
    phase = 'idle';
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    await act(async () => root.render(<Harness />));
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('maps accepted start, stop, delivery, and failure events', () => {
    mocks.listeners.get('dictation-generation-started')?.({ payload: { recordingId: 1 } });
    mocks.listeners.get('recording-status-changed')?.({ payload: 'processing' });
    mocks.listeners.get('dictation-delivery-outcome')?.({ payload: { outcome: 'autoPastePosted' } });
    mocks.listeners.get('dictation-delivery-outcome')?.({ payload: { outcome: 'unconfirmed' } });
    expect(mocks.play.mock.calls.map(([cue]) => cue)).toEqual(['start', 'stop', 'success', 'failure']);
  });

  it('reports the start cue play-out only after the cue settles, when muting is on', async () => {
    settings = { ...settings, muteSpeakersWhileDictating: true };
    await act(async () => root.render(<Harness />));
    let finish: () => void = () => {};
    mocks.play.mockReturnValue(new Promise<void>((resolve) => { finish = resolve; }));
    mocks.listeners.get('dictation-generation-started')?.({ payload: { recordingId: 7 } });
    await act(async () => {});
    expect(mocks.play).toHaveBeenCalledWith('start', 45);
    expect(mocks.report).not.toHaveBeenCalled();
    await act(async () => finish());
    expect(mocks.report).toHaveBeenCalledWith(7);
  });

  it('reports at once when no start cue plays, and never when muting is off', async () => {
    settings = { ...settings, soundCuesEnabled: false, muteSpeakersWhileDictating: true };
    await act(async () => root.render(<Harness />));
    mocks.listeners.get('dictation-generation-started')?.({ payload: { recordingId: 8 } });
    await act(async () => {});
    expect(mocks.play).not.toHaveBeenCalled();
    expect(mocks.report).toHaveBeenCalledWith(8);
    mocks.listeners.get('dictation-generation-started')?.({ payload: {} });
    await act(async () => {});
    expect(mocks.report).toHaveBeenCalledTimes(1);

    settings = { ...settings, soundCuesEnabled: true, muteSpeakersWhileDictating: false };
    await act(async () => root.render(<Harness />));
    mocks.play.mockResolvedValue(undefined);
    mocks.listeners.get('dictation-generation-started')?.({ payload: { recordingId: 9 } });
    await act(async () => {});
    expect(mocks.report).toHaveBeenCalledTimes(1);
  });

  it('maps every camelCase delivery outcome and ignores unknown payloads', () => {
    const emit = (payload: unknown) => mocks.listeners.get('dictation-delivery-outcome')?.({ payload });
    for (const outcome of ['autoPastePosted', 'clipboardOnly', 'clipboardWriteFailed', 'unconfirmed', 'bogus', 'auto_pasted']) {
      emit({ outcome });
    }
    emit(null);
    expect(mocks.play.mock.calls.map(([cue]) => cue)).toEqual(['success', 'success', 'failure', 'failure']);
  });

  it('suppresses dictation cues during active meetings but allows them after failure', async () => {
    phase = 'recording';
    await act(async () => root.render(<Harness />));
    mocks.listeners.get('dictation-generation-started')?.({ payload: {} });
    expect(mocks.play).not.toHaveBeenCalled();
    phase = 'failed';
    await act(async () => root.render(<Harness />));
    mocks.listeners.get('dictation-generation-started')?.({ payload: {} });
    expect(mocks.play).toHaveBeenCalledWith('start', 45);
  });

  it('cleans up successful listeners and tolerates rejected registration', async () => {
    await act(async () => root.unmount());
    expect(mocks.listeners.size).toBe(0);
    root = createRoot(container);
    mocks.listen.mockRejectedValue(new Error('unavailable'));
    await act(async () => root.render(<Harness />));
    expect(mocks.listeners.size).toBe(0);
  });
});
