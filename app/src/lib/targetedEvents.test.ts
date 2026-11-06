import { mockWindows } from '@tauri-apps/api/mocks';
import { beforeEach, expect, it, vi } from 'vitest';
const mocks = vi.hoisted(() => ({ listen: vi.fn(async () => () => {}) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: mocks.listen }));
import { listen } from './targetedEvents';

beforeEach(() => vi.clearAllMocks());
it.each(['main', 'overlay', 'dictation-preview', 'diagnostics'])('binds %s subscriptions to that window instead of Any', async label => {
  mockWindows(label);
  const handler = vi.fn();
  await listen('audio-level', handler);
  expect(mocks.listen).toHaveBeenCalledWith('audio-level', handler, { target: { kind: 'WebviewWindow', label } });
});
