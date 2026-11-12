import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { DictationStatus } from '../types';

const mocks = vi.hoisted(() => ({
  listen: vi.fn(),
  unlisten: vi.fn(),
  handlers: new Map<string, (event: { payload: number }) => void>(),
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: (name: string, handler: (event: { payload: number }) => void) => {
    mocks.handlers.set(name, handler);
    mocks.listen(name);
    return Promise.resolve(mocks.unlisten);
  },
}));

import { useWaveform } from './useWaveform';

function Harness({ status }: { status: DictationStatus }) {
  const { barRefs } = useWaveform(status);
  return (
    <div>
      <div ref={(el) => { barRefs.current[0] = el; }} data-testid="bar" />
    </div>
  );
}

describe('useWaveform', () => {
  let root: Root;
  let container: HTMLDivElement;

  beforeEach(() => {
    mocks.listen.mockClear();
    mocks.unlisten.mockClear();
    mocks.handlers.clear();
    container = document.createElement('div');
    root = createRoot(container);
  });
  afterEach(async () => {
    await act(async () => root.unmount());
  });

  it('listens only to the overlay feed, never the raw audio-level stream', async () => {
    // A global listen() receives every emit_to for its event name, whatever
    // window it targets. The raw stream belongs to Stop on Silence in the main
    // window; the overlay has its own peak-held event.
    await act(async () => root.render(<Harness status="recording" />));
    const names = mocks.listen.mock.calls.map(([name]) => name);
    expect(names).toEqual(['overlay-audio-level']);
    expect(names).not.toContain('audio-level');
  });

  it('releases the listener on unmount', async () => {
    await act(async () => root.render(<Harness status="recording" />));
    await act(async () => root.unmount());
    expect(mocks.unlisten).toHaveBeenCalledOnce();
    root = createRoot(container);
  });
});
