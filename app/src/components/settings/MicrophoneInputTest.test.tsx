import { act, memo } from 'react';
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
import { SettingsSurfaceActiveContext } from './SettingsSurfaceContext';

const MemoizedMicrophoneInputTest = memo(MicrophoneInputTest);
const devices = [
  { id: 'built-in', name: 'Built-in Microphone' },
  { id: 'usb', name: 'USB Microphone' },
];

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
  let surfaceActive = true;
  let appReady = true;
  let vadSensitivity = 60;
  let dictationBusy = false;
  let inventoryAvailable = true;
  let inventoryLoading = false;
  let missingDevice = false;
  let frames: Map<number, FrameRequestCallback>;
  let nextFrame: number;

  function handleMicrophoneChange(microphone: string) {
    selected = microphone;
  }

  async function render() {
    await act(async () => {
      root.render(
        <SettingsSurfaceActiveContext.Provider value={surfaceActive}>
          <MemoizedMicrophoneInputTest
            microphone={selected}
            devices={devices}
            active={activePage}
            ready={appReady}
            vadSensitivity={vadSensitivity}
            dictationBusy={dictationBusy}
            missingDevice={missingDevice}
            inventoryAvailable={inventoryAvailable}
            inventoryLoading={inventoryLoading}
            onChange={handleMicrophoneChange}
          />
        </SettingsSurfaceActiveContext.Provider>,
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
    surfaceActive = true;
    appReady = true;
    vadSensitivity = 60;
    dictationBusy = false;
    inventoryAvailable = true;
    inventoryLoading = false;
    missingDevice = false;
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
      if (command === 'update_microphone_preview_vad_sensitivity') return true;
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
      vadSensitivity: 60,
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
    const paintedLevel = Number(meter.getAttribute('aria-valuenow'));
    expect(paintedLevel).toBeGreaterThan(0);
    expect(paintedLevel).toBeLessThan(20);
    expect(meter.getAttribute('aria-valuetext')).toContain('Signal detected');
    expect(container.textContent).toContain('Signal detected');
  });

  it('does not preview or allow selection from stale inventory', async () => {
    inventoryAvailable = false;
    await render();
    expect(mocks.invoke).not.toHaveBeenCalledWith('start_microphone_preview', expect.anything());
    expect((container.querySelector('[aria-label="Microphone input"]') as HTMLButtonElement).disabled).toBe(true);
  });

  it('describes loading before unavailable or missing-device selector states', async () => {
    inventoryAvailable = false;
    inventoryLoading = true;
    missingDevice = true;
    await render();
    const selector = container.querySelector('[aria-label="Microphone input"]') as HTMLButtonElement;
    const helper = document.getElementById(selector.getAttribute('aria-describedby') ?? '');
    expect(helper?.textContent).toBe('Loading available microphones…');
    expect(container.textContent).not.toContain('Selected device not found');

    inventoryLoading = false;
    await render();
    const unavailableHelper = document.getElementById(selector.getAttribute('aria-describedby') ?? '');
    expect(unavailableHelper?.textContent).toBe('Microphone choices are temporarily unavailable.');
    expect(container.textContent).not.toContain('Selected device not found');
  });

  it('describes a missing selected device once inventory is authoritative', async () => {
    missingDevice = true;
    await render();
    const selector = container.querySelector('[aria-label="Microphone input"]') as HTMLButtonElement;
    const helper = document.getElementById(selector.getAttribute('aria-describedby') ?? '');
    expect(helper?.textContent).toContain('Selected device not found');
  });

  it('pauses for dictation while connecting and resumes automatically afterward', async () => {
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === 'get_microphone_preview_status') return idle;
      if (command === 'start_microphone_preview') return connecting;
      if (command === 'update_microphone_preview_vad_sensitivity') return true;
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
      if (command === 'update_microphone_preview_vad_sensitivity') return true;
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

  it('starts when the warm-mounted Settings surface becomes visible', async () => {
    surfaceActive = false;
    await render();
    expect(mocks.invoke).not.toHaveBeenCalledWith('start_microphone_preview', expect.anything());

    surfaceActive = true;
    await render();
    expect(mocks.invoke).toHaveBeenCalledWith('start_microphone_preview', {
      deviceId: 'system_default',
      vadSensitivity: 60,
    });
  });

  it('does not monitor when another Settings category is selected', async () => {
    activePage = false;
    await render();
    expect(mocks.invoke).not.toHaveBeenCalledWith('start_microphone_preview', expect.anything());
  });

  it('waits for fresh-launch initialization and starts as soon as Murmur is ready', async () => {
    appReady = false;
    await render();
    expect(mocks.invoke).not.toHaveBeenCalledWith('start_microphone_preview', expect.anything());
    expect(container.textContent).toContain('Preparing microphone monitoring');

    appReady = true;
    await render();
    expect(mocks.invoke).toHaveBeenCalledWith('start_microphone_preview', {
      deviceId: 'system_default',
      vadSensitivity: 60,
    });
  });

  it('shows live VAD decisions and drops results from an older slider value', async () => {
    await render();
    await emitStatus(active);
    await act(async () => {
      mocks.listeners.get('microphone-preview-vad')?.({
        payload: { previewId: 7, sensitivity: 60, decision: 'no_speech' },
      });
    });
    expect(container.textContent).toContain('No speech · filtered');

    vadSensitivity = 20;
    await render();
    expect(mocks.invoke).toHaveBeenCalledWith(
      'update_microphone_preview_vad_sensitivity',
      { previewId: 7, vadSensitivity: 20 },
    );
    await act(async () => {
      mocks.listeners.get('microphone-preview-vad')?.({
        payload: { previewId: 7, sensitivity: 60, decision: 'speech_detected' },
      });
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(container.textContent).toContain('Listening');
    expect(mocks.invoke).toHaveBeenLastCalledWith(
      'update_microphone_preview_vad_sensitivity',
      { previewId: 7, vadSensitivity: 20 },
    );

    await act(async () => {
      mocks.listeners.get('microphone-preview-vad')?.({
        payload: { previewId: 7, sensitivity: 20, decision: 'speech_detected' },
      });
    });
    expect(container.textContent).toContain('Speech detected · kept');
  });

  it('ignores a VAD decision from another preview generation and resets after stop', async () => {
    await render();
    await emitStatus(active);
    await act(async () => {
      mocks.listeners.get('microphone-preview-vad')?.({
        payload: { previewId: 8, sensitivity: 60, decision: 'speech_detected' },
      });
    });
    expect(container.textContent).toContain('Listening');

    await act(async () => {
      mocks.listeners.get('microphone-preview-vad')?.({
        payload: { previewId: 7, sensitivity: 60, decision: 'speech_detected' },
      });
    });
    expect(container.textContent).toContain('Speech detected · kept');

    await emitStatus(idle);
    expect(container.textContent).toContain('Listening');
  });

  it('shows explicit Off and recording-paused VAD states', async () => {
    vadSensitivity = 0;
    await render();
    expect(container.textContent).toContain('Off · all audio kept');

    vadSensitivity = 60;
    dictationBusy = true;
    await render();
    expect(container.textContent).toContain('Paused while recording');
  });
});
