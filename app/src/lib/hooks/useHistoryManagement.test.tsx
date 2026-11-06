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
  await act(async () => state.updateEntry(target, 'edited synthetic'));
  expect(mocks.invoke).toHaveBeenCalledWith('mutate_history', { mutation: { kind: 'edit', target, text: 'edited synthetic' } });
  await act(async () => state.deleteEntries([target]));
  expect(mocks.invoke).toHaveBeenCalledWith('mutate_history', { mutation: { kind: 'delete', targets: [target] } });
  await act(async () => state.clearHistory());
  expect(mocks.invoke).toHaveBeenCalledWith('clear_completion_destination', { destination: 'history' });
  expect(localStorage.getItem('dictation-history')).toBeNull();
  await act(async () => root.unmount());
});

it('edits the selected synthetic legacy row when timestamp IDs collide', async () => {
  const first = { schemaVersion: 2 as const, id: 'duplicate', text: 'first synthetic', timestamp: 1, duration: 1 };
  const selected = { ...first, text: 'second synthetic' };
  const snapshot = { history: [first, selected], statistics: loadStats(), reports: [], v1Committed: true, removeKeys: [], retryNotice: false };
  mocks.invoke.mockResolvedValue(snapshot);
  await refreshCompletion();
  let state!: ReturnType<typeof useHistoryManagement>;
  function Harness() { state = useHistoryManagement(); return null; }
  const root = createRoot(document.createElement('div'));
  await act(async () => root.render(<Harness />));
  await act(async () => state.updateEntry(state.historyEntries[1], 'edited synthetic'));
  expect(mocks.invoke).toHaveBeenCalledWith('mutate_history', { mutation: { kind: 'edit', target: selected, text: 'edited synthetic' } });
  await act(async () => root.unmount());
});
