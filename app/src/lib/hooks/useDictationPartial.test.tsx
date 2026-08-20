import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
  listen: vi.fn(),
}));
vi.mock('@tauri-apps/api/event', () => ({ listen: mocks.listen }));

import { useDictationPartial } from './useDictationPartial';

describe('useDictationPartial', () => {
  let root: Root;
  let container: HTMLDivElement;

  function Harness() {
    return <span>{useDictationPartial()}</span>;
  }

  beforeEach(async () => {
    mocks.listeners.clear();
    mocks.listen.mockReset();
    mocks.listen.mockImplementation(async (name: string, handler: (event: { payload: unknown }) => void) => {
      mocks.listeners.set(name, handler);
      return () => mocks.listeners.delete(name);
    });
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    await act(async () => root.render(<Harness />));
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('renders the current generation and drops older recordings', async () => {
    await act(async () => {
      mocks.listeners.get('dictation-generation-started')?.({ payload: { recordingId: 7 } });
      mocks.listeners.get('dictation-partial')?.({ payload: { recordingId: 6, text: 'stale' } });
      mocks.listeners.get('dictation-partial')?.({ payload: { recordingId: 7, text: ' provisional words ' } });
    });
    expect(container.textContent).toBe('provisional words');
  });

  it('clears the card when a new recording starts', async () => {
    await act(async () => {
      mocks.listeners.get('dictation-generation-started')?.({ payload: { recordingId: 7 } });
      mocks.listeners.get('dictation-partial')?.({ payload: { recordingId: 7, text: 'first take' } });
      mocks.listeners.get('dictation-generation-started')?.({ payload: { recordingId: 8 } });
    });
    expect(container.textContent).toBe('');
  });

  it('adopts a partial that arrives before this window sees the generation event', async () => {
    // The preview window is only shown once words exist, so its very first
    // event can be the partial itself.
    await act(async () => {
      mocks.listeners.get('dictation-partial')?.({ payload: { recordingId: 12, text: 'okay so' } });
    });
    expect(container.textContent).toBe('okay so');
  });

  it('rejects malformed and unbounded payloads and removes listeners', async () => {
    await act(async () => {
      mocks.listeners.get('dictation-generation-started')?.({ payload: { recordingId: 8 } });
      mocks.listeners.get('dictation-partial')?.({ payload: { recordingId: 8, text: 'x'.repeat(4097) } });
      mocks.listeners.get('dictation-partial')?.({ payload: { recordingId: 8 } });
      mocks.listeners.get('dictation-generation-started')?.({ payload: { recordingId: -1 } });
    });
    expect(container.textContent).toBe('');
    await act(async () => root.unmount());
    expect(mocks.listeners.size).toBe(0);
    root = createRoot(container);
  });
});
