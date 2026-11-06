import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, expect, it, vi } from 'vitest';

const mock = vi.hoisted(() => ({
  invoke: vi.fn(), label: 'main', notices: [] as { label: string; name: string; payload: unknown }[],
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
}));
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => true, invoke: mock.invoke }));
vi.mock('@tauri-apps/api/webviewWindow', () => ({ getCurrentWebviewWindow: () => ({ label: mock.label }) }));
vi.mock('@tauri-apps/api/event', () => ({
  emitTo: vi.fn(async (label, name, payload) => { mock.notices.push({ label, name, payload }); }),
  listen: vi.fn(async (name, handler) => {
    const key = `${mock.label}:${name}`;
    mock.listeners.set(key, handler);
    return () => { mock.listeners.delete(key); };
  }),
}));
vi.mock('@tauri-apps/api/app', () => ({ setTheme: vi.fn(async () => {}) }));
vi.mock('@tauri-apps/plugin-autostart', () => ({ isEnabled: vi.fn(async () => false), enable: vi.fn(), disable: vi.fn() }));
vi.mock('./dictation', () => ({
  configure: vi.fn(async (_options, settlement) => { settlement?.onSuccess?.(); }),
  buildConfigureOptions: (settings: unknown) => settings,
}));

const roots: Root[] = [];
const windowListeners: [string, EventListenerOrEventListenerObject][] = [];
afterEach(async () => {
  for (const root of roots.splice(0)) await act(async () => root.unmount());
  for (const [name, handler] of windowListeners.splice(0)) window.removeEventListener(name, handler);
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  mock.listeners.clear(); mock.notices.length = 0;
  localStorage.clear(); document.body.innerHTML = '';
});

async function setup(cacheFails = false) {
  vi.resetModules();
  const { DEFAULT_SETTINGS } = await import('./settings');
  const { createAppearanceDocument } = await import('./appearance');
  const disk: Record<string, unknown> = {
    'dictation-settings': { ...DEFAULT_SETTINGS, settingsVersion: 7, autoPaste: true, hotkeyMissFeedback: true,
      overlayPlacement: 'notch', microphoneIdMigrationComplete: true, launchOnboardingComplete: true },
    'murmur-appearance': createAppearanceDocument('dark'),
    'murmur-theme-library': { version: 1, revision: 17, themes: [] },
  };
  mock.invoke.mockReset().mockImplementation(async (command: string, args?: { blob: string; key: string; value: unknown }) => {
    if (command === 'webkit_state_snapshot') return { v1: true, v2: true, values: structuredClone(disk) };
    if (command === 'load_settings_blob') return JSON.stringify(disk['dictation-settings']);
    if (command === 'save_settings_blob' || command === 'save_theme_library_blob') {
      const key = command === 'save_settings_blob' ? 'dictation-settings' : 'murmur-theme-library';
      disk[key] = JSON.parse(args!.blob); return JSON.stringify(disk[key]);
    }
    if (command === 'save_webkit_source') disk[args!.key] = args!.value;
  });
  const add = window.addEventListener.bind(window);
  vi.spyOn(window, 'addEventListener').mockImplementation((name, handler, options) => {
    if (handler) windowListeners.push([name, handler]);
    add(name, handler, options);
  });
  vi.spyOn(console, 'warn').mockImplementation(() => {});
  vi.stubGlobal('matchMedia', () => ({ matches: false, addEventListener: vi.fn(), removeEventListener: vi.fn() }));
  if (cacheFails) vi.spyOn(localStorage, 'setItem').mockImplementation(() => { throw new Error('synthetic quota'); });
  const old = structuredClone(disk);
  const sessions = [];
  for (const label of ['main', 'overlay', 'diagnostics']) {
    vi.resetModules(); mock.label = label;
    const lifecycle = await import('./webkitLifecycle');
    await lifecycle.initializeWebkitState();
    const storage = await import('./presentationStorage');
    const settings = await import('./settings');
    const appearance = await import('./appearance');
    const settingsHook = await import('./hooks/useSettings');
    const appearanceHook = await import('./hooks/useAppearance');
    const overlayHook = await import('./hooks/useOverlaySettingsMirror');
    const popoverHook = await import('./hooks/usePopoverAppearance');
    if (label === 'diagnostics') await import('../diagnostics-window');
    sessions.push({ label, lifecycle, storage, settings, appearance, settingsHook, appearanceHook, overlayHook, popoverHook });
  }
  return { disk, old, sessions };
}

