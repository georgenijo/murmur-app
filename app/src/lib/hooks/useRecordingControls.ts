import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { flog } from '../log';
import type { DictationStatus } from '../types';
import { listen } from '@tauri-apps/api/event';

export interface UseRecordingControlsArgs {
  /** Current dictation status (reactive value, drives the locked-mode reset). */
  status: DictationStatus;
  statusRef: React.MutableRefObject<DictationStatus>;
  disabledRef: React.MutableRefObject<boolean>;
  /** From useOverlayExpansion — a double-click while the card is up is ignored. */
  expandedRef: React.MutableRefObject<boolean>;
}

export interface RecordingControls {
  lockedMode: boolean;
  handleMouseDown: (e: React.MouseEvent) => void;
  handleClick: (e: React.MouseEvent) => void;
  handleDoubleClick: (e: React.MouseEvent) => void;
}

/**
 * Owns click/double-click/mousedown disambiguation and "locked mode" — whether
 * recording was started from the overlay itself (vs. the keyboard hotkey).
 */
export function useRecordingControls({
  status,
  statusRef,
  disabledRef,
  expandedRef,
}: UseRecordingControlsArgs): RecordingControls {
  const [lockedMode, setLockedMode] = useState(false);
  const lockedRef = useRef(lockedMode);
  const clickTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => { lockedRef.current = lockedMode; }, [lockedMode]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<boolean>('dictation-lock-changed', ({ payload }) => setLockedMode(payload))
      .then((stop) => { if (disposed) stop(); else unlisten = stop; });
    return () => { disposed = true; unlisten?.(); };
  }, []);
  useEffect(() => {
    if (status === 'idle' || status === 'recovering') setLockedMode(false);
  }, [status]);

  // Clear the pending single-click debounce timer on unmount.
  useEffect(() => () => {
    if (clickTimerRef.current) clearTimeout(clickTimerRef.current);
  }, []);

  // Double-click: toggle locked mode. Cancel any pending single-click first.
  const handleDoubleClick = useCallback(async (e: React.MouseEvent) => {
    flog.info('overlay', 'DOUBLE-CLICK', {
      locked: lockedRef.current, status: statusRef.current,
      x: Math.round(e.clientX), y: Math.round(e.clientY),
      target: (e.target as HTMLElement).tagName,
    });
    const currentStatus = statusRef.current;
    if (currentStatus === 'processing' || currentStatus === 'recovering') return;
    if (disabledRef.current && currentStatus === 'idle') return;
    if (clickTimerRef.current) {
      clearTimeout(clickTimerRef.current);
      clickTimerRef.current = null;
    }
    if (expandedRef.current) {
      flog.info('overlay', 'double-click ignored while expanded', { status: currentStatus });
      return;
    }
    try {
      await invoke('toggle_dictation_lock');
    } catch { /* authoritative state arrives through native events */ }

  }, [statusRef, disabledRef, expandedRef]);

  // Single-click: debounced so it doesn't fire when the user double-clicks.
  const handleClick = useCallback((e: React.MouseEvent) => {
    if (clickTimerRef.current) {
      clearTimeout(clickTimerRef.current);
    }
    flog.info('overlay', 'CLICK (pending 250ms debounce)', {
      locked: lockedRef.current, status: statusRef.current,
      x: Math.round(e.clientX), y: Math.round(e.clientY),
      target: (e.target as HTMLElement).tagName,
    });
    clickTimerRef.current = setTimeout(async () => {
      clickTimerRef.current = null;
      flog.info('overlay', 'click fired', { locked: lockedRef.current, status: statusRef.current });
      // Single click stops recording (regardless of locked mode)
      if (statusRef.current === 'starting') {
        setLockedMode(false);
        try {
          await invoke('cancel_native_recording');
        } catch {
          // status will sync via event
        }
      } else if (statusRef.current === 'recording') {
        setLockedMode(false);
        try {
          await invoke('stop_native_recording', { stopTrigger: 'overlay' });
        } catch {
          // status will sync via event
        }
      }
    }, 250);
  }, [statusRef]);

  // Raw mousedown — fires before click/double-click debouncing
  const handleMouseDown = useCallback((e: React.MouseEvent) => {
    flog.info('overlay', 'MOUSEDOWN', {
      button: e.button, x: Math.round(e.clientX), y: Math.round(e.clientY),
      target: (e.target as HTMLElement).tagName,
      className: (e.target as HTMLElement).className?.slice(0, 50),
      locked: lockedRef.current, status: statusRef.current,
    });
  }, [statusRef]);

  return { lockedMode, handleMouseDown, handleClick, handleDoubleClick };
}
