import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { StreamingVadSettings } from './StreamingVadSettings';

vi.mock('../ui/animated-switch/animated-switch', () => ({
  default: ({ checked, onCheckedChange, 'aria-label': label }: { checked: boolean; onCheckedChange: () => void; 'aria-label': string }) => (
    <button type="button" role="switch" aria-checked={checked} aria-label={label} onClick={onCheckedChange} />
  ),
}));

const mocks = vi.hoisted(() => ({ status: vi.fn(), download: vi.fn(), remove: vi.fn() }));
vi.mock('../../lib/streamingVad', () => ({
  STREAMING_VAD_MODEL_SIZE_LABEL: '2.3 MB',
  streamingVadModelStatus: mocks.status,
  downloadStreamingVadModel: mocks.download,
  removeStreamingVadModel: mocks.remove,
}));

let container: HTMLDivElement;
let root: Root;
beforeEach(() => {
  vi.useFakeTimers();
  mocks.status.mockReset();
  mocks.download.mockReset();
  mocks.remove.mockReset();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});
afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  vi.useRealTimers();
});

const buttonByText = (text: string) => Array.from(container.querySelectorAll('button')).find((b) => b.textContent === text);
const toggle = () => container.querySelector<HTMLButtonElement>('[role="switch"]')!;

it('does not download on mount or while the setting is off', async () => {
  mocks.status.mockResolvedValue({ state: 'notDownloaded', sizeBytes: 2327524 });
  await act(async () => root.render(<StreamingVadSettings enabled={false} isRecording={false} onEnabledChange={() => {}} />));
  expect(mocks.download).not.toHaveBeenCalled();
});

it('switching the setting on starts the download when the model is absent', async () => {
  mocks.status.mockResolvedValue({ state: 'notDownloaded', sizeBytes: 2327524 });
  mocks.download.mockResolvedValue(undefined);
  const onEnabledChange = vi.fn();
  await act(async () => root.render(<StreamingVadSettings enabled={false} isRecording={false} onEnabledChange={onEnabledChange} />));
  await act(async () => toggle().click());
  expect(onEnabledChange).toHaveBeenCalledExactlyOnceWith(true);
  expect(mocks.download).toHaveBeenCalledTimes(1);
});

it('switching the setting on with the model already installed downloads nothing', async () => {
  mocks.status.mockResolvedValue({ state: 'ready', sizeBytes: 2327524 });
  await act(async () => root.render(<StreamingVadSettings enabled={false} isRecording={false} onEnabledChange={() => {}} />));
  await act(async () => toggle().click());
  expect(mocks.download).not.toHaveBeenCalled();
});

it('switching the setting off never removes or downloads', async () => {
  mocks.status.mockResolvedValue({ state: 'ready', sizeBytes: 2327524 });
  const onEnabledChange = vi.fn();
  await act(async () => root.render(<StreamingVadSettings enabled isRecording={false} onEnabledChange={onEnabledChange} />));
  await act(async () => toggle().click());
  expect(onEnabledChange).toHaveBeenCalledExactlyOnceWith(false);
  expect(mocks.download).not.toHaveBeenCalled();
  expect(mocks.remove).not.toHaveBeenCalled();
});

it('shows a failed download with its error and retries on demand', async () => {
  mocks.status.mockResolvedValue({ state: 'failed', sizeBytes: 2327524, error: 'hash mismatch' });
  mocks.download.mockResolvedValue(undefined);
  await act(async () => root.render(<StreamingVadSettings enabled isRecording={false} onEnabledChange={() => {}} />));
  expect(container.textContent).toContain('Download or verification failed');
  expect(container.querySelector('[role="alert"]')?.textContent).toBe('hash mismatch');
  await act(async () => buttonByText('Retry download')!.click());
  expect(mocks.download).toHaveBeenCalledTimes(1);
});

