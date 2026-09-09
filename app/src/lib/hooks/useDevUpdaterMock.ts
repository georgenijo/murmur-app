import { useCallback, useRef, useState } from 'react';
import type { useAutoUpdater } from './useAutoUpdater';
import type { CompletedUpdate, UpdateStatus } from '../updater';

/**
 * DEV-only cycling mock for the updater/post-update-modal states, used for
 * visual testing (`npm run tauri:dev`). Extracted verbatim from `App.tsx`.
 *
 * Always calls the same hooks regardless of `import.meta.env.DEV` (Rules of
 * Hooks) — the mock arrays/state are simply empty/unused outside dev, and
 * `checkForUpdate` falls straight through to the real `updater.checkForUpdate`
 * in production builds.
 */
export function useDevUpdaterMock(updater: ReturnType<typeof useAutoUpdater>) {
  const devUpdateIndex = useRef(-1);
  const devMockStates: UpdateStatus[] = import.meta.env.DEV ? [
    {
      phase: 'error',
      stage: 'install',
      message: 'macOS opened Murmur from a read-only security location. Quit Murmur, then use Finder to move or reinstall it in Applications before reopening it and trying the update again.',
      isForced: false,
      recovery: 'reinstall',
    },
    { phase: 'available', version: '0.7.0', notes: '## What\'s New\n- OTA auto-updater\n- Bug fixes\n- Performance improvements', isForced: false },
    { phase: 'available', version: '0.7.0', notes: 'Critical security fix.', isForced: true },
    { phase: 'preparing', version: '0.7.0' },
    { phase: 'downloading', version: '0.7.0', progress: 65 },
  ] : [];
  const [devUpdateStatus, setDevUpdateStatus] = useState<UpdateStatus | null>(null);
  const [devCompletedUpdate, setDevCompletedUpdate] = useState<CompletedUpdate | null>(null);
  const [devUpdateDialogOpen, setDevUpdateDialogOpen] = useState(false);

  const checkForUpdate = useCallback(async () => {
    if (import.meta.env.DEV) {
      devUpdateIndex.current = (devUpdateIndex.current + 1) % (devMockStates.length + 1);
      if (devUpdateIndex.current === 0) {
        setDevUpdateStatus(null);
        setDevUpdateDialogOpen(false);
        setDevCompletedUpdate({
          version: '0.22.0',
          notes: '## New Features\n\n- Faster local transcription\n- Selected-text transforms\n\n## Bug Fixes\n\n- More reliable microphone startup\n- Smoother overlay behavior',
        });
      } else {
        setDevCompletedUpdate(null);
        setDevUpdateStatus(devMockStates[devUpdateIndex.current - 1]);
        setDevUpdateDialogOpen(true);
      }
      return;
    }
    return updater.checkForUpdate();
  }, [updater.checkForUpdate]);

  const updateStatus = devUpdateStatus ?? updater.updateStatus;
  const isUpdateDialogOpen = devUpdateStatus
    ? devUpdateDialogOpen
    : updater.isUpdateDialogOpen;
  const showAvailableUpdate = useCallback(() => {
    if (
      devUpdateStatus?.phase === 'available' ||
      (devUpdateStatus?.phase === 'error' && devUpdateStatus.stage === 'install')
    ) {
      setDevUpdateDialogOpen(true);
      return;
    }
    updater.showAvailableUpdate();
  }, [devUpdateStatus, updater.showAvailableUpdate]);
  const dismissUpdate = useCallback(() => {
    if (devUpdateStatus) { setDevUpdateDialogOpen(false); return; }
    updater.dismissUpdate();
  }, [devUpdateStatus, updater.dismissUpdate]);
  const skipVersion = useCallback(() => {
    if (devUpdateStatus) {
      setDevUpdateDialogOpen(false);
      setDevUpdateStatus(null);
      return;
    }
    updater.skipVersion();
  }, [devUpdateStatus, updater.skipVersion]);
  const startDownload = updater.startDownload;
  const completedUpdate = devCompletedUpdate ?? updater.completedUpdate;
  const dismissCompletedUpdate = useCallback(() => {
    if (devCompletedUpdate) {
      setDevCompletedUpdate(null);
      return;
    }
    updater.dismissCompletedUpdate();
  }, [devCompletedUpdate, updater.dismissCompletedUpdate]);

  return {
    checkForUpdate,
    updateStatus,
    isUpdateDialogOpen,
    showAvailableUpdate,
    dismissUpdate,
    skipVersion,
    startDownload,
    completedUpdate,
    dismissCompletedUpdate,
  };
}
