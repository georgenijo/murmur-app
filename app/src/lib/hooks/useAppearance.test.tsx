import { StrictMode } from 'react';
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  APPEARANCE_STORAGE_KEY,
  MAX_APPEARANCE_REVISION,
  MURMUR_TOKEN_NAMES,
  THEME_LIBRARY_STORAGE_KEY,
  createAppearanceDocument,
  exportAppearanceText,
  makeLocalThemeEntry,
  type AppearanceChangedEvent,
  type AppearanceController,
} from '../appearance';

const mocks = vi.hoisted(() => ({
  setTheme: vi.fn(async () => {}),
  emit: vi.fn(async (_event: string, _payload?: unknown): Promise<void> => {}),
  invoke: vi.fn(async (_command: string, _args?: unknown): Promise<unknown> => undefined),
  beforeListenResolve: null as (() => void) | null,
  listeners: [] as Array<{
    event: string;
    active: boolean;
    callback: (event: { payload: unknown }) => void;
  }>,
}));

vi.mock('@tauri-apps/api/app', () => ({ setTheme: mocks.setTheme }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
vi.mock('@tauri-apps/api/event', () => ({
  emit: mocks.emit,
  listen: vi.fn(async (event: string, callback: (event: { payload: unknown }) => void) => {
    const registration = { event, active: true, callback };
    mocks.listeners.push(registration);
    mocks.beforeListenResolve?.();
    return () => { registration.active = false; };
  }),
}));

import {
  APPEARANCE_CHANGED_EVENT,
  AppearanceProvider as Provider,
  useAppearance,
} from './useAppearance';

interface MediaHarness {
  matches: boolean;
  listeners: Set<() => void>;
  addEventListener: ReturnType<typeof vi.fn>;
  removeEventListener: ReturnType<typeof vi.fn>;
  dispatch(matches: boolean): void;
}

function mediaHarness(initial: boolean): MediaHarness {
  const listeners = new Set<() => void>();
  return {
    matches: initial,
    listeners,
    addEventListener: vi.fn((_event: string, callback: () => void) => listeners.add(callback)),
    removeEventListener: vi.fn((_event: string, callback: () => void) => listeners.delete(callback)),
    dispatch(matches: boolean) {
      this.matches = matches;
      for (const listener of [...listeners]) listener();
    },
  };
}

describe('appearance runtime hooks', () => {
  let container: HTMLDivElement;
  let root: Root;
  let media: MediaHarness;
  let controller: AppearanceController | null;

  function Consumer() {
    controller = useAppearance();
    return null;
  }

  beforeEach(() => {
    localStorage.clear();
    document.documentElement.removeAttribute('data-appearance');
    document.documentElement.removeAttribute('style');
    mocks.setTheme.mockClear();
    mocks.emit.mockClear();
    mocks.invoke.mockReset();
    mocks.beforeListenResolve = null;
    mocks.listeners.length = 0;
    media = mediaHarness(false);
    Object.defineProperty(window, 'matchMedia', {
      configurable: true,
      value: vi.fn(() => media),
    });
    let clipboardText = '';
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: {
        writeText: vi.fn(async (value: string) => { clipboardText = value; }),
        readText: vi.fn(async () => clipboardText),
      },
    });
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    controller = null;
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.useRealTimers();
  });

  it('maps modes to native setTheme and emits revisioned writes', async () => {
    await act(async () => {
      root.render(<Provider><Consumer /></Provider>);
    });
    expect(mocks.setTheme).toHaveBeenCalledWith(null);
    expect(controller).not.toBeNull();

    await act(async () => {
      await controller!.setMode('dark');
    });
    expect(mocks.setTheme).toHaveBeenLastCalledWith('dark');
    expect(mocks.emit).toHaveBeenLastCalledWith(
      APPEARANCE_CHANGED_EVENT,
      { revision: 1, reason: 'user' },
    );
    expect(JSON.parse(localStorage.getItem(APPEARANCE_STORAGE_KEY)!)).toMatchObject({
      revision: 1,
      mode: 'dark',
    });
    expect(document.documentElement.dataset.appearance).toBe('dark');

    await act(async () => {
      await controller!.setMode('light');
    });
    expect(mocks.setTheme).toHaveBeenLastCalledWith('light');
  });

  it('applies system changes locally with zero emitted events', async () => {
    await act(async () => root.render(<Provider><Consumer /></Provider>));
    mocks.emit.mockClear();
    await act(async () => media.dispatch(true));
    expect(document.documentElement.dataset.appearance).toBe('dark');
    expect(controller!.resolvedAppearance).toBe('dark');
    expect(mocks.emit).not.toHaveBeenCalled();
    expect(JSON.parse(localStorage.getItem(APPEARANCE_STORAGE_KEY) ?? 'null')).toBeNull();
  });

  it('does not duplicate repair writes or media listeners in Strict Mode', async () => {
    const partial = createAppearanceDocument('system');
    delete (partial.cache.dark as Partial<typeof partial.cache.dark>).warning;
    localStorage.setItem(APPEARANCE_STORAGE_KEY, JSON.stringify(partial));
    await act(async () => {
      root.render(<StrictMode><Provider><Consumer /></Provider></StrictMode>);
    });
    expect(mocks.emit.mock.calls.filter((call) =>
      (call[1] as AppearanceChangedEvent)?.reason === 'repair',
    )).toHaveLength(1);
    expect(media.listeners.size).toBe(1);
    expect(media.addEventListener).toHaveBeenCalledTimes(2);
    expect(media.removeEventListener).toHaveBeenCalledTimes(1);
    const stored = JSON.parse(localStorage.getItem(APPEARANCE_STORAGE_KEY)!);
    expect(stored.revision).toBe(1);
    expect(Object.keys(stored.cache.dark)).toEqual([...MURMUR_TOKEN_NAMES]);
  });

  it('repairs the reserved maximum revision and keeps every write path monotonic', async () => {
    const high = createAppearanceDocument('system');
    high.revision = MAX_APPEARANCE_REVISION - 1;
    localStorage.setItem(APPEARANCE_STORAGE_KEY, JSON.stringify(high));
    await act(async () => root.render(<Provider><Consumer /></Provider>));
    expect(controller!.document.revision).toBe(1);

    await act(async () => {
      await controller!.reset();
    });
    expect(controller!.document.revision).toBe(2);

    await act(async () => {
      await controller!.setMode('dark');
    });
    expect(controller!.document.revision).toBe(3);

    await act(async () => {
      await controller!.updateTheme({ presetId: 'custom', accent: '#123456' });
    });
    expect(controller!.document.revision).toBe(4);

    const preview = controller!.previewImport(exportAppearanceText(
      createAppearanceDocument('light', {
        version: 1,
        presetId: 'custom',
        background: '#abcdef',
      }),
    ));
    await act(async () => {
      await controller!.commitImport(preview);
    });
    expect(controller!.document.revision).toBe(5);
    expect(mocks.emit).toHaveBeenLastCalledWith(
      APPEARANCE_CHANGED_EVENT,
      { revision: 5, reason: 'import' },
    );
    expect(mocks.emit.mock.calls.map((call) => call[1])).toEqual([
      { revision: 1, reason: 'repair' },
      { revision: 2, reason: 'reset' },
      { revision: 3, reason: 'user' },
      { revision: 4, reason: 'user' },
      { revision: 5, reason: 'import' },
    ]);
  });

  it('rolls an exhausted canonical revision and emits a repair event', async () => {
    localStorage.setItem(
      APPEARANCE_STORAGE_KEY,
      JSON.stringify(createAppearanceDocument(
        'system',
        undefined,
        MAX_APPEARANCE_REVISION - 2,
      )),
    );
    await act(async () => root.render(<Provider><Consumer /></Provider>));

    await act(async () => {
      await controller!.setMode('dark');
    });
    expect(controller!.document.revision).toBe(1);
    expect(controller!.document.mode).toBe('dark');
    expect(mocks.emit).toHaveBeenCalledWith(
      APPEARANCE_CHANGED_EVENT,
      { revision: 1, reason: 'repair' },
    );

    await act(async () => {
      await controller!.setMode('light');
    });
    expect(controller!.document.revision).toBe(2);
  });

  it('uses bounded command transport and preserves clipboard content', async () => {
    const fileDocument = createAppearanceDocument('dark', {
      version: 1,
      presetId: 'custom',
      accent: '#abcdef',
    });
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === 'read_theme_file') return exportAppearanceText(fileDocument);
      return undefined;
    });
    await act(async () => root.render(<Provider><Consumer /></Provider>));
    await navigator.clipboard.writeText('keep me');
    let preview;
    await act(async () => {
      preview = await controller!.importFromPath('/tmp/in.murmur-theme.json');
      await controller!.commitImport(preview!);
      await controller!.exportToPath('/tmp/out.murmur-theme.json');
    });
    expect(mocks.invoke).toHaveBeenCalledWith('read_theme_file', { path: '/tmp/in.murmur-theme.json' });
    expect(mocks.invoke).toHaveBeenCalledWith('write_theme_file', expect.objectContaining({
      path: '/tmp/out.murmur-theme.json',
    }));
    expect(await navigator.clipboard.readText()).toBe('keep me');
  });

  it('rejects a failed commit so callers retain import preview state', async () => {
    await act(async () => root.render(<Provider><Consumer /></Provider>));
    const preview = controller!.previewImport(exportAppearanceText(
      createAppearanceDocument('dark', {
        version: 1,
        presetId: 'custom',
        accent: '#abcdef',
      }),
    ));
    const setItem = vi.spyOn(window.localStorage, 'setItem')
      .mockImplementationOnce(() => { throw new Error('disk full'); });
    await act(async () => {
      await expect(controller!.commitImport(preview)).rejects.toThrow('disk full');
    });
    expect(controller!.document.mode).toBe('system');
    expect(preview.mode).toBe('dark');
    expect(controller!.error).toContain('disk full');
    setItem.mockRestore();
  });

  it('installs, selects, and detaches a saved theme when the user edits it', async () => {
    await act(async () => root.render(<Provider><Consumer /></Provider>));
    const entry = makeLocalThemeEntry('ocean', 'Ocean', {
      version: 1,
      presetId: 'custom',
      accent: '#12759a',
      background: '#102027',
      foreground: '#eef8fb',
    });
    await act(async () => {
      await controller!.library.install([entry]);
      await controller!.commitImport(controller!.library.previewSelection(entry.id));
    });
    expect(controller!.document.selection).toEqual({ light: 'ocean', dark: 'ocean' });
    expect(JSON.parse(localStorage.getItem(THEME_LIBRARY_STORAGE_KEY)!)).toMatchObject({
      revision: 1,
      themes: [{ id: 'ocean' }],
    });

    await act(async () => {
      await controller!.updateTheme({ accent: '#cc5500' });
    });
    expect(controller!.document.selection).toEqual({ light: 'custom', dark: 'custom' });
    expect(controller!.document.theme).toMatchObject({
      presetId: 'custom',
      accent: '#cc5500',
    });
    expect(controller!.document.theme.light).toBeUndefined();
    expect(controller!.document.theme.dark).toBeUndefined();
  });

  it('falls active collection variants back to Sonic when an update removes their IDs', async () => {
    await act(async () => root.render(<Provider><Consumer /></Provider>));
    const collection = { id: 'open-vsx:sample.aurora', label: 'Aurora' };
    const source = {
      kind: 'open-vsx' as const,
      extensionId: 'sample.aurora',
      version: '1.0.0',
      license: 'MIT',
    };
    const oldEntry = {
      ...makeLocalThemeEntry('aurora-old', 'Aurora', {
        version: 1,
        presetId: 'custom',
        accent: '#1680a8',
      }),
      source,
      collection,
    };
    const newEntry = {
      ...makeLocalThemeEntry('aurora-new', 'Aurora Next', {
        version: 1,
        presetId: 'custom',
        accent: '#7ec9e8',
      }),
      source: { ...source, version: '2.0.0' },
      collection,
    };
    await act(async () => {
      await controller!.library.install([oldEntry]);
      await controller!.commitImport(controller!.library.previewSelection(oldEntry.id));
      await controller!.library.replaceCollection(collection.id, [newEntry], [oldEntry]);
    });
    expect(controller!.library.document.themes.map((theme) => theme.id)).toEqual(['aurora-new']);
    expect(controller!.document.selection).toEqual({ light: 'sonic', dark: 'sonic' });
    expect(controller!.document.theme.presetId).toBe('sonic');
    expect(mocks.emit).toHaveBeenLastCalledWith(
      APPEARANCE_CHANGED_EVENT,
      expect.objectContaining({ reason: 'library' }),
    );
  });

  it('reconciles a stale active cache from the durable library during startup', async () => {
    const entry = makeLocalThemeEntry('current-theme', 'Current Theme', {
      version: 1,
      presetId: 'custom',
      accent: '#8cd7f5',
      background: '#111518',
      foreground: '#edf4f7',
    });
    localStorage.setItem(THEME_LIBRARY_STORAGE_KEY, JSON.stringify({
      version: 1,
      revision: 4,
      themes: [entry],
    }));
    localStorage.setItem(APPEARANCE_STORAGE_KEY, JSON.stringify(
      createAppearanceDocument('dark', {
        version: 1,
        presetId: 'custom',
        accent: '#ff0000',
      }, 7, { light: entry.id, dark: entry.id }),
    ));

    await act(async () => root.render(<Provider><Consumer /></Provider>));

    expect(controller!.document.selection).toEqual({
      light: entry.id,
      dark: entry.id,
    });
    expect(controller!.document.theme).toEqual(entry.theme);
    expect(mocks.emit).toHaveBeenCalledWith(
      APPEARANCE_CHANGED_EVENT,
      expect.objectContaining({ reason: 'repair' }),
    );
  });

});
