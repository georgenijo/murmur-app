import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { flog } from '../log';
import { DEFAULT_SETTINGS, loadSettings } from '../settings';
import type { DictationStatus } from '../types';
import type { DictationResponse } from '../dictation';

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

  // When status returns to idle, locked mode is automatically reset — mirrors
  // the idle branch of the original combined "status changed" effect.
  useEffect(() => {
    if (status === 'idle') setLockedMode(false);
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
    if (currentStatus === 'processing') return;
    if (disabledRef.current && currentStatus === 'idle') return;
    if (clickTimerRef.current) {
      clearTimeout(clickTimerRef.current);
      clickTimerRef.current = null;
    }
    if (expandedRef.current) {
      flog.info('overlay', 'double-click ignored while expanded', { status: currentStatus });
      return;
    }
    const currentLocked = lockedRef.current;
    if (!currentLocked) {
      // Enter locked mode — start recording
      setLockedMode(true);
      if (currentStatus !== 'recording') {
        try {
          // Read the microphone setting via the validated settings API (the
          // overlay has no React settings context, so it loads localStorage
          // directly, but through loadSettings() rather than a raw parse).
          let deviceName: string | null = null;
          try {
            const settings = loadSettings();
            if (settings.microphone && settings.microphone !== DEFAULT_SETTINGS.microphone) {
              deviceName = settings.microphone;
            }
          } catch { /* ignore parse errors */ }
          flog.info('overlay', 'invoking start_native_recording', { deviceName });
          const res = await invoke<DictationResponse>('start_native_recording', { deviceName });
          flog.info('overlay', 'start_native_recording result', { type: res.type, state: res.state });
          if (res.type !== 'recording_started') {
            flog.warn('overlay', 'recording start declined', { type: res.type });
            setLockedMode(false);
          }
        } catch (err) {
          flog.error('overlay', 'start_native_recording error', { error: String(err) });
          setLockedMode(false);
        }
      }
    } else {
      // Exit locked mode — stop recording
      setLockedMode(false);
      if (currentStatus === 'recording') {
        try {
          flog.info('overlay', 'invoking stop_native_recording');
          const res = await invoke('stop_native_recording');
          flog.info('overlay', 'stop_native_recording result', { res: res as Record<string, unknown> });
        } catch (err) {
          flog.error('overlay', 'stop_native_recording error', { error: String(err) });
        }
      }
    }
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
      if (statusRef.current === 'recording') {
        setLockedMode(false);
        try {
          await invoke('stop_native_recording');
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
