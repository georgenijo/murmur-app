import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from './App';
import { DEFAULT_SETTINGS, ONBOARDING_KEY, STORAGE_KEY } from './lib/settings';
import { IDLE_MEETING_STATUS } from './lib/meetings';
import type { ModelRuntimeSnapshot } from './lib/modelRuntime';
import type { DictationResponse } from './lib/dictation';

// Keep App, its UI, and all capture hooks real. Only native IPC is replaced.
const native = vi.hoisted(() => ({
  invoke: vi.fn(),
  listeners: new Map<string, Set<(event: { payload: unknown }) => void>>(),
  modelInstalled: true,
  captureBusy: false,
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: native.invoke, isTauri: () => false }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, callback: (event: { payload: unknown }) => void) => {
    const callbacks = native.listeners.get(name) ?? new Set();
    callbacks.add(callback);
    native.listeners.set(name, callbacks);
    return () => { callbacks.delete(callback); };
  }),
  emit: vi.fn(async () => {}),
  emitTo: vi.fn(async () => {}),
}));
vi.mock('@tauri-apps/plugin-autostart', () => ({
  enable: vi.fn(async () => {}), disable: vi.fn(async () => {}), isEnabled: vi.fn(async () => false),
}));
vi.mock('@tauri-apps/api/app', () => ({ getVersion: vi.fn(async () => '0.59.5') }));
vi.mock('@tauri-apps/plugin-updater', () => ({ check: vi.fn(async () => null) }));
vi.mock('@tauri-apps/plugin-notification', () => ({
  isPermissionGranted: vi.fn(async () => false), requestPermission: vi.fn(async () => 'denied'), sendNotification: vi.fn(),
}));

let container: HTMLDivElement;
let root: Root;

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: Error) => void;
  const promise = new Promise<T>((accept, fail) => { resolve = accept; reject = fail; });
  return { promise, resolve, reject };
}

function modelSnapshot(): ModelRuntimeSnapshot {
  return {
    generation: 1, modelName: 'base.en', label: 'Whisper Base', size: '142 MB',
    backend: 'whisper', accelerator: 'cpu', supportedPlatforms: ['macos'], supported: true,
    unavailableReason: null, installState: native.modelInstalled ? 'installed' : 'notInstalled',
    lifecycleState: 'unloaded', failurePresent: false,
    capabilities: { partialResults: false, initialPrompts: true, multilingual: false,
      translation: false, timestamps: true, confidence: false, punctuationControl: false },
  };
}

function response(command: string): unknown {
  switch (command) {
    case 'check_specific_model_exists': return native.modelInstalled;
    case 'check_accessibility_permission': return true;
    case 'check_microphone_permission_status': return 'granted';
    case 'get_model_runtime_catalog': return [modelSnapshot()];
    case 'get_meeting_audio_capture_busy': return native.captureBusy;
    case 'get_meeting_status': return IDLE_MEETING_STATUS;
    case 'get_system_audio_permission_status': return 'granted';
    case 'get_calendar_permission_status': return 'granted';
    case 'list_meetings': return { sessions: [], searchMatches: {}, total: 0, offset: 0, limit: 50 };
    case 'configure_dictation': return { status: 'success', state: 'idle', message: '' };
    case 'start_native_recording': return { status: 'success', state: 'recording', recordingId: 1, message: '' };
    case 'stop_native_recording': return { status: 'success', state: 'idle', message: '' };
    case 'list_running_applications': return [];
    default: return undefined;
  }
}

