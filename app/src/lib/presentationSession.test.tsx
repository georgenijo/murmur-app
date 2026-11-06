import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, expect, it, vi } from 'vitest';

const mock = vi.hoisted(() => ({
  invoke: vi.fn(),
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
}));
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => true, invoke: mock.invoke }));
vi.mock('@tauri-apps/api/event', () => ({
  emitTo: vi.fn(async () => {}),
  listen: vi.fn(async (name, handler) => {
    mock.listeners.set(name, handler);
    return () => { mock.listeners.delete(name); };
  }),
}));
vi.mock('@tauri-apps/api/app', () => ({ setTheme: vi.fn(async () => {}) }));
vi.mock('@tauri-apps/plugin-autostart', () => ({
  isEnabled: vi.fn(async () => false), enable: vi.fn(), disable: vi.fn(),
}));
vi.mock('./dictation', () => ({
  configure: vi.fn(async (_options, settlement) => { settlement?.onSuccess?.(); }),
  buildConfigureOptions: (settings: unknown) => settings,
}));

let root: Root | undefined;
const storageListeners: EventListenerOrEventListenerObject[] = [];
afterEach(async () => {
  if (root) await act(async () => root!.unmount());
  root = undefined;
  for (const listener of storageListeners.splice(0)) window.removeEventListener('storage', listener);
  vi.restoreAllMocks();
  mock.listeners.clear();
  localStorage.clear();
  document.body.innerHTML = '';
});

async function hydrate(failure: 'reads' | 'all') {
  vi.resetModules();
  const add = window.addEventListener.bind(window);
  vi.spyOn(window, 'addEventListener').mockImplementation((name, listener, options) => {
    if (name === 'storage' && listener) storageListeners.push(listener);
    add(name, listener, options);
  });
  const { DEFAULT_SETTINGS } = await import('./settings');
  let disk = { ...DEFAULT_SETTINGS, settingsVersion: 7, language: 'ja', autoPaste: true, hotkeyMissFeedback: true,
    microphoneIdMigrationComplete: true, launchOnboardingComplete: true };
  mock.invoke.mockReset().mockImplementation(async (command: string, args?: { blob: string }) => {
    if (command === 'webkit_state_snapshot') return { v1: true, v2: true, v3: true, values: {
      'dictation-settings': disk,
      'murmur-appearance': { version: 1, mode: 'dark', theme: { version: 1, presetId: 'custom' } },
      'murmur-theme-library': { version: 1, revision: 17, themes: [] },
    } };
    if (command === 'save_settings_blob') { disk = JSON.parse(args!.blob); return JSON.stringify(disk); }
    if (command === 'load_settings_blob') return JSON.stringify(disk);
  });
  const read = vi.spyOn(localStorage, 'getItem').mockImplementation(() => { throw new Error('synthetic private cache error'); });
  let write: ReturnType<typeof vi.spyOn> | undefined;
  let remove: ReturnType<typeof vi.spyOn> | undefined;
  if (failure === 'all') {
    write = vi.spyOn(localStorage, 'setItem').mockImplementation(() => { throw new Error('synthetic'); });
    remove = vi.spyOn(localStorage, 'removeItem').mockImplementation(() => { throw new Error('synthetic'); });
  }
  const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
  const lifecycle = await import('./webkitLifecycle');
  await lifecycle.initializeWebkitState();
  document.body.innerHTML = '<div id="root"></div>';
  root = createRoot(document.getElementById('root')!);
  return { lifecycle, disk: () => disk, setDisk: (patch: Partial<typeof disk>) => { disk = { ...disk, ...patch }; },
    recover: () => { read.mockRestore(); write?.mockRestore(); remove?.mockRestore(); }, warn };
}

