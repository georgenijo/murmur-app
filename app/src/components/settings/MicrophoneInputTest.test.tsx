import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { MicrophonePreviewStatus } from '../../lib/microphonePreview';

const mocks = vi.hoisted(() => {
  const listeners = new Map<string, (event: { payload: unknown }) => void>();
  return {
    listeners,
    invoke: vi.fn(),
    listen: vi.fn(async (event: string, handler: (event: { payload: unknown }) => void) => {
      listeners.set(event, handler);
      return () => listeners.delete(event);
    }),
  };
});

vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: mocks.listen }));

import { MicrophoneInputTest } from './MicrophoneInputTest';

const idle: MicrophonePreviewStatus = {
  previewId: null,
  state: 'idle',
  stillConnecting: false,
  errorKind: null,
  message: null,
};

const active: MicrophonePreviewStatus = {
  previewId: 7,
  state: 'active',
  stillConnecting: false,
  errorKind: null,
  message: null,
};

const connecting: MicrophonePreviewStatus = {
  previewId: 7,
  state: 'connecting',
  stillConnecting: false,
  errorKind: null,
  message: null,
};

describe('MicrophoneInputTest', () => {
  let container: HTMLDivElement;
  let root: Root;
  let selected = 'system_default';
  let activePage = true;
  let dictationBusy = false;
  let frames: Map<number, FrameRequestCallback>;
  let nextFrame: number;

  async function render() {
    await act(async () => {
      root.render(
        <MicrophoneInputTest
          microphone={selected}
          devices={[
            { id: 'built-in', name: 'Built-in Microphone' },
            { id: 'usb', name: 'USB Microphone' },
          ]}
          active={activePage}
          dictationBusy={dictationBusy}
          missingDevice={false}
          onChange={(microphone) => {
            selected = microphone;
          }}
        />,
      );
      await Promise.resolve();
      await Promise.resolve();
    });
  }

  async function emitStatus(status: MicrophonePreviewStatus) {
    await act(async () => {
      mocks.listeners.get('microphone-preview-status')?.({ payload: status });
    });
  }

  beforeEach(async () => {
    vi.clearAllMocks();
    mocks.listeners.clear();
    selected = 'system_default';
    activePage = true;
    dictationBusy = false;
    frames = new Map();
    nextFrame = 1;
    vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => {
      const id = nextFrame++;
      frames.set(id, callback);
      return id;
    });
    vi.stubGlobal('cancelAnimationFrame', (id: number) => frames.delete(id));
    Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', {
      value: vi.fn(),
      configurable: true,
    });
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === 'get_microphone_preview_status') return idle;
      if (command === 'start_microphone_preview') return active;
      if (command === 'stop_microphone_preview') return idle;
      if (command === 'cancel_microphone_preview') return true;
      throw new Error(`unexpected command: ${command}`);
    });
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    vi.unstubAllGlobals();
    container.remove();
  });

  it('starts an exact preview automatically and paints level events through animation frames', async () => {
    await render();
    expect(mocks.invoke).toHaveBeenCalledWith('start_microphone_preview', {
      deviceId: 'system_default',
    });
    await emitStatus(active);
    await act(async () => {
      mocks.listeners.get('microphone-preview-level')?.({
        payload: { previewId: 7, rms: 0.04, peak: 0.6, classification: 'signal_detected' },
      });
      const [frame] = frames.values();
      frame?.(0);
    });

    const meter = container.querySelector('[role="meter"]') as HTMLElement;
    expect(meter.getAttribute('aria-valuenow')).toBe('20');
    expect(meter.getAttribute('aria-valuetext')).toContain('Signal detected');
    expect(container.textContent).toContain('Signal detected');
  });

  it('pauses for dictation while connecting and resumes automatically afterward', async () => {
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === 'get_microphone_preview_status') return idle;
      if (command === 'start_microphone_preview') return connecting;
      if (command === 'stop_microphone_preview') return idle;
      if (command === 'cancel_microphone_preview') return true;
      throw new Error(`unexpected command: ${command}`);
    });
    await render();

    dictationBusy = true;
    await render();
    expect(mocks.invoke).toHaveBeenCalledWith('cancel_microphone_preview', { previewId: 7 });
    expect(container.textContent).toContain('resumes automatically');

    await emitStatus(idle);
    dictationBusy = false;
    await render();
    expect(mocks.invoke.mock.calls.filter(([command]) => command === 'start_microphone_preview')).toHaveLength(2);
  });

  it('confirms teardown before persisting and reopening a switched device', async () => {
    await render();
    await emitStatus(active);
    const combobox = container.querySelector('[role="combobox"]') as HTMLButtonElement;
    await act(async () => combobox.click());
    const option = Array.from(container.querySelectorAll('[role="option"]'))
      .find((item) => item.textContent?.includes('USB Microphone')) as HTMLElement;
    await act(async () => {
      option.click();
      await Promise.resolve();
      await Promise.resolve();
    });
    await render();

    const calls = mocks.invoke.mock.calls.map(([command, args]) => [command, args]);
    const stopIndex = calls.findIndex(([command]) => command === 'stop_microphone_preview');
    const restartIndex = calls.findIndex(([command, args]) => (
      command === 'start_microphone_preview' && args.deviceId === 'usb'
    ));
    expect(stopIndex).toBeGreaterThanOrEqual(0);
    expect(restartIndex).toBeGreaterThan(stopIndex);
    expect(selected).toBe('usb');
  });

  it('keeps a new selection but does not reopen audio when teardown fails', async () => {
    await render();
    await emitStatus(active);
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === 'get_microphone_preview_status') return idle;
      if (command === 'stop_microphone_preview') throw new Error('cleanup timed out');
      if (command === 'cancel_microphone_preview') return true;
      if (command === 'start_microphone_preview') return active;
      throw new Error(`unexpected command: ${command}`);
    });
    const combobox = container.querySelector('[role="combobox"]') as HTMLButtonElement;
    await act(async () => combobox.click());
    const option = Array.from(container.querySelectorAll('[role="option"]'))
      .find((item) => item.textContent?.includes('Built-in Microphone')) as HTMLElement;
    await act(async () => {
      option.click();
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(selected).toBe('built-in');
    expect(mocks.invoke).not.toHaveBeenCalledWith('start_microphone_preview', { deviceId: 'built-in' });
    expect(container.textContent).toContain('cleanup timed out');
  });

  it('cancels only its exact preview generation when unmounted', async () => {
    await render();
    await emitStatus(active);
    await act(async () => root.render(null));
    expect(mocks.invoke).toHaveBeenCalledWith('cancel_microphone_preview', { previewId: 7 });
  });

  it('cancels a preview generation that resolves after the settings page unmounts', async () => {
    let resolveStart!: (status: MicrophonePreviewStatus) => void;
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === 'get_microphone_preview_status') return idle;
      if (command === 'start_microphone_preview') {
        return new Promise<MicrophonePreviewStatus>((resolve) => { resolveStart = resolve; });
      }
      if (command === 'cancel_microphone_preview') return true;
      if (command === 'stop_microphone_preview') return idle;
      throw new Error(`unexpected command: ${command}`);
    });
    await render();
    await act(async () => root.render(null));
    await act(async () => {
      resolveStart(active);
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(mocks.invoke).toHaveBeenCalledWith('cancel_microphone_preview', { previewId: 7 });
  });

  it('does not monitor while the Settings page is hidden', async () => {
    activePage = false;
    await render();
    expect(mocks.invoke).not.toHaveBeenCalledWith('start_microphone_preview', expect.anything());
  });
});
