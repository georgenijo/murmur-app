import React, { act } from 'react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { Root } from 'react-dom/client';

const native = vi.hoisted(() => ({
  invoke: vi.fn(),
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
  settings: vi.fn(),
  root: null as Root | null,
}));
vi.mock('react-dom/client', async () => {
  const actual = await vi.importActual<typeof import('react-dom/client')>('react-dom/client');
  return { ...actual, default: { createRoot: (host: HTMLElement) => {
    native.root = actual.createRoot(host);
    return native.root;
  } } };
});
vi.mock('@tauri-apps/api/core', () => ({ invoke: native.invoke }));
vi.mock('./lib/targetedEvents', () => ({
  listen: vi.fn(async (name: string, handler: (event: { payload: unknown }) => void) => {
    native.listeners.set(name, handler);
    return () => { if (native.listeners.get(name) === handler) native.listeners.delete(name); };
  }),
}));
vi.mock('./App', () => ({ default: () => <div>Hydrated app</div> }));
vi.mock('./components/CompletionNotice', () => ({ CompletionNotice: () => null }));
vi.mock('./components/SourceStorageNotice', () => ({ SourceStorageNotice: () => null }));
vi.mock('./lib/windowLifecycle', () => ({ NativeWindowLifecycle: ({ children }: { children: React.ReactNode }) => children }));
vi.mock('./lib/hooks/useAppearance', () => ({ AppearanceProvider: ({ children }: { children: React.ReactNode }) => children }));
vi.mock('./lib/webkitLifecycle', () => ({ initializeWebkitState: async () => {} }));
vi.mock('./lib/settings', () => ({ hydrateSettingsFromDisk: () => native.settings(), ownSettingsMigrations: vi.fn() }));
vi.mock('./lib/durableUserData', () => ({ hydrateUserDataFromDisk: async () => {}, hydrateThemeLibraryFromDisk: async () => {} }));
vi.mock('./lib/log', () => ({ flog: { info: vi.fn() } }));

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: Error) => void;
  const promise = new Promise<T>((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}
let unsaved: boolean;
beforeEach(() => {
  vi.resetModules();
  vi.clearAllMocks();
  native.listeners.clear();
  unsaved = false;
  native.invoke.mockImplementation(async (command: string) => {
    if (command === 'get_dictation_settings_unsaved') return unsaved;
    throw new Error('Unexpected synthetic command');
  });
  document.body.innerHTML = '<div id="root"></div>';
});
afterEach(async () => {
  await act(async () => native.root?.unmount());
  native.root = null;
  document.body.innerHTML = '';
});

const warning = () => document.querySelector('[data-testid="main-error-banner"]');
async function change(value: boolean) {
  await act(async () => {
    unsaved = value;
    native.listeners.get('dictation-settings-warning-changed')?.({ payload: null });
  });
}

it.each([false, true])('retains a tray failure before hydration (hydration fails=%s)', async fails => {
  const hydration = deferred<void>();
  native.settings.mockReturnValue(hydration.promise);
  await act(async () => { await import('./main'); });
  expect(native.listeners.size).toBe(0);
  // The native tray applies Disable and retains its failed publication while
  // React has no listeners. A transient error emitted here would be dropped.
  unsaved = true;
  await act(async () => {
    if (fails) hydration.reject(new Error('synthetic unreadable settings'));
    else hydration.resolve();
  });
  expect(document.body.textContent).toContain(fails ? 'Saved data needs recovery' : 'Hydrated app');
  expect(warning()?.textContent).toContain('could not save the enabled/disabled setting');
  expect(warning()?.getAttribute('data-placement')).toBe('window');
  expect(native.invoke).toHaveBeenCalledWith('get_dictation_settings_unsaved');
  await change(false); // Durable retry or a durably saved superseding choice.
  expect(warning()).toBeNull();
  await change(true);
  expect(warning()?.textContent).toContain('tray checkmark');
});

it('shows tray failure arriving after SourceRecovery mounted', async () => {
  native.settings.mockRejectedValue(new Error('synthetic hydration failure'));
  await act(async () => { await import('./main'); });
  expect(warning()).toBeNull();
  await change(true);
  expect(warning()?.getAttribute('role')).toBe('alert');
});

it.each([false, true])('a late initial query cannot overwrite newer state (current unsaved=%s)', async current => {
  const stale = deferred<boolean>();
  native.settings.mockResolvedValue(undefined);
  native.invoke.mockImplementationOnce(() => stale.promise);
  await act(async () => { await import('./main'); });
  await change(current);
  await act(async () => stale.resolve(!current));
  expect(Boolean(warning())).toBe(current);
});

it('keeps query failures content-free and recovers on the next notification', async () => {
  native.settings.mockResolvedValue(undefined);
  native.invoke.mockRejectedValueOnce(new Error('synthetic private path or content'));
  await act(async () => { await import('./main'); });
  expect(warning()?.textContent).toContain('could not verify whether');
  expect(document.body.textContent).not.toContain('synthetic private');
  await change(false);
  expect(warning()).toBeNull();
});