it.each(['reads', 'all'] as const)('real settings/appearance hooks and queue preserve hydrated state with %s failing, then recover', async failure => {
  const session = await hydrate(failure);
  const { useSettings } = await import('./hooks/useSettings');
  const { useOverlaySettingsSync } = await import('./hooks/useOverlaySettingsSync');
  const { AppearanceProvider, useAppearance } = await import('./hooks/useAppearance');
  const { loadSettings } = await import('./settings');
  const { presentationStorage } = await import('./presentationStorage');
  let settings!: ReturnType<typeof useSettings>;
  let appearance!: ReturnType<typeof useAppearance>;
  function Harness() {
    settings = useSettings();
    useOverlaySettingsSync(settings.applyExternalSettings);
    appearance = useAppearance();
    return null;
  }
  await act(async () => root!.render(<AppearanceProvider><Harness /></AppearanceProvider>));
  expect(settings.settings.language).toBe('ja');
  expect(appearance.document.mode).toBe('dark');
  expect(appearance.library.document.revision).toBe(17);
  await act(async () => {
    settings.updateSettings({ hotkeyMissFeedback: false });
    await session.lifecycle.flushSourceState();
  });
  expect(session.disk()).toMatchObject({ language: 'ja', autoPaste: true, hotkeyMissFeedback: false });
  session.setDisk({ autoPaste: false, disabled: true });
  await act(async () => mock.listeners.get('overlay-owned-setting-committed')!({ payload: {
    field: 'disabled', value: true, autoPaste: false, disabled: true, ownedFieldsRevision: 9,
  } }));
  expect(loadSettings()).toMatchObject({ language: 'ja', autoPaste: false, disabled: true });
  expect(presentationStorage.getItem('murmur-overlay-vertical-offset')).toBeNull();
  // Unknown cache reads also fail quietly, with the same content-free notice.
  expect(() => presentationStorage.getItem('synthetic-unknown')).toThrow('Presentation cache unavailable');
  expect(session.warn).toHaveBeenCalledExactlyOnceWith('Presentation cache unavailable; using in-memory state.');
  session.recover();
  await act(async () => {
    settings.updateSettings({ hotkeyMissFeedback: true });
    await session.lifecycle.flushSourceState();
  });
  expect(session.disk()).toMatchObject({ language: 'ja', autoPaste: false, disabled: true, hotkeyMissFeedback: true });
  expect(JSON.parse(localStorage.getItem('dictation-settings')!)).toMatchObject({ language: 'ja' });
  expect(loadSettings().language).toBe('ja');
});

it.each(['reads', 'all'] as const)('typed cross-window notifications refresh the real overlay hook and session cache with %s failing', async failure => {
  const session = await hydrate(failure);
  const { useOverlaySettingsMirror } = await import('./hooks/useOverlaySettingsMirror');
  const { loadSettings } = await import('./settings');
  let mirror!: ReturnType<typeof useOverlaySettingsMirror>;
  const args = { setDisabled: vi.fn(), setShowHotkeyMiss: vi.fn(), hotkeyMissFeedbackRef: { current: false } };
  function Harness() { mirror = useOverlaySettingsMirror(args); return null; }
  await act(async () => root!.render(<Harness />));
  expect(mirror.autoPaste).toBe(true);
  const cacheWrites = vi.spyOn(localStorage, 'setItem');
  const writeCount = cacheWrites.mock.calls.length;
  for (const event of ['settings-changed', 'overlay-settings-persisted']) {
    session.setDisk({ autoPaste: !mirror.autoPaste, overlayPlacement: 'floating_pill' });
    await act(async () => mock.listeners.get(event)!({ payload: null }));
    expect(mirror.autoPaste).toBe(session.disk().autoPaste);
    expect(loadSettings()).toMatchObject({ language: 'ja', autoPaste: session.disk().autoPaste, overlayPlacement: 'floating_pill' });
    expect(cacheWrites.mock.calls).toHaveLength(writeCount);
  }
  await act(async () => mirror.refresh());
  expect(loadSettings().language).toBe('ja');
  expect(mock.invoke.mock.calls.some(([command]) => command === 'save_settings_blob')).toBe(false);
});

