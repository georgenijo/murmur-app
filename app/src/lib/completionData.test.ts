import { beforeEach, expect, it, vi } from 'vitest';
const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke, isTauri: () => false }));
import { importCompletionV1 } from './completionData';
beforeEach(() => { localStorage.clear(); mocks.invoke.mockReset(); });
it.each([
  ['history', ['dictation-history']],
  ['statistics', ['dictation-stats']],
  ['reports', ['murmur-benchmark-report', 'murmur-benchmark-reports']],
] as const)('removes actual legacy keys after a durable %s clear before late import', async (destination, keys) => {
  const all = ['dictation-history', 'dictation-stats', 'murmur-benchmark-report', 'murmur-benchmark-reports'];
  for (const key of [...all, 'unrelated-setting']) localStorage.setItem(key, '{}');
  const snapshot = { history: [], statistics: {}, reports: [], v1Committed: false, removeKeys: [...keys], retryNotice: false };
  mocks.invoke.mockImplementation(async (command: string) => {
    if (command === 'import_completion_v1') return { ...snapshot, v1Committed: true, removeKeys: all };
    return snapshot;
  });
  await importCompletionV1();
  expect(mocks.invoke).toHaveBeenCalledWith('acknowledge_completion_keys', { destinations: [destination] });
  const source = mocks.invoke.mock.calls.find(([command]) => command === 'import_completion_v1')?.[1].source;
  expect(source[destination === 'statistics' ? 'statistics' : destination]).toBeNull();
  for (const key of all) expect(localStorage.getItem(key)).toBeNull();
  expect(localStorage.getItem('unrelated-setting')).toBe('{}');
});
