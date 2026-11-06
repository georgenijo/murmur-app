import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
  listen: vi.fn(),
  play: vi.fn(),
}));

vi.mock('@tauri-apps/api/event', () => ({ listen: mocks.listen }));
vi.mock('../soundCues', () => ({ playSoundCue: mocks.play }));

import { useSoundCues } from './useSoundCues';
import type { MeetingRuntimePhase } from '../meetings';

describe('useSoundCues', () => {
  let container: HTMLDivElement;
  let root: Root;
  let phase: MeetingRuntimePhase;
  const settings = { soundCuesEnabled: true, soundCueVolume: 45, meetingSoundCuesEnabled: false };

  function Harness() {
    useSoundCues(settings, phase);
    return null;
  }

  beforeEach(async () => {
    mocks.listeners.clear();
    mocks.play.mockReset();
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

  it('leaves dictation cues to the native host', () => {
    expect(mocks.listeners.size).toBe(0);
    expect(mocks.play).not.toHaveBeenCalled();
  });
});
