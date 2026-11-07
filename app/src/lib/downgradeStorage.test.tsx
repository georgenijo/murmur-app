import { StrictMode, act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { mockIPC } from '@tauri-apps/api/mocks';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import fixtures from '../../src-tauri/crates/murmur-core/src/state/gpui-downgrade-fixtures.json';
import type { AppearanceController } from './appearance/types';

// Production decoders, controller, storage and queued writer; only the native
// invoke boundary is replaced. All source data is checked-in and synthetic.
const roots: Root[] = [];
const listeners: [string, EventListenerOrEventListenerObject][] = [];
beforeEach(() => {
  vi.resetModules();
  localStorage.clear();
  vi.stubGlobal('isTauri', true);
  const add = window.addEventListener.bind(window);
  vi.spyOn(window, 'addEventListener').mockImplementation((name, handler, options) => {
    if (handler) listeners.push([name, handler]);
    add(name, handler, options);
  });
});
afterEach(async () => {
  for (const root of roots.splice(0)) await act(async () => root.unmount());
  for (const [name, handler] of listeners.splice(0)) window.removeEventListener(name, handler);
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

function expectOpaqueUnchanged(original: unknown, written: unknown): void {
  if (Array.isArray(original)) {
    original.forEach((value, index) => expectOpaqueUnchanged(value, (written as unknown[])[index]));
  } else if (original && typeof original === 'object') {
    for (const [key, value] of Object.entries(original)) {
      const actual = (written as Record<string, unknown> | undefined)?.[key];
      if (key.startsWith('future')) expect(JSON.stringify(actual), key).toBe(JSON.stringify(value));
      else expectOpaqueUnchanged(value, actual);
    }
  }
}

async function setup(appearance: unknown = fixtures.appearance, library: unknown = fixtures.library) {
  const disk: Record<string, unknown> = structuredClone({
    'murmur-appearance': appearance, 'murmur-theme-library': library,
    'dictation-settings': fixtures.settings,
  });
  const ipc = vi.fn(async (command: string, args?: Record<string, unknown>) => {
    if (command === 'webkit_state_snapshot') return { v1: true, v2: true, v3: true, values: structuredClone(disk) };
    if (command === 'save_webkit_source') disk[args!.key as string] = structuredClone(args!.value);
    else if (command === 'save_theme_library_blob' || command === 'save_settings_blob') {
      const key = command === 'save_settings_blob' ? 'dictation-settings' : 'murmur-theme-library';
      disk[key] = JSON.parse(args!.blob as string);
      return JSON.stringify(disk[key]);
    } else if (command === 'completion_snapshot' || command === 'mutate_history') {
      return { history: [structuredClone(fixtures.history)], statistics: {}, reports: [], v1Committed: true, removeKeys: [], retryNotice: false };
    } else if (command !== 'register_source_session' && !command.startsWith('plugin:')) {
      throw new Error(`Unexpected synthetic invoke: ${command}`);
    }
  });
  mockIPC((command, args) => ipc(command, args as Record<string, unknown> | undefined));
  const lifecycle = await import('./webkitLifecycle');
  await lifecycle.initializeWebkitState();
  const storage = await import('./appearance');
  const mount = async () => {
    const { AppearanceProvider, useAppearance } = await import('./hooks/useAppearance');
    let controller!: AppearanceController;
    function Consumer() { controller = useAppearance(); return null; }
    const root = createRoot(document.createElement('div'));
    roots.push(root);
    await act(async () => root.render(<StrictMode><AppearanceProvider><Consumer /></AppearanceProvider></StrictMode>));
    await lifecycle.flushSourceState();
    return () => controller;
  };
  return { disk, ipc, lifecycle, storage, mount };
}

it('loads GPUI appearance/library without repair writes and preserves opaque fields through real saves', async () => {
  const { disk, ipc, lifecycle, storage, mount } = await setup();
  expect(storage.loadAppearanceDocument().needsRepair).toBe(false);
  expect(storage.loadThemeLibrary()).toMatchObject({ status: 'ready', needsRepair: false });
  const controller = await mount();
  expect(ipc.mock.calls.filter(([command]) => command.startsWith('save_'))).toEqual([]);
  expect(disk['murmur-appearance']).toEqual(fixtures.appearance);
  expect(disk['murmur-theme-library']).toEqual(fixtures.library);

  await act(async () => controller().setMode('dark'));
  storage.writeThemeLibrary(controller().library.document);
  await lifecycle.flushSourceState();
  const installed = storage.makeLocalThemeEntry('another-synthetic', 'Another synthetic', { version: 1, presetId: 'sonic' });
  await act(async () => controller().library.install([installed]));
  await act(async () => controller().library.remove([installed.id]));
  await lifecycle.flushSourceState();
  expect(ipc.mock.calls.some(([command]) => command === 'save_webkit_source')).toBe(true);
  expect(ipc.mock.calls.some(([command]) => command === 'save_theme_library_blob')).toBe(true);
  expectOpaqueUnchanged(fixtures.appearance, disk['murmur-appearance']);
  expectOpaqueUnchanged(fixtures.library, disk['murmur-theme-library']);
  expectOpaqueUnchanged(fixtures.appearance, storage.loadAppearanceDocument().document);
  expect(storage.loadThemeLibrary()).toMatchObject({ needsRepair: false });
  expect(controller().error).toBeNull();
  expect(controller().library.error).toBeNull();

  const exportedAppearance = JSON.parse(controller().exportText());
  expect(exportedAppearance).toEqual({ version: 1, mode: 'dark', theme: storage.sanitizeTheme(fixtures.appearance.theme) });
  const exportedLibrary = JSON.parse(storage.exportThemeLibraryEntryText(controller().library.document.themes[0]));
  expect(exportedLibrary).toEqual({ version: 2, name: 'Synthetic', modes: ['dark'], theme: { version: 1, presetId: 'custom' } });
  expect(JSON.stringify([exportedAppearance, exportedLibrary])).not.toContain('future');
});

it('repairs invalid known appearance fields without dropping opaque fields in the repair write', async () => {
  const invalid = { ...structuredClone(fixtures.appearance), mode: 'invalid', revision: -4 };
  invalid.theme.accent = 'invalid';
  invalid.theme.dark.background = 'invalid';
  const { disk, ipc, lifecycle, storage, mount } = await setup(invalid);
  expect(storage.loadAppearanceDocument().needsRepair).toBe(true);
  const controller = await mount();
  await lifecycle.flushSourceState();
  expectOpaqueUnchanged(fixtures.appearance, disk['murmur-appearance']);
  expect(disk['murmur-appearance']).toMatchObject({ mode: 'system', revision: 1 });
  expect((disk['murmur-appearance'] as { theme: object }).theme).not.toHaveProperty('accent');
  expect((disk['murmur-appearance'] as { theme: { dark: object } }).theme.dark).not.toHaveProperty('background');
  expect(storage.loadAppearanceDocument().needsRepair).toBe(false);
  expect(ipc.mock.calls.filter(([command]) => command === 'save_webkit_source')).toHaveLength(1);
  expect(controller().error).toBeNull();
});

it('repairs known library fields and preserves opaque entry/source/collection/theme data on write', async () => {
  const invalid = structuredClone(fixtures.library);
  invalid.revision = -1;
  invalid.themes[0].label = '  Synthetic  ';
  const { disk, lifecycle, storage } = await setup(fixtures.appearance, invalid);
  const loaded = storage.loadThemeLibrary();
  expect(loaded).toMatchObject({ status: 'ready', needsRepair: true });
  if (loaded.status !== 'ready') throw new Error('Expected a repairable synthetic library');
  storage.writeThemeLibrary(loaded.document);
  await lifecycle.flushSourceState();
  expectOpaqueUnchanged(fixtures.library, disk['murmur-theme-library']);
  expect(disk['murmur-theme-library']).toMatchObject({ revision: 0, themes: [{ label: 'Synthetic' }] });
  expect(storage.loadThemeLibrary()).toMatchObject({ needsRepair: false });
});

it('preserves settings through frontend load/edit/save and sends intact History exact-row mutations', async () => {
  const { disk, ipc, lifecycle } = await setup();
  const { loadSettings, saveSettings } = await import('./settings');
  saveSettings({ ...loadSettings(), language: 'ja' });
  await lifecycle.flushSourceState();
  expectOpaqueUnchanged(fixtures.settings, disk['dictation-settings']);
  expect(disk['dictation-settings']).toMatchObject({ language: 'ja', settingsVersion: 7 });

  const { refreshCompletion } = await import('./completionData');
  await refreshCompletion();
  const { useHistoryManagement } = await import('./hooks/useHistoryManagement');
  let history!: ReturnType<typeof useHistoryManagement>;
  function Consumer() { history = useHistoryManagement(); return null; }
  const root = createRoot(document.createElement('div'));
  roots.push(root);
  await act(async () => root.render(<Consumer />));
  expectOpaqueUnchanged(fixtures.history, history.historyEntries[0]);
  await act(async () => history.togglePinned(history.historyEntries[0]));
  await act(async () => history.updateEntry(history.historyEntries[0], 'Synthetic edited transcript.'));
  const mutations = ipc.mock.calls.filter(([command]) => command === 'mutate_history');
  expect(mutations).toHaveLength(2);
  for (const [, args] of mutations) {
    expectOpaqueUnchanged(fixtures.history, (args!.mutation as { target: unknown }).target);
    expect((args!.mutation as { target: unknown }).target).toEqual(fixtures.history);
  }
  expect(ipc.mock.calls.some(([command]) => command === 'save_history_blob')).toBe(false);
});
