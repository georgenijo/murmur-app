import { StrictMode, act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { mockIPC } from '@tauri-apps/api/mocks';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import fixtures from '../../src-tauri/crates/murmur-core/src/state/gpui-downgrade-fixtures.json';
import type { AppearanceController, AppearanceDocumentV1, ThemeLibraryEntryV1 } from './appearance/types';

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

it.each([
  'mode', 'custom edits', 'reset', 'import', 'single selection', 'paired selection',
  'save current', 'save preview', 'install', 'duplicate', 'rename collection entry',
  'remove selected', 'replace selected', 'remove unselected',
])('preserves nested GPUI metadata through %s and the native save payloads', async (edit) => {
  const library = structuredClone(fixtures.library);
  // A distinct retained entry catches deletion/replacement accidentally wiping
  // another theme's metadata. Use only derivatives of the synthetic fixture.
  const other = { ...structuredClone(library.themes[0]), id: 'retained-theme', futureEntry: 99 };
  library.themes.push(other);
  const { disk, ipc, lifecycle, storage, mount } = await setup(fixtures.appearance, library);
  const controller = await mount();
  const targetId = library.themes[0].id;
  let deletedId: string | undefined;
  const imported = storage.previewAppearanceImport(JSON.stringify({
    version: 1, mode: 'light', theme: { version: 1, presetId: 'custom', accent: '#112233' },
  }));
  const run = async <T,>(operation: () => Promise<T>): Promise<T> => {
    let result!: T;
    await act(async () => { result = await operation(); });
    return result;
  };
  switch (edit) {
    case 'mode':
      await run(() => controller().setMode('dark'));
      expect(controller().document.mode).toBe('dark');
      break;
    case 'custom edits':
      await run(() => controller().updateTheme({ accent: '#112233' }));
      expect(controller().document.theme.accent).toBe('#112233');
      expect(storage.sanitizeTheme(controller().document.theme).dark).toBeUndefined();
      await run(() => controller().updateTheme({ background: '#223344' }));
      expect(controller().document.theme).toMatchObject({ accent: '#112233', background: '#223344' });
      await run(() => controller().updateTheme({ accent: undefined }));
      expect(controller().document.theme).not.toHaveProperty('accent');
      expect(controller().document.theme.background).toBe('#223344');
      expect(controller().document.selection).toMatchObject({ light: 'custom', dark: 'custom' });
      await run(() => controller().updateTheme({ dark: { background: '#334455' } }));
      expect(controller().document.theme.dark?.background).toBe('#334455');
      await run(() => controller().updateTheme({ dark: undefined }));
      expect(storage.sanitizeTheme(controller().document.theme).dark).toBeUndefined();
      break;
    case 'reset':
      await run(() => controller().reset());
      expect(storage.sanitizeTheme(controller().document.theme)).toEqual({ version: 1, presetId: 'sonic' });
      expect(controller().document.selection).toMatchObject({ light: 'sonic', dark: 'sonic' });
      break;
    case 'import':
      await run(() => controller().commitImport(imported));
      expect(storage.sanitizeTheme(controller().document.theme)).toEqual(imported.theme);
      expect(controller().document.mode).toBe('light');
      break;
    case 'single selection':
    case 'paired selection': {
      const paired = edit === 'paired selection';
      await run(() => controller().commitImport(controller().library.previewSelection(paired ? 'opencode' : targetId)));
      expect(controller().document.selection).toMatchObject({
        light: paired ? 'opencode' : 'custom', dark: paired ? 'opencode' : targetId,
      });
      break;
    }
    case 'save current': {
      const saved = await run(() => controller().library.saveCurrent('Saved synthetic'));
      expect(saved.label).toBe('Saved synthetic');
      expectOpaqueUnchanged(fixtures.appearance.theme, saved.theme);
      expect(storage.sanitizeTheme(saved.theme)).toEqual(storage.sanitizeTheme(fixtures.appearance.theme));
      break;
    }
    case 'save preview': {
      const saved = await run(() => controller().library.savePreview('Imported synthetic', imported));
      expect(saved).toMatchObject({ label: 'Imported synthetic', theme: imported.theme });
      break;
    }
    case 'install':
      await run(() => controller().library.install([storage.makeLocalThemeEntry('new-synthetic', 'New synthetic', imported.theme)]));
      expect(controller().library.document.themes.map(entry => entry.id)).toContain('new-synthetic');
      break;
    case 'duplicate': {
      // Same makeLocalThemeEntry + install path as the Duplicate card action.
      const source = controller().library.document.themes[0];
      const copy = storage.makeLocalThemeEntry('copy-synthetic', 'Copy of Synthetic', source.theme, source.modes);
      await run(() => controller().library.install([copy]));
      expectOpaqueUnchanged(source.theme, controller().library.document.themes[2].theme);
      expect(controller().library.document.themes[2]).toMatchObject({ id: copy.id, label: copy.label });
      break;
    }
    case 'rename collection entry':
    case 'replace selected': {
      await run(() => controller().commitImport(controller().library.previewSelection(targetId)));
      const previous = controller().library.document.themes.filter(entry => entry.collection?.id === 'synthetic');
      const old = previous[0];
      // Replacements contain validated known fields only: preservation must
      // come from the stored entry with the same ID, never from an array index.
      const replacement: ThemeLibraryEntryV1 = {
        version: 1, id: old.id, label: 'Renamed synthetic', modes: ['dark'],
        theme: { version: 1, presetId: 'custom', accent: '#112233' },
        source: { kind: 'local' }, collection: { id: 'synthetic', label: 'Renamed collection' },
      };
      if (edit === 'replace selected') {
        replacement.id = 'replacement-synthetic';
        deletedId = targetId;
      }
      await run(() => controller().library.replaceCollection('synthetic', [replacement, other as ThemeLibraryEntryV1], previous));
      expect(controller().library.document.themes[0]).toMatchObject(replacement);
      if (deletedId) expect(controller().document.selection?.dark).toBe('sonic');
      else expect(controller().document.theme.dark?.primary).toBe(storage.resolveTheme(replacement.theme, 'dark').tokens.primary);
      break;
    }
    case 'remove selected':
    case 'remove unselected':
      if (edit === 'remove selected') await run(() => controller().commitImport(controller().library.previewSelection(targetId)));
      await run(() => controller().library.remove([targetId]));
      deletedId = targetId;
      expect(controller().library.document.themes.map(entry => entry.id)).toEqual([other.id]);
      expect(controller().document.selection?.dark).toBe(edit === 'remove selected' ? 'sonic' : 'custom');
      break;
  }
  await lifecycle.flushSourceState();
  const saves = ipc.mock.calls.filter(([command]) => command.startsWith('save_'));
  expect(saves.length).toBeGreaterThan(0);
  const expectLibraryOpaque = (written: unknown) => {
    const actual = written as { themes: ThemeLibraryEntryV1[] };
    expectOpaqueUnchanged({ ...library, themes: [] }, actual);
    for (const original of library.themes) {
      if (original.id !== deletedId) {
        expectOpaqueUnchanged(original, actual.themes.find(entry => entry.id === original.id));
      }
    }
  };
  // Inspect every native write, including intermediate edits, rather than just
  // checking the final in-memory snapshot or a localStorage cache.
  for (const [command, args] of saves) {
    if (command === 'save_webkit_source') {
      expect(args!.key).toBe('murmur-appearance');
      expectOpaqueUnchanged(fixtures.appearance, args!.value);
    } else if (command === 'save_theme_library_blob') {
      expectLibraryOpaque(JSON.parse(args!.blob as string));
    }
  }
  expectOpaqueUnchanged(fixtures.appearance, disk['murmur-appearance']);
  expectLibraryOpaque(disk['murmur-theme-library']);
  const persisted = disk['murmur-appearance'] as AppearanceDocumentV1;
  expect(storage.sanitizeTheme(persisted.theme)).toEqual(storage.sanitizeTheme(controller().document.theme));
  expect(persisted.mode).toBe(controller().document.mode);
  expect(persisted.selection).toEqual(controller().document.selection);
  expect(disk['murmur-theme-library']).toEqual(controller().library.document);
  expect(storage.loadAppearanceDocument().needsRepair).toBe(false);
  expect(storage.loadThemeLibrary()).toMatchObject({ needsRepair: false });
  expect(controller().error).toBeNull();
  expect(controller().library.error).toBeNull();
  const exported = [controller().exportText(), ...controller().library.document.themes.map(storage.exportThemeLibraryEntryText)];
  expect(exported.join('')).not.toContain('future');
});

it('keeps nested opaque metadata during automatic library ID rename and selection repair', async () => {
  const library = structuredClone(fixtures.library);
  library.themes[0].id = 'opencode';
  const appearance = structuredClone(fixtures.appearance);
  appearance.selection.dark = 'opencode';
  const { ipc, lifecycle, storage, mount } = await setup(appearance, library);
  const controller = await mount();
  await lifecycle.flushSourceState();
  const renamed = controller().library.document.themes[0];
  expect(renamed.id).toBe('synthetic');
  expectOpaqueUnchanged(library.themes[0], renamed);
  expect(controller().document.selection?.dark).toBe(renamed.id);
  for (const [, args] of ipc.mock.calls.filter(([command]) => command === 'save_webkit_source')) {
    expectOpaqueUnchanged(appearance, args!.value);
  }
  expect(ipc.mock.calls.some(([command]) => command === 'save_webkit_source')).toBe(true);
  // The library's repaired IDs are persisted by its next user mutation.
  await act(async () => { await controller().library.saveCurrent('Synthetic copy'); });
  await lifecycle.flushSourceState();
  const writes = ipc.mock.calls.filter(([command]) => command === 'save_theme_library_blob');
  expect(writes).toHaveLength(1);
  expectOpaqueUnchanged(library, JSON.parse(writes[0][1]!.blob as string));
  expect(storage.loadThemeLibrary()).toMatchObject({ needsRepair: false });
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
