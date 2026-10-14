import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { expect, it, vi } from 'vitest';
import { createAppearanceDocument, writeAppearanceDocument } from '../appearance';
import { useQueryAppearance } from './useQueryAppearance';

it('follows saved and system appearance without changing settings or native theme', async () => {
  const add = vi.fn();
  const remove = vi.fn();
  const media = { matches: false, addEventListener: add, removeEventListener: remove };
  vi.stubGlobal('matchMedia', () => media);
  const host = document.createElement('div');
  const root = createRoot(host);
  const before = window.localStorage.getItem('murmur-appearance');
  function Harness() { useQueryAppearance(); return null; }
  try {
    writeAppearanceDocument(createAppearanceDocument('dark'));
    const stored = window.localStorage.getItem('murmur-appearance');
    await act(async () => root.render(<Harness />));
    expect(document.documentElement.dataset.appearance).toBe('dark');
    expect(window.localStorage.getItem('murmur-appearance')).toBe(stored);
    writeAppearanceDocument(createAppearanceDocument('light'));
    await act(async () => window.dispatchEvent(new StorageEvent('storage')));
    expect(document.documentElement.dataset.appearance).toBe('light');
    writeAppearanceDocument(createAppearanceDocument('system'));
    media.matches = true;
    await act(async () => add.mock.calls[0][1]());
    expect(document.documentElement.dataset.appearance).toBe('dark');
  } finally {
    await act(async () => root.unmount());
    expect(remove).toHaveBeenCalled();
    if (before === null) window.localStorage.removeItem('murmur-appearance');
    else window.localStorage.setItem('murmur-appearance', before);
    vi.unstubAllGlobals();
  }
});
