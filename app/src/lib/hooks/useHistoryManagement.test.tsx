import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { expect, it, vi } from 'vitest';
import { useHistoryManagement } from './useHistoryManagement';
import { refreshCompletion } from '../completionData';
import { loadStats } from '../stats';
const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke, isTauri: () => false }));
it('reads snapshots and sends exact-row mutations without any localStorage writer', async () => {
  const target = { schemaVersion: 2 as const, id: 'a', text: 'synthetic', timestamp: 1, duration: 1 };
  const snapshot = { history: [target], statistics: loadStats(), reports: [], v1Committed: true, removeKeys: [], retryNotice: false };
  mocks.invoke.mockResolvedValue(snapshot);
  await refreshCompletion();
  let state!: ReturnType<typeof useHistoryManagement>;
  function Harness() { state = useHistoryManagement(false); return null; }
  const root = createRoot(document.createElement('div'));
  await act(async () => root.render(<Harness />));
  expect(state.historyEntries).toEqual([target]);
  await act(async () => state.togglePinned(target));
  expect(mocks.invoke).toHaveBeenCalledWith('mutate_history', { mutation: { kind: 'pin', target } });
  await act(async () => state.updateEntry('a', 'edited synthetic'));
  expect(mocks.invoke).toHaveBeenCalledWith('mutate_history', { mutation: { kind: 'edit', target, text: 'edited synthetic' } });
  await act(async () => state.deleteEntries([target]));
  expect(mocks.invoke).toHaveBeenCalledWith('mutate_history', { mutation: { kind: 'delete', targets: [target] } });
  await act(async () => state.clearHistory());
  expect(mocks.invoke).toHaveBeenCalledWith('clear_completion_destination', { destination: 'history' });
  expect(localStorage.getItem('dictation-history')).toBeNull();
  await act(async () => root.unmount());
});
