import { useCallback, useEffect, useRef, useState } from 'react';
import type { useAutoUpdater } from './useAutoUpdater';
import type { CompletedUpdate, UpdateStatus } from '../updater';

const DEV_RELEASE = {
  version: '0.44.2',
  notes: '## What’s New\n\n- Download updates directly from Settings or the toolbar.\n- Restart when you’re ready.\n- A simpler update dialog.',
};

/** Exercise the complete flow in a dev bundle without replacing that bundle. */
export function useDevUpdaterMock(updater: ReturnType<typeof useAutoUpdater>) {
  const [devStatus, setDevStatus] = useState<UpdateStatus | null>(null);
  const [devCompleted, setDevCompleted] = useState<CompletedUpdate | null>(null);
  const [devDialogOpen, setDevDialogOpen] = useState(false);
  const timers = useRef<ReturnType<typeof setTimeout>[]>([]);
  const downloading = useRef(false);

  useEffect(() => () => timers.current.forEach(clearTimeout), []);

  const checkForUpdate = useCallback(async () => {
    if (!import.meta.env.DEV) return updater.checkForUpdate();
    if (downloading.current || devStatus?.phase === 'ready') {
      setDevDialogOpen(true);
      return;
    }
    setDevStatus({ phase: 'available', ...DEV_RELEASE, isForced: false });
    setDevDialogOpen(true);
  }, [devStatus, updater.checkForUpdate]);

  const showAvailableUpdate = useCallback(() => {
    if (devStatus) setDevDialogOpen(true);
    else updater.showAvailableUpdate();
  }, [devStatus, updater.showAvailableUpdate]);

  const dismissUpdate = useCallback(() => {
    if (devStatus) setDevDialogOpen(false);
    else updater.dismissUpdate();
  }, [devStatus, updater.dismissUpdate]);

  const startDownload = useCallback(async () => {
    if (!devStatus) return updater.startDownload();
    if (downloading.current || devStatus.phase !== 'available') return;
    downloading.current = true;
    setDevDialogOpen(true);
    setDevStatus({ phase: 'downloading', version: DEV_RELEASE.version, progress: 0 });
    timers.current = [25, 60, 100].map((progress, index) => setTimeout(() => {
      setDevStatus({ phase: 'downloading', version: DEV_RELEASE.version, progress });
    }, (index + 1) * 500));
    timers.current.push(setTimeout(() => {
      downloading.current = false;
      setDevStatus({ phase: 'ready', version: DEV_RELEASE.version, isForced: false });
    }, 2000));
  }, [devStatus, updater.startDownload]);

  const restartUpdate = useCallback(async () => {
    if (!devStatus) return updater.restartUpdate();
    if (devStatus.phase !== 'ready') return;
    setDevStatus(null);
    setDevDialogOpen(false);
    setDevCompleted(DEV_RELEASE);
  }, [devStatus, updater.restartUpdate]);

  const dismissCompletedUpdate = useCallback(() => {
    if (devCompleted) setDevCompleted(null);
    else updater.dismissCompletedUpdate();
  }, [devCompleted, updater.dismissCompletedUpdate]);

  return {
    checkForUpdate,
    updateStatus: devStatus ?? updater.updateStatus,
    isUpdateDialogOpen: devStatus ? devDialogOpen : updater.isUpdateDialogOpen,
    showAvailableUpdate,
    dismissUpdate,
    startDownload,
    restartUpdate,
    completedUpdate: devCompleted ?? updater.completedUpdate,
    dismissCompletedUpdate,
  };
}
