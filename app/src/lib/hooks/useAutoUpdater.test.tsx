import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { beforeEach, expect, it, vi } from 'vitest';
import { useAutoUpdater } from './useAutoUpdater';
import type { UpdaterSnapshot } from '../updater';
const mocks = vi.hoisted(() => ({ invoke: vi.fn(), listeners: new Map<string, (event: { payload: UpdaterSnapshot }) => void>() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
vi.mock('../targetedEvents', () => ({ listen: vi.fn(async (name, cb) => { mocks.listeners.set(name, cb); return () => mocks.listeners.delete(name); }) }));
let view: ReturnType<typeof useAutoUpdater>;
let state: UpdaterSnapshot;
beforeEach(() => {
  mocks.invoke.mockReset(); mocks.listeners.clear();
  state = { revision: 0, updateStatus: { phase: 'idle' }, completedUpdate: null, isUpdateDialogOpen: false };
  mocks.invoke.mockImplementation(async () => state);
});
async function mount(setupComplete = true) {
  const container = document.createElement('div');
  const root = createRoot(container);
  function Harness() { view = useAutoUpdater({ automaticChecksEnabled: true, setupComplete }); return null; }
  await act(async () => root.render(<Harness />));
  return async () => { await act(async () => root.unmount()); };
}
it('forwards setup gating to Rust and keeps manual checks available', async () => {
  const cleanup = await mount(false);
  expect(mocks.invoke).toHaveBeenCalledWith('updater_action', { action: { kind: 'configure', automatic: false } });
  await act(async () => view.checkForUpdate());
  expect(mocks.invoke).toHaveBeenCalledWith('updater_action', { action: { kind: 'check', background: false } });
  expect(mocks.listeners.has('updater-background-check-requested')).toBe(false);
  await cleanup();
});
it('renders native state, rejects stale snapshots, forwards actions without plugin calls', async () => {
  const cleanup = await mount();
  state = { revision: 2, updateStatus: { phase: 'ready', version: '1.2.3', isForced: true }, completedUpdate: null, isUpdateDialogOpen: true };
  await act(async () => mocks.listeners.get('updater-state-changed')!({ payload: state }));
  await act(async () => mocks.listeners.get('updater-state-changed')!({ payload: { ...state, revision: 1, updateStatus: { phase: 'idle' } } }));
  expect(view.updateStatus.phase).toBe('ready');
  await act(async () => { await view.startDownload(); await view.restartUpdate(); view.dismissUpdate(); view.dismissCompletedUpdate(); });
  for (const kind of ['download', 'install', 'dismiss', 'dismissCompleted']) expect(mocks.invoke).toHaveBeenCalledWith('updater_action', { action: { kind } });
  expect(localStorage.getItem('pending-update-release-notes')).toBeNull();
  await cleanup();
});
it('forwards lifecycle intent; scheduling and due checks remain in Rust', async () => {
  const cleanup = await mount();
  await act(async () => { window.dispatchEvent(new Event('focus')); });
  expect(mocks.invoke).toHaveBeenCalledWith('updater_action', { action: { kind: 'check', background: true } });
  await cleanup();
});
