import { act, memo } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { MicrophonePreviewStatus } from '../../lib/microphonePreview';
import type { Settings } from '../../lib/settings';

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
  { id: 'built-in', name: 'Built-in Microphone', kind: 'builtIn' as const, connected: true, hasInput: true },
  { id: 'usb', name: 'USB Microphone', kind: 'external' as const, connected: true, hasInput: true },
  { id: 'unknown-input', name: 'Legacy Audio Input', kind: 'unknown' as const, connected: true, hasInput: true },
  { id: 'speakers', name: 'Built-in Speakers', kind: 'builtIn' as const, connected: true, hasInput: false },
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
  let defaultInputId = 'usb';
  let activePage = true;
  let surfaceActive = true;
  let appReady = true;
  let vadSensitivity = 60;
  let dictationBusy = false;
  let inventoryAvailable = true;
  let inventoryLoading = false;
  let missingDevice = false;
  let renderedDevices = [...devices];
  let includeSmartAuto = false;
  let smartAuto: Pick<Settings, 'smartAutoMicrophoneEnabled' | 'smartAutoProbeEnabled' | 'smartAutoApprovedDeviceIds' | 'smartAutoPreferredDeviceIds' | 'smartAutoAllowContinuity'>;
  let handleSmartAutoChange: ReturnType<typeof vi.fn<(updates: Partial<Settings>) => void>>;
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
            devices={renderedDevices}
            defaultInputId={defaultInputId}
            active={activePage}
            ready={appReady}
            vadSensitivity={vadSensitivity}
            dictationBusy={dictationBusy}
            missingDevice={missingDevice}
            inventoryAvailable={inventoryAvailable}
            inventoryLoading={inventoryLoading}
            lidState="open"
            onChange={handleMicrophoneChange}
            {...(includeSmartAuto ? { smartAuto, onSmartAutoChange: handleSmartAutoChange } : {})}
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

  function microphoneDialog(): HTMLElement | null {
    return document.querySelector('[role="dialog"][aria-label="Choose microphone mode"]');
  }

  function microphoneMode(label: string): HTMLInputElement {
    const radio = Array.from(microphoneDialog()?.querySelectorAll<HTMLInputElement>('input[type="radio"]') ?? [])
      .find((input) => input.parentElement?.querySelector('.font-medium')?.textContent === label);
    if (!radio) throw new Error(`Missing microphone mode: ${label}`);
    return radio;
  }

  beforeEach(async () => {
    vi.clearAllMocks();
    mocks.listeners.clear();
    selected = 'system_default';
    defaultInputId = 'usb';
    activePage = true;
    surfaceActive = true;
    appReady = true;
    vadSensitivity = 60;
    dictationBusy = false;
    inventoryAvailable = true;
    inventoryLoading = false;
    missingDevice = false;
    renderedDevices = [...devices];
    includeSmartAuto = false;
    smartAuto = {
      smartAutoMicrophoneEnabled: true,
      smartAutoProbeEnabled: false,
      smartAutoApprovedDeviceIds: ['usb', 'built-in', 'missing-device'],
      smartAutoPreferredDeviceIds: ['usb', 'built-in'],
      smartAutoAllowContinuity: false,
    };
    handleSmartAutoChange = vi.fn();
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
      if (command === 'verify_microphone_preview_signal') return 'verified';
      if (command === 'retry_smart_auto_probe') return 1;
      if (command === 'get_smart_auto_microphone_status') {
        return { state: 'blocked', message: 'No included microphone has recent signal.' };
      }
      throw new Error(`unexpected command: ${command}`);
    });
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    vi.useRealTimers();
    vi.unstubAllGlobals();
    container.remove();
  });

  it('uses the live meter without a separate signal verification action', async () => {
    includeSmartAuto = true;
    await render();
    expect(container.textContent).not.toContain('Verify signal');
    expect(container.textContent).not.toContain('Verify preview candidate');
    expect(container.querySelector('[role="meter"]')).toBeTruthy();
    expect(container.textContent).toContain('Live input from USB Microphone');
    expect(mocks.invoke).not.toHaveBeenCalledWith('verify_microphone_preview_signal', expect.anything());
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

  it('shows that automatic mode follows the currently resolved macOS input', async () => {
    await render();
    const selector = container.querySelector('[aria-label="Microphone input"]') as HTMLButtonElement;
    expect(selector.textContent).toContain('Follow macOS Default — USB Microphone');
    const helper = document.getElementById(selector.getAttribute('aria-describedby') ?? '');
    expect(helper?.textContent).toContain('Following macOS: USB Microphone');
    expect(helper?.textContent).toContain('next recording');

    defaultInputId = 'built-in';
    await render();
    expect(selector.textContent).toContain('Follow macOS Default — Built-in Microphone');
    expect(selected).toBe('system_default');
  });

  it('reveals Smart Auto inclusion controls in context with explicit saved states', async () => {
    includeSmartAuto = true;
    await render();

    const picker = container.querySelector('[aria-label="Microphone input"]') as HTMLButtonElement;
    expect(picker.textContent).toContain('Smart Auto · Available: USB Microphone');
    const submenu = container.querySelector('[aria-label="Smart Auto microphone inclusion"]') as HTMLFieldSetElement;
    expect(submenu.closest('[data-expanded]')?.getAttribute('data-expanded')).toBe('true');
    expect(submenu.textContent).not.toContain('Built-in Speakers');
    expect(submenu.textContent).not.toContain('Legacy Audio Input');
    expect(submenu.textContent).toContain('Built-in MicrophoneIncluded');
    expect(submenu.textContent).toContain('USB MicrophoneIncludedCandidate');
    expect(submenu.textContent).toContain('Disconnected microphone 1Included');
    const approvals = Array.from(submenu.querySelectorAll('input[type="checkbox"]')) as HTMLInputElement[];
    expect(approvals.some((approval) => approval.checked)).toBe(true);
    expect(submenu.textContent).toContain('Its saved preference is kept. Uncheck to forget it.');
    expect(submenu.querySelector('.settings-microphone-active-badge')?.textContent).toBe('Candidate');
    const backgroundChecks = submenu.querySelector('[aria-label="Check included microphones in the background"]') as HTMLElement;
    expect(backgroundChecks.getAttribute('aria-checked')).toBe('false');
    expect(submenu.textContent).toContain('Briefly checks included inputs while Murmur is idle');
    expect(submenu.textContent).toContain('never transcribed or saved');
    await act(async () => backgroundChecks.click());
    expect(handleSmartAutoChange).toHaveBeenCalledWith({ smartAutoProbeEnabled: true });
    const unavailableApproval = Array.from(submenu.querySelectorAll('label')).find((label) => label.textContent?.includes('saved preference'))?.querySelector('input') as HTMLInputElement;
    await act(async () => unavailableApproval.click());
    expect(handleSmartAutoChange).toHaveBeenCalledWith({
      smartAutoApprovedDeviceIds: ['usb', 'built-in'],
      smartAutoPreferredDeviceIds: ['usb', 'built-in'],
    });

    const preferBuiltIn = submenu.querySelector('[aria-label="Prefer Built-in Microphone for Smart Auto"]') as HTMLButtonElement;
    await act(async () => preferBuiltIn.click());
    expect(handleSmartAutoChange).toHaveBeenCalledWith({ smartAutoPreferredDeviceIds: ['built-in', 'usb'] });
  });

  it('explains when automatic checks are required without offering a manual check', async () => {
    includeSmartAuto = true;
    await render();

    expect(container.textContent).toContain('Live input from USB Microphone.');
    expect(container.textContent).toContain('Smart Auto is not ready. Turn on Background signal checks');
    expect(container.textContent).not.toContain('Verify preview candidate');
    expect(mocks.invoke).toHaveBeenCalledWith('start_microphone_preview', {
      deviceId: 'system_default',
      vadSensitivity: 60,
      smartAuto: {
        approvedDeviceIds: ['usb', 'built-in', 'missing-device'],
        preferredDeviceIds: ['usb', 'built-in'],
        allowContinuity: false,
      },
    });

    const pinButton = Array.from(container.querySelectorAll('button'))
      .find((button) => button.textContent === 'Use USB Microphone only');
    await act(async () => pinButton?.click());
    expect(handleSmartAutoChange).toHaveBeenCalledWith({ smartAutoMicrophoneEnabled: false });
    expect(mocks.invoke).toHaveBeenCalledWith('stop_microphone_preview', { previewId: 7 });
    expect(selected).toBe('usb');
  });

  it('renders an automatic probe separately from the visible manual preview', async () => {
    includeSmartAuto = true;
    smartAuto = { ...smartAuto, smartAutoProbeEnabled: true };
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === 'get_microphone_preview_status') return idle;
      if (command === 'start_microphone_preview') return active;
      if (command === 'update_microphone_preview_vad_sensitivity') return true;
      if (command === 'cancel_microphone_preview') return true;
      if (command === 'get_smart_auto_microphone_status') {
        return { state: 'probing', deviceId: 'built-in', phase: 'verifying', remainingMs: 4_000 };
      }
      throw new Error(`unexpected command: ${command}`);
    });

    await render();

    expect(container.textContent).toContain('Live input from USB Microphone.');
    expect(container.textContent).toContain('Checking Built-in Microphone: checking signal.');
    expect(container.textContent).toContain('not transcribed or saved');
    expect(container.textContent).not.toContain('Next capture ready');
  });

  it('rereads the backend when evidence expires and shows a different fresh candidate', async () => {
    vi.useFakeTimers();
    includeSmartAuto = true;
    let statusCalls = 0;
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === 'get_microphone_preview_status') return idle;
      if (command === 'start_microphone_preview') return active;
      if (command === 'update_microphone_preview_vad_sensitivity') return true;
      if (command === 'cancel_microphone_preview') return true;
      if (command === 'get_smart_auto_microphone_status') {
        statusCalls += 1;
        return statusCalls === 1
          ? { state: 'ready', deviceId: 'built-in', reason: 'previous_verified_rollback', validForMs: 1_000 }
          : { state: 'ready', deviceId: 'usb', reason: 'preferred_approved', validForMs: 5_000 };
      }
      throw new Error(`unexpected command: ${command}`);
    });
    await render();

    expect(container.textContent).toContain('Smart Auto · Available: USB Microphone');
    expect(container.textContent).toContain('Smart Auto will use Built-in Microphone.');
    expect(container.textContent).toContain('Why: previously active with recent signal');

    await act(async () => vi.advanceTimersByTimeAsync(1_000));
    expect(container.textContent).toContain('Smart Auto will use USB Microphone.');
    expect(container.textContent).toContain('Why: your preferred included microphone');
    expect(mocks.invoke.mock.calls.filter(([command]) => command === 'get_smart_auto_microphone_status')).toHaveLength(2);
    vi.useRealTimers();
  });

  it('explains why Smart Auto cannot run when every microphone is excluded', async () => {
    includeSmartAuto = true;
    smartAuto = {
      ...smartAuto,
      smartAutoApprovedDeviceIds: [],
      smartAutoPreferredDeviceIds: [],
    };
    await render();

    expect(container.textContent).toContain('Built-in MicrophoneExcluded');
    expect(container.textContent).toContain('USB MicrophoneExcluded');
    expect(container.textContent).toContain('No microphones are included. Include at least one available input for Smart Auto.');
    expect(container.textContent).not.toContain('Retry background checks');
    expect(mocks.invoke).not.toHaveBeenCalledWith('start_microphone_preview', expect.anything());
  });

  it('retains disconnected inclusion preferences without treating them as eligible', async () => {
    includeSmartAuto = true;
    smartAuto = {
      ...smartAuto,
      smartAutoApprovedDeviceIds: ['missing-device'],
      smartAutoPreferredDeviceIds: ['missing-device'],
    };
    await render();

    expect(container.textContent).toContain('Disconnected microphone 1Included');
    expect(container.textContent).toContain('Its saved preference is kept.');
    expect(container.textContent).toContain('None of the included microphones are available right now.');
    expect(smartAuto.smartAutoApprovedDeviceIds).toEqual(['missing-device']);
  });

  it('refreshes Smart Auto status after inventory, preview, and configuration changes', async () => {
    includeSmartAuto = true;
    await render();
    const initialCalls = mocks.invoke.mock.calls.filter(([command]) => command === 'get_smart_auto_microphone_status').length;

    await act(async () => {
      mocks.listeners.get('audio-input-inventory-changed')?.({ payload: {} });
      await Promise.resolve();
    });
    await emitStatus(active);
    await act(async () => {
      mocks.listeners.get('smart-auto-microphone-changed')?.({ payload: {} });
      await Promise.resolve();
    });
    smartAuto = { ...smartAuto, smartAutoPreferredDeviceIds: ['built-in', 'usb'] };
    await render();

    expect(mocks.invoke.mock.calls.filter(([command]) => command === 'get_smart_auto_microphone_status').length)
      .toBeGreaterThanOrEqual(initialCalls + 4);
    expect(mocks.invoke).toHaveBeenCalledWith('stop_microphone_preview', { previewId: 7 });
    expect(container.textContent).toContain('Smart Auto · Available: Built-in Microphone');
  });

  it('commits a different microphone and closes through the shared popup transition', async () => {
    includeSmartAuto = true;
    smartAuto = { ...smartAuto, smartAutoMicrophoneEnabled: false };
    await render();
    const picker = container.querySelector('[aria-label="Microphone input"]') as HTMLButtonElement;

    await act(async () => picker.click());
    expect(microphoneDialog()?.className).toContain('data-ending-style:opacity-0');
    await act(async () => microphoneMode('USB Microphone').click());

    expect(selected).toBe('usb');
    expect(picker.getAttribute('aria-expanded')).toBe('false');
    expect(mocks.invoke).toHaveBeenCalledWith('stop_microphone_preview', { previewId: 7 });
  });

  it('closes when the selected microphone row is pressed without restarting its preview', async () => {
    includeSmartAuto = true;
    smartAuto = { ...smartAuto, smartAutoMicrophoneEnabled: false };
    await render();
    const picker = container.querySelector('[aria-label="Microphone input"]') as HTMLButtonElement;

    await act(async () => picker.click());
    expect(microphoneMode('Follow macOS Default').checked).toBe(true);
    await act(async () => microphoneMode('Follow macOS Default').click());

    expect(picker.getAttribute('aria-expanded')).toBe('false');
    expect(selected).toBe('system_default');
    expect(mocks.invoke).not.toHaveBeenCalledWith('stop_microphone_preview', expect.anything());
  });

  it('toggles an open picker closed from the field and chevron trigger', async () => {
    includeSmartAuto = true;
    await render();
    const picker = container.querySelector('[aria-label="Microphone input"]') as HTMLButtonElement;

    await act(async () => picker.click());
    expect(picker.getAttribute('aria-expanded')).toBe('true');
    await act(async () => picker.click());

    expect(picker.getAttribute('aria-expanded')).toBe('false');
  });

  it('closes on an outside press without taking focus from the outside control', async () => {
    includeSmartAuto = true;
    await render();
    const picker = container.querySelector('[aria-label="Microphone input"]') as HTMLButtonElement;
    const outside = document.createElement('button');
    document.body.appendChild(outside);

    await act(async () => picker.click());
    await act(async () => {
      outside.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true }));
      outside.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }));
      outside.click();
      outside.focus();
    });

    expect(picker.getAttribute('aria-expanded')).toBe('false');
    expect(document.activeElement).toBe(outside);
    outside.remove();
  });

  it('closes on Escape and returns focus to the picker', async () => {
    includeSmartAuto = true;
    await render();
    const picker = container.querySelector('[aria-label="Microphone input"]') as HTMLButtonElement;

    await act(async () => picker.click());
    await act(async () => microphoneMode('Smart Auto').focus());
    await act(async () => document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })));

    expect(picker.getAttribute('aria-expanded')).toBe('false');
    expect(document.activeElement).toBe(picker);
  });

  it('selects Smart Auto, closes the picker, and reveals its inclusion submenu', async () => {
    includeSmartAuto = true;
    smartAuto = { ...smartAuto, smartAutoMicrophoneEnabled: false };
    await render();
    const picker = container.querySelector('[aria-label="Microphone input"]') as HTMLButtonElement;

    await act(async () => picker.click());
    await act(async () => microphoneMode('Smart Auto').click());
    expect(handleSmartAutoChange).toHaveBeenCalledWith({ smartAutoMicrophoneEnabled: true });
    expect(picker.getAttribute('aria-expanded')).toBe('false');

    smartAuto = { ...smartAuto, smartAutoMicrophoneEnabled: true };
    await render();
    const submenu = container.querySelector('[aria-label="Smart Auto microphone inclusion"]') as HTMLFieldSetElement;
    expect(submenu.closest('[data-expanded]')?.getAttribute('data-expanded')).toBe('true');
  });

  it('closes the picker when capture makes its controls unavailable', async () => {
    includeSmartAuto = true;
    await render();
    await act(async () => (container.querySelector('[aria-label="Microphone input"]') as HTMLButtonElement).click());
    expect(microphoneDialog()).toBeTruthy();

    inventoryAvailable = false;
    await render();
    const picker = container.querySelector('[aria-label="Microphone input"]') as HTMLButtonElement;
    expect(picker.getAttribute('aria-expanded')).toBe('false');
    expect(picker.disabled).toBe(true);
  });

  it('restores picker focus after an active preview switches microphones', async () => {
    includeSmartAuto = true;
    let finishStop: ((status: MicrophonePreviewStatus) => void) | null = null;
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === 'get_microphone_preview_status') return idle;
      if (command === 'start_microphone_preview') return active;
      if (command === 'stop_microphone_preview') {
        return new Promise<MicrophonePreviewStatus>((resolve) => { finishStop = resolve; });
      }
      if (command === 'update_microphone_preview_vad_sensitivity') return true;
      if (command === 'cancel_microphone_preview') return true;
      throw new Error(`unexpected command: ${command}`);
    });
    await render();
    await emitStatus(active);
    const picker = container.querySelector('[aria-label="Microphone input"]') as HTMLButtonElement;
    await act(async () => picker.click());
    const builtInRadio = microphoneMode('Built-in Microphone');
    await act(async () => builtInRadio.click());

    expect(picker.disabled).toBe(true);
    expect(picker.getAttribute('aria-expanded')).toBe('false');
    await act(async () => finishStop?.(idle));
    expect(picker.disabled).toBe(false);
    expect(document.activeElement).toBe(picker);
  });

  it('does not steal focus when the user moves on during microphone teardown', async () => {
    includeSmartAuto = true;
    let finishStop: ((status: MicrophonePreviewStatus) => void) | null = null;
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === 'get_microphone_preview_status') return idle;
      if (command === 'start_microphone_preview') return active;
      if (command === 'stop_microphone_preview') {
        return new Promise<MicrophonePreviewStatus>((resolve) => { finishStop = resolve; });
      }
      if (command === 'update_microphone_preview_vad_sensitivity') return true;
      if (command === 'cancel_microphone_preview') return true;
      throw new Error(`unexpected command: ${command}`);
    });
    await render();
    await emitStatus(active);
    const picker = container.querySelector('[aria-label="Microphone input"]') as HTMLButtonElement;
    await act(async () => picker.click());
    const builtInRadio = microphoneMode('Built-in Microphone');
    await act(async () => builtInRadio.click());

    const nextControl = document.createElement('button');
    document.body.appendChild(nextControl);
    nextControl.focus();
    await act(async () => finishStop?.(idle));
    expect(document.activeElement).toBe(nextControl);
    nextControl.remove();
  });

  it('keeps a saved unknown-kind input available for manual selection', async () => {
    includeSmartAuto = true;
    selected = 'unknown-input';
    smartAuto = { ...smartAuto, smartAutoMicrophoneEnabled: false };
    await render();

    const picker = container.querySelector('[aria-label="Microphone input"]') as HTMLButtonElement;
    expect(picker.textContent).toContain('Legacy Audio Input');
    await act(async () => picker.click());
    expect(microphoneDialog()?.querySelectorAll('input[type="radio"]')).toHaveLength(5);
    const dialog = microphoneDialog() as HTMLElement;
    expect(dialog.textContent?.match(/Legacy Audio Input/g)).toHaveLength(1);
  });

  it('disambiguates duplicate microphone names in manual and Smart Auto controls', async () => {
    includeSmartAuto = true;
    renderedDevices = [
      { id: 'usb-a', name: 'USB Microphone', kind: 'external', connected: true, hasInput: true },
      { id: 'usb-b', name: 'USB Microphone', kind: 'external', connected: true, hasInput: true },
    ];
    defaultInputId = 'usb-a';
    smartAuto = {
      ...smartAuto,
      smartAutoApprovedDeviceIds: ['usb-a', 'usb-b'],
      smartAutoPreferredDeviceIds: ['usb-a'],
    };
    await render();

    await act(async () => (container.querySelector('[aria-label="Microphone input"]') as HTMLButtonElement).click());
    expect(container.textContent).toContain('USB Microphone (usb-a)');
    expect(container.textContent).toContain('USB Microphone (usb-b)');
    expect(container.querySelector('[aria-label="Prefer USB Microphone (usb-a) for Smart Auto"]')).toBeTruthy();
    expect(container.querySelector('[aria-label="Prefer USB Microphone (usb-b) for Smart Auto"]')).toBeTruthy();
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

  it('does not reopen a terminally failed preview until an explicit retry', async () => {
    await render();
    await emitStatus({
      previewId: null,
      state: 'error',
      stillConnecting: false,
      errorKind: 'unsupported_config',
      message: 'The microphone format is unsupported.',
    });
    await emitStatus(idle);

    vadSensitivity = 55;
    await render();
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(mocks.invoke.mock.calls.filter(([command]) => command === 'start_microphone_preview')).toHaveLength(1);
    expect(mocks.invoke.mock.calls.filter(([command]) => command === 'stop_microphone_preview')).toHaveLength(0);
    expect(mocks.invoke.mock.calls.filter(([command]) => command === 'cancel_microphone_preview')).toHaveLength(0);
    expect(container.textContent).toContain('The microphone format is unsupported.');
    expect(container.textContent).toContain('Voice detection · 55%');
    expect(container.textContent).toContain('Unavailable');
    expect(container.textContent).not.toContain('Listening…');

    const retry = Array.from(container.querySelectorAll('button'))
      .find((button) => button.textContent === 'Retry microphone preview');
    await act(async () => retry?.click());
    expect(mocks.invoke.mock.calls.filter(([command]) => command === 'start_microphone_preview')).toHaveLength(2);
  });

  it('labels a stopping preview without calling it started or active', async () => {
    await render();
    await emitStatus({
      previewId: 7,
      state: 'stopping',
      stillConnecting: false,
      errorKind: null,
      message: null,
    });

    expect(container.textContent).toContain('Stopping…');
    expect(container.textContent).not.toContain('Starting…');
    expect(container.textContent).not.toContain('Listening…');
  });

  it('confirms teardown before persisting and reopening a switched device', async () => {
    await render();
    await emitStatus(active);
    const combobox = container.querySelector('[role="combobox"]') as HTMLButtonElement;
    await act(async () => combobox.click());
    const option = Array.from(container.querySelectorAll('[role="option"]'))
      .find((item) => item.textContent?.trim() === 'USB Microphone') as HTMLElement;
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