function storageEvent(key: string | null, value: unknown = null) {
  window.dispatchEvent(new StorageEvent('storage', { key, newValue: value === null ? null : JSON.stringify(value) }));
}
async function deliverNotices() {
  for (const notice of mock.notices.splice(0).reverse()) {
    mock.listeners.get(`${notice.label}:${notice.name}`)?.({ payload: notice.payload });
  }
  await Promise.resolve(); await Promise.resolve();
}

it.each([false, true])('real main hooks and persistence propagate all sources despite stale events, cache failure=%s', async cacheFails => {
  const { sessions, old } = await setup(cacheFails);
  const main = sessions[0];
  mock.label = 'main';
  const { useSettings } = main.settingsHook;
  const { AppearanceProvider, useAppearance } = main.appearanceHook;
  const { flushSourceState } = main.lifecycle;
  let settings!: ReturnType<typeof useSettings>, appearance!: ReturnType<typeof useAppearance>;
  const root = createRoot(document.createElement('div')); roots.push(root);
  function Harness() { settings = useSettings(); appearance = useAppearance(); return null; }
  await act(async () => root.render(<AppearanceProvider><Harness /></AppearanceProvider>));
  mock.label = 'overlay';
  const { useOverlaySettingsMirror } = sessions[1].overlayHook;
  const { usePopoverAppearance } = sessions[1].popoverHook;
  let mirror!: ReturnType<typeof useOverlaySettingsMirror>;
  const args = { setDisabled: vi.fn(), setShowHotkeyMiss: vi.fn(), hotkeyMissFeedbackRef: { current: true } };
  const overlay = createRoot(document.createElement('div')); roots.push(overlay);
  function Overlay() { mirror = useOverlaySettingsMirror(args); usePopoverAppearance(); return null; }
  await act(async () => overlay.render(<Overlay />));
  expect(mirror.autoPaste).toBe(true);
  expect(document.documentElement.dataset.appearance).toBe('dark');
  const check = () => {
    for (const session of sessions) {
      expect(session.settings.loadSettings()).toMatchObject({ autoPaste: false, hotkeyMissFeedback: false, overlayPlacement: 'floating_pill' });
      expect(session.appearance.loadAppearanceDocument().document.mode).toBe('light');
      expect(session.appearance.loadThemeLibrary().document).toMatchObject({ revision: 18, themes: [{ label: 'Synthetic' }] });
    }
    expect(mirror.autoPaste).toBe(false);
    expect(mirror.overlayPlacement).toBe('floating_pill');
    expect(args.hotkeyMissFeedbackRef.current).toBe(false);
    expect(document.documentElement.dataset.appearance).toBe('light');
  };
  await act(async () => {
    settings.updateSettings({ autoPaste: false, hotkeyMissFeedback: false, overlayPlacement: 'floating_pill' });
    await appearance.setMode('light');
    await appearance.library.saveCurrent('Synthetic');
    await flushSourceState();
    await deliverNotices();
  });
  // With quota failures, no successful DOM write/event can repair a missed
  // notification: the committed typed hints must suffice on their own.
  check();
  await act(async () => {
    for (const [key, value] of Object.entries(old)) storageEvent(key, value);
    storageEvent(null);
  });
  check();
});

