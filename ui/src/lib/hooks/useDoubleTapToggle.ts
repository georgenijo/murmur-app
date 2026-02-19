import { useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { DictationStatus } from '../types';

interface UseDoubleTapToggleProps {
  enabled: boolean;
  initialized: boolean;
  doubleTapKey: string;
  status: DictationStatus;
  onToggle: () => void;
}

export function useDoubleTapToggle({ enabled, initialized, doubleTapKey, status, onToggle }: UseDoubleTapToggleProps) {
  const onToggleRef = useRef(onToggle);
  useEffect(() => { onToggleRef.current = onToggle; }, [onToggle]);

  // Keep the backend in sync with recording state
  useEffect(() => {
    if (!enabled) return;
    invoke('set_double_tap_recording', { recording: status === 'recording' }).catch(() => {});
  }, [enabled, status]);

  useEffect(() => {
    if (!enabled || !initialized) return;

    let unlisten: (() => void) | null = null;
    let cancelled = false;

    const setup = async () => {
      // Listen for double-tap events from the Rust backend
      unlisten = await listen('double-tap-toggle', () => {
        onToggleRef.current();
      });

      if (cancelled) {
        unlisten();
        return;
      }

      // Start the rdev listener
      try {
        await invoke('start_double_tap_listener', { hotkey: doubleTapKey });
        if (cancelled) {
          invoke('stop_double_tap_listener').catch(() => {});
        }
      } catch (err) {
        console.error('Failed to start double-tap listener:', err);
      }
    };

    setup();

    return () => {
      cancelled = true;
      unlisten?.();
      invoke('stop_double_tap_listener').catch((err) => {
        console.warn('Failed to stop double-tap listener on cleanup:', err);
      });
    };
  }, [enabled, initialized, doubleTapKey]);
}
