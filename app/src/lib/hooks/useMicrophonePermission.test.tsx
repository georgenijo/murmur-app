import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { expect, it, vi } from 'vitest';
const native = vi.hoisted(() => ({ check: vi.fn() }));
vi.mock('../dictation', () => ({ checkMicrophonePermissionStatus: native.check }));
import { useMicrophonePermission } from './useMicrophonePermission';

it('refreshes on focus, ignores stale results, and releases its listener on unmount', async () => {
  const pending: Array<(value: string) => void> = [];
  native.check.mockImplementation(() => new Promise((resolve) => pending.push(resolve)));
  const container = document.createElement('div');
  const root = createRoot(container);
  function Harness() { return <output>{useMicrophonePermission()}</output>; }
  await act(async () => root.render(<Harness />));
  expect(container.textContent).toBe('unknown');
  await act(async () => window.dispatchEvent(new Event('focus')));
  await act(async () => pending[1]('granted'));
  await act(async () => pending[0]('denied'));
  expect(container.textContent).toBe('granted');
  await act(async () => window.dispatchEvent(new Event('focus')));
  await act(async () => pending[2]('denied'));
  expect(container.textContent).toBe('denied');
  await act(async () => root.unmount());
  window.dispatchEvent(new Event('focus'));
  expect(native.check).toHaveBeenCalledTimes(3);
});
