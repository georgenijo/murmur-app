import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, expect, it, vi } from 'vitest';
const mocks = vi.hoisted(() => ({ invoke: vi.fn(async (_command: string, _args: unknown) => {}), listen: vi.fn(), stop: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
vi.mock('../targetedEvents', () => ({ listen: mocks.listen }));
import { useAudioLevel } from './useAudioLevel';
const container = document.createElement('div');
let root = createRoot(container);
afterEach(async () => {
  await act(async () => root.unmount());
  root = createRoot(container);
  vi.clearAllMocks();
});
function Harness({ consumer, active, accept }: { consumer: 'main-recording' | 'corpus' | 'overlay'; active: boolean; accept: (n: number) => void }) {
  useAudioLevel(consumer, active, accept);
  return null;
}
it.each(['main-recording', 'corpus', 'overlay'] as const)('registers %s after listening, stops off-surface, and resumes without restarting capture', async consumer => {
  mocks.listen.mockResolvedValue(mocks.stop);
  const accept = vi.fn();
  await act(async () => root.render(<Harness consumer={consumer} active={false} accept={accept} />));
  expect(mocks.listen).not.toHaveBeenCalled();
  await act(async () => root.render(<Harness consumer={consumer} active accept={accept} />));
  expect(mocks.invoke).toHaveBeenLastCalledWith('set_waveform_subscription', { consumer, active: true });
  expect(mocks.listen.mock.invocationCallOrder[0]).toBeLessThan(mocks.invoke.mock.invocationCallOrder[0]);
  const receive = mocks.listen.mock.calls[0][1];
  receive({ payload: 0.25 });
  expect(accept).toHaveBeenCalledWith(0.25);
  await act(async () => root.render(<Harness consumer={consumer} active={false} accept={accept} />));
  expect(mocks.stop).toHaveBeenCalledTimes(1);
  expect(mocks.invoke).toHaveBeenLastCalledWith('set_waveform_subscription', { consumer, active: false });
  receive({ payload: 0.5 });
  expect(accept).toHaveBeenCalledTimes(1);
  await act(async () => root.render(<Harness consumer={consumer} active accept={accept} />));
  expect(mocks.invoke).toHaveBeenLastCalledWith('set_waveform_subscription', { consumer, active: true });
});
it('does not activate a consumer when listen finishes after unmount', async () => {
  let resolve!: (stop: () => void) => void;
  mocks.listen.mockReturnValue(new Promise(r => { resolve = r; }));
  await act(async () => root.render(<Harness consumer="overlay" active accept={() => {}} />));
  await act(async () => root.unmount());
  root = createRoot(container);
  await act(async () => resolve(mocks.stop));
  expect(mocks.stop).toHaveBeenCalled();
  expect(mocks.invoke.mock.calls.every(([, args]) => (args as { active: boolean }).active === false)).toBe(true);
});