it('refreshes a remounted in-flight download until the backend finishes', async () => {
  let backend = 'downloading';
  mocks.status.mockImplementation(async () => ({ state: backend, sizeBytes: 2327524 }));
  const page = <StreamingVadSettings enabled isRecording={false} onEnabledChange={() => {}} />;
  await act(async () => root.render(page));
  expect(container.textContent).toContain('Downloading…');
  backend = 'ready';
  await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
  expect(container.textContent).toContain('Installed locally, 2.3 MB');
  await act(async () => root.render(null));
  expect(vi.getTimerCount()).toBe(0);
});

it('removes an installed model only after confirmation and not while recording', async () => {
  mocks.status.mockResolvedValue({ state: 'ready', sizeBytes: 2327524 });
  mocks.remove.mockResolvedValue(undefined);
  await act(async () => root.render(<StreamingVadSettings enabled isRecording onEnabledChange={() => {}} />));
  expect(buttonByText('Remove model…')!.disabled).toBe(true);
  await act(async () => root.render(<StreamingVadSettings enabled isRecording={false} onEnabledChange={() => {}} />));
  await act(async () => buttonByText('Remove model…')!.click());
  expect(mocks.remove).not.toHaveBeenCalled();
  mocks.status.mockResolvedValue({ state: 'notDownloaded', sizeBytes: 2327524 });
  await act(async () => buttonByText('Confirm remove…')!.click());
  expect(mocks.remove).toHaveBeenCalledTimes(1);
  expect(buttonByText('Download model')).toBeDefined();
});

it('switching on before the status has loaded downloads once it resolves not-installed', async () => {
  let resolveStatus!: (value: unknown) => void;
  mocks.status.mockImplementationOnce(() => new Promise((resolve) => { resolveStatus = resolve; }));
  mocks.status.mockResolvedValue({ state: 'notDownloaded', sizeBytes: 2327524 });
  mocks.download.mockResolvedValue(undefined);
  const onEnabledChange = vi.fn();
  const page = (enabled: boolean) => <StreamingVadSettings enabled={enabled} isRecording={false} onEnabledChange={onEnabledChange} />;
  await act(async () => root.render(page(false)));
  await act(async () => toggle().click());
  expect(onEnabledChange).toHaveBeenCalledExactlyOnceWith(true);
  await act(async () => root.render(page(true)));
  expect(mocks.download).not.toHaveBeenCalled();
  await act(async () => resolveStatus({ state: 'notDownloaded', sizeBytes: 2327524 }));
  expect(mocks.download).toHaveBeenCalledTimes(1);
});

it('switching on before the status loads does not download an installed model', async () => {
  let resolveStatus!: (value: unknown) => void;
  mocks.status.mockImplementationOnce(() => new Promise((resolve) => { resolveStatus = resolve; }));
  await act(async () => root.render(<StreamingVadSettings enabled={false} isRecording={false} onEnabledChange={() => {}} />));
  await act(async () => toggle().click());
  await act(async () => root.render(<StreamingVadSettings enabled isRecording={false} onEnabledChange={() => {}} />));
  await act(async () => resolveStatus({ state: 'ready', sizeBytes: 2327524 }));
  expect(mocks.download).not.toHaveBeenCalled();
});

it('switching on then straight off before the status loads downloads nothing', async () => {
  let resolveStatus!: (value: unknown) => void;
  mocks.status.mockImplementationOnce(() => new Promise((resolve) => { resolveStatus = resolve; }));
  const page = (enabled: boolean) => <StreamingVadSettings enabled={enabled} isRecording={false} onEnabledChange={() => {}} />;
  await act(async () => root.render(page(false)));
  await act(async () => toggle().click());
  await act(async () => root.render(page(true)));
  await act(async () => root.render(page(false)));
  await act(async () => resolveStatus({ state: 'notDownloaded', sizeBytes: 2327524 }));
  expect(mocks.download).not.toHaveBeenCalled();
});
