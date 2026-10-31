import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, expect, it, vi } from 'vitest';
const mocks = vi.hoisted(() => ({
  invoke: vi.fn(), listeners: new Map<string, (event: { payload: unknown }) => void>(),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke, isTauri: () => true }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async (name, handler) => {
  mocks.listeners.set(name, handler);
  return () => { if (mocks.listeners.get(name) === handler) mocks.listeners.delete(name); };
}) }));
vi.mock('../../lib/hooks/useQueryAppearance', () => ({ useQueryAppearance: () => {} }));
import { mountDictationPreview } from './bootstrap';
import { DEFAULT_SETTINGS, STORAGE_KEY } from '../../lib/settings';
afterEach(() => { localStorage.clear(); mocks.listeners.clear(); vi.clearAllMocks(); });

it.each([true, false])('waits for durable consent %s before preview can mount or show', async (durableEnabled) => {
  localStorage.clear();
  if (!durableEnabled) localStorage.setItem(STORAGE_KEY, JSON.stringify({ ...DEFAULT_SETTINGS, overlayLiveWords: true }));
  let resolve!: (blob: string) => void;
  const durable = new Promise<string>(r => { resolve = r; });
  mocks.invoke.mockImplementation((command: string) => command === 'load_settings_blob' ? durable : Promise.resolve(undefined));
  const container = document.createElement('div'); document.body.append(container);
  const root = createRoot(container);
  try {
    const boot = mountDictationPreview(root);
    expect(container.textContent).toBe('');
    expect(mocks.listeners.has('dictation-partial')).toBe(false);
    expect(mocks.invoke).not.toHaveBeenCalledWith('show_dictation_preview', expect.anything());
    await act(async () => { resolve(JSON.stringify({ ...DEFAULT_SETTINGS, overlayLiveWords: durableEnabled })); await boot; });
    await act(async () => {
      mocks.listeners.get('dictation-generation-started')?.({ payload: { recordingId: 7 } });
      mocks.listeners.get('dictation-partial')?.({ payload: { recordingId: 7, text: 'synthetic consent probe' } });
    });
    expect(container.textContent?.includes('synthetic consent probe')).toBe(durableEnabled);
    const shows = mocks.invoke.mock.calls.filter(([command]) => command === 'show_dictation_preview');
    expect(shows).toHaveLength(durableEnabled ? 1 : 0);
  } finally { await act(async () => root.unmount()); container.remove(); }
});
