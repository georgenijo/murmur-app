import { useState, useEffect, useCallback, useRef } from 'react';
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
  fetchMinVersionPolicy,
  CHECK_TIMER_TICK_MS,
} from '../updater';
import { getUpdateInstallEnvironment } from '../updaterEnvironment';

const APP_TRANSLOCATION_MESSAGE =
  'macOS opened Murmur from a read-only security location. Quit Murmur, then use Finder to move or reinstall it in Applications before reopening it and trying the update again.';
const UPDATE_POLICY_UNAVAILABLE_MESSAGE =
  'Could not verify the update policy. Check your connection and try again.';

type UpdaterOperation = 'idle' | 'checking' | 'installing';

interface UseAutoUpdaterOptions {
  automaticChecksEnabled?: boolean;
}

export interface UseAutoUpdaterReturn {
  updateStatus: UpdateStatus;
  completedUpdate: CompletedUpdate | null;
  isUpdateDialogOpen: boolean;
  checkForUpdate: () => Promise<void>;
  showAvailableUpdate: () => void;
  startDownload: () => Promise<void>;
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

  const performCheck = useCallback(async (opts: { isBackground: boolean }) => {
    if (operationRef.current === 'installing') {
      flog.info('updater', 'check ignored while install owns updater');
      return;
    }
    if (!opts.isBackground) {
      clearSkippedVersion();
      manualPresentationRequestedRef.current = true;
      setUpdateStatus({ phase: 'checking' });
    }
    if (operationRef.current === 'checking') return;
    operationRef.current = 'checking';
    let settleCheck!: () => void;
    pendingCheckRef.current = new Promise<void>((resolve) => {
      settleCheck = resolve;
    });
    isForcedRef.current = false;

    const shouldPresentManualResult = () =>
      !opts.isBackground || manualPresentationRequestedRef.current;

    try {
      const update = await check();

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
        return;
      }

      flog.info('updater', 'update available', { version: update.version });

      // Check min_version (custom field not exposed by Tauri updater)
      const currentVersion = await getVersion();
      const policy = await fetchMinVersionPolicy();
      if (policy.status === 'unavailable') {
        flog.warn('updater', 'could not verify update policy', {
          error: policy.message,
        });
        if (shouldPresentManualResult()) {
          setIsUpdateDialogOpen(false);
          setUpdateStatus({
            phase: 'error',
            stage: 'check',
            message: UPDATE_POLICY_UNAVAILABLE_MESSAGE,
            isForced: false,
          });
        }
        return;
      }
      setLastCheckTimestamp(Date.now());
      const isForced =
        policy.status === 'present' &&
        isBelowMinVersion(currentVersion, policy.minVersion);

      // If not forced and user previously skipped this version, suppress
      if (!isForced && getSkippedVersion() === update.version) {
        flog.info('updater', 'user skipped this version', { version: update.version });
        if (shouldPresentManualResult()) {
          setUpdateStatus({ phase: 'idle' });
        }
        return;
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
      if (opts.isBackground && !wasAlreadyAvailable) {
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
    } catch (err) {
      flog.error('updater', 'check failed', {
        event_code: 'updater.check_failed',
        error: String(err),
      });
      if (shouldPresentManualResult()) {
        setIsUpdateDialogOpen(false);
        setUpdateStatus({
          phase: 'error',
          stage: 'check',
          message: String(err),
          isForced: isForcedRef.current,
        });
      }
      // Background errors are silent
    } finally {
      if (operationRef.current === 'checking') {
        operationRef.current = 'idle';
      }
      pendingCheckRef.current = null;
      settleCheck();
      manualPresentationRequestedRef.current = false;
    }
  }, []);

  // On mount: always check on launch. A short, inert timer only asks whether
  // the six-hour network interval is due; native wake events and foreground
  // activation use the same gate so hidden/sleeping webviews do not strand the
  // update indicator.
  // Skip entirely in dev — a dev build auto-updating to a prod release would
  // download+relaunch into /Applications, making `tauri dev` impossible.
  useEffect(() => {
    if (!automaticChecksEnabled) {
      flog.info('updater', 'dev build — skipping automatic update check');
      return;
    }

    performCheck({ isBackground: true });

    const checkIfDue = () => {
      if (isDueForCheck()) {
        performCheck({ isBackground: true });
      }
    };
    const interval = setInterval(checkIfDue, CHECK_TIMER_TICK_MS);
    const onFocus = () => checkIfDue();
    const onVisibilityChange = () => {
      if (!document.hidden) checkIfDue();
    };
    window.addEventListener('focus', onFocus);
    document.addEventListener('visibilitychange', onVisibilityChange);

    let disposed = false;
    let unlistenWake: (() => void) | undefined;
    listen<unknown>('updater-background-check-requested', checkIfDue)
      .then((unlisten) => {
        if (disposed) unlisten();
        else unlistenWake = unlisten;
      })
      .catch((err) => {
        flog.warn('updater', 'could not listen for native wake checks', {
          error: String(err),
        });
      });

    return () => {
      disposed = true;
      clearInterval(interval);
      window.removeEventListener('focus', onFocus);
      document.removeEventListener('visibilitychange', onVisibilityChange);
      unlistenWake?.();
    };
  }, [automaticChecksEnabled, performCheck]);

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

  const startDownload = useCallback(async () => {
    if (operationRef.current === 'checking') {
      flog.info('updater', 'install waiting for in-flight update check');
      await pendingCheckRef.current;
    }
    if (operationRef.current !== 'idle') {
      flog.info('updater', 'install ignored because updater is already busy', {
        operation: operationRef.current,
      });
      return;
    }
    const update = updateRef.current;
    if (!update) return;

    const version =
      updateStatus.phase === 'available' ? updateStatus.version
      : updateRef.current?.version ?? 'unknown';
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
        return;
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
            break;
        }
      });

      setUpdateStatus({ phase: 'ready', version });
      flog.info('updater', 'installed, relaunching', {
        event_code: 'updater.install_ready',
      });
      clearSkippedVersion();
      await relaunch();
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
    }
  }, [updateStatus]);

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
