import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { expect, it, vi } from 'vitest';
const mocks = vi.hoisted(() => ({ handler: null as null | ((event: { payload: unknown }) => void), unlisten: vi.fn(), invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke, isTauri: () => true }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async (_name, handler) => { mocks.handler = handler; return mocks.unlisten; }) }));
import { useOverlaySettingsSync } from './useOverlaySettingsSync';
it('applies only validated committed fields and ignores events after teardown', async () => {
  const apply = vi.fn();
  const host = document.createElement('div');
  const root = createRoot(host);
  function Harness() { useOverlaySettingsSync(apply); return null; }
  await act(async () => root.render(<Harness />));
  for (const payload of [null, { field: 'overlayLiveWords', value: true }, { field: 'autoPaste', value: 'true' }]) {
    mocks.handler!({ payload });
  }
  expect(apply).not.toHaveBeenCalled();
  mocks.handler!({ payload: { field: 'disabled', value: true } });
  expect(apply).toHaveBeenCalledWith({ disabled: true });
  mocks.handler!({ payload: { field: 'autoPaste', value: false } });
  expect(apply).toHaveBeenLastCalledWith({ autoPaste: false });
  await act(async () => root.unmount());
  mocks.handler!({ payload: { field: 'autoPaste', value: true } });
  expect(apply).toHaveBeenCalledTimes(2);
  expect(mocks.unlisten).toHaveBeenCalled();
});

it('a later revision repairs a missed Disable event before retrying the full save', async () => {
  const { persistSourceState, flushSourceState, rebasePendingOwnedSettings } = await import('../webkitLifecycle');
  let current = { autoPaste: false, disabled: false, language: 'es', ownedFieldsRevision: 0 };
  const host = document.createElement('div'); const root = createRoot(host);
  function Harness() {
    useOverlaySettingsSync(patch => { current = { ...current, ...patch }; rebasePendingOwnedSettings(current); });
    return null;
  }
  await act(async () => root.render(<Harness />));
  mocks.invoke.mockRejectedValue(new Error('unavailable'));
  persistSourceState('dictation-settings', JSON.stringify(current));
  await expect(flushSourceState()).rejects.toThrow();
  // Disable revision 1 was missed before subscription; revision 2 carries both.
  mocks.handler!({ payload: { field: 'autoPaste', value: true, ownedFieldsRevision: 2, autoPaste: true, disabled: true } });
  mocks.invoke.mockResolvedValue(undefined);
  await flushSourceState();
  const calls = mocks.invoke.mock.calls.filter(([command]) => command === 'save_settings_blob');
  expect(JSON.parse(calls[calls.length - 1][1].blob)).toMatchObject({ disabled: true, autoPaste: true, ownedFieldsRevision: 2, language: 'es' });
  await act(async () => root.unmount());
});

it('preserves the origin generation in main-save acknowledgements', async () => {
  const apply = vi.fn(); const host = document.createElement('div'); const root = createRoot(host);
  function Harness() { useOverlaySettingsSync(apply); return null; }
  await act(async () => root.render(<Harness />));
  mocks.handler!({ payload: { field: 'autoPaste', value: true, autoPaste: true, disabled: false, ownedFieldsRevision: 2, intentGeneration: 17 } });
  expect(apply).toHaveBeenCalledWith({ autoPaste: true, disabled: false, ownedFieldsRevision: 2, intentGeneration: 17 });
  mocks.handler!({ payload: { field: 'autoPaste', value: true, autoPaste: true, disabled: false, ownedFieldsRevision: 2, intentGeneration: -1 } });
  expect(apply).toHaveBeenCalledTimes(1);
  await act(async () => root.unmount());
});
