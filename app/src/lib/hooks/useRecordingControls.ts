import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { DEFAULT_SETTINGS, loadSettings, smartAutoMicrophoneRequest } from '../settings';
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
  handleClick: () => void;
  handleDoubleClick: () => void;
}

/**
 * Owns click/double-click disambiguation and "locked mode" — whether
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

  // Cancellation briefly reports Recovering before immediately returning to
  // Idle, so release locked mode at the first cancellation boundary.
  useEffect(() => {
    if (status === 'idle' || status === 'recovering') setLockedMode(false);
  }, [status]);

  // Clear the pending single-click debounce timer on unmount.
  useEffect(() => () => {
    if (clickTimerRef.current) clearTimeout(clickTimerRef.current);
  }, []);

  // Double-click: toggle locked mode. Cancel any pending single-click first.
  const handleDoubleClick = useCallback(async () => {
    const currentStatus = statusRef.current;
    if (currentStatus === 'processing' || currentStatus === 'recovering') return;
    if (disabledRef.current && currentStatus === 'idle') return;
    if (clickTimerRef.current) {
      clearTimeout(clickTimerRef.current);
      clickTimerRef.current = null;
    }
    if (expandedRef.current) {
      return;
    }
    const currentLocked = lockedRef.current;
    if (currentStatus === 'starting') {
      setLockedMode(false);
      try {
        await invoke('cancel_native_recording');
      } catch {
        // status will sync via event
      }
      return;
    }
    if (!currentLocked) {
      // Enter locked mode — start recording
      setLockedMode(true);
      if (currentStatus !== 'recording') {
        try {
          // Read the microphone setting via the validated settings API (the
          // overlay has no React settings context, so it loads localStorage
          // directly, but through loadSettings() rather than a raw parse).
          let deviceName: string | null = null;
          let smartAuto = null;
          try {
            const settings = loadSettings();
            if (settings.microphone && settings.microphone !== DEFAULT_SETTINGS.microphone) {
              deviceName = settings.microphone;
            }
            smartAuto = smartAutoMicrophoneRequest(settings);
            if (smartAuto) deviceName = null;
          } catch { /* ignore parse errors */ }
          const res = await invoke<DictationResponse>('start_native_recording', {
            deviceName,
            smartAuto,
            origin: 'toggle',
          });
          if (res.type !== 'recording_starting') {
            setLockedMode(false);
          }
        } catch {
          setLockedMode(false);
        }
      }
    } else {
      // Exit locked mode — stop recording
      setLockedMode(false);
      if (currentStatus === 'recording') {
        try {
          await invoke('stop_native_recording', { stopTrigger: 'overlay' });
        } catch {
          // status will sync via event
        }
      }
    }
  }, [statusRef, disabledRef, expandedRef]);

  // Single-click: debounced so it doesn't fire when the user double-clicks.
  const handleClick = useCallback(() => {
    if (clickTimerRef.current) {
      clearTimeout(clickTimerRef.current);
    }
    clickTimerRef.current = setTimeout(async () => {
      clickTimerRef.current = null;
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

  return { lockedMode, handleClick, handleDoubleClick };
}