it('stale DOM storage events cannot replace a newer committed overlay snapshot', async () => {
  const session = await hydrate('reads');
  const { useOverlaySettingsMirror } = await import('./hooks/useOverlaySettingsMirror');
  const { loadSettings } = await import('./settings');
  let mirror!: ReturnType<typeof useOverlaySettingsMirror>;
  const args = { setDisabled: vi.fn(), setShowHotkeyMiss: vi.fn(), hotkeyMissFeedbackRef: { current: true } };
  function Harness() { mirror = useOverlaySettingsMirror(args); return null; }
  await act(async () => root!.render(<Harness />));
  const old = JSON.stringify(session.disk());
  session.setDisk({ autoPaste: false, hotkeyMissFeedback: false, overlayPlacement: 'floating_pill' });
  await act(async () => mock.listeners.get('overlay-settings-persisted')!({ payload: null }));
  await act(async () => window.dispatchEvent(new StorageEvent('storage', { key: 'dictation-settings', newValue: old })));
  expect(mirror.autoPaste).toBe(false);
  expect(loadSettings()).toMatchObject({ autoPaste: false, hotkeyMissFeedback: false, overlayPlacement: 'floating_pill' });
  await act(async () => mirror.refresh());
  expect(args.hotkeyMissFeedbackRef.current).toBe(false);
  session.recover();
});

it('ignores 10,000 unowned deletion events rather than retaining tombstones', async () => {
  const session = await hydrate('reads');
  const { presentationStorage } = await import('./presentationStorage');
  const calls = mock.invoke.mock.calls.length;
  for (let i = 0; i < 10_000; i++) {
    window.dispatchEvent(new StorageEvent('storage', { key: `unowned-${i}`, newValue: null }));
  }
  expect(mock.invoke.mock.calls).toHaveLength(calls);
  session.recover();
  const read = vi.spyOn(localStorage, 'getItem').mockReturnValue('synthetic external value');
  for (let i = 0; i < 10_000; i++) expect(presentationStorage.getItem(`unowned-${i}`)).toBe('synthetic external value');
  expect(read).toHaveBeenCalledTimes(10_000);
});

it('a newer storage-triggered read owns the session before an older overlay read completes', async () => {
  const session = await hydrate('all');
  const { useOverlaySettingsMirror } = await import('./hooks/useOverlaySettingsMirror');
  const { loadSettings } = await import('./settings');
  let mirror!: ReturnType<typeof useOverlaySettingsMirror>;
  const args = { setDisabled: vi.fn(), setShowHotkeyMiss: vi.fn(), hotkeyMissFeedbackRef: { current: true } };
  function Harness() { mirror = useOverlaySettingsMirror(args); return null; }
  await act(async () => root!.render(<Harness />));
  const old = JSON.stringify(session.disk());
  const normal = mock.invoke.getMockImplementation()!;
  let finishOverlay!: (value: string) => void;
  let finishStorage: ((value: unknown) => void) | undefined;
  mock.invoke.mockImplementation((command: string, args: unknown) => {
    if (command === 'load_settings_blob') return new Promise(resolve => { finishOverlay = resolve; });
    if (command === 'webkit_state_snapshot') return new Promise(resolve => { finishStorage = resolve; });
    return normal(command, args);
  });
  await act(async () => mock.listeners.get('overlay-settings-persisted')!({ payload: null }));
  session.setDisk({ autoPaste: false, hotkeyMissFeedback: false, overlayPlacement: 'floating_pill' });
  window.dispatchEvent(new StorageEvent('storage', { key: 'dictation-settings', newValue: old }));
  await act(async () => finishOverlay(old));
  await act(async () => finishStorage?.({ v1: true, v2: true, v3: true, values: { 'dictation-settings': session.disk() } }));
  expect(loadSettings()).toMatchObject({ autoPaste: false, hotkeyMissFeedback: false, overlayPlacement: 'floating_pill' });
  expect(mirror.autoPaste).toBe(false);
  expect(mirror.overlayPlacement).toBe('floating_pill');
  expect(args.hotkeyMissFeedbackRef.current).toBe(false);
});