it.each(['dictation-settings', 'murmur-appearance', 'murmur-theme-library'])('a %s change during an in-flight snapshot is applied by the coalesced follow-up', async key => {
  const { sessions, disk, old } = await setup();
  const held: ((value: unknown) => void)[] = [];
  const normal = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation((command: string, args: unknown) => command === 'webkit_state_snapshot'
    ? new Promise(resolve => { held.push(resolve); }) : normal(command, args));
  storageEvent(key, old[key]);
  expect(held).toHaveLength(3);
  const older = held.splice(0);
  disk[key] = key === 'dictation-settings' ? { ...(disk[key] as object), autoPaste: false }
    : key === 'murmur-appearance' ? { ...(disk[key] as object), mode: 'light', revision: 9 }
    : { version: 1, revision: 19, themes: [] };
  const latest = structuredClone(disk);
  storageEvent(key, old[key]);
  expect(held).toHaveLength(0);
  await act(async () => {
    for (const resolve of older.reverse()) resolve({ v1: true, v2: true, values: old });
  });
  // The old response was invalidated by the later hint, and the next native
  // read starts only after it settles.
  expect(held).toHaveLength(3);
  for (const session of sessions) expect(JSON.parse(session.storage.presentationStorage.getItem(key)!)).toEqual(old[key]);
  await act(async () => {
    for (const resolve of held.splice(0)) resolve({ v1: true, v2: true, values: latest });
  });
  for (const session of sessions) expect(JSON.parse(session.storage.presentationStorage.getItem(key)!)).toEqual(latest[key]);
});

it('an async snapshot cannot overwrite a newer local intent or native notification', async () => {
  const { sessions, old } = await setup();
  const held: ((value: unknown) => void)[] = [];
  mock.invoke.mockImplementation((command: string) => command === 'webkit_state_snapshot'
    ? new Promise(resolve => { held.push(resolve); }) : Promise.reject(new Error('synthetic pending write')));
  storageEvent('dictation-settings', old['dictation-settings']);
  const newer = JSON.stringify({ ...(old['dictation-settings'] as object), autoPaste: false });
  sessions[0].lifecycle.sourceStorage.setItem('dictation-settings', newer);
  await expect(sessions[0].lifecycle.flushSourceState()).rejects.toThrow('Source storage unavailable');
  sessions[1].storage.presentationStorage.rememberItem('dictation-settings', newer);
  await act(async () => { for (const resolve of held) resolve({ v1: true, v2: true, values: old }); });
  expect(sessions[0].storage.presentationStorage.getItem('dictation-settings')).toBe(newer);
  expect(sessions[1].storage.presentationStorage.getItem('dictation-settings')).toBe(newer);
});

it('a reload started during a failed pending source intent preserves the retry and session value', async () => {
  const { sessions, old } = await setup(true);
  const main = sessions[0];
  const normal = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation((command: string, args: unknown) => command === 'save_webkit_source'
    ? Promise.reject(new Error('synthetic disk failure')) : normal(command, args));
  main.lifecycle.sourceStorage.setItem('murmur-appearance', JSON.stringify({ ...(old['murmur-appearance'] as object), mode: 'light' }));
  await expect(main.lifecycle.flushSourceState()).rejects.toThrow('Source storage unavailable');
  await act(async () => storageEvent('murmur-appearance', old['murmur-appearance']));
  expect(main.appearance.loadAppearanceDocument().document.mode).toBe('light');
  mock.invoke.mockImplementation(normal);
  await main.lifecycle.flushSourceState();
  await act(async () => deliverNotices());
  expect(sessions[2].appearance.loadAppearanceDocument().document.mode).toBe('light');
});

it.each(['storage', 'native'] as const)('1,000 %s hints per renderer use one active snapshot and one bounded follow-up', async transport => {
  const { sessions, disk } = await setup();
  const keys = [...sessions[0].lifecycle.V2_KEYS, 'murmur-overlay-vertical-offset', 'murmur-ui-latency-v1'];
  const held: ((value: unknown) => void)[] = [];
  mock.invoke.mockClear().mockImplementation(() => new Promise(resolve => { held.push(resolve); }));
  for (let i = 0; i < 1000; i++) {
    const key = keys[i % keys.length];
    if (transport === 'storage') storageEvent(key);
    else for (const session of sessions) {
      mock.listeners.get(`${session.label}:presentation-source-committed`)!({ payload: { key } });
    }
  }
  // Each independently imported renderer owns exactly one active read.
  expect(held).toHaveLength(3);
  expect(mock.invoke).toHaveBeenCalledTimes(3);
  disk['usage-dashboard-collapsed'] = false;
  await act(async () => {
    for (const resolve of held.splice(0)) resolve({ v1: true, v2: true, values: {} });
  });
  expect(held).toHaveLength(3);
  expect(mock.invoke).toHaveBeenCalledTimes(6);
  await act(async () => {
    for (const resolve of held.splice(0)) resolve({ v1: true, v2: true, values: structuredClone(disk) });
  });
  expect(held).toHaveLength(0);
  expect(mock.invoke).toHaveBeenCalledTimes(6);
  for (const session of sessions) {
    for (const key of keys) {
      expect(session.storage.presentationStorage.getItem(key)).toBe(key in disk ? JSON.stringify(disk[key]) : null);
    }
  }
});

