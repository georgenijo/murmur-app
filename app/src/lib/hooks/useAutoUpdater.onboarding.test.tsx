import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useAutoUpdater } from './useAutoUpdater';
import { CHECK_INTERVAL_MS, CHECK_TIMER_TICK_MS } from '../updater';
import type { Update } from '@tauri-apps/plugin-updater';

const native = vi.hoisted(() => ({
  invoke: vi.fn(), check: vi.fn(), isPermissionGranted: vi.fn(),
  requestPermission: vi.fn(), sendNotification: vi.fn(),
  listeners: new Map<string, () => void>(),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: native.invoke }));
vi.mock('@tauri-apps/api/app', () => ({ getVersion: vi.fn(async () => '0.59.5') }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, callback: () => void) => {
    native.listeners.set(name, callback);
    return () => native.listeners.delete(name);
  }),
}));
vi.mock('@tauri-apps/plugin-updater', () => ({ check: native.check }));
vi.mock('@tauri-apps/plugin-process', () => ({ relaunch: vi.fn() }));
vi.mock('@tauri-apps/plugin-notification', () => ({
  isPermissionGranted: native.isPermissionGranted,
  requestPermission: native.requestPermission,
  sendNotification: native.sendNotification,
}));

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

describe('automatic update checks during first-run setup', () => {
  let root: Root;
  let container: HTMLDivElement;
  let current!: ReturnType<typeof useAutoUpdater>;
  function Harness({ ready }: { ready: boolean }) {
    current = useAutoUpdater({ automaticChecksEnabled: true, setupComplete: ready });
    return null;
  }
  beforeEach(() => {
    vi.useFakeTimers();
    vi.clearAllMocks();
    localStorage.clear();
    native.listeners.clear();
    native.invoke.mockImplementation(async (command: string) => {
      if (command === 'updater_canary') return { path: null, result: null };
      if (command === 'get_update_install_environment') return { appTranslocated: false };
    });
    native.check.mockResolvedValue({
      available: true, version: '0.60.0', body: 'Release notes', rawJson: {},
      download: vi.fn(), install: vi.fn(),
    });
    // Match the macOS desktop plugin: permission_state returns Granted.
    native.isPermissionGranted.mockResolvedValue(true);
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.useRealTimers();
  });

  it('stays quiet throughout setup and starts the normal background check when ready', async () => {
    await act(async () => root.render(<Harness ready={false} />));
    await act(async () => {
      window.dispatchEvent(new Event('focus'));
      document.dispatchEvent(new Event('visibilitychange'));
      await vi.advanceTimersByTimeAsync(CHECK_INTERVAL_MS + CHECK_TIMER_TICK_MS);
    });
    expect(native.check).not.toHaveBeenCalled();
    expect(native.isPermissionGranted).not.toHaveBeenCalled();
    expect(native.requestPermission).not.toHaveBeenCalled();
    expect(native.sendNotification).not.toHaveBeenCalled();
    expect(native.listeners.has('updater-background-check-requested')).toBe(false);

    await act(async () => root.render(<Harness ready />));
    expect(native.check).toHaveBeenCalledOnce();
    expect(native.sendNotification).toHaveBeenCalledWith(expect.objectContaining({ title: 'Update Available' }));
    expect(current.updateStatus).toMatchObject({ phase: 'available', isForced: false });
    expect(current.isUpdateDialogOpen).toBe(false);
    await act(async () => root.render(<Harness ready />));
    expect(native.check).toHaveBeenCalledOnce();
  });

  it('preserves explicit manual update checks while setup defers background work', async () => {
    await act(async () => root.render(<Harness ready={false} />));
    await act(async () => current.checkForUpdate());
    expect(native.check).toHaveBeenCalledOnce();
    expect(current.isUpdateDialogOpen).toBe(true);
    expect(native.sendNotification).not.toHaveBeenCalled();
  });

  it.each([false, true])('keeps an in-flight background result quiet after setup restarts (forced: %s)', async (forced) => {
    const response = deferred<Update | null>();
    const update = await native.check();
    update.rawJson = forced ? { min_version: '0.60.0' } : {};
    native.check.mockClear().mockReturnValueOnce(response.promise);
    await act(async () => root.render(<Harness ready />));
    expect(native.check).toHaveBeenCalledOnce();

    await act(async () => root.render(<Harness ready={false} />));
    await act(async () => response.resolve(update));

    expect(current.updateStatus).toMatchObject({ phase: 'available', isForced: forced });
    expect(current.isUpdateDialogOpen).toBe(false);
    expect(native.isPermissionGranted).not.toHaveBeenCalled();
    expect(native.requestPermission).not.toHaveBeenCalled();
    expect(native.sendNotification).not.toHaveBeenCalled();

    // An explicit manual request still presents the result during setup.
    await act(async () => current.checkForUpdate());
    expect(current.isUpdateDialogOpen).toBe(true);
    expect(native.sendNotification).not.toHaveBeenCalled();
  });

  it.each([false, true])('notifies a previously suppressed update after setup completes, once per version (forced: %s)', async (forced) => {
    const response = deferred<Update | null>();
    const update = await native.check();
    update.rawJson = forced ? { min_version: '0.60.0' } : {};
    native.check.mockClear().mockReturnValueOnce(response.promise);
    await act(async () => root.render(<Harness ready />));
    expect(native.check).toHaveBeenCalledOnce();

    await act(async () => root.render(<Harness ready={false} />));
    await act(async () => response.resolve(update));
    expect(current.updateStatus).toMatchObject({ phase: 'available', isForced: forced });
    expect(current.isUpdateDialogOpen).toBe(false);
    expect(native.sendNotification).not.toHaveBeenCalled();

    await act(async () => root.render(<Harness ready />));
    await act(async () => vi.advanceTimersByTimeAsync(CHECK_INTERVAL_MS + CHECK_TIMER_TICK_MS));
    expect(native.check).toHaveBeenCalledTimes(2);
    expect(native.sendNotification).toHaveBeenCalledOnce();
    expect(native.sendNotification).toHaveBeenCalledWith({
      title: 'Update Available', body: 'Murmur v0.60.0 is ready to install.',
    });
    expect(current.isUpdateDialogOpen).toBe(forced);

    await act(async () => vi.advanceTimersByTimeAsync(CHECK_INTERVAL_MS + CHECK_TIMER_TICK_MS));
    expect(native.check).toHaveBeenCalledTimes(3);
    expect(native.sendNotification).toHaveBeenCalledOnce();
  });

  it('rechecks readiness after waiting for native notification permission', async () => {
    const permission = deferred<boolean>();
    native.isPermissionGranted.mockReturnValueOnce(permission.promise);
    await act(async () => root.render(<Harness ready />));
    expect(native.isPermissionGranted).toHaveBeenCalledOnce();

    await act(async () => root.render(<Harness ready={false} />));
    await act(async () => permission.resolve(true));

    expect(native.requestPermission).not.toHaveBeenCalled();
    expect(native.sendNotification).not.toHaveBeenCalled();
    expect(current.isUpdateDialogOpen).toBe(false);
  });

  it('honors a manual request joining a background check after setup restarts', async () => {
    const response = deferred<Update | null>();
    const update = await native.check();
    native.check.mockClear().mockReturnValueOnce(response.promise);
    await act(async () => root.render(<Harness ready />));
    await act(async () => root.render(<Harness ready={false} />));
    await act(async () => current.checkForUpdate());
    await act(async () => response.resolve(update));

    expect(native.check).toHaveBeenCalledOnce();
    expect(current.isUpdateDialogOpen).toBe(true);
    expect(native.sendNotification).not.toHaveBeenCalled();
  });

  it('resumes due checks after setup is run again without repeating the startup check', async () => {
    native.check.mockResolvedValue({ available: false });
    await act(async () => root.render(<Harness ready />));
    expect(native.check).toHaveBeenCalledOnce();
    await act(async () => root.render(<Harness ready={false} />));
    expect(native.listeners.has('updater-background-check-requested')).toBe(false);
    await act(async () => vi.advanceTimersByTimeAsync(CHECK_INTERVAL_MS + 1));
    expect(native.check).toHaveBeenCalledOnce();
    await act(async () => root.render(<Harness ready />));
    expect(native.check).toHaveBeenCalledOnce();
    await act(async () => native.listeners.get('updater-background-check-requested')!());
    expect(native.check).toHaveBeenCalledTimes(2);
  });
});
