import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { beforeEach, expect, it, vi } from 'vitest';
import { useAutoUpdater } from './useAutoUpdater';
import type { UpdaterSnapshot } from '../updater';
const mocks = vi.hoisted(() => ({ invoke: vi.fn(), listeners: new Map<string, (event: { payload: UpdaterSnapshot }) => void>() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
vi.mock('../targetedEvents', () => ({ listen: vi.fn(async (name, cb) => { mocks.listeners.set(name, cb); return () => { if (mocks.listeners.get(name) === cb) mocks.listeners.delete(name); }; }) }));
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
it.each(['focus', 'visibility', 'wake'])('forwards %s failures as quiet background checks', async (trigger) => {
  const cleanup = await mount();
  mocks.invoke.mockClear();
  state = { revision: 3, updateStatus: { phase: 'error', stage: 'check', message: 'Offline', isForced: false }, completedUpdate: null, isUpdateDialogOpen: false };
  mocks.invoke.mockImplementation(async command => {
    if (command === 'updater_action') throw new Error('Offline');
    return state;
  });
  await act(async () => {
    if (trigger === 'focus') window.dispatchEvent(new Event('focus'));
    else if (trigger === 'visibility') document.dispatchEvent(new Event('visibilitychange'));
    else mocks.listeners.get('updater-background-check-requested')!({ payload: state });
  });
  expect(mocks.invoke).toHaveBeenCalledWith('updater_action', { action: { kind: 'check', background: true } });
  expect(view.updateStatus.phase).toBe('error');
  expect(view.isUpdateDialogOpen).toBe(false);
  await cleanup();
});
it('does not forward hidden visibility and removes lifecycle listeners during setup and unmount', async () => {
  const root = createRoot(document.createElement('div'));
  function Harness({ ready }: { ready: boolean }) { view = useAutoUpdater({ automaticChecksEnabled: true, setupComplete: ready }); return null; }
  await act(async () => root.render(<Harness ready />));
  const hidden = vi.spyOn(document, 'hidden', 'get').mockReturnValue(true);
  mocks.invoke.mockClear();
  await act(async () => document.dispatchEvent(new Event('visibilitychange')));
  expect(mocks.invoke).not.toHaveBeenCalled();
  hidden.mockRestore();
  await act(async () => root.render(<Harness ready={false} />));
  expect(mocks.listeners.has('updater-background-check-requested')).toBe(false);
  mocks.invoke.mockClear();
  await act(async () => {
    window.dispatchEvent(new Event('focus'));
    document.dispatchEvent(new Event('visibilitychange'));
  });
  expect(mocks.invoke).not.toHaveBeenCalled();
  await act(async () => view.checkForUpdate());
  expect(mocks.invoke).toHaveBeenCalledWith('updater_action', { action: { kind: 'check', background: false } });
  await act(async () => root.unmount());
  expect(mocks.listeners.size).toBe(0);
});
it('keeps Ready and completed notes through setup reconfiguration and forwards explicit consumption', async () => {
  const root = createRoot(document.createElement('div'));
  function Harness({ ready }: { ready: boolean }) { view = useAutoUpdater({ automaticChecksEnabled: true, setupComplete: ready }); return null; }
  state = { revision: 3, updateStatus: { phase: 'ready', version: '1.2.3', isForced: false }, completedUpdate: null, isUpdateDialogOpen: true };
  await act(async () => root.render(<Harness ready />));
  await act(async () => root.render(<Harness ready={false} />));
  expect(view.updateStatus.phase).toBe('ready');
  state = { ...state, revision: 4, completedUpdate: { version: '1.2.3', notes: 'Missed releases' } };
  await act(async () => mocks.listeners.get('updater-state-changed')!({ payload: state }));
  expect(view.completedUpdate?.notes).toBe('Missed releases');
  await act(async () => view.dismissCompletedUpdate());
  expect(mocks.invoke).toHaveBeenCalledWith('updater_action', { action: { kind: 'dismissCompleted' } });
  await act(async () => root.unmount());
});
it('ignores legacy skipped-version storage and leaves native policy authoritative', async () => {
  localStorage.setItem('skipped-update-version', '1.2.3');
  state = { revision: 3, updateStatus: { phase: 'available', version: '1.2.3', notes: 'Notes', isForced: false }, completedUpdate: null, isUpdateDialogOpen: true };
  const cleanup = await mount();
  expect(view.updateStatus.phase).toBe('available');
  expect(view.isUpdateDialogOpen).toBe(true);
  await cleanup();
  localStorage.removeItem('skipped-update-version');
});
