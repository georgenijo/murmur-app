import { beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  isTauri: vi.fn(() => true),
}));

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => mocks.invoke(...args),
  isTauri: () => mocks.isTauri(),
}));

import {
  HISTORY_STORE,
  STATS_STORE,
  THEME_LIBRARY_STORE,
  clearDurableBlob,
  hydrateThemeLibraryFromDisk,
  hydrateUserDataFromDisk,
  mirrorDurableBlob,
  saveDurableBlob,
} from './durableUserData';

beforeEach(() => {
  localStorage.clear();
  mocks.invoke.mockReset();
  mocks.invoke.mockResolvedValue(undefined);
  mocks.isTauri.mockReturnValue(true);
});

describe('durable history and statistics hydration', () => {
  it('seeds both localStorage caches from their durable files', async () => {
    const history = '[{"id":"durable","text":"local only"}]';
    const stats = '{"totalWords":42}';
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === 'load_history_blob') return history;
      if (command === 'load_stats_blob') return stats;
      return undefined;
    });

    await hydrateUserDataFromDisk();

    expect(localStorage.getItem(HISTORY_STORE.storageKey)).toBe(history);
    expect(localStorage.getItem(STATS_STORE.storageKey)).toBe(stats);
  });

  it('lets disk overwrite stale localStorage caches', async () => {
    localStorage.setItem(HISTORY_STORE.storageKey, '[{"id":"stale"}]');
    localStorage.setItem(STATS_STORE.storageKey, '{"totalWords":1}');
    mocks.invoke.mockImplementation(async (command: string) => (
      command === 'load_history_blob'
        ? '[{"id":"disk"}]'
        : command === 'load_stats_blob'
          ? '{"totalWords":2}'
          : undefined
    ));

    await hydrateUserDataFromDisk();

    expect(localStorage.getItem(HISTORY_STORE.storageKey)).toBe('[{"id":"disk"}]');
    expect(localStorage.getItem(STATS_STORE.storageKey)).toBe('{"totalWords":2}');
  });

  it('migrates legacy localStorage blobs when durable files are absent', async () => {
    const history = '[{"id":"legacy"}]';
    const stats = '{"totalWords":7}';
    localStorage.setItem(HISTORY_STORE.storageKey, history);
    localStorage.setItem(STATS_STORE.storageKey, stats);
    mocks.invoke.mockImplementation(async (command: string) => (
      command.startsWith('load_') ? null : undefined
    ));

    await hydrateUserDataFromDisk();

    expect(mocks.invoke).toHaveBeenCalledWith('save_history_blob', { blob: history });
    expect(mocks.invoke).toHaveBeenCalledWith('save_stats_blob', { blob: stats });
  });

  it('does not create empty durable files on a fresh install', async () => {
    mocks.invoke.mockResolvedValue(null);

    await hydrateUserDataFromDisk();

    expect(mocks.invoke).toHaveBeenCalledTimes(2);
    expect(mocks.invoke).not.toHaveBeenCalledWith('save_history_blob', expect.anything());
    expect(mocks.invoke).not.toHaveBeenCalledWith('save_stats_blob', expect.anything());
  });

  it('isolates a failed store and still hydrates the other', async () => {
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === 'load_history_blob') throw new Error('history unavailable');
      if (command === 'load_stats_blob') return '{"totalWords":9}';
      return undefined;
    });

    await expect(hydrateUserDataFromDisk()).resolves.toBeUndefined();

    expect(localStorage.getItem(HISTORY_STORE.storageKey)).toBeNull();
    expect(localStorage.getItem(STATS_STORE.storageKey)).toBe('{"totalWords":9}');
  });

  it('does not call the host outside Tauri', async () => {
    mocks.isTauri.mockReturnValue(false);
    localStorage.setItem(HISTORY_STORE.storageKey, '[{"id":"browser"}]');

    await hydrateUserDataFromDisk();
    mirrorDurableBlob(HISTORY_STORE, '[]');
    clearDurableBlob(STATS_STORE);

    expect(mocks.invoke).not.toHaveBeenCalled();
    expect(localStorage.getItem(HISTORY_STORE.storageKey)).toBe('[{"id":"browser"}]');
  });

  it('hydrates the main-window theme library through its independent blob commands', async () => {
    const library = '{"version":1,"revision":3,"themes":[]}';
    mocks.invoke.mockImplementation(async (command: string) => (
      command === 'load_theme_library_blob' ? library : undefined
    ));

    await hydrateThemeLibraryFromDisk();

    expect(localStorage.getItem(THEME_LIBRARY_STORE.storageKey)).toBe(library);
    expect(mocks.invoke).toHaveBeenCalledWith('load_theme_library_blob');
  });
});

