import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { SmartAutoStopSettings } from './SmartAutoStopSettings';

vi.mock('../ui/animated-switch/animated-switch', () => ({ default: () => <span /> }));

const mocks = vi.hoisted(() => ({ status: vi.fn(), download: vi.fn() }));
vi.mock('../../lib/smartTurn', () => ({
  SMART_TURN_MODEL_SIZE_LABEL: '~8.7 MB',
  smartTurnModelStatus: mocks.status,
  downloadSmartTurnModel: mocks.download,
  removeSmartTurnModel: vi.fn(),
}));

let container: HTMLDivElement;
let root: Root;
beforeEach(() => {
  vi.useFakeTimers();
  mocks.status.mockReset();
  mocks.download.mockReset();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});
afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  vi.useRealTimers();
});

it.each(['ready', 'notDownloaded'] as const)('refreshes a remounted in-flight download after %s', async (finalState) => {
  let finish!: () => void;
  let backend = 'notDownloaded';
  mocks.status.mockImplementation(async () => ({ state: backend, sizeBytes: 8679182 }));
  mocks.download.mockImplementation(() => {
    backend = 'downloading';
    return new Promise<void>((resolve) => { finish = resolve; });
  });
  const page = <SmartAutoStopSettings enabled={false} silenceAutoStopOn isRecording={false} onEnabledChange={() => {}} />;
  await act(async () => root.render(page));
  await act(async () => Array.from(container.querySelectorAll('button')).find((b) => b.textContent === 'Download model')!.click());
  await act(async () => root.render(null));
  await act(async () => root.render(page));
  expect(container.textContent).toContain('Downloading...');
  backend = finalState;
  await act(async () => finish());
  await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
  const button = Array.from(container.querySelectorAll('button')).find((b) => b.textContent === (finalState === 'ready' ? 'Remove model' : 'Download model'));
  expect(button).toBeDefined();
  expect(button!.disabled).toBe(false);
  await act(async () => root.render(null));
  expect(vi.getTimerCount()).toBe(0);
});
