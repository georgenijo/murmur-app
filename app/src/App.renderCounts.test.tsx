import { act, memo } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from './App';
import { DEFAULT_SETTINGS, ONBOARDING_KEY, STORAGE_KEY } from './lib/settings';
import { IDLE_MEETING_STATUS } from './lib/meetings';

const native = vi.hoisted(() => ({
  invoke: vi.fn(),
  listeners: new Map<string, Set<(event: { payload: unknown }) => void>>(),
  counts: {} as Record<string, number>,
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

// Counting wrappers: keep the real component and count how often React renders
// it. A memo-wrapped export keeps its own boundary: the counter is wrapped in a
// memo with the same comparison, so it only runs when the real component would.
type AnyComponent = React.ComponentType<object> & { type?: React.ComponentType<object>; compare?: never };
function counted(name: string, Real: unknown) {
  const real = Real as AnyComponent;
  const inner = real.type ?? real;
  const Counter = (props: object) => {
    native.counts[name] = (native.counts[name] ?? 0) + 1;
    const Component = inner as React.ComponentType<object>;
    return <Component {...props} />;
  };
  return real.type ? memo(Counter, real.compare ?? undefined) : Counter;
}
vi.mock('./components/history/HistoryPanel', async (importOriginal) => {
  const real = await importOriginal<typeof import('./components/history/HistoryPanel')>();
  return { ...real, HistoryPanel: counted('HistoryPanel', real.HistoryPanel) };
});
vi.mock('./components/home/HomeSidebar', async (importOriginal) => {
  const real = await importOriginal<typeof import('./components/home/HomeSidebar')>();
  return { ...real, HomeSidebar: counted('HomeSidebar', real.HomeSidebar) };
});
vi.mock('./components/MainHeader', async (importOriginal) => {
  const real = await importOriginal<typeof import('./components/MainHeader')>();
  return { ...real, MainHeader: counted('MainHeader', real.MainHeader) };
});
vi.mock('./components/PermissionsBanner', async (importOriginal) => {
  const real = await importOriginal<typeof import('./components/PermissionsBanner')>();
  return { ...real, PermissionsBanner: counted('PermissionsBanner', real.PermissionsBanner) };
});
vi.mock('./components/home/HomeRecordingBar', async (importOriginal) => {
  const real = await importOriginal<typeof import('./components/home/HomeRecordingBar')>();
  return { ...real, HomeRecordingBar: counted('HomeRecordingBar', real.HomeRecordingBar) };
});
vi.mock('./components/home/HomeDashboard', async (importOriginal) => {
  const real = await importOriginal<typeof import('./components/home/HomeDashboard')>();
  return { ...real, HomeDashboard: counted('HomeDashboard', real.HomeDashboard) };
});

let container: HTMLDivElement;
let root: Root;

function response(command: string): unknown {
  switch (command) {
    case 'check_specific_model_exists': return true;
    case 'check_accessibility_permission': return true;
    case 'check_microphone_permission_status': return 'granted';
    case 'get_model_runtime_catalog': return [{
      generation: 1, modelName: 'base.en', label: 'Whisper Base', size: '142 MB',
      backend: 'whisper', accelerator: 'cpu', supportedPlatforms: ['macos'], supported: true,
      unavailableReason: null, installState: 'installed', lifecycleState: 'unloaded', failurePresent: false,
      capabilities: { partialResults: false, initialPrompts: true, multilingual: false,
        translation: false, timestamps: true, confidence: false, punctuationControl: false },
    }];
    case 'get_meeting_audio_capture_busy': return false;
    case 'get_meeting_status': return IDLE_MEETING_STATUS;
    case 'get_system_audio_permission_status': return 'granted';
    case 'get_calendar_permission_status': return 'granted';
    case 'list_meetings': return { sessions: [], searchMatches: {}, total: 0, offset: 0, limit: 50 };
    case 'configure_dictation': return { status: 'success', state: 'idle', message: '' };
    case 'list_running_applications': return [];
    default: return undefined;
  }
}

beforeEach(() => {
  // The recording clock re-renders the header once per second, so a loaded machine that
  // takes over a second for the sample loop would see one extra render that is not caused
  // by an audio level. Freeze only the interval; the rest of the timers stay real.
  vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval'] });
  // App loads the settings chunk on mount and preloads the meetings chunk when idle (a
  // 300 ms timer where jsdom lacks requestIdleCallback), and re-renders when each lands.
  // That is startup work, not a per-sample render, so run it up front instead of whenever
  // the chunk or timer happens to finish.
  vi.stubGlobal('requestIdleCallback', (callback: () => void) => window.setTimeout(callback, 0));
  vi.stubGlobal('cancelIdleCallback', (handle: number) => window.clearTimeout(handle));
  localStorage.clear();
  native.listeners.clear();
  native.counts = {};
  native.invoke.mockReset().mockImplementation(async (command: string) => response(command));
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});
afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

// Wait until no counted component has rendered for a few consecutive ticks.
async function settle() {
  let last = '';
  let stable = 0;
  for (let tick = 0; tick < 200 && stable < 5; tick += 1) {
    await act(async () => { await new Promise((resolve) => setTimeout(resolve, 20)); });
    const now = JSON.stringify(native.counts);
    stable = now === last ? stable + 1 : 0;
    last = now;
  }
}

describe('App render counts while recording with the window visible', () => {
  it('keeps level-independent children from re-rendering on each audio-level sample', async () => {
    localStorage.setItem(ONBOARDING_KEY, 'true');
    localStorage.setItem(STORAGE_KEY, JSON.stringify({
      ...DEFAULT_SETTINGS, model: 'base.en', recordingMode: 'double_tap', microphoneIdMigrationComplete: true,
    }));
    // Load the deferred chunks before rendering, so they resolve as soon as App asks.
    await Promise.all([
      import('./components/settings/SettingsPanel'),
      import('./components/history/MeetingsPanel'),
    ]);
    await act(async () => root.render(<App />));
    await settle();
    await act(async () => {
      for (const cb of native.listeners.get('recording-status-changed') ?? []) cb({ payload: 'recording' });
    });
    await settle();
    expect(document.visibilityState).toBe('visible');
    const before = { ...native.counts };
    const SAMPLES = 100;
    for (let i = 0; i < SAMPLES; i += 1) {
      await act(async () => {
        for (const cb of native.listeners.get('audio-level') ?? []) cb({ payload: 0.01 + (i % 7) * 0.003 });
      });
    }
    const delta = Object.fromEntries(Object.keys(native.counts).map((k) => [k, native.counts[k] - (before[k] ?? 0)]));
    // Components fed the level itself must follow every sample.
    expect(delta.MainHeader).toBe(SAMPLES);
    expect(delta.HomeRecordingBar).toBe(SAMPLES);
    // Components with no dependency on the level must stay out of its way.
    expect(delta.PermissionsBanner ?? 0).toBe(0);
    expect(delta.HistoryPanel ?? 0).toBe(0);
  });
});