it.each([false, true])('reload failure storms preserve state and emit one content-free diagnostic per renderer, cache failure=%s', async cacheFails => {
  const { sessions, old } = await setup(cacheFails);
  const warn = vi.mocked(console.warn);
  warn.mockClear();
  mock.invoke.mockClear().mockRejectedValue(new Error('synthetic private exception and values must not be logged'));
  for (let wave = 0; wave < 3; wave++) {
    await act(async () => {
      for (let i = 0; i < 1000; i++) storageEvent(null, { syntheticPrivate: 'must not be logged' });
    });
    for (const session of sessions) {
      for (const [key, value] of Object.entries(old)) {
        expect(session.storage.presentationStorage.getItem(key)).toBe(JSON.stringify(value));
      }
    }
  }
  expect(warn.mock.calls).toEqual(Array.from({ length: 3 }, () => [
    'Presentation cache reload failed; keeping in-memory state.',
  ]));
  expect(mock.invoke).toHaveBeenCalledTimes(18); // 2 reads × 3 renderers × 3 waves.
});

it('a failed active reload still drains the change queued while it was in flight', async () => {
  const { sessions, disk, old } = await setup();
  const held: { resolve: (value: unknown) => void; reject: (error: Error) => void }[] = [];
  mock.invoke.mockImplementation(() => new Promise((resolve, reject) => { held.push({ resolve, reject }); }));
  storageEvent('usage-dashboard-collapsed');
  disk['usage-dashboard-collapsed'] = false;
  storageEvent('usage-dashboard-collapsed');
  await act(async () => {
    for (const request of held.splice(0)) request.reject(new Error('synthetic failure'));
  });
  expect(held).toHaveLength(3);
  for (const session of sessions) {
    expect(session.storage.presentationStorage.getItem('usage-dashboard-collapsed')).toBeNull();
    expect(JSON.parse(session.storage.presentationStorage.getItem('dictation-settings')!)).toEqual(old['dictation-settings']);
  }
  await act(async () => {
    for (const request of held.splice(0)) request.resolve({ v1: true, v2: true, values: structuredClone(disk) });
  });
  for (const session of sessions) expect(session.storage.presentationStorage.getItem('usage-dashboard-collapsed')).toBe('false');
});

it('a queued follow-up cannot overwrite a later session write or pending intent', async () => {
  const { sessions, old } = await setup();
  const held: ((value: unknown) => void)[] = [];
  mock.invoke.mockImplementation((command: string) => command === 'webkit_state_snapshot'
    ? new Promise(resolve => { held.push(resolve); }) : Promise.reject(new Error('synthetic pending write')));
  storageEvent('dictation-settings');
  storageEvent('dictation-settings');
  const newer = JSON.stringify({ ...(old['dictation-settings'] as object), autoPaste: false });
  sessions[0].lifecycle.sourceStorage.setItem('dictation-settings', newer);
  await expect(sessions[0].lifecycle.flushSourceState()).rejects.toThrow('Source storage unavailable');
  sessions[1].storage.presentationStorage.rememberItem('dictation-settings', newer);
  for (let batch = 0; batch < 2; batch++) {
    expect(held).toHaveLength(3);
    await act(async () => {
      for (const resolve of held.splice(0)) resolve({ v1: true, v2: true, values: old });
    });
    expect(sessions[0].storage.presentationStorage.getItem('dictation-settings')).toBe(newer);
    expect(sessions[1].storage.presentationStorage.getItem('dictation-settings')).toBe(newer);
  }
  expect(held).toHaveLength(0);
});