async function settle() {
  await act(async () => { await new Promise((resolve) => setTimeout(resolve, 0)); });
}
async function mount({ setup = false, modelInstalled = true, recordingMode = 'double_tap' } = {}) {
  native.modelInstalled = modelInstalled;
  if (!setup) localStorage.setItem(ONBOARDING_KEY, 'true');
  localStorage.setItem(STORAGE_KEY, JSON.stringify({
    ...DEFAULT_SETTINGS, model: 'base.en', recordingMode, microphoneIdMigrationComplete: true,
  }));
  // A missing model prevents grandfathering for the initial setup fixture.
  if (setup) native.modelInstalled = false;
  await act(async () => root.render(<App />));
  await settle();
}
async function emit(name: string, payload: unknown) {
  await act(async () => {
    for (const callback of native.listeners.get(name) ?? []) callback({ payload });
  });
}
async function shortcut(key: string) {
  await act(async () => {
    window.dispatchEvent(new KeyboardEvent('keydown', { key, metaKey: true, bubbles: true, cancelable: true }));
  });
}
function button(label: string) {
  const result = Array.from(container.querySelectorAll('button')).find((node) => label === 'Download' ? node.textContent?.trim().startsWith('Download ·') : node.textContent?.trim() === label);
  if (!result) throw new Error(`Missing button: ${label}`);
  return result;
}
async function click(label: string) {
  await act(async () => button(label).click());
  await settle();
}
async function rerun(entry: 'settings' | 'palette') {
  if (entry === 'settings') {
    await shortcut(',');
    await click('Open…');
  } else {
    await shortcut('k');
    const command = Array.from(container.querySelectorAll<HTMLElement>('[role="option"]'))
      .find((node) => node.textContent?.includes('Re-run setup assistant'));
    expect(command).toBeDefined();
    await act(async () => command!.click());
    await settle();
  }
}
async function finishSetup() {
  native.modelInstalled = true;
  await click('Get started');
  await click('Continue'); // Microphone
  await click('Continue'); // Accessibility
  await click('Continue'); // System Audio
  await click('Continue'); // Calendar
  await click('Continue'); // Installed model
  await click('Continue'); // Hotkey
  await click('Start using Murmur');
}
function expectMain() {
  expect(container.querySelector('.main-app-shell')).not.toBeNull();
  expect(container.querySelector('.main-first-run-onboarding')).toBeNull();
}

beforeEach(() => {
  localStorage.clear();
  native.listeners.clear();
  native.captureBusy = false;
  native.modelInstalled = true;
  native.invoke.mockReset().mockImplementation(async (command: string) => response(command));
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});
afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
});

