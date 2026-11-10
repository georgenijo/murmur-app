import { act, type ComponentProps } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ModelDownloadProgress } from '../lib/modelDownload';
import { ModelDownloadPanel } from './ModelDownloader';
import type { ModelHardwareGuidance } from '../lib/modelHardwareGuidance';

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
  listener: null as null | ((event: { payload: ModelDownloadProgress }) => void),
  unlisten: vi.fn(),
}));
const guidanceMocks = vi.hoisted(() => ({
  current: null as ModelHardwareGuidance | null,
}));

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (command: string, ...arguments_: unknown[]) => command === 'get_model_hardware_guidance'
    ? Promise.resolve(guidanceMocks.current)
    : mocks.invoke(command, ...arguments_),
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
      (candidate) => label === 'Download' ? candidate.textContent?.trim().startsWith('Download ·') : candidate.textContent?.trim() === label,
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
    guidanceMocks.current = null;
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

  it('moves download status into a fixed host when the list scrolls', async () => {
    const statusHost = document.createElement('div');
    document.body.appendChild(statusHost);
    const pending = deferred<void>();
    mocks.invoke.mockReturnValue(pending.promise);
    try {
      await renderPanel({ initialModel: 'base.en', statusPortalTarget: statusHost });
      await act(async () => (statusHost.querySelector('button') as HTMLButtonElement).click());
      await settle();

      expect(container.textContent).not.toContain('Cancel');
      expect(statusHost.textContent).toContain('Cancel');
      expect(statusHost.querySelector('[role="progressbar"]')).not.toBeNull();

      await act(async () => mocks.listener?.({
        payload: {
          requestId: mocks.invoke.mock.calls[0][1].requestId,
          modelName: 'base.en',
          attemptId: 1,
          received: 50,
          total: 100,
          phase: 'downloading',
        },
      }));
      expect(statusHost.querySelector('[role="progressbar"]')?.getAttribute('aria-valuenow')).toBe('50');
    } finally {
      await act(async () => pending.resolve());
      statusHost.remove();
    }
  });

  it('accepts progress only for the requested model and first correlated attempt', async () => {
    const pending = deferred<void>();
    const onComplete = vi.fn();
    mocks.invoke.mockReturnValue(pending.promise);
    await renderPanel({ onComplete });

    await click('Download');
    expect(mocks.invoke).toHaveBeenCalledWith('download_model', {
      modelName: 'parakeet-tdt-0.6b-v3-coreml',
      requestId: expect.any(String),
    });

    await act(async () => mocks.listener?.({
      payload: {
        modelName: 'base.en',
        requestId: mocks.invoke.mock.calls[0][1].requestId,
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
        requestId: mocks.invoke.mock.calls[0][1].requestId,
        attemptId: 7,
        received: 0,
        total: 0,
        phase: 'repairing',
        repeatedRepair: true,
      },
    }));
    expect(container.textContent).toContain('Repairing incomplete install again…');

    await act(async () => mocks.listener?.({
      payload: {
        modelName: 'parakeet-tdt-0.6b-v3-coreml',
        requestId: mocks.invoke.mock.calls[0][1].requestId,
        attemptId: 8,
        received: 0,
        total: 0,
        phase: 'validating',
      },
    }));
    expect(container.textContent).not.toContain('Validating installation…');

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
    expect(button('Try again').hasAttribute('disabled')).toBe(false);
    expect(button('Use Whisper base')).toBeDefined();
    expect(onDownloadingChange.mock.calls).toEqual([[true], [false]]);

    await click('Use Whisper base');
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
    expect(button('Try again').hasAttribute('disabled')).toBe(false);
    expect(mocks.invoke).not.toHaveBeenCalled();
  });

  it('shows low-memory guidance while keeping warned model choices enabled', async () => {
    guidanceMocks.current = {
      schemaVersion: 1,
      chip: 'Apple M1',
      chipTier: 'legacyAppleSilicon',
      physicalMemoryGib: 8,
      modelMemoryBudgetMib: 1536,
      recommendedModel: 'parakeet-tdt-0.6b-v3-coreml',
      warnedModels: ['parakeet-tdt-0.6b-v2-fp16', 'medium.en', 'large-v3-turbo'],
    };
    await renderPanel();

    expect(container.textContent).toContain('Recommended for this Mac');
    expect(container.textContent?.match(/Higher memory/g)).toHaveLength(2);
    const warnedModel = container.querySelector('#download-model-large-v3-turbo') as HTMLButtonElement;
    expect(warnedModel.disabled).toBe(false);

    await act(async () => warnedModel.click());

    expect(warnedModel.getAttribute('aria-pressed')).toBe('true');
    expect(container.textContent).toContain('You can still select and use it');
    expect(button('Download').hasAttribute('disabled')).toBe(false);
  });
  it('keeps Back and model choices locked until cancellation and the install settle', async () => {
    const download = deferred<void>();
    const cancellation = deferred<void>();
    const onDownloadingChange = vi.fn();
    const onComplete = vi.fn();
    mocks.invoke.mockImplementation((command: string) => command === 'cancel_model_download' ? cancellation.promise : download.promise);
    await renderPanel({ onDownloadingChange, onComplete });
    await click('Download');
    const requestId = mocks.invoke.mock.calls[0][1].requestId;
    await click('Cancel');
    expect(mocks.invoke).toHaveBeenLastCalledWith('cancel_model_download', {
      modelName: 'parakeet-tdt-0.6b-v3-coreml', requestId,
    });
    expect(button('Cancelling…').disabled).toBe(true);
    expect(onDownloadingChange.mock.calls).toEqual([[true]]);
    await act(async () => download.reject('Model installation cancelled'));
    expect(onDownloadingChange.mock.calls).toEqual([[true]]);
    await act(async () => cancellation.resolve());
    expect(onDownloadingChange.mock.calls).toEqual([[true], [false]]);
    expect(onComplete).not.toHaveBeenCalled();
    expect(button('Download').disabled).toBe(false);
    expect((container.querySelector('[id="download-model-base.en"]') as HTMLButtonElement).disabled).toBe(false);
  });

  it('does not unlock navigation if native cancellation cannot confirm termination', async () => {
    const download = deferred<void>();
    const onDownloadingChange = vi.fn();
    mocks.invoke.mockImplementation((command: string) => command === 'cancel_model_download'
      ? Promise.reject('Installer termination is not yet confirmed') : download.promise);
    await renderPanel({ onDownloadingChange });
    await click('Download');
    await click('Cancel');
    expect(container.querySelector('[role="alert"]')?.textContent).toContain('termination');
    expect(onDownloadingChange.mock.calls).toEqual([[true]]);
    expect((container.querySelector('[id="download-model-base.en"]') as HTMLButtonElement).disabled).toBe(true);
    mocks.invoke.mockImplementation(() => Promise.resolve());
    await act(async () => download.reject('cancelled'));
    await click('Cancel');
    expect(onDownloadingChange.mock.calls).toEqual([[true], [false]]);
  });

  it('cancels before a pending subscription without starting or completing a stale install', async () => {
    const listening = deferred<() => void>();
    mocks.listen.mockReturnValueOnce(listening.promise);
    const onComplete = vi.fn();
    await renderPanel({ onComplete });
    await click('Download');
    await click('Cancel');
    expect(button('Download').disabled).toBe(false);
    await act(async () => listening.resolve(mocks.unlisten));
    expect(mocks.invoke).not.toHaveBeenCalled();
    expect(onComplete).not.toHaveBeenCalled();
    expect(mocks.unlisten).toHaveBeenCalledOnce();
  });

  it('cancels on unmount and ignores late native success', async () => {
    const download = deferred<void>();
    const onComplete = vi.fn();
    mocks.invoke.mockImplementation((command: string) => command === 'download_model' ? download.promise : Promise.resolve());
    await renderPanel({ onComplete });
    await click('Download');
    const requestId = mocks.invoke.mock.calls[0][1].requestId;
    await act(async () => root.render(null));
    expect(mocks.invoke).toHaveBeenCalledWith('cancel_model_download', {
      modelName: 'parakeet-tdt-0.6b-v3-coreml', requestId,
    });
    await act(async () => download.resolve());
    expect(onComplete).not.toHaveBeenCalled();
    expect(mocks.unlisten).toHaveBeenCalledOnce();
  });

  it('allows slow progress without a frontend deadline and displays a resumed offset on retry', async () => {
    vi.useFakeTimers();
    try {
      const first = deferred<void>();
      const second = deferred<void>();
      mocks.invoke.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
      await renderPanel({ initialModel: 'base.en' });
      await act(async () => button('Download').click());
      const firstRequest = mocks.invoke.mock.calls[0][1].requestId;
      for (let n = 1; n <= 20; n++) {
        await act(async () => {
          vi.advanceTimersByTime(60_000);
          mocks.listener?.({ payload: { requestId: firstRequest, modelName: 'base.en', attemptId: 1, received: n, total: 100, phase: 'downloading' } });
        });
      }
      expect(container.querySelector('[role="progressbar"]')?.getAttribute('aria-valuenow')).toBe('20');
      expect(container.querySelector('[role="alert"]')).toBeNull();
      await act(async () => first.reject('Download stalled: no data received. Retry to resume.'));
      expect(container.textContent).toContain('stalled');
      await act(async () => button('Try again').click());
      const secondRequest = mocks.invoke.mock.calls[1][1].requestId;
      expect(secondRequest).not.toBe(firstRequest);
      await act(async () => mocks.listener?.({ payload: { requestId: firstRequest, modelName: 'base.en', attemptId: 1, received: 99, total: 100 } }));
      expect(container.querySelector('[role="progressbar"]')?.getAttribute('aria-valuenow')).toBeNull();
      await act(async () => mocks.listener?.({ payload: { requestId: secondRequest, modelName: 'base.en', attemptId: 2, received: 20, total: 100 } }));
      expect(container.querySelector('[role="progressbar"]')?.getAttribute('aria-valuenow')).toBe('20');
      await act(async () => second.resolve());
    } finally { vi.useRealTimers(); }
  });

  it('shows native disk preflight failure and permits another model without completing', async () => {
    const onComplete = vi.fn();
    const onDownloadingChange = vi.fn();
    mocks.invoke.mockRejectedValueOnce('Not enough disk space: need 1630 MB, available 100 MB. Free space and retry.');
    await renderPanel({ initialModel: 'large-v3-turbo', onComplete, onDownloadingChange });
    await click('Download');
    expect(container.querySelector('[role="alert"]')?.textContent).toContain('need 1630 MB, available 100 MB');
    expect(onComplete).not.toHaveBeenCalled();
    expect(onDownloadingChange.mock.calls).toEqual([[true], [false]]);
    expect(button('Try again').disabled).toBe(false);
    await act(async () => (container.querySelector('[id="download-model-base.en"]') as HTMLButtonElement).click());
    expect(container.querySelector('[role="alert"]')).toBeNull();
  });

  it('derives the estimated action size from selection and disables every option during a live attempt', async () => {
    const download = deferred<void>();
    mocks.invoke.mockReturnValue(download.promise);
    await renderPanel();
    expect(button('Download').textContent).toBe('Download · ~470 MB');
    await act(async () => container.querySelector<HTMLButtonElement>('[id="download-model-base.en"]')!.click());
    expect(button('Download').textContent).toBe('Download · ~150 MB');
    await click('Download');
    const options = [...container.querySelectorAll<HTMLButtonElement>('.model-option')];
    expect(options.every((option) => option.disabled && option.getAttribute('aria-disabled') === 'true')).toBe(true);
    expect(button('Cancel').disabled).toBe(false);
    await act(async () => download.reject('Network unavailable.'));
    expect(options.every((option) => !option.disabled)).toBe(true);
    expect(button('Try again').disabled).toBe(false);
    expect(container.querySelector('[role="alert"]')?.textContent).toContain('Network unavailable.');
  });

});
