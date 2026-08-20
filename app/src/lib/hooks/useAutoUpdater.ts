import { useState, useEffect, useCallback, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { check, type Update } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';
import { getVersion } from '@tauri-apps/api/app';
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from '@tauri-apps/plugin-notification';
import { flog } from '../log';
import {
  type UpdateStatus,
  type CompletedUpdate,
  isBelowMinVersion,
  getSkippedVersion,
  setSkippedVersion,
  clearSkippedVersion,
  setPendingUpdate,
  clearPendingUpdate,
  getPendingUpdateForVersion,
  isDueForCheck,
  setLastCheckTimestamp,
  parseMinVersionPolicy,
  CHECK_TIMER_TICK_MS,
} from '../updater';
import { getUpdateInstallEnvironment } from '../updaterEnvironment';

const APP_TRANSLOCATION_MESSAGE =
  'macOS opened Murmur from a read-only security location. Quit Murmur, then use Finder to move or reinstall it in Applications before reopening it and trying the update again.';

type UpdaterOperation = 'idle' | 'checking' | 'installing';
type CanaryStage = 'pending' | 'passed' | 'failed';

export interface UpdaterCanaryResult {
  schemaVersion: 1;
  status: 'pending' | 'passed' | 'failed' | 'dry-run';
  checkedVersion: string;
  offeredVersion: string | null;
  forced: boolean;
  dryRun: boolean;
  stages: {
    discover: CanaryStage;
    policy: CanaryStage;
    download: CanaryStage;
    signatureVerify: CanaryStage;
    install: CanaryStage;
    relaunch: CanaryStage;
  };
  error: string | null;
}

interface UpdaterCanaryState {
  path: string | null;
  result: UpdaterCanaryResult | null;
  dryRun: boolean;
}

type CheckOutcome =
  | { kind: 'available'; version: string; forced: boolean; policyVerified: boolean }
  | { kind: 'current'; version: string }
  | { kind: 'failed'; version: string; stage: 'discover' | 'policy'; message: string };

type InstallOutcome =
  | { kind: 'installed'; version: string }
  | { kind: 'failed'; version: string; stage: 'download' | 'install' | 'relaunch'; message: string };

interface InstallLifecycle {
  onInstalled?: () => Promise<void>;
}

const UPDATE_CHECK_ATTEMPTS = 2;

interface UseAutoUpdaterOptions {
  automaticChecksEnabled?: boolean;
}

export interface UseAutoUpdaterReturn {
  updateStatus: UpdateStatus;
  completedUpdate: CompletedUpdate | null;
  isUpdateDialogOpen: boolean;
  checkForUpdate: () => Promise<void>;
  showAvailableUpdate: () => void;
  startDownload: (lifecycle?: InstallLifecycle) => Promise<InstallOutcome | undefined>;
  skipVersion: () => void;
  dismissUpdate: () => void;
  dismissCompletedUpdate: () => void;
}

export function useAutoUpdater(
  options: UseAutoUpdaterOptions = {},
): UseAutoUpdaterReturn {
  const automaticChecksEnabled = options.automaticChecksEnabled ?? !import.meta.env.DEV;
  const [updateStatus, setUpdateStatus] = useState<UpdateStatus>({ phase: 'idle' });
  const [completedUpdate, setCompletedUpdate] = useState<CompletedUpdate | null>(null);
  const [isUpdateDialogOpen, setIsUpdateDialogOpen] = useState(false);
  const updateRef = useRef<Update | null>(null);
  const operationRef = useRef<UpdaterOperation>('idle');
  // Resolves when the in-flight check settles, so an Install click that races
  // a background check waits instead of being silently dropped.
  const pendingCheckRef = useRef<Promise<void> | null>(null);
  const isForcedRef = useRef(false);
  const manualPresentationRequestedRef = useRef(false);
  const automaticStartupRef = useRef(false);

  // The updater process replaces the app before relaunching, so the release
  // payload is recovered from localStorage and shown only after the installed
  // version confirms that the update actually completed.
  useEffect(() => {
    let cancelled = false;
    getVersion()
      .then((currentVersion) => {
        if (cancelled) return;
        const completed = getPendingUpdateForVersion(currentVersion);
        if (completed) {
          flog.info('updater', 'showing post-update release notes', {
            version: completed.version,
          });
          setCompletedUpdate(completed);
        }
      })
      .catch((err) => {
        flog.warn('updater', 'could not resolve installed version for release notes', {
          error: String(err),
        });
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const performCheck = useCallback(async (opts: { isBackground: boolean; canary?: boolean }): Promise<CheckOutcome> => {
    if (operationRef.current === 'installing') {
      flog.info('updater', 'check ignored while install owns updater');
      return { kind: 'failed', version: 'unknown', stage: 'discover', message: 'Updater is installing.' };
    }
    if (!opts.isBackground) {
      clearSkippedVersion();
      manualPresentationRequestedRef.current = true;
      setUpdateStatus({ phase: 'checking' });
    }
    if (operationRef.current === 'checking') {
      return { kind: 'failed', version: 'unknown', stage: 'discover', message: 'Updater check already in progress.' };
    }
    operationRef.current = 'checking';
    let settleCheck!: () => void;
    pendingCheckRef.current = new Promise<void>((resolve) => {
      settleCheck = resolve;
    });
    isForcedRef.current = false;

    const shouldPresentManualResult = () =>
      !opts.isBackground || manualPresentationRequestedRef.current;

    try {
      let update: Update | null = null;
      let lastCheckError: unknown;
      let checkSucceeded = false;
      for (let attempt = 1; attempt <= UPDATE_CHECK_ATTEMPTS; attempt += 1) {
        try {
          update = await check();
          checkSucceeded = true;
          break;
        } catch (error) {
          lastCheckError = error;
          if (attempt < UPDATE_CHECK_ATTEMPTS) {
            flog.warn('updater', 'check failed; retrying', {
              attempt,
              error: String(error),
            });
          }
        }
      }
      if (!checkSucceeded) throw lastCheckError;

      if (!update?.available || !update.version) {
        setLastCheckTimestamp(Date.now());
        flog.info('updater', 'no update available', {
          event_code: 'updater.check_current',
        });
        if (shouldPresentManualResult()) {
          setIsUpdateDialogOpen(false);
          setUpdateStatus({ phase: 'up-to-date' });
          // Reset back to idle after a brief display
          setTimeout(() => setUpdateStatus(s => s.phase === 'up-to-date' ? { phase: 'idle' } : s), 3000);
        }
        return { kind: 'current', version: await getVersion() };
      }

      flog.info('updater', 'update available', { version: update.version });

      // Tauri exposes the exact native response as rawJson, so policy parsing
      // does not need a second cross-origin request from the webview.
      const policy = parseMinVersionPolicy(update.rawJson);
      if (policy.status === 'unavailable') {
        flog.warn('updater', 'could not verify update policy', {
          error: policy.message,
        });
      }
      // A secondary policy read may fail to force an update; it must never
      // fail to offer a verified update.
      let isForced = false;
      let currentVersion: string = 'unknown';
      if (policy.status === 'present') {
        currentVersion = await getVersion();
        isForced = isBelowMinVersion(currentVersion, policy.minVersion);
      }
      setLastCheckTimestamp(Date.now());

      // If not forced and user previously skipped this version, suppress
      if (!opts.canary && !isForced && getSkippedVersion() === update.version) {
        flog.info('updater', 'user skipped this version', { version: update.version });
        if (shouldPresentManualResult()) {
          setUpdateStatus({ phase: 'idle' });
        }
        return { kind: 'current', version: currentVersion };
      }

      const wasAlreadyAvailable = updateRef.current?.version === update.version;
      updateRef.current = update;
      isForcedRef.current = isForced;
      setUpdateStatus({
        phase: 'available',
        version: update.version,
        notes: update.body ?? '',
        isForced,
      });
      setIsUpdateDialogOpen(isForced || shouldPresentManualResult());

      // Background check: fire macOS notification
      if (opts.isBackground && !opts.canary && !wasAlreadyAvailable) {
        try {
          let permGranted = await isPermissionGranted();
          if (!permGranted) {
            const perm = await requestPermission();
            permGranted = perm === 'granted';
          }
          if (permGranted) {
            sendNotification({
              title: 'Update Available',
              body: `Murmur v${update.version} is ready to install.`,
            });
          }
        } catch (err) {
          flog.warn('updater', 'notification failed', { error: String(err) });
        }
      }
      return {
        kind: 'available',
        version: update.version,
        forced: isForced,
        policyVerified: policy.status !== 'unavailable',
      };
    } catch (err) {
      flog.error('updater', 'check failed', {
        event_code: 'updater.check_failed',
        error: String(err),
      });
      if (shouldPresentManualResult()) {
        setIsUpdateDialogOpen(true);
        setUpdateStatus({
          phase: 'error',
          stage: 'check',
          message: String(err),
          isForced: isForcedRef.current,
        });
      }
      // Background errors are silent
      return { kind: 'failed', version: 'unknown', stage: 'discover', message: String(err) };
    } finally {
      if (operationRef.current === 'checking') {
        operationRef.current = 'idle';
      }
      pendingCheckRef.current = null;
      settleCheck();
      manualPresentationRequestedRef.current = false;
    }
  }, []);

  const checkForUpdate = useCallback(async () => {
    await performCheck({ isBackground: false });
  }, [performCheck]);

  const showAvailableUpdate = useCallback(() => {
    if (
      updateStatus.phase === 'available' ||
      (updateStatus.phase === 'error' && updateStatus.stage === 'install')
    ) {
      setIsUpdateDialogOpen(true);
    }
  }, [updateStatus]);

  const startDownload = useCallback(async (lifecycle?: InstallLifecycle): Promise<InstallOutcome | undefined> => {
    if (operationRef.current === 'checking') {
      flog.info('updater', 'install waiting for in-flight update check');
      await pendingCheckRef.current;
    }
    if (operationRef.current !== 'idle') {
      flog.info('updater', 'install ignored because updater is already busy', {
        operation: operationRef.current,
      });
      return undefined;
    }
    const update = updateRef.current;
    if (!update) return undefined;

    const version =
      updateStatus.phase === 'available' ? updateStatus.version
      : updateRef.current?.version ?? 'unknown';
    let downloadFinished = false;
    let relaunchStarted = false;
    operationRef.current = 'installing';
    setUpdateStatus({ phase: 'preparing', version });

    try {
      const environment = await getUpdateInstallEnvironment();
      if (environment.appTranslocated) {
        flog.warn('updater', 'install blocked by macOS App Translocation', {
          event_code: 'updater.install_blocked',
        });
        setUpdateStatus({
          phase: 'error',
          stage: 'install',
          message: APP_TRANSLOCATION_MESSAGE,
          isForced: isForcedRef.current,
          recovery: 'reinstall',
        });
        operationRef.current = 'idle';
        return { kind: 'failed', version, stage: 'install', message: APP_TRANSLOCATION_MESSAGE };
      }

      setUpdateStatus({ phase: 'downloading', version, progress: 0 });
      flog.info('updater', 'starting download', { version });
      setPendingUpdate({ version, notes: update.body ?? '' });

      let totalContentLength = 0;
      let totalDownloaded = 0;
      await update.downloadAndInstall((event) => {
        switch (event.event) {
          case 'Started':
            totalContentLength = event.data.contentLength ?? 0;
            flog.info('updater', 'download started', { contentLength: totalContentLength });
            break;
          case 'Progress':
            totalDownloaded += event.data.chunkLength;
            setUpdateStatus({
              phase: 'downloading',
              version,
              progress: totalContentLength > 0
                ? Math.round((totalDownloaded / totalContentLength) * 100)
                : 0,
            });
            break;
          case 'Finished':
            flog.info('updater', 'download finished');
            downloadFinished = true;
            break;
        }
      });

      setUpdateStatus({ phase: 'ready', version });
      flog.info('updater', 'installed, relaunching', {
        event_code: 'updater.install_ready',
      });
      clearSkippedVersion();
      await lifecycle?.onInstalled?.();
      relaunchStarted = true;
      await relaunch();
      return { kind: 'installed', version };
    } catch (err) {
      operationRef.current = 'idle';
      flog.error('updater', 'download/install failed', {
        event_code: 'updater.install_failed',
        error: String(err),
      });
      setUpdateStatus({
        phase: 'error',
        stage: 'install',
        message: String(err),
        isForced: isForcedRef.current,
      });
      return {
        kind: 'failed',
        version,
        stage: relaunchStarted ? 'relaunch' : downloadFinished ? 'install' : 'download',
        message: String(err),
      };
    }
  }, [updateStatus]);

  const writeCanary = useCallback(async (result: UpdaterCanaryResult) => {
    await invoke<UpdaterCanaryState>('updater_canary', {
      request: { action: 'write', result },
    });
  }, []);

  const runCanary = useCallback(async (previous: UpdaterCanaryResult | null, dryRun: boolean) => {
    const checkedVersion = await getVersion();
    // A pending result with the offered version means this is the post-install
    // launch. The running version is the final assertion of the OTA.
    if (!dryRun && previous?.status === 'pending' && previous.offeredVersion === checkedVersion) {
      await writeCanary({
        ...previous,
        status: 'passed',
        stages: { ...previous.stages, relaunch: 'passed' },
        error: null,
      });
      return;
    }

    const outcome = await performCheck({ isBackground: true, canary: true });
    if (outcome.kind === 'available' && !outcome.policyVerified) {
      await writeCanary({
        schemaVersion: 1,
        status: 'failed',
        checkedVersion,
        offeredVersion: outcome.version,
        forced: outcome.forced,
        dryRun,
        stages: {
          discover: 'passed',
          policy: 'failed',
          download: 'pending',
          signatureVerify: 'pending',
          install: 'pending',
          relaunch: 'pending',
        },
        error: 'Update policy could not be parsed from the native updater response.',
      });
      return;
    }

    if (dryRun) {
      await writeCanary({
        schemaVersion: 1,
        status: outcome.kind === 'available' ? 'dry-run' : 'failed',
        checkedVersion,
        offeredVersion: outcome.kind === 'available' ? outcome.version : null,
        forced: outcome.kind === 'available' ? outcome.forced : false,
        dryRun: true,
        stages: {
          discover: outcome.kind === 'failed' && outcome.stage === 'discover' ? 'failed' : 'passed',
          policy: outcome.kind === 'failed' && outcome.stage === 'policy' ? 'failed' : 'passed',
          download: 'pending',
          signatureVerify: 'pending',
          install: 'pending',
          relaunch: 'pending',
        },
        error: outcome.kind === 'available' ? null : outcome.kind === 'current'
          ? 'No update was available for the canary dry run.'
          : outcome.message,
      });
      return;
    }

    if (outcome.kind !== 'available') {
      await writeCanary({
        schemaVersion: 1,
        status: 'failed',
        checkedVersion,
        offeredVersion: null,
        forced: false,
        dryRun: false,
        stages: {
          discover: outcome.kind === 'current' ? 'passed' : 'failed',
          policy: outcome.kind === 'current' ? 'passed' : outcome.stage === 'policy' ? 'failed' : 'pending',
          download: 'pending',
          signatureVerify: 'pending',
          install: 'pending',
          relaunch: 'pending',
        },
        error: outcome.kind === 'current'
          ? 'No update was available for the canary run.'
          : outcome.message,
      });
      return;
    }

    await writeCanary({
      schemaVersion: 1,
      status: 'pending',
      checkedVersion,
      offeredVersion: outcome.version,
      forced: outcome.forced,
      dryRun: false,
      stages: {
        discover: 'passed',
        policy: 'passed',
        download: 'pending',
        signatureVerify: 'pending',
        install: 'pending',
        relaunch: 'pending',
      },
      error: null,
    });

    const install = await startDownload({
      onInstalled: () => writeCanary({
        schemaVersion: 1,
        status: 'pending',
        checkedVersion,
        offeredVersion: outcome.version,
        forced: outcome.forced,
        dryRun: false,
        stages: {
          discover: 'passed',
          policy: 'passed',
          download: 'passed',
          signatureVerify: 'passed',
          install: 'passed',
          relaunch: 'pending',
        },
        error: null,
      }),
    });
    if (!install || install.kind === 'installed') return;
    await writeCanary({
      schemaVersion: 1,
      status: 'failed',
      checkedVersion,
      offeredVersion: outcome.version,
      forced: outcome.forced,
      dryRun: false,
      stages: {
        discover: 'passed',
        policy: 'passed',
        download: install.stage === 'download' ? 'failed' : 'passed',
        signatureVerify: install.stage === 'download' ? 'pending' : 'passed',
        install: install.stage === 'download' ? 'pending' : install.stage === 'install' ? 'failed' : 'passed',
        relaunch: install.stage === 'relaunch' ? 'failed' : 'pending',
      },
      error: install.message,
    });
  }, [performCheck, startDownload, writeCanary]);

  // On mount, inspect the opt-in canary marker before the normal launch check.
  // An absent marker falls straight through to the existing updater behavior.
  useEffect(() => {
    if (!automaticChecksEnabled) {
      flog.info('updater', 'dev build — skipping automatic update check');
      return;
    }
    if (automaticStartupRef.current) return;
    automaticStartupRef.current = true;

    let disposed = false;
    const start = async () => {
      try {
        const canary = await invoke<UpdaterCanaryState>('updater_canary', {
          request: { action: 'read' },
        });
        if (disposed) return;
        if (canary.path) await runCanary(canary.result, canary.dryRun);
        else await performCheck({ isBackground: true });
      } catch (error) {
        flog.warn('updater', 'could not inspect updater canary state', { error: String(error) });
        if (!disposed) await performCheck({ isBackground: true });
      }
    };
    start();

    const checkIfDue = () => {
      if (isDueForCheck()) performCheck({ isBackground: true });
    };
    const interval = setInterval(checkIfDue, CHECK_TIMER_TICK_MS);
    const onFocus = () => checkIfDue();
    const onVisibilityChange = () => {
      if (!document.hidden) checkIfDue();
    };
    window.addEventListener('focus', onFocus);
    document.addEventListener('visibilitychange', onVisibilityChange);

    let unlistenWake: (() => void) | undefined;
    listen<unknown>('updater-background-check-requested', checkIfDue)
      .then((unlisten) => {
        if (disposed) unlisten();
        else unlistenWake = unlisten;
      })
      .catch((err) => {
        flog.warn('updater', 'could not listen for native wake checks', { error: String(err) });
      });

    return () => {
      disposed = true;
      clearInterval(interval);
      window.removeEventListener('focus', onFocus);
      document.removeEventListener('visibilitychange', onVisibilityChange);
      unlistenWake?.();
    };
  }, [automaticChecksEnabled, performCheck, runCanary]);

  const skipVersion = useCallback(() => {
    if (updateStatus.phase === 'available') {
      setSkippedVersion(updateStatus.version);
      flog.info('updater', 'version skipped', { version: updateStatus.version });
    }
    updateRef.current = null;
    setIsUpdateDialogOpen(false);
    setUpdateStatus({ phase: 'idle' });
  }, [updateStatus]);

  const dismissUpdate = useCallback(() => {
    setIsUpdateDialogOpen(false);
  }, []);

  const dismissCompletedUpdate = useCallback(() => {
    clearPendingUpdate();
    setCompletedUpdate(null);
  }, []);

  return {
    updateStatus,
    completedUpdate,
    isUpdateDialogOpen,
    checkForUpdate,
    showAvailableUpdate,
    startDownload,
    skipVersion,
    dismissUpdate,
    dismissCompletedUpdate,
  };
}
