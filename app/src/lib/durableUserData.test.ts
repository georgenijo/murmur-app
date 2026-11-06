import { beforeEach, expect, it, vi } from 'vitest';
const mocks = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn(async () => () => {}) }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke, isTauri: () => true }));
vi.mock('@tauri-apps/api/event', () => ({ listen: mocks.listen }));
import { HISTORY_STORE, STATS_STORE, THEME_LIBRARY_STORE, saveDurableBlob, clearDurableBlob, hydrateThemeLibraryFromDisk } from './durableUserData';
import { importCompletionV1, completionMutation } from './completionData';
import { loadStats } from './stats';
beforeEach(() => { localStorage.clear(); mocks.invoke.mockReset(); });
const snapshot = (committed: boolean, keys: string[] = []) => ({ history: [], statistics: loadStats(), reports: [], v1Committed: committed, removeKeys: keys, retryNotice: false });
it('prevents the generic blob writer from caching or replacing core-owned content', () => {
  for (const store of [HISTORY_STORE, STATS_STORE]) {
    expect(() => saveDurableBlob(store, '[]')).toThrow('Rust-owned');
    expect(() => clearDurableBlob(store)).toThrow('Rust-owned');
    expect(localStorage.getItem(store.storageKey)).toBeNull();
  }
});
it('imports WebKit only through v1 and removes targeted keys after their durable receipt', async () => {
  localStorage.setItem('dictation-history', '[]'); localStorage.setItem('dictation-stats', '{}');
  localStorage.setItem('murmur-benchmark-report', '{"synthetic":true}'); localStorage.setItem('unrelated', 'keep');
  let committed = false;
  mocks.invoke.mockImplementation(async (command) => {
    if (command === 'import_completion_v1') committed = true;
    return snapshot(committed, committed ? ['dictation-history', 'dictation-stats', 'murmur-benchmark-report', 'murmur-benchmark-reports'] : []);
  });
  await importCompletionV1();
  expect(mocks.invoke).toHaveBeenCalledWith('import_completion_v1', { source: { history: [], statistics: {}, reports: [{ synthetic: true }] } });
  expect(localStorage.getItem('dictation-history')).toBeNull(); expect(localStorage.getItem('dictation-stats')).toBeNull(); expect(localStorage.getItem('murmur-benchmark-report')).toBeNull(); expect(localStorage.getItem('unrelated')).toBe('keep');
  mocks.invoke.mockClear(); await importCompletionV1(); expect(mocks.invoke.mock.calls.some(([name]) => name === 'import_completion_v1')).toBe(false);
});
it('removes keys for a partially committed destination even when another import fails', async () => {
  localStorage.setItem('dictation-history', '[]'); localStorage.setItem('dictation-stats', '{}');
  let partial = false;
  mocks.invoke.mockImplementation(async command => {
    if (command === 'import_completion_v1') { partial = true; throw new Error('unavailable'); }
    return snapshot(false, partial ? ['dictation-history'] : []);
  });
  await expect(importCompletionV1()).rejects.toThrow();
  expect(localStorage.getItem('dictation-history')).toBeNull(); expect(localStorage.getItem('dictation-stats')).toBe('{}');
});
it('clear removes legacy keys from its durable destination receipt', async () => {
  localStorage.setItem('dictation-history', '[]');
  mocks.invoke.mockResolvedValue(snapshot(false, ['dictation-history']));
  await completionMutation('clear_completion_destination', { destination: 'history' });
  expect(localStorage.getItem('dictation-history')).toBeNull();
});
it('retains theme-library hydration through the generic blob store', async () => {
  mocks.invoke.mockResolvedValue('{"library":[]}'); await hydrateThemeLibraryFromDisk();
  expect(localStorage.getItem(THEME_LIBRARY_STORE.storageKey)).toBe('{"library":[]}');
});
it('ignores a delayed older IPC snapshot after a newer command has published', async () => {
  const { acceptCompletionSnapshot, completionSnapshot } = await import('./completionData');
  acceptCompletionSnapshot({ ...snapshot(true), revision: 20, history: [{ id: 'new', text: 'synthetic', duration: 1, timestamp: 1 }] });
  acceptCompletionSnapshot({ ...snapshot(true), revision: 19 });
  expect(completionSnapshot()?.history[0].id).toBe('new');
});