describe('App setup capture safety', () => {
  it.each([
    ['hold_down', 'idle', 'Start dictation', 'Hold'],
    ['hold_down', 'starting', 'Cancel microphone connection', undefined],
    ['hold_down', 'recording', 'Stop recording', 'Release'],
    ['double_tap', 'idle', 'Start dictation', 'Double-tap'],
    ['double_tap', 'starting', 'Cancel microphone connection', undefined],
    ['double_tap', 'recording', 'Stop recording', 'Tap'],
    ['both', 'idle', 'Start dictation', 'Hold'],
    ['both', 'starting', 'Cancel microphone connection', undefined],
    ['both', 'recording', 'Stop recording', undefined],
  ] as const)('shows the %s/%s palette gesture', async (recordingMode, status, title, gesture) => {
    await mount({ recordingMode });
    if (status !== 'idle') await emit('recording-status-changed', status);
    await shortcut('k');
    const command = container.querySelector('#command-recording-toggle');
    expect(command).not.toBeNull();
    expect(command?.firstElementChild?.textContent).toBe(title);
    if (gesture) {
      expect(command?.querySelector('.palette-binding > span')?.textContent).toBe(gesture);
      expect(command?.querySelector('kbd')?.textContent).toBe('⇧ Left Shift');
    } else {
      expect(command?.querySelector('.palette-binding')).toBeNull();
    }
  });

  it.each(['settings', 'palette'] as const)('enters setup through %s when idle', async (entry) => {
    await mount();
    await rerun(entry);
    expect(container.querySelector('.main-first-run-onboarding')).not.toBeNull();
    expect(localStorage.getItem(ONBOARDING_KEY)).toBeNull();
  });

  it.each(['settings', 'palette'] as const)('preserves dictation stop controls through %s', async (entry) => {
    await mount();
    await emit('recording-status-changed', 'recording');
    native.captureBusy = true;
    await rerun(entry);
    expectMain();
    expect(localStorage.getItem(ONBOARDING_KEY)).toBe('true');
    expect(container.textContent).toContain('Stop recording first');
    expect(native.listeners.get('double-tap-toggle')?.size).toBeGreaterThan(0);
    expect(native.listeners.get('escape-cancel')?.size).toBeGreaterThan(0);
    await emit('double-tap-toggle', null);
    expect(native.invoke).toHaveBeenCalledWith('stop_native_recording', { stopTrigger: 'toggle' });
  });

  it.each(['hold_down', 'double_tap', 'both'])('does not arm %s capture behind the missing-model screen', async (recordingMode) => {
    await mount({ modelInstalled: false, recordingMode });
    expect(container.querySelector('.main-app-shell')).toBeNull();
    expect(native.invoke.mock.calls.filter(([command]) => command === 'start_keyboard_listener')).toEqual([]);
    expect(native.invoke.mock.calls.filter(([command]) => command === 'start_keyboard_listener')).toEqual([]);
    await emit('hold-down-start', null);
    await emit('double-tap-toggle', null);
    expect(native.invoke.mock.calls.filter(([command]) => command === 'start_native_recording')).toEqual([]);
  });

  for (const entry of ['settings', 'palette'] as const) {
    it.each(['starting', 'recording'] as const)(`keeps %s meeting stop controls available through ${entry}`, async (phase) => {
      await mount();
      await emit('meeting-status-changed', { ...IDLE_MEETING_STATUS, generation: 1, phase, sessionId: 'meeting-1' });
      // Exercise the frontend's starting/recording check even before the native
      // ownership read catches up with the local Start action.
      await rerun(entry);
      expectMain();
      expect(container.textContent).toContain('Stop recording first');
      expect(localStorage.getItem(ONBOARDING_KEY)).toBe('true');
      await shortcut('k');
      const stop = container.querySelector<HTMLButtonElement>('#command-meeting-toggle');
      expect(stop?.textContent).toContain('Stop meeting');
      await act(async () => stop!.click());
      expect(native.invoke).toHaveBeenCalledWith('stop_meeting');
    });
  }

  it('keeps setup closed when the ownership check fails', async () => {
    await mount();
    native.invoke.mockImplementation(async (command: string) => {
      if (command === 'get_meeting_audio_capture_busy') throw new Error('unavailable');
      return response(command);
    });
    await rerun('settings');
    expectMain();
    expect(container.textContent).toContain('Could not check recording state');
    expect(localStorage.getItem(ONBOARDING_KEY)).toBe('true');
  });

  it('rechecks dictation that starts while the setup ownership read is pending', async () => {
    await mount();
    let resolveOwnership!: (busy: boolean) => void;
    const ownership = new Promise<boolean>((resolve) => { resolveOwnership = resolve; });
    native.invoke.mockImplementation(async (command: string) => command === 'get_meeting_audio_capture_busy'
      ? ownership : response(command));
    await rerun('settings');
    await emit('recording-status-changed', 'starting');
    await act(async () => resolveOwnership(false));
    expectMain();
    expect(container.textContent).toContain('Stop recording first');
    expect(localStorage.getItem(ONBOARDING_KEY)).toBe('true');
    await emit('escape-cancel', {});
    expect(native.invoke).toHaveBeenCalledWith('cancel_native_recording');
  });

  it.each(['hold_down', 'double_tap', 'both'])('treats an unconfirmed %s start as busy after an idle ownership snapshot', async (recordingMode) => {
    await mount({ recordingMode });
    const ownership = deferred<boolean>();
    const start = deferred<DictationResponse>();
    native.invoke.mockImplementation(async (command: string) => {
      if (command === 'get_meeting_audio_capture_busy') return ownership.promise;
      if (command === 'start_native_recording') return start.promise;
      return response(command);
    });
    await rerun('settings');
    await emit(recordingMode === 'hold_down' ? 'hold-down-start' : 'double-tap-toggle', null);
    expect(native.invoke).toHaveBeenCalledWith('start_native_recording', expect.any(Object));

    // Neither native confirmation nor a lifecycle event has reached JS yet.
    await act(async () => ownership.resolve(false));
    expectMain();
    expect(container.textContent).toContain('Stop recording first');
    expect(localStorage.getItem(ONBOARDING_KEY)).toBe('true');

    await emit('recording-status-changed', 'starting');
    await act(async () => start.resolve({ type: 'recording_starting', state: 'starting', recordingId: 1 }));
    await emit('recording-status-changed', 'recording');
    await emit('escape-cancel', {});
    expect(native.invoke).toHaveBeenCalledWith('cancel_native_recording');
  });

  it.each(['rejected', 'refused', 'confirmed'] as const)('settles pending admission when a start is %s', async (outcome) => {
    await mount();
    const start = deferred<DictationResponse>();
    native.invoke.mockImplementation(async (command: string) => command === 'start_native_recording'
      ? start.promise : response(command));
    await emit('double-tap-toggle', null);
    await act(async () => {
      if (outcome === 'rejected') start.reject(new Error('start failed'));
      else start.resolve(outcome === 'refused'
        ? { type: 'error', error: 'start refused' }
        : { type: 'recording_starting', state: 'starting', recordingId: 1 });
    });
    await rerun('settings');
    if (outcome === 'confirmed') {
      expectMain();
      expect(container.textContent).toContain('Stop recording first');
    } else {
      expect(container.querySelector('.main-first-run-onboarding')).not.toBeNull();
    }
  });

  it.each(['hold_down', 'double_tap', 'both'])('ignores a queued %s start once setup is admitted, then rearms after completion', async (recordingMode) => {
    await mount({ recordingMode });
    const ownership = deferred<boolean>();
    native.invoke.mockImplementation(async (command: string) => command === 'get_meeting_audio_capture_busy'
      ? ownership.promise : response(command));
    const event = recordingMode === 'hold_down' ? 'hold-down-start' : 'double-tap-toggle';
    const queuedCallbacks = [...native.listeners.get(event)!];
    expect(queuedCallbacks.length).toBeGreaterThan(0);
    await rerun('settings');
    await act(async () => {
      ownership.resolve(false);
      // Let admission settle while React still batches the setup render. The
      // queued callback retains the main surface's currently armed handler.
      await new Promise((resolve) => setTimeout(resolve, 0));
      expect(localStorage.getItem(ONBOARDING_KEY)).toBeNull();
      expect(container.querySelector('.main-app-shell')).not.toBeNull();
      for (const callback of queuedCallbacks) callback({ payload: null });
    });
    expect(container.querySelector('.main-first-run-onboarding')).not.toBeNull();
    for (const callback of queuedCallbacks) callback({ payload: null });
    expect(native.invoke.mock.calls.filter(([command]) => command === 'start_native_recording')).toEqual([]);
    await finishSetup();
    await emit(event, null);
    expect(native.invoke).toHaveBeenCalledWith('start_native_recording', expect.any(Object));
  });

  it.each([',', 'k', 'open-settings'])('ignores %s during setup and finishes on Home', async (action) => {
    await mount({ setup: true });
    expect(container.querySelector('.main-first-run-onboarding')).not.toBeNull();
    if (action === 'open-settings') await emit('open-settings', null);
    else await shortcut(action);
    await finishSetup();
    expectMain();
    expect(container.querySelector('[role="listbox"][aria-label="Commands"]')).toBeNull();
    expect(container.querySelector('main')?.getAttribute('aria-hidden')).toBe('false');
    // The same actions still work once the main surface is available.
    if (action === 'open-settings') await emit('open-settings', null);
    else await shortcut(action);
    if (action === 'k') expect(container.querySelector('[role="listbox"][aria-label="Commands"]')).not.toBeNull();
    else expect(container.querySelector('main')?.getAttribute('aria-hidden')).toBe('true');
  });

  it.each([',', 'k', 'open-settings'])('ignores %s during model download and finishes on Home', async (action) => {
    await mount({ modelInstalled: false });
    if (action === 'open-settings') await emit('open-settings', null);
    else await shortcut(action);
    await click('Download');
    expectMain();
    expect(container.querySelector('[role="listbox"][aria-label="Commands"]')).toBeNull();
    expect(container.querySelector('main')?.getAttribute('aria-hidden')).toBe('false');
    expect(native.invoke).toHaveBeenCalledWith('start_keyboard_listener', { hotkey: DEFAULT_SETTINGS.doubleTapKey, mode: 'double_tap' });
  });
});
