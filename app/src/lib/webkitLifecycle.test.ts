import { beforeEach, expect, it, vi } from 'vitest';
const mock = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mock.invoke, isTauri: () => true }));
import { initializeWebkitState, V2_KEYS } from './webkitLifecycle';
beforeEach(() => { localStorage.clear(); mock.invoke.mockReset(); });
it('sends unmigrated raw strings and hydrates only after the Rust commit', async () => {
  localStorage.setItem('murmur-appearance', '{bad');
  mock.invoke.mockImplementation(async (command: string, args?: { source: Record<string, unknown> }) => {
    if(command === 'webkit_state_snapshot') return { v1: false, v2: false, values: {} };
    expect(args?.source['murmur-appearance']).toBe('{bad');
    V2_KEYS.forEach(key => expect(Object.prototype.hasOwnProperty.call(args!.source, key)).toBe(true));
    throw new Error('preserved');
  });
  await expect(initializeWebkitState(true)).rejects.toThrow('preserved');
  expect(localStorage.getItem('murmur-appearance')).toBe('{bad');
});
it('already imported state never sends a second source snapshot and disk wins', async () => {
  localStorage.setItem('usage-dashboard-collapsed', 'true');
  mock.invoke.mockResolvedValue({ v1: true, v2: true, values: { 'usage-dashboard-collapsed': false } });
  await initializeWebkitState(true);
  expect(mock.invoke.mock.calls.filter(([command]) => command === 'import_webkit_state')).toHaveLength(0);
  expect(localStorage.getItem('usage-dashboard-collapsed')).toBe('false');
});
it('removes durably imported transcript keys independently of a corrupt v2 import', async () => {
  const keys = ['dictation-history', 'dictation-stats', 'murmur-benchmark-report', 'murmur-benchmark-reports'];
  keys.forEach(key => localStorage.setItem(key, '[]'));
  mock.invoke.mockImplementation(async (command: string) => {
    if(command === 'webkit_state_snapshot') return { v1: true, v2: false, values: {} };
    if(command === 'import_webkit_state') throw new Error('preserved');
    if(command === 'completion_snapshot') return { revision: 999, v1Committed: true, history: [], reports: [], statistics: {}, removeKeys: keys };
  });
  await expect(initializeWebkitState(true)).rejects.toThrow('preserved');
  keys.forEach(key => expect(localStorage.getItem(key)).toBeNull());
  expect(mock.invoke).toHaveBeenCalledWith('acknowledge_completion_keys', { destinations: ['history','statistics','reports'] });
});
it('retry rebases retained settings on a newer native owned-field commit', async () => {
  const { persistSourceState, flushSourceState, rebasePendingOwnedSettings } = await import('./webkitLifecycle');
  mock.invoke.mockRejectedValueOnce(new Error('disk unavailable'));
  persistSourceState('dictation-settings', JSON.stringify({ autoPaste: false, disabled: false, language: 'es' }));
  await Promise.resolve();
  await expect(flushSourceState()).resolves.toBeUndefined();
  // Add a failed retained intent explicitly before the external commit.
  mock.invoke.mockRejectedValue(new Error('disk unavailable'));
  persistSourceState('dictation-settings', JSON.stringify({ autoPaste: false, disabled: false, language: 'es' }));
  await expect(flushSourceState()).rejects.toThrow();
  rebasePendingOwnedSettings({ autoPaste: true, disabled: true, ownedFieldsRevision: 1 });
  mock.invoke.mockResolvedValue(undefined);
  await flushSourceState();
  const args = mock.invoke.mock.calls.filter(([command]) => command === 'save_settings_blob').slice(-1)[0]?.[1];
  expect(JSON.parse(args.blob)).toMatchObject({ autoPaste: true, disabled: true, ownedFieldsRevision: 1, language: 'es' });
});
it('snapshot failure still removes already-receipted transcript data', async () => {
  localStorage.setItem('dictation-history', '[]');
  mock.invoke.mockImplementation(async (command: string) => {
    if(command === 'webkit_state_snapshot') throw new Error('future schema');
    if(command === 'completion_snapshot') return { revision: 1000, v1Committed: true, history: [], reports: [], statistics: {}, removeKeys: ['dictation-history'] };
  });
  await expect(initializeWebkitState(true)).rejects.toThrow('future schema');
  expect(localStorage.getItem('dictation-history')).toBeNull();
});