describe('durable write-through operations', () => {
  it('mirrors the exact serialized blob', () => {
    const blob = '[{"id":"one"}]';

    mirrorDurableBlob(HISTORY_STORE, blob);

    expect(mocks.invoke).toHaveBeenCalledWith('save_history_blob', { blob });
  });

  it('still writes durably when the synchronous cache is unavailable', () => {
    const error = vi.spyOn(localStorage, 'setItem').mockImplementationOnce(() => {
      throw new Error('quota exceeded');
    });

    saveDurableBlob(STATS_STORE, '{"totalWords":10}');

    expect(mocks.invoke).toHaveBeenCalledWith('save_stats_blob', { blob: '{"totalWords":10}' });
    error.mockRestore();
  });

  it('clears the cache synchronously and the matching durable file', () => {
    localStorage.setItem(HISTORY_STORE.storageKey, '[{"id":"one"}]');

    clearDurableBlob(HISTORY_STORE);

    expect(localStorage.getItem(HISTORY_STORE.storageKey)).toBeNull();
    expect(mocks.invoke).toHaveBeenCalledWith('clear_history_blob');
  });
});

describe('statistics persistence ordering and restart', () => {
  it('serializes counter writes and reset, then hydrates the latest completed state', async () => {
    let disk: string | null = null;
    let finishFirst: () => void = () => {};
    const firstWrite = new Promise<void>((resolve) => { finishFirst = resolve; });
    let saves = 0;
    mocks.invoke.mockImplementation(async (command: string, args?: { blob: string }) => {
      if (command === 'save_stats_blob') {
        saves++;
        if (saves === 1) await firstWrite;
        disk = args?.blob ?? null;
      }
      if (command === 'clear_stats_blob') disk = null;
      if (command === 'load_stats_blob') return disk;
      return null;
    });
    const first = '{"activity":{"pasteLastUses":1}}';
    const second = '{"activity":{"pasteLastUses":2}}';
    const afterReset = '{"activity":{"pasteLastUses":1,"corrections":{"taught":3}}}';
    saveDurableBlob(STATS_STORE, first);
    saveDurableBlob(STATS_STORE, second);
    clearDurableBlob(STATS_STORE);
    saveDurableBlob(STATS_STORE, afterReset);
    expect(saves).toBe(1);
    expect(localStorage.getItem(STATS_STORE.storageKey)).toBe(afterReset);
    finishFirst();
    await vi.waitFor(() => expect(disk).toBe(afterReset));
    expect(mocks.invoke.mock.calls.filter(([command]) => command.endsWith('stats_blob')).map(([command]) => command)).toEqual([
      'save_stats_blob', 'save_stats_blob', 'clear_stats_blob', 'save_stats_blob',
    ]);
    localStorage.clear();
    await hydrateUserDataFromDisk();
    expect(localStorage.getItem(STATS_STORE.storageKey)).toBe(afterReset);
  });

  it('continues after a failed stats write without logging private error text', async () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {});
    mocks.invoke.mockRejectedValueOnce(new Error('PRIVATE_PRESET_LABEL'));
    saveDurableBlob(STATS_STORE, '{"activity":{"pasteLastUses":1}}');
    saveDurableBlob(STATS_STORE, '{"activity":{"pasteLastUses":2}}');
    await vi.waitFor(() => expect(mocks.invoke).toHaveBeenCalledTimes(2));
    expect(JSON.stringify(error.mock.calls)).not.toContain('PRIVATE_PRESET_LABEL');
    error.mockRestore();
  });
});
