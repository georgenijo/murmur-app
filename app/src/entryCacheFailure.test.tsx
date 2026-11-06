import React, { act } from 'react';
import { afterEach, expect, it, vi } from 'vitest';
const mock = vi.hoisted(() => ({ invoke: vi.fn(), render: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => true, invoke: mock.invoke }));
vi.mock('react-dom/client', () => ({ default: { createRoot: () => ({ render: mock.render }) } }));
vi.mock('./App', () => ({ default: () => <div /> }));
vi.mock('./components/OverlayWidget', () => ({ OverlayWidget: () => <div /> }));
vi.mock('./components/log-viewer/DiagnosticsWindowApp', () => ({ DiagnosticsWindowApp: () => <div /> }));
vi.mock('./lib/hooks/useAppearance', () => ({ AppearanceProvider: ({ children }: { children: React.ReactNode }) => children }));
vi.mock('./lib/windowLifecycle', () => ({ NativeWindowLifecycle: ({ children }: { children: React.ReactNode }) => children }));
vi.mock('./lib/log', () => ({ flog: { info: vi.fn() } }));
vi.mock('./lib/completionData', () => ({ refreshCompletion: vi.fn(), removeImportedKeys: vi.fn(), initializeCompletionReader: vi.fn() }));
afterEach(() => { vi.restoreAllMocks(); localStorage.clear(); document.body.innerHTML = ''; });
it.each(['main', 'overlay', 'diagnostics'].flatMap(entry =>
  ['writes', 'reads', 'all'].map(failure => ({ entry, failure })),
))('$entry mounts authoritative state despite $failure failing in the presentation cache', async ({ entry, failure }) => {
  vi.resetModules(); mock.render.mockClear();
  document.body.innerHTML = '<div id="root"></div>';
  mock.invoke.mockImplementation(async (command: string) => {
    if (command === 'webkit_state_snapshot') return { v1: true, v2: true, values: {
      'dictation-settings': { settingsVersion: 7, language: 'ja', autoPaste: true },
      'murmur-appearance': { version: 1, mode: 'dark', theme: { version: 1, presetId: 'custom' } },
      'murmur-theme-library': { version: 1, revision: 17, themes: [] },
    } };
    if (command === 'load_settings_blob') return JSON.stringify({ settingsVersion: 7, language: 'ja', autoPaste: true });
    if (command === 'load_theme_library_blob') return JSON.stringify({ version: 1, revision: 17, themes: [] });
  });
  if (failure !== 'writes') vi.spyOn(localStorage, 'getItem').mockImplementation(() => { throw new DOMException('synthetic', 'SecurityError'); });
  if (failure !== 'reads') {
    vi.spyOn(localStorage, 'setItem').mockImplementation(() => { throw new DOMException('synthetic', 'QuotaExceededError'); });
    vi.spyOn(localStorage, 'removeItem').mockImplementation(() => { throw new DOMException('synthetic', 'SecurityError'); });
  }
  await act(async () => {
    if (entry === 'main') await import('./main');
    else if (entry === 'overlay') await import('./overlay');
    else await import('./diagnostics');
  });
  expect(mock.render).toHaveBeenCalledOnce();
  const tree = mock.render.mock.calls[0]![0];
  expect(tree.type).toBe(React.StrictMode);
  const { loadSettings } = await import('./lib/settings');
  expect(loadSettings()).toMatchObject({ language: 'ja', autoPaste: true });
  const { loadAppearanceDocument } = await import('./lib/appearance/storage');
  expect(loadAppearanceDocument().document.mode).toBe('dark');
  const { loadThemeLibrary } = await import('./lib/appearance/themeLibrary');
  expect(loadThemeLibrary().document.revision).toBe(17);
});
it('unreadable legacy input remains fail closed', async () => {
  vi.resetModules();
  mock.invoke.mockResolvedValue({ v1: false, v2: false, values: {} });
  vi.spyOn(localStorage, 'getItem').mockImplementation(() => { throw new DOMException('synthetic', 'SecurityError'); });
  const { initializeWebkitState } = await import('./lib/webkitLifecycle');
  await expect(initializeWebkitState(true)).rejects.toThrow();
  expect(mock.invoke.mock.calls.some(([c]) => c === 'import_webkit_state')).toBe(false);
});
