import { act, type ComponentProps } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ModelDownloadProgress } from '../lib/modelDownload';
import { ModelDownloadPanel } from './ModelDownloader';

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
  listener: null as null | ((event: { payload: ModelDownloadProgress }) => void),
  unlisten: vi.fn(),
}));

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...arguments_: unknown[]) => mocks.invoke(...arguments_),
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: (event: string, listener: (event: { payload: ModelDownloadProgress }) => void) => {
    mocks.listener = listener;
    return mocks.listen(event, listener);
  },
}));

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

describe('ModelDownloadPanel', () => {
  let container: HTMLDivElement;
  let root: Root;

  const settle = async (milliseconds = 0) => {
    await act(async () => {
      await new Promise((resolve) => window.setTimeout(resolve, milliseconds));
    });
  };

  const button = (label: string) => {
    const match = Array.from(container.querySelectorAll('button')).find(
      (candidate) => candidate.textContent?.trim() === label,
    );
    expect(match, `Expected button "${label}"`).toBeDefined();
    return match!;
  };

  const click = async (label: string) => {
    await act(async () => button(label).click());
    await settle();
  };

  const renderPanel = async (overrides: Partial<ComponentProps<typeof ModelDownloadPanel>> = {}) => {
    const props: ComponentProps<typeof ModelDownloadPanel> = {
      initialModel: 'parakeet-tdt-0.6b-v3-coreml',
      installedModels: {},
      onComplete: vi.fn(),
      ...overrides,
    };
    await act(async () => root.render(<ModelDownloadPanel {...props} />));
    return props;
  };

  beforeEach(() => {
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    mocks.invoke.mockReset();
    mocks.listen.mockReset().mockResolvedValue(mocks.unlisten);
    mocks.unlisten.mockReset();
    mocks.listener = null;
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('continues with an installed model without invoking a download', async () => {
    const onComplete = vi.fn();
    await renderPanel({
      initialModel: 'base.en',
      installedModels: { 'base.en': true },
      onComplete,
    });

    await click('Continue');

    expect(onComplete).toHaveBeenCalledWith('base.en');
    expect(mocks.invoke).not.toHaveBeenCalled();
  });

  it('accepts progress only for the requested model and first correlated attempt', async () => {
    const pending = deferred<void>();
    const onComplete = vi.fn();
    mocks.invoke.mockReturnValue(pending.promise);
    await renderPanel({ onComplete });

    await click('Download');
    expect(mocks.invoke).toHaveBeenCalledWith('download_model', {
      modelName: 'parakeet-tdt-0.6b-v3-coreml',
    });

    await act(async () => mocks.listener?.({
      payload: {
        modelName: 'base.en',
        attemptId: 2,
        received: 0,
        total: 0,
        phase: 'repairing',
      },
    }));
    expect(container.textContent).not.toContain('Repairing incomplete install');

    await act(async () => mocks.listener?.({
      payload: {
        modelName: 'parakeet-tdt-0.6b-v3-coreml',
        attemptId: 7,
        received: 0,
        total: 0,
        phase: 'repairing',
        repeatedRepair: true,
      },
    }));
    expect(container.textContent).toContain('Repairing incomplete install again...');

    await act(async () => mocks.listener?.({
      payload: {
        modelName: 'parakeet-tdt-0.6b-v3-coreml',
        attemptId: 8,
        received: 0,
        total: 0,
        phase: 'validating',
      },
    }));
    expect(container.textContent).not.toContain('Validating installation...');

    await act(async () => pending.resolve());
    await settle();
    expect(onComplete).toHaveBeenCalledWith('parakeet-tdt-0.6b-v3-coreml');
    expect(mocks.unlisten).toHaveBeenCalledOnce();
  });

  it('unlocks retry and installed fallback after a bounded Core ML failure', async () => {
    const onComplete = vi.fn();
    const onDownloadingChange = vi.fn();
    mocks.invoke.mockRejectedValueOnce('Core ML setup stopped after reaching its time limit.');
    await renderPanel({
      installedModels: { 'base.en': true },
      onComplete,
      onDownloadingChange,
    });

    await click('Download');

    const alert = container.querySelector('[role="alert"]');
    expect(alert?.textContent).toContain('time limit');
    expect(button('Retry Download').hasAttribute('disabled')).toBe(false);
    expect(button('Use Whisper Base')).toBeDefined();
    expect(onDownloadingChange.mock.calls).toEqual([[true], [false]]);

    await click('Use Whisper Base');
    await settle(20);
    expect(container.querySelector('[role="alert"]')).toBeNull();
    expect(document.activeElement?.id).toBe('download-model-base.en');
    await click('Continue');
    expect(onComplete).toHaveBeenCalledWith('base.en');
    expect(mocks.invoke).toHaveBeenCalledOnce();
  });

  it('releases the embedding navigation lock when event subscription fails', async () => {
    const onDownloadingChange = vi.fn();
    mocks.listen.mockRejectedValueOnce(new Error('listener unavailable'));
    await renderPanel({ onDownloadingChange });

    await click('Download');

    expect(container.querySelector('[role="alert"]')?.textContent)
      .toContain('listener unavailable');
    expect(onDownloadingChange.mock.calls).toEqual([[true], [false]]);
    expect(button('Retry Download').hasAttribute('disabled')).toBe(false);
    expect(mocks.invoke).not.toHaveBeenCalled();
  });
});
