import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  check: vi.fn(),
  invoke: vi.fn(),
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
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
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
import type { DownloadEvent } from '@tauri-apps/plugin-updater';

describe('useAutoUpdater presentation state', () => {
  let container: HTMLDivElement;
  let root: Root;
  let current: UseAutoUpdaterReturn;
  let automaticChecksEnabled: boolean;

  function Harness() {
    current = useAutoUpdater({ automaticChecksEnabled });
    return null;
  }

  beforeEach(async () => {
    localStorage.clear();
    vi.clearAllMocks();
    automaticChecksEnabled = false;
    mocks.getVersion.mockResolvedValue('0.22.1');
    mocks.listen.mockResolvedValue(vi.fn());
    mocks.isPermissionGranted.mockResolvedValue(true);
    mocks.getUpdateInstallEnvironment.mockResolvedValue({ appTranslocated: false });
    mocks.invoke.mockResolvedValue({ path: null, result: null });
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
      rawJson: {},
      download: vi.fn(),
      install: vi.fn(),
    });

    await act(async () => current.checkForUpdate());
    expect(current.updateStatus).toMatchObject({
      phase: 'available',
      version: '0.23.0',
      isForced: false,
    });
    expect(current.isUpdateDialogOpen).toBe(true);
    expect(vi.mocked(fetch)).not.toHaveBeenCalled();

    await act(async () => current.dismissUpdate());
    expect(current.updateStatus.phase).toBe('available');
    expect(current.isUpdateDialogOpen).toBe(false);

    await act(async () => current.showAvailableUpdate());
    expect(current.isUpdateDialogOpen).toBe(true);
  });

  it('does not suppress an available update from a legacy skipped-version value', async () => {
    localStorage.setItem('skipped-update-version', '0.23.0');
    mocks.check.mockResolvedValue({
      available: true,
      version: '0.23.0',
      body: '',
      rawJson: {},
      download: vi.fn(),
      install: vi.fn(),
    });

    await act(async () => current.checkForUpdate());

    expect(current.updateStatus).toMatchObject({
      phase: 'available',
      version: '0.23.0',
    });
    expect(current.isUpdateDialogOpen).toBe(true);
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

  it('leaves the canary path inert when the launch marker is absent', async () => {
    await act(async () => root.unmount());
    automaticChecksEnabled = true;
    root = createRoot(container);
    mocks.check.mockResolvedValue({ available: false });
    await act(async () => root.render(<Harness />));

    expect(mocks.invoke).toHaveBeenCalledWith('updater_canary', {
      request: { action: 'read' },
    });
    expect(mocks.check).toHaveBeenCalledOnce();
    expect(mocks.relaunch).not.toHaveBeenCalled();
    expect(mocks.invoke).not.toHaveBeenCalledWith('updater_canary', {
      request: expect.objectContaining({ action: 'write' }),
    });
  });

  async function launchCanary(state: { path: string; result: unknown; dryRun: boolean }) {
    await act(async () => root.unmount());
    automaticChecksEnabled = true;
    mocks.invoke.mockImplementation(async (
      _command: string,
      payload: { request: { action: string; result?: unknown } },
    ) => (
      payload.request.action === 'read'
        ? state
        : { ...state, result: payload.request.result }
    ));
    root = createRoot(container);
    await act(async () => root.render(<Harness />));
    await act(async () => Promise.resolve());
  }

  function canaryWrites() {
    return mocks.invoke.mock.calls
      .filter(([, payload]) => payload?.request?.action === 'write')
      .map(([, payload]) => payload.request.result);
  }

  it('uses the Rust command boundary schema for canary reads and writes', async () => {
    mocks.check.mockResolvedValue({
      available: true,
      version: '0.23.0',
      body: '',
      rawJson: {},
      download: vi.fn(),
      install: vi.fn(),
    });

    await launchCanary({ path: '/tmp/canary.json', result: null, dryRun: true });

    expect(mocks.invoke).toHaveBeenNthCalledWith(1, 'updater_canary', {
      request: { action: 'read' },
    });
    expect(mocks.invoke).toHaveBeenCalledWith('updater_canary', {
      request: expect.objectContaining({
        action: 'write',
        result: expect.objectContaining({ status: 'dry-run' }),
      }),
    });
  });

  it('canary uses the real download, install, and relaunch path', async () => {
    const download = vi.fn().mockResolvedValue(undefined);
    const install = vi.fn().mockResolvedValue(undefined);
    mocks.check.mockResolvedValue({
      available: true,
      version: '0.23.0',
      body: '',
      rawJson: {},
      download,
      install,
    });

    await launchCanary({ path: '/tmp/canary.json', result: null, dryRun: false });

    expect(download).toHaveBeenCalledOnce();
    expect(install).toHaveBeenCalledOnce();
    expect(mocks.relaunch).toHaveBeenCalledOnce();
  });

  it('canary dry-run launches discovery and policy but never downloads or relaunches', async () => {
    const download = vi.fn();
    mocks.check.mockResolvedValue({
      available: true,
      version: '0.23.0',
      body: '',
      rawJson: {},
      download,
      install: vi.fn(),
    });

    await launchCanary({ path: '/tmp/canary.json', result: null, dryRun: true });

    expect(download).not.toHaveBeenCalled();
    expect(mocks.relaunch).not.toHaveBeenCalled();
    const writes = canaryWrites();
    expect(writes[writes.length - 1]).toMatchObject({ status: 'dry-run', dryRun: true });
  });

  it('records production stages as pending until install evidence is available', async () => {
    const download = vi.fn().mockResolvedValue(undefined);
    mocks.check.mockResolvedValue({
      available: true,
      version: '0.23.0',
      body: '',
      rawJson: {},
      download,
      install: vi.fn(),
    });

    await launchCanary({ path: '/tmp/canary.json', result: null, dryRun: false });

    const writes = canaryWrites();
    expect(writes[0]).toMatchObject({ status: 'pending', stages: {
      download: 'pending', signatureVerify: 'pending', install: 'pending', relaunch: 'pending',
    } });
    expect(writes[1]).toMatchObject({ status: 'pending', stages: {
      download: 'passed', signatureVerify: 'passed', install: 'passed', relaunch: 'pending',
    } });
  });

  it('records policy failure without fabricating production stages', async () => {
    mocks.check.mockResolvedValue({
      available: true,
      version: '0.23.0',
      body: '',
      rawJson: undefined,
      download: vi.fn(),
      install: vi.fn(),
    });

    await launchCanary({ path: '/tmp/canary.json', result: null, dryRun: false });

    const writes = canaryWrites();
    expect(writes[writes.length - 1]).toMatchObject({ status: 'failed', stages: {
      discover: 'passed', policy: 'failed', download: 'pending', signatureVerify: 'pending', install: 'pending', relaunch: 'pending',
    } });
  });

  it('records a canary download failure at the download stage', async () => {
    mocks.check.mockResolvedValue({
      available: true,
      version: '0.23.0',
      body: '',
      rawJson: {},
      download: vi.fn().mockRejectedValue(new Error('disk full')),
      install: vi.fn(),
    });

    await launchCanary({ path: '/tmp/canary.json', result: null, dryRun: false });

    const writes = canaryWrites();
    expect(writes[writes.length - 1]).toMatchObject({ status: 'failed', stages: {
      download: 'failed', signatureVerify: 'pending', install: 'pending', relaunch: 'pending',
    } });
  });

  it('records a canary relaunch failure after install evidence', async () => {
    mocks.relaunch.mockRejectedValueOnce(new Error('relaunch refused'));
    mocks.check.mockResolvedValue({
      available: true,
      version: '0.23.0',
      body: '',
      rawJson: {},
      download: vi.fn().mockResolvedValue(undefined),
      install: vi.fn(),
    });

    await launchCanary({ path: '/tmp/canary.json', result: null, dryRun: false });

    const writes = canaryWrites();
    expect(writes[writes.length - 1]).toMatchObject({ status: 'failed', stages: {
      download: 'passed', signatureVerify: 'passed', install: 'passed', relaunch: 'failed',
    } });
  });

  it('recovers a pending canary only after the installed version matches', async () => {
    const previous = {
      schemaVersion: 1 as const,
      status: 'pending' as const,
      checkedVersion: '0.22.1',
      offeredVersion: '0.23.0',
      forced: false,
      dryRun: false,
      stages: { discover: 'passed' as const, policy: 'passed' as const, download: 'passed' as const, signatureVerify: 'passed' as const, install: 'passed' as const, relaunch: 'pending' as const },
      error: null,
    };
    mocks.getVersion.mockResolvedValue('0.23.0');
    await launchCanary({ path: '/tmp/canary.json', result: previous, dryRun: false });
    const writes = canaryWrites();
    expect(writes[writes.length - 1]).toMatchObject({ status: 'passed', stages: { relaunch: 'passed' } });
    expect(mocks.check).not.toHaveBeenCalled();
  });

  it('opens a recovery modal after a failed manual check', async () => {
    mocks.check.mockRejectedValue(new Error('offline'));

    await act(async () => current.checkForUpdate());

    expect(current.updateStatus).toMatchObject({
      phase: 'error',
      stage: 'check',
      message: 'Error: offline',
    });
    expect(current.isUpdateDialogOpen).toBe(true);
    expect(mocks.check).toHaveBeenCalledTimes(2);
    expect(mocks.flogError).toHaveBeenCalledWith(
      'updater',
      'check failed',
      {
        event_code: 'updater.check_failed',
        error: 'Error: offline',
      },
    );
  });

  it('recovers from a transient updater-feed failure before showing an error', async () => {
    mocks.check
      .mockRejectedValueOnce(new Error('temporary feed failure'))
      .mockResolvedValueOnce({
        available: true,
        version: '0.23.0',
        body: 'Release notes',
        rawJson: {},
        download: vi.fn(),
        install: vi.fn(),
      });

    await act(async () => current.checkForUpdate());

    expect(mocks.check).toHaveBeenCalledTimes(2);
    expect(current.updateStatus).toMatchObject({
      phase: 'available',
      version: '0.23.0',
    });
    expect(mocks.flogWarn).toHaveBeenCalledWith(
      'updater',
      'check failed; retrying',
      {
        attempt: 1,
        error: 'Error: temporary feed failure',
      },
    );
    expect(mocks.flogError).not.toHaveBeenCalled();
  });

  it('offers a verified update as optional when policy cannot be verified', async () => {
    mocks.getVersion.mockRejectedValue(new Error('installed version unavailable'));
    mocks.getVersion.mockClear();
    mocks.check.mockResolvedValue({
      available: true,
      version: '0.24.2',
      body: 'Release notes',
      rawJson: undefined,
      download: vi.fn(),
      install: vi.fn(),
    });

    await act(async () => current.checkForUpdate());

    expect(current.updateStatus).toMatchObject({
      phase: 'available',
      version: '0.24.2',
      isForced: false,
    });
    expect(current.isUpdateDialogOpen).toBe(true);
    expect(mocks.getVersion).not.toHaveBeenCalled();
    expect(localStorage.getItem('updater-last-check')).not.toBeNull();
    expect(mocks.flogWarn).toHaveBeenCalledWith(
      'updater',
      'could not verify update policy',
      { error: 'Update manifest was not an object.' },
    );
    expect(mocks.flogError).not.toHaveBeenCalled();
  });

  it('uses an absent policy from the native response without a webview fetch', async () => {
    mocks.getVersion.mockRejectedValue(new Error('installed version unavailable'));
    mocks.getVersion.mockClear();
    mocks.check.mockResolvedValue({
      available: true,
      version: '0.24.2',
      body: 'Release notes',
      rawJson: { version: '0.24.2' },
      download: vi.fn(),
      install: vi.fn(),
    });

    await act(async () => current.checkForUpdate());

    expect(vi.mocked(fetch)).not.toHaveBeenCalled();
    expect(mocks.getVersion).not.toHaveBeenCalled();
    expect(current.updateStatus).toMatchObject({
      phase: 'available',
      version: '0.24.2',
      isForced: false,
    });
  });

  it('opens a required update when the verified policy is above the installed version', async () => {
    mocks.check.mockResolvedValue({
      available: true,
      version: '0.24.2',
      body: 'Required reliability update',
      rawJson: { min_version: '0.24.0' },
      download: vi.fn(),
      install: vi.fn(),
    });

    await act(async () => current.checkForUpdate());

    expect(current.updateStatus).toMatchObject({
      phase: 'available',
      version: '0.24.2',
      isForced: true,
    });
    expect(current.isUpdateDialogOpen).toBe(true);
  });

  it('blocks installation before download when Gatekeeper translocated the app', async () => {
    const download = vi.fn();
    mocks.check.mockResolvedValue({
      available: true,
      version: '0.24.2',
      body: 'Release notes',
      rawJson: {},
      download,
      install: vi.fn(),
    });
    mocks.getUpdateInstallEnvironment.mockResolvedValue({ appTranslocated: true });

    await act(async () => current.checkForUpdate());
    await act(async () => current.startDownload());

    expect(download).not.toHaveBeenCalled();
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

  it('downloads to a ready state and waits for explicit restart before installing', async () => {
    const download = vi.fn().mockImplementation(async (
      onEvent?: (event: DownloadEvent) => void,
    ) => {
      onEvent?.({ event: 'Started', data: { contentLength: 100 } });
      onEvent?.({ event: 'Progress', data: { chunkLength: 45 } });
      onEvent?.({ event: 'Finished' });
    });
    const install = vi.fn().mockResolvedValue(undefined);
    mocks.check.mockResolvedValue({
      available: true,
      version: '0.24.2',
      body: 'Release notes',
      rawJson: {},
      download,
      install,
    });

    await act(async () => current.checkForUpdate());
    await act(async () => current.startDownload());

    expect(download).toHaveBeenCalledOnce();
    expect(install).not.toHaveBeenCalled();
    expect(mocks.relaunch).not.toHaveBeenCalled();
    expect(current.updateStatus).toEqual({
      phase: 'ready',
      version: '0.24.2',
      isForced: false,
    });
    expect(mocks.flogInfo).toHaveBeenCalledWith(
      'updater',
      'download ready',
      { event_code: 'updater.download_ready' },
    );

    await act(async () => current.dismissUpdate());
    expect(current.isUpdateDialogOpen).toBe(false);
    await act(async () => current.showAvailableUpdate());
    expect(current.isUpdateDialogOpen).toBe(true);

    await act(async () => current.restartUpdate());

    expect(install).toHaveBeenCalledOnce();
    expect(mocks.relaunch).toHaveBeenCalledOnce();
    expect(current.updateStatus).toEqual({ phase: 'restarting', version: '0.24.2' });
  });

  it('shows indeterminate progress and opens the modal for a direct download', async () => {
    let finishDownload!: () => void;
    const download = vi.fn().mockImplementation(async (
      onEvent?: (event: DownloadEvent) => void,
    ) => {
      onEvent?.({ event: 'Started', data: {} });
      onEvent?.({ event: 'Progress', data: { chunkLength: 64 } });
      await new Promise<void>((resolve) => {
        finishDownload = resolve;
      });
    });
    mocks.check.mockResolvedValue({
      available: true,
      version: '0.24.2',
      body: '',
      rawJson: {},
      download,
      install: vi.fn(),
    });

    await act(async () => current.checkForUpdate());
    await act(async () => current.dismissUpdate());
    expect(current.isUpdateDialogOpen).toBe(false);

    let pending!: Promise<unknown>;
    await act(async () => {
      pending = current.startDownload();
      await Promise.resolve();
    });

    expect(current.isUpdateDialogOpen).toBe(true);
    expect(current.updateStatus).toEqual({
      phase: 'downloading',
      version: '0.24.2',
      progress: null,
    });

    finishDownload();
    await act(async () => pending);
  });

  it('keeps a forced ready update open', async () => {
    mocks.check.mockResolvedValue({
      available: true,
      version: '0.24.2',
      body: '',
      rawJson: { min_version: '0.24.0' },
      download: vi.fn().mockResolvedValue(undefined),
      install: vi.fn(),
    });

    await act(async () => current.checkForUpdate());
    await act(async () => current.startDownload());

    expect(current.updateStatus).toEqual({
      phase: 'ready',
      version: '0.24.2',
      isForced: true,
    });
    await act(async () => current.dismissUpdate());
    expect(current.isUpdateDialogOpen).toBe(true);
  });

  it('keeps the downloaded resource stable when a manual check is requested', async () => {
    const install = vi.fn().mockResolvedValue(undefined);
    mocks.check.mockResolvedValue({
      available: true,
      version: '0.24.2',
      body: '',
      rawJson: {},
      download: vi.fn().mockResolvedValue(undefined),
      install,
    });

    await act(async () => current.checkForUpdate());
    await act(async () => current.startDownload());
    mocks.check.mockResolvedValue({
      available: true,
      version: '0.25.0',
      body: '',
      rawJson: {},
      download: vi.fn(),
      install: vi.fn(),
    });

    await act(async () => current.checkForUpdate());

    expect(mocks.check).toHaveBeenCalledOnce();
    expect(current.updateStatus).toEqual({
      phase: 'ready',
      version: '0.24.2',
      isForced: false,
    });
    await act(async () => current.restartUpdate());
    expect(install).toHaveBeenCalledOnce();
  });

  it('reports a download failure and permits a retry', async () => {
    const download = vi.fn()
      .mockRejectedValueOnce(new Error('disk full'))
      .mockResolvedValueOnce(undefined);
    mocks.check.mockResolvedValue({
      available: true,
      version: '0.24.2',
      body: 'Release notes',
      rawJson: {},
      download,
      install: vi.fn(),
    });

    await act(async () => current.checkForUpdate());
    await act(async () => current.startDownload());

    expect(mocks.relaunch).not.toHaveBeenCalled();
    expect(current.updateStatus).toMatchObject({ phase: 'error', stage: 'install' });
    expect(mocks.flogError).toHaveBeenCalledWith(
      'updater',
      'download failed',
      {
        event_code: 'updater.install_failed',
        error: 'Error: disk full',
      },
    );

    await act(async () => current.startDownload());
    expect(download).toHaveBeenCalledTimes(2);
    expect(current.updateStatus).toEqual({
      phase: 'ready',
      version: '0.24.2',
      isForced: false,
    });
  });

  it('allows only one restart owner', async () => {
    let finishInstall!: () => void;
    const install = vi.fn().mockImplementation(() => new Promise<void>((resolve) => {
      finishInstall = resolve;
    }));
    mocks.check.mockResolvedValue({
      available: true,
      version: '0.24.2',
      body: '',
      rawJson: {},
      download: vi.fn().mockResolvedValue(undefined),
      install,
    });

    await act(async () => current.checkForUpdate());
    await act(async () => current.startDownload());
    let first!: Promise<unknown>;
    let second!: Promise<unknown>;
    await act(async () => {
      first = current.restartUpdate();
      second = current.restartUpdate();
      await Promise.resolve();
    });

    expect(install).toHaveBeenCalledOnce();
    expect(current.updateStatus).toEqual({ phase: 'restarting', version: '0.24.2' });
    finishInstall();
    await act(async () => Promise.all([first, second]));
    expect(mocks.relaunch).toHaveBeenCalledOnce();
  });

  it('retries installation without downloading again', async () => {
    const download = vi.fn().mockResolvedValue(undefined);
    const install = vi.fn()
      .mockRejectedValueOnce(new Error('install refused'))
      .mockResolvedValueOnce(undefined);
    mocks.check.mockResolvedValue({
      available: true,
      version: '0.24.2',
      body: '',
      rawJson: {},
      download,
      install,
    });

    await act(async () => current.checkForUpdate());
    await act(async () => current.startDownload());
    await act(async () => current.restartUpdate());
    expect(current.updateStatus).toMatchObject({ phase: 'error', stage: 'restart' });

    const retry = current.updateStatus.phase === 'error' && current.updateStatus.stage === 'restart'
      ? current.restartUpdate
      : current.startDownload;
    await act(async () => retry());
    expect(download).toHaveBeenCalledOnce();
    expect(install).toHaveBeenCalledTimes(2);
    expect(mocks.relaunch).toHaveBeenCalledOnce();
  });

  it('retries only relaunch after the update has installed', async () => {
    const install = vi.fn().mockResolvedValue(undefined);
    mocks.relaunch
      .mockRejectedValueOnce(new Error('relaunch refused'))
      .mockResolvedValueOnce(undefined);
    mocks.check.mockResolvedValue({
      available: true,
      version: '0.24.2',
      body: '',
      rawJson: {},
      download: vi.fn().mockResolvedValue(undefined),
      install,
    });

    await act(async () => current.checkForUpdate());
    await act(async () => current.startDownload());
    await act(async () => current.restartUpdate());
    expect(current.updateStatus).toMatchObject({ phase: 'error', stage: 'restart' });

    await act(async () => current.restartUpdate());
    expect(install).toHaveBeenCalledOnce();
    expect(mocks.relaunch).toHaveBeenCalledTimes(2);
  });

  it('allows only one download owner while environment verification is pending', async () => {
    let resolveEnvironment!: (value: { appTranslocated: boolean }) => void;
    const environment = new Promise<{ appTranslocated: boolean }>((resolve) => {
      resolveEnvironment = resolve;
    });
    const download = vi.fn().mockResolvedValue(undefined);
    mocks.check.mockResolvedValue({
      available: true,
      version: '0.24.2',
      body: 'Release notes',
      rawJson: {},
      download,
      install: vi.fn(),
    });
    mocks.getUpdateInstallEnvironment.mockReturnValue(environment);

    await act(async () => current.checkForUpdate());
    let first!: Promise<unknown>;
    let second!: Promise<unknown>;
    await act(async () => {
      first = current.startDownload();
      second = current.startDownload();
      await Promise.resolve();
    });

    expect(current.updateStatus).toEqual({ phase: 'preparing', version: '0.24.2' });
    expect(mocks.getUpdateInstallEnvironment).toHaveBeenCalledOnce();

    resolveEnvironment({ appTranslocated: false });
    await act(async () => Promise.all([first, second]));
    expect(download).toHaveBeenCalledOnce();
    expect(mocks.relaunch).not.toHaveBeenCalled();
  });

  it('waits out an in-flight check instead of dropping the download click', async () => {
    const download = vi.fn().mockResolvedValue(undefined);
    const update = {
      available: true,
      version: '0.24.2',
      body: '',
      rawJson: {},
      download,
      install: vi.fn(),
    };
    mocks.check.mockResolvedValue(update);
    await act(async () => current.checkForUpdate());
    expect(current.updateStatus).toMatchObject({ phase: 'available', version: '0.24.2' });

    // A second check is still in flight when Download is clicked.
    let resolveCheck!: (value: typeof update) => void;
    mocks.check.mockReturnValue(
      new Promise((resolve) => {
        resolveCheck = resolve;
      })
    );
    let pendingCheck!: Promise<unknown>;
    let install!: Promise<unknown>;
    await act(async () => {
      pendingCheck = current.checkForUpdate();
      install = current.startDownload();
      await Promise.resolve();
    });
    expect(download).not.toHaveBeenCalled();

    resolveCheck(update);
    await act(async () => Promise.all([pendingCheck, install]));
    expect(download).toHaveBeenCalledOnce();
    expect(mocks.relaunch).not.toHaveBeenCalled();
  });

  it('keeps the download dialog open when a background check finishes first', async () => {
    const download = vi.fn().mockResolvedValue(undefined);
    const update = {
      available: true,
      version: '0.24.2',
      body: '',
      rawJson: {},
      download,
      install: vi.fn(),
    };
    let resolveCheck!: (value: typeof update) => void;
    mocks.check.mockReturnValue(new Promise((resolve) => {
      resolveCheck = resolve;
    }));

    await act(async () => root.unmount());
    automaticChecksEnabled = true;
    root = createRoot(container);
    await act(async () => {
      root.render(<Harness />);
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(mocks.check).toHaveBeenCalledOnce();

    let pendingDownload!: Promise<unknown>;
    await act(async () => {
      pendingDownload = current.startDownload();
      await Promise.resolve();
    });
    expect(current.isUpdateDialogOpen).toBe(true);

    resolveCheck(update);
    await act(async () => pendingDownload);

    expect(download).toHaveBeenCalledOnce();
    expect(current.isUpdateDialogOpen).toBe(true);
    expect(current.updateStatus).toEqual({
      phase: 'ready',
      version: '0.24.2',
      isForced: false,
    });
  });

  it('ignores a manual check while download owns the updater', async () => {
    let resolveEnvironment!: (value: { appTranslocated: boolean }) => void;
    const environment = new Promise<{ appTranslocated: boolean }>((resolve) => {
      resolveEnvironment = resolve;
    });
    const download = vi.fn().mockResolvedValue(undefined);
    mocks.check.mockResolvedValue({
      available: true,
      version: '0.24.2',
      body: '',
      rawJson: {},
      download,
      install: vi.fn(),
    });
    mocks.getUpdateInstallEnvironment.mockReturnValue(environment);

    await act(async () => current.checkForUpdate());
    let install!: Promise<unknown>;
    await act(async () => {
      install = current.startDownload();
      await Promise.resolve();
    });
    await act(async () => current.checkForUpdate());

    expect(mocks.check).toHaveBeenCalledOnce();
    expect(current.updateStatus).toEqual({ phase: 'preparing', version: '0.24.2' });

    resolveEnvironment({ appTranslocated: false });
    await act(async () => install);
  });

  it('ignores a due native wake check while download owns the updater', async () => {
    let wakeCheck: (() => void) | undefined;
    mocks.listen.mockImplementation(async (event: string, callback: () => void) => {
      if (event === 'updater-background-check-requested') wakeCheck = callback;
      return vi.fn();
    });
    const download = vi.fn().mockResolvedValue(undefined);
    mocks.check.mockResolvedValue({
      available: true,
      version: '0.24.2',
      body: '',
      rawJson: {},
      download,
      install: vi.fn(),
    });

    await act(async () => root.unmount());
    automaticChecksEnabled = true;
    root = createRoot(container);
    await act(async () => {
      root.render(<Harness />);
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    expect(current.updateStatus.phase).toBe('available');
    expect(mocks.check).toHaveBeenCalledOnce();
    expect(wakeCheck).toBeDefined();

    let resolveEnvironment!: (value: { appTranslocated: boolean }) => void;
    mocks.getUpdateInstallEnvironment.mockReturnValue(
      new Promise<{ appTranslocated: boolean }>((resolve) => {
        resolveEnvironment = resolve;
      }),
    );
    let install!: Promise<unknown>;
    await act(async () => {
      install = current.startDownload();
      await Promise.resolve();
    });
    localStorage.setItem('updater-last-check', '0');
    await act(async () => wakeCheck?.());

    expect(mocks.check).toHaveBeenCalledOnce();
    expect(current.updateStatus).toEqual({ phase: 'preparing', version: '0.24.2' });

    resolveEnvironment({ appTranslocated: false });
    await act(async () => install);

    expect(current.updateStatus).toEqual({
      phase: 'ready',
      version: '0.24.2',
      isForced: false,
    });
    localStorage.setItem('updater-last-check', '0');
    await act(async () => wakeCheck?.());
    expect(mocks.check).toHaveBeenCalledOnce();
  });

  it('removes withdrawn availability after an authoritative background check', async () => {
    let wakeCheck: (() => void) | undefined;
    mocks.listen.mockImplementation(async (event: string, callback: () => void) => {
      if (event === 'updater-background-check-requested') wakeCheck = callback;
      return vi.fn();
    });
    const download = vi.fn().mockResolvedValue(undefined);
    mocks.check.mockResolvedValueOnce({
      available: true,
      version: '0.24.2',
      body: '',
      rawJson: {},
      download,
      install: vi.fn(),
    });

    await act(async () => root.unmount());
    automaticChecksEnabled = true;
    root = createRoot(container);
    await act(async () => {
      root.render(<Harness />);
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    expect(current.updateStatus).toMatchObject({ phase: 'available', version: '0.24.2' });
    expect(wakeCheck).toBeDefined();

    await act(async () => current.showAvailableUpdate());
    expect(current.isUpdateDialogOpen).toBe(true);
    mocks.check.mockResolvedValueOnce({ available: false });
    localStorage.setItem('updater-last-check', '0');
    await act(async () => wakeCheck?.());

    expect(current.updateStatus).toEqual({ phase: 'idle' });
    expect(current.isUpdateDialogOpen).toBe(false);
    await act(async () => current.startDownload());
    expect(download).not.toHaveBeenCalled();
  });
});
