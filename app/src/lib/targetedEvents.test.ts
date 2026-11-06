import { mockWindows } from '@tauri-apps/api/mocks';
import { beforeEach, expect, it, vi } from 'vitest';
const mocks = vi.hoisted(() => ({ listen: vi.fn(async () => () => {}), emitTo: vi.fn(async (..._args: unknown[]) => {}) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: mocks.listen, emitTo: mocks.emitTo }));
import { listen, sendTo } from './targetedEvents';

beforeEach(() => vi.clearAllMocks());
it.each(['main', 'overlay', 'dictation-preview', 'diagnostics'])('binds %s subscriptions to that window instead of Any', async label => {
  mockWindows(label);
  const handler = vi.fn();
  await listen('audio-level', handler);
  expect(mocks.listen).toHaveBeenCalledWith('audio-level', handler, { target: { kind: 'WebviewWindow', label } });
});

it('sends to a named consumer without adding or changing its payload', async () => {
  const payload = { active: true };
  await sendTo('overlay', 'overlay-calibration-changed', payload);
  expect(mocks.emitTo).toHaveBeenCalledWith('overlay', 'overlay-calibration-changed', payload);
  await sendTo('main', 'open-settings');
  expect(mocks.emitTo).toHaveBeenLastCalledWith('main', 'open-settings', undefined);
});
it('silently tolerates an absent main or a companion closing during IPC', async () => {
  mocks.emitTo.mockRejectedValueOnce(new Error('absent main'));
  await expect(sendTo('main', 'open-settings')).resolves.toBeUndefined();
});
