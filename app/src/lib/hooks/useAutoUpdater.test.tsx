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
  flogInfo: vi.fn(),
  flogWarn: vi.fn(),
  flogError: vi.fn(),
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
    info: mocks.flogInfo,
    warn: mocks.flogWarn,
    error: mocks.flogError,
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

  it('logs the stable current-version event code when no update is available', async () => {
    mocks.check.mockResolvedValue({ available: false });

    await act(async () => current.checkForUpdate());

    expect(mocks.flogInfo).toHaveBeenCalledWith(
      'updater',
      'no update available',
      { event_code: 'updater.check_current' },
    );
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
    expect(mocks.flogError).toHaveBeenCalledWith(
      'updater',
      'check failed',
      {
        event_code: 'updater.check_failed',
        error: 'Error: offline',
      },
    );
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
    expect(mocks.flogWarn).toHaveBeenCalledWith(
      'updater',
      'install blocked by macOS App Translocation',
      { event_code: 'updater.install_blocked' },
    );
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
    expect(mocks.flogInfo).toHaveBeenCalledWith(
      'updater',
      'installed, relaunching',
      { event_code: 'updater.install_ready' },
    );
  });

  it('logs the stable install-failure event code when download fails', async () => {
    const downloadAndInstall = vi.fn().mockRejectedValue(new Error('disk full'));
    mocks.check.mockResolvedValue({
      available: true,
      version: '0.24.2',
      body: 'Release notes',
      downloadAndInstall,
    });

    await act(async () => current.checkForUpdate());
    await act(async () => current.startDownload());

    expect(mocks.relaunch).not.toHaveBeenCalled();
    expect(mocks.flogError).toHaveBeenCalledWith(
      'updater',
      'download/install failed',
      {
        event_code: 'updater.install_failed',
        error: 'Error: disk full',
      },
    );
  });
});
