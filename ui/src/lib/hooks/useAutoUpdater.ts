import { useState, useEffect, useCallback, useRef } from 'react';
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
  isBelowMinVersion,
  getSkippedVersion,
  setSkippedVersion,
  clearSkippedVersion,
  isDueForCheck,
  setLastCheckTimestamp,
  fetchMinVersion,
  CHECK_INTERVAL_MS,
} from '../updater';

export interface UseAutoUpdaterReturn {
  updateStatus: UpdateStatus;
  checkForUpdate: () => Promise<void>;
  startDownload: () => Promise<void>;
  skipVersion: () => void;
  dismissUpdate: () => void;
}

export function useAutoUpdater(): UseAutoUpdaterReturn {
  const [updateStatus, setUpdateStatus] = useState<UpdateStatus>({ phase: 'idle' });
  const updateRef = useRef<Update | null>(null);
  const isCheckingRef = useRef(false);
  const isForcedRef = useRef(false);

  const performCheck = useCallback(async (opts: { isBackground: boolean }) => {
    if (isCheckingRef.current) return;
    isCheckingRef.current = true;

    if (!opts.isBackground) {
      setUpdateStatus({ phase: 'checking' });
    }

    try {
      const update = await check();
      setLastCheckTimestamp(Date.now());

      if (!update?.available || !update.version) {
        flog.info('updater', 'no update available');
        if (!opts.isBackground) {
          setUpdateStatus({ phase: 'up-to-date' });
          // Reset back to idle after a brief display
          setTimeout(() => setUpdateStatus(s => s.phase === 'up-to-date' ? { phase: 'idle' } : s), 3000);
        }
        isCheckingRef.current = false;
        return;
      }

      flog.info('updater', 'update available', { version: update.version });

      // Check min_version (custom field not exposed by Tauri updater)
      const currentVersion = await getVersion();
      const minVersion = await fetchMinVersion();
      const isForced = minVersion !== null && isBelowMinVersion(currentVersion, minVersion);

      // If not forced and user previously skipped this version, suppress
      if (!isForced && getSkippedVersion() === update.version) {
        flog.info('updater', 'user skipped this version', { version: update.version });
        isCheckingRef.current = false;
        return;
      }

      updateRef.current = update;
      isForcedRef.current = isForced;
      setUpdateStatus({
        phase: 'available',
        version: update.version,
        notes: update.body ?? '',
        isForced,
      });

      // Background check: fire macOS notification
      if (opts.isBackground) {
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
      flog.error('updater', 'check failed', { error: String(err) });
      if (!opts.isBackground) {
        setUpdateStatus({ phase: 'error', message: String(err), isForced: isForcedRef.current });
      }
      // Background errors are silent
    } finally {
      isCheckingRef.current = false;
    }
  }, []);

  // On mount: always check on launch, then set up 24h periodic interval
  useEffect(() => {
    performCheck({ isBackground: true });

    const interval = setInterval(() => {
      if (isDueForCheck()) {
        performCheck({ isBackground: true });
      }
    }, CHECK_INTERVAL_MS);

    return () => clearInterval(interval);
  }, [performCheck]);

  const checkForUpdate = useCallback(async () => {
    await performCheck({ isBackground: false });
  }, [performCheck]);

  const startDownload = useCallback(async () => {
    const update = updateRef.current;
    if (!update) return;

    const version =
      updateStatus.phase === 'available' ? updateStatus.version
      : updateRef.current?.version ?? 'unknown';

    setUpdateStatus({ phase: 'downloading', version, progress: 0 });
    flog.info('updater', 'starting download', { version });

    try {
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
      flog.info('updater', 'installed, relaunching');
      clearSkippedVersion();
      await relaunch();
    } catch (err) {
      flog.error('updater', 'download/install failed', { error: String(err) });
      setUpdateStatus({ phase: 'error', message: String(err), isForced: isForcedRef.current });
    }
  }, [updateStatus]);

  const skipVersion = useCallback(() => {
    if (updateStatus.phase === 'available') {
      setSkippedVersion(updateStatus.version);
      flog.info('updater', 'version skipped', { version: updateStatus.version });
    }
    updateRef.current = null;
    setUpdateStatus({ phase: 'idle' });
  }, [updateStatus]);

  const dismissUpdate = useCallback(() => {
    setUpdateStatus({ phase: 'idle' });
  }, []);

  return { updateStatus, checkForUpdate, startDownload, skipVersion, dismissUpdate };
}
