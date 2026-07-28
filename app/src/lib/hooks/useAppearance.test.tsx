import { StrictMode } from 'react';
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  APPEARANCE_STORAGE_KEY,
  MAX_APPEARANCE_REVISION,
  MURMUR_TOKEN_NAMES,
  createAppearanceDocument,
  exportAppearanceText,
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
  useAppearanceReader,
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

  it('reconciles a write that lands between reader snapshot and listener registration', async () => {
    localStorage.setItem(
      APPEARANCE_STORAGE_KEY,
      JSON.stringify(createAppearanceDocument('light', undefined, 1)),
    );
    mocks.beforeListenResolve = () => {
      mocks.beforeListenResolve = null;
      localStorage.setItem(
        APPEARANCE_STORAGE_KEY,
        JSON.stringify(createAppearanceDocument('dark', undefined, 2)),
      );
    };
    function Reader() {
      useAppearanceReader();
      return null;
    }
    await act(async () => root.render(<Reader />));
    expect(document.documentElement.dataset.appearance).toBe('dark');
  });

  it('reader rejects stale revisions, retries visibility lag, and cleans async listeners', async () => {
    vi.useFakeTimers();
    localStorage.setItem(
      APPEARANCE_STORAGE_KEY,
      JSON.stringify(createAppearanceDocument('light', undefined, 1)),
    );
    function Reader() {
      useAppearanceReader();
      return null;
    }
    await act(async () => {
      root.render(<StrictMode><Reader /></StrictMode>);
    });
    const active = mocks.listeners.filter((listener) =>
      listener.event === APPEARANCE_CHANGED_EVENT && listener.active
    );
    expect(active).toHaveLength(1);

    active[0].callback({ payload: { revision: 1, reason: 'user' } });
    expect(document.documentElement.dataset.appearance).toBe('light');

    active[0].callback({ payload: { revision: 3, reason: 'user' } });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(12);
      localStorage.setItem(
        APPEARANCE_STORAGE_KEY,
        JSON.stringify(createAppearanceDocument('dark', undefined, 3)),
      );
      await vi.advanceTimersByTimeAsync(30);
    });
    expect(document.documentElement.dataset.appearance).toBe('dark');
  });

  it('reader accepts an explicit revision rollover instead of treating it as stale', async () => {
    localStorage.setItem(
      APPEARANCE_STORAGE_KEY,
      JSON.stringify(createAppearanceDocument(
        'light',
        undefined,
        MAX_APPEARANCE_REVISION - 2,
      )),
    );
    function Reader() {
      useAppearanceReader();
      return null;
    }
    await act(async () => root.render(<Reader />));
    const active = mocks.listeners.find((listener) =>
      listener.event === APPEARANCE_CHANGED_EVENT && listener.active
    )!;

    localStorage.setItem(
      APPEARANCE_STORAGE_KEY,
      JSON.stringify(createAppearanceDocument('dark', undefined, 1)),
    );
    await act(async () => {
      active.callback({ payload: { revision: 1, reason: 'repair' } });
    });
    expect(document.documentElement.dataset.appearance).toBe('dark');
  });
});
