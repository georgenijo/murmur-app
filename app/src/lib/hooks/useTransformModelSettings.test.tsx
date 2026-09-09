import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { DEFAULT_SETTINGS } from '../settings';
import type { TransformModelStatus } from '../transformSettings';

const mocks = vi.hoisted(() => ({
  transformModelStatus: vi.fn(),
  listen: vi.fn(),
}));

vi.mock('@tauri-apps/api/event', () => ({ listen: mocks.listen }));
vi.mock('../transformSettings', () => ({
  transformModelStatus: mocks.transformModelStatus,
  downloadTransformModel: vi.fn(),
  removeTransformModel: vi.fn(),
  resetTransformRuntime: vi.fn(),
  setTransformKey: vi.fn(),
  startTransformListener: vi.fn(),
  stopTransformListener: vi.fn(),
}));

import {
  useTransformModelSettings,
  type TransformModelSettingsPage,
} from './useTransformModelSettings';

const DOWNLOADING = {
  state: 'downloading',
  path: null,
  sizeBytes: 0,
  sha256: '',
  runtimeDisabled: false,
} satisfies TransformModelStatus;

const READY_DISABLED = {
  state: 'ready',
  path: '/models/transform.gguf',
  sizeBytes: 1024,
  sha256: 'abc',
  runtimeDisabled: true,
} satisfies TransformModelStatus;

const NOT_DOWNLOADED = {
  state: 'notDownloaded',
  path: null,
  sizeBytes: 0,
  sha256: '',
  runtimeDisabled: false,
} satisfies TransformModelStatus;

const onUpdateSettings = vi.fn();

function Harness({ page }: { page: TransformModelSettingsPage }) {
  const model = useTransformModelSettings({
    settings: DEFAULT_SETTINGS,
    onUpdateSettings,
    activePage: page,
  }).transformModel;
  return <p>{model ? `${model.state}:${model.runtimeDisabled}` : ''}</p>;
}

describe('useTransformModelSettings', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    vi.clearAllMocks();
    mocks.listen.mockResolvedValue(vi.fn());
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  async function render(activePage: TransformModelSettingsPage) {
    await act(async () => {
      root.render(<Harness page={activePage} />);
      await Promise.resolve();
      await Promise.resolve();
    });
  }

  it('refreshes model state across both AI overview and rewrite page transitions', async () => {
    mocks.transformModelStatus
      .mockResolvedValueOnce(DOWNLOADING)
      .mockResolvedValueOnce(READY_DISABLED)
      .mockResolvedValueOnce(NOT_DOWNLOADED)
      .mockResolvedValueOnce(DOWNLOADING);

    await render(null);
    expect(mocks.transformModelStatus).not.toHaveBeenCalled();

    await render('ai');
    expect(mocks.transformModelStatus).toHaveBeenCalledTimes(1);
    expect(container.textContent).toBe('downloading:false');

    await render('ai');
    expect(mocks.transformModelStatus).toHaveBeenCalledTimes(1);

    await render('ai-transform');
    expect(mocks.transformModelStatus).toHaveBeenCalledTimes(2);
    expect(container.textContent).toBe('ready:true');

    await render('ai-transform');
    expect(mocks.transformModelStatus).toHaveBeenCalledTimes(2);

    await render(null);
    expect(mocks.transformModelStatus).toHaveBeenCalledTimes(2);

    await render('ai-transform');
    expect(mocks.transformModelStatus).toHaveBeenCalledTimes(3);
    expect(container.textContent).toBe('notDownloaded:false');

    await render('ai');
    expect(mocks.transformModelStatus).toHaveBeenCalledTimes(4);
    expect(container.textContent).toBe('downloading:false');
  });
});
