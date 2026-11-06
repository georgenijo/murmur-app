import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '../targetedEvents';
import type { UpdaterSnapshot } from '../updater';

interface UseAutoUpdaterOptions { automaticChecksEnabled?: boolean; setupComplete?: boolean }
type Action =
  | { kind: 'configure'; automatic: boolean }
  | { kind: 'check'; background: boolean }
  | { kind: 'download' | 'install' | 'show' | 'dismiss' | 'dismissCompleted' };

/** React renders native state and forwards user/lifecycle intent only. */
export function useAutoUpdater(options: UseAutoUpdaterOptions = {}) {
  const [snapshot, setSnapshot] = useState<UpdaterSnapshot>({
    revision: -1, updateStatus: { phase: 'idle' }, completedUpdate: null, isUpdateDialogOpen: false,
  });
  const alive = useRef(true);
  const apply = useCallback((next: UpdaterSnapshot) => {
    if (alive.current && next) setSnapshot(previous => next.revision >= previous.revision ? next : previous);
  }, []);
  const action = useCallback(async (intent: Action): Promise<void> => {
    try { apply(await invoke<UpdaterSnapshot>('updater_action', { action: intent })); }
    catch {
      // Operational failures are already in Rust state; preserve their stage,
      // retained download and recovery action instead of reconstructing them.
      try { apply(await invoke<UpdaterSnapshot>('updater_snapshot')); } catch { /* Startup retry below. */ }
    }
  }, [apply]);
  const automatic = (options.automaticChecksEnabled ?? !import.meta.env.DEV) && (options.setupComplete ?? true);
  useEffect(() => {
    alive.current = true;
    let disposed = false;
    let retry: ReturnType<typeof setTimeout> | undefined;
    const start = async () => {
      try {
        apply(await invoke<UpdaterSnapshot>('updater_snapshot'));
        if (!disposed) await action({ kind: 'configure', automatic });
      } catch { if (!disposed) retry = setTimeout(start, 250); }
    };
    void start();
    const subscription = listen<UpdaterSnapshot>('updater-state-changed', event => apply(event.payload));
    return () => { disposed = true; alive.current = false; clearTimeout(retry); void subscription.then(unlisten => unlisten()); };
  }, [action, apply, automatic]);
  useEffect(() => {
    if (!automatic) return;
    const due = () => { void action({ kind: 'check', background: true }); };
    const visible = () => { if (!document.hidden) due(); };
    window.addEventListener('focus', due);
    document.addEventListener('visibilitychange', visible);
    const subscription = listen('updater-background-check-requested', due);
    return () => {
      window.removeEventListener('focus', due);
      document.removeEventListener('visibilitychange', visible);
      void subscription.then(unlisten => unlisten());
    };
  }, [action, automatic]);
  return {
    updateStatus: snapshot.updateStatus,
    completedUpdate: snapshot.completedUpdate,
    isUpdateDialogOpen: snapshot.isUpdateDialogOpen,
    checkForUpdate: useCallback(() => action({ kind: 'check', background: false }), [action]),
    showAvailableUpdate: useCallback(() => { void action({ kind: 'show' }); }, [action]),
    startDownload: useCallback(() => action({ kind: 'download' }), [action]),
    restartUpdate: useCallback(() => action({ kind: 'install' }), [action]),
    dismissUpdate: useCallback(() => { void action({ kind: 'dismiss' }); }, [action]),
    dismissCompletedUpdate: useCallback(() => { void action({ kind: 'dismissCompleted' }); }, [action]),
  };
}
