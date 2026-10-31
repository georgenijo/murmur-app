import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { expect, it, vi } from 'vitest';
const mocks = vi.hoisted(() => ({ handler: null as null | ((event: { payload: unknown }) => void), unlisten: vi.fn() }));
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
