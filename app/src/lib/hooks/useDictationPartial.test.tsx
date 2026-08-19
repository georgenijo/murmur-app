import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
  listen: vi.fn(),
}));
vi.mock('@tauri-apps/api/event', () => ({ listen: mocks.listen }));

import { useDictationPartial } from './useDictationPartial';
import type { DictationStatus } from '../types';

describe('useDictationPartial', () => {
  let root: Root;
  let container: HTMLDivElement;
  let status: DictationStatus;

  function Harness() {
    const text = useDictationPartial(status);
    return <span>{text}</span>;
  }

  beforeEach(async () => {
    mocks.listeners.clear();
    mocks.listen.mockReset();
    mocks.listen.mockImplementation(async (name: string, handler: (event: { payload: unknown }) => void) => {
      mocks.listeners.set(name, handler);
      return () => mocks.listeners.delete(name);
    });
    status = 'recording';
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    await act(async () => root.render(<Harness />));
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('accepts only the current recording generation and clears on final processing', async () => {
    await act(async () => {
      mocks.listeners.get('dictation-generation-started')?.({ payload: { recordingId: 7 } });
      mocks.listeners.get('dictation-partial')?.({ payload: { recordingId: 6, text: 'stale' } });
      mocks.listeners.get('dictation-partial')?.({ payload: { recordingId: 7, text: ' provisional words ' } });
    });
    expect(container.textContent).toBe('provisional words');
    status = 'processing';
    await act(async () => root.render(<Harness />));
    expect(container.textContent).toBe('');
    await act(async () => {
      mocks.listeners.get('dictation-partial')?.({ payload: { recordingId: 7, text: 'late' } });
    });
    expect(container.textContent).toBe('');
  });

  it('rejects malformed and unbounded payloads and removes listeners', async () => {
    await act(async () => {
      mocks.listeners.get('dictation-generation-started')?.({ payload: { recordingId: 8 } });
      mocks.listeners.get('dictation-partial')?.({ payload: { recordingId: 8, text: 'x'.repeat(4097) } });
    });
    expect(container.textContent).toBe('');
    await act(async () => root.unmount());
    expect(mocks.listeners.size).toBe(0);
    root = createRoot(container);
  });
});
