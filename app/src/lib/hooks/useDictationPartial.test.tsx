import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
  listen: vi.fn(),
  invoke: vi.fn(),
}));
vi.mock('@tauri-apps/api/event', () => ({ listen: mocks.listen }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));

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
    mocks.invoke.mockReset();
    mocks.invoke.mockResolvedValue(undefined);
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

  it('shows the native window only after the current partial renders', async () => {
    await act(async () => {
      mocks.listeners.get('dictation-generation-started')?.({ payload: { recordingId: 7 } });
      mocks.listeners.get('dictation-partial')?.({ payload: { recordingId: 7, text: 'rendered first' } });
    });

    expect(container.textContent).toBe('rendered first');
    expect(mocks.invoke).toHaveBeenCalledWith('show_dictation_preview', { recordingId: 7 });
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

  it('clears provisional words the moment capture leaves recording', async () => {
    // Rust hides the window when the ticker exits, which can trail the actual
    // stop by a tick or an in-flight decode; the card must empty immediately.
    await act(async () => {
      mocks.listeners.get('dictation-generation-started')?.({ payload: { recordingId: 9 } });
      mocks.listeners.get('dictation-partial')?.({ payload: { recordingId: 9, text: 'mid sentence' } });
    });
    expect(container.textContent).toBe('mid sentence');
    await act(async () => {
      mocks.listeners.get('recording-status-changed')?.({ payload: 'processing' });
    });
    expect(container.textContent).toBe('');
  });

  it('keeps rendering while the status event still says recording', async () => {
    await act(async () => {
      mocks.listeners.get('dictation-generation-started')?.({ payload: { recordingId: 10 } });
      mocks.listeners.get('recording-status-changed')?.({ payload: 'recording' });
      mocks.listeners.get('dictation-partial')?.({ payload: { recordingId: 10, text: 'still going' } });
    });
    expect(container.textContent).toBe('still going');
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
