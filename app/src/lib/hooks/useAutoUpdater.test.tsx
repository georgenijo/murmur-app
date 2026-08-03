import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  check: vi.fn(),
  getVersion: vi.fn(),
  relaunch: vi.fn(),
  isPermissionGranted: vi.fn(),
  requestPermission: vi.fn(),
  sendNotification: vi.fn(),
  listen: vi.fn(),
  getUpdateInstallEnvironment: vi.fn(),
}));

vi.mock('@tauri-apps/plugin-updater', () => ({ check: mocks.check }));
vi.mock('@tauri-apps/api/app', () => ({ getVersion: mocks.getVersion }));
vi.mock('@tauri-apps/plugin-process', () => ({ relaunch: mocks.relaunch }));
vi.mock('@tauri-apps/plugin-notification', () => ({
  isPermissionGranted: mocks.isPermissionGranted,
  requestPermission: mocks.requestPermission,
  sendNotification: mocks.sendNotification,
}));
vi.mock('@tauri-apps/api/event', () => ({ listen: mocks.listen }));
vi.mock('../updaterEnvironment', () => ({
  getUpdateInstallEnvironment: mocks.getUpdateInstallEnvironment,
}));
vi.mock('../log', () => ({
  flog: {
    info: vi.fn(),
    warn: vi.fn(),
    error: vi.fn(),
  },
}));

import { useAutoUpdater, type UseAutoUpdaterReturn } from './useAutoUpdater';

describe('useAutoUpdater presentation state', () => {
  let container: HTMLDivElement;
  let root: Root;
  let current: UseAutoUpdaterReturn;

  function Harness() {
    current = useAutoUpdater();
    return null;
  }

  beforeEach(async () => {
    localStorage.clear();
    vi.clearAllMocks();
    mocks.getVersion.mockResolvedValue('0.22.1');
    mocks.listen.mockResolvedValue(vi.fn());
    mocks.isPermissionGranted.mockResolvedValue(true);
    mocks.getUpdateInstallEnvironment.mockResolvedValue({ appTranslocated: false });
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({}),
    }));

    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    await act(async () => root.render(<Harness />));
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.unstubAllGlobals();
  });

  it('keeps optional availability after Later and can reopen it from the pill', async () => {
    mocks.check.mockResolvedValue({
      available: true,
      version: '0.23.0',
      body: 'Release notes',
      downloadAndInstall: vi.fn(),
    });

    await act(async () => current.checkForUpdate());
    expect(current.updateStatus).toMatchObject({
      phase: 'available',
      version: '0.23.0',
      isForced: false,
    });
    expect(current.isUpdateDialogOpen).toBe(true);

    await act(async () => current.dismissUpdate());
    expect(current.updateStatus.phase).toBe('available');
    expect(current.isUpdateDialogOpen).toBe(false);

    await act(async () => current.showAvailableUpdate());
    expect(current.isUpdateDialogOpen).toBe(true);
  });

  it('removes the passive indicator when the user skips that version', async () => {
    mocks.check.mockResolvedValue({
      available: true,
      version: '0.23.0',
      body: '',
      downloadAndInstall: vi.fn(),
    });

    await act(async () => current.checkForUpdate());
    await act(async () => current.skipVersion());

    expect(current.updateStatus.phase).toBe('idle');
    expect(current.isUpdateDialogOpen).toBe(false);
    expect(localStorage.getItem('skipped-update-version')).toBe('0.23.0');
  });

  it('keeps a failed manual check inline instead of opening a broken retry modal', async () => {
    mocks.check.mockRejectedValue(new Error('offline'));

    await act(async () => current.checkForUpdate());

    expect(current.updateStatus).toMatchObject({
      phase: 'error',
      stage: 'check',
      message: 'Error: offline',
    });
    expect(current.isUpdateDialogOpen).toBe(false);
  });

  it('blocks installation before download when Gatekeeper translocated the app', async () => {
    const downloadAndInstall = vi.fn();
    mocks.check.mockResolvedValue({
      available: true,
      version: '0.24.2',
      body: 'Release notes',
      downloadAndInstall,
    });
    mocks.getUpdateInstallEnvironment.mockResolvedValue({ appTranslocated: true });

    await act(async () => current.checkForUpdate());
    await act(async () => current.startDownload());

    expect(downloadAndInstall).not.toHaveBeenCalled();
    expect(current.updateStatus).toMatchObject({
      phase: 'error',
      stage: 'install',
      recovery: 'reinstall',
      isForced: false,
    });
    expect(current.updateStatus.phase === 'error' && current.updateStatus.message)
      .toContain('read-only security location');
    expect(localStorage.getItem('pending-update-release-notes')).toBeNull();
    expect(current.isUpdateDialogOpen).toBe(true);
  });

  it('keeps the normal writable installation path unchanged', async () => {
    const downloadAndInstall = vi.fn().mockResolvedValue(undefined);
    mocks.check.mockResolvedValue({
      available: true,
      version: '0.24.2',
      body: 'Release notes',
      downloadAndInstall,
    });

    await act(async () => current.checkForUpdate());
    await act(async () => current.startDownload());

    expect(downloadAndInstall).toHaveBeenCalledOnce();
    expect(mocks.relaunch).toHaveBeenCalledOnce();
    expect(current.updateStatus).toEqual({ phase: 'ready', version: '0.24.2' });
  });
});
