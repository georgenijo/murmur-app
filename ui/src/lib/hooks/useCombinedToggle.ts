import { useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { DictationStatus } from '../types';

interface UseCombinedToggleProps {
  enabled: boolean;
  initialized: boolean;
  accessibilityGranted: boolean | null;
  triggerKey: string;
  status: DictationStatus;
  onStart: () => void;
  onStop: () => void;
  onToggle: () => void;
}

export function useCombinedToggle({ enabled, initialized, accessibilityGranted, triggerKey, status, onStart, onStop, onToggle }: UseCombinedToggleProps) {
  const onStartRef = useRef(onStart);
  const onStopRef = useRef(onStop);
  const onToggleRef = useRef(onToggle);
  useEffect(() => { onStartRef.current = onStart; }, [onStart]);
  useEffect(() => { onStopRef.current = onStop; }, [onStop]);
  useEffect(() => { onToggleRef.current = onToggle; }, [onToggle]);

  // Track whether a hold-down press is currently active. When true, we skip
  // syncing recording state to the backend double-tap detector — otherwise
  // the eager hold-start would set dtap.recording=true, causing the first
  // release of a double-tap to fire as "single tap to stop" instead of
  // advancing to WaitingSecondDown.
  const holdActiveRef = useRef(false);

  // Keep the backend double-tap detector in sync with recording state,
  // but ONLY for double-tap-initiated recordings (not eager hold presses).
  useEffect(() => {
    if (!enabled) return;
    if (holdActiveRef.current) return;
    invoke('set_keyboard_recording', { recording: status === 'recording' }).catch(() => {});
  }, [enabled, status]);

  useEffect(() => {
    if (!enabled || !initialized || !accessibilityGranted) return;

    let unlistenStart: (() => void) | null = null;
    let unlistenStop: (() => void) | null = null;
    let unlistenCancel: (() => void) | null = null;
    let unlistenToggle: (() => void) | null = null;
    let unlistenError: (() => void) | null = null;
    let cancelled = false;

    const setup = async () => {
      unlistenStart = await listen('hold-down-start', () => {
        holdActiveRef.current = true;
        onStartRef.current();
      });
      if (cancelled) { unlistenStart(); return; }

      unlistenStop = await listen('hold-down-stop', () => {
        holdActiveRef.current = false;
        onStopRef.current();
      });
      if (cancelled) { unlistenStart(); unlistenStop(); return; }

      // Cancel: discard speculative recording from a short tap (no transcription)
      unlistenCancel = await listen('hold-down-cancel', () => {
        holdActiveRef.current = false;
        invoke('cancel_native_recording').catch(() => {});
      });
      if (cancelled) { unlistenStart(); unlistenStop(); unlistenCancel(); return; }

      unlistenToggle = await listen('double-tap-toggle', () => {
        holdActiveRef.current = false;
        onToggleRef.current();
      });
      if (cancelled) { unlistenStart(); unlistenStop(); unlistenCancel(); unlistenToggle(); return; }

      unlistenError = await listen<string>('keyboard-listener-error', async (event) => {
        console.error('Keyboard listener error:', event.payload);
        if (cancelled) return;
        await new Promise<void>((r) => setTimeout(r, 2000));
        if (!cancelled) {
          try {
            await invoke('start_keyboard_listener', { hotkey: triggerKey, mode: 'both' });
          } catch (err) {
            console.error('Failed to restart combined listener after error:', err);
          }
        }
      });
      if (cancelled) { unlistenStart(); unlistenStop(); unlistenCancel(); unlistenToggle(); unlistenError(); return; }

      try {
        await invoke('start_keyboard_listener', { hotkey: triggerKey, mode: 'both' });
        if (cancelled) {
          invoke('stop_keyboard_listener').catch(() => {});
        }
      } catch (err) {
        console.error('Failed to start combined listener:', err);
      }
    };

    setup();

    return () => {
      cancelled = true;
      holdActiveRef.current = false;
      unlistenStart?.();
      unlistenStop?.();
      unlistenCancel?.();
      unlistenToggle?.();
      unlistenError?.();
      invoke('stop_keyboard_listener').catch(() => {});
    };
  }, [enabled, initialized, accessibilityGranted, triggerKey]);
}
