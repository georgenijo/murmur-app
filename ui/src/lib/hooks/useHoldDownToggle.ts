import { useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

interface UseHoldDownToggleProps {
  enabled: boolean;
  initialized: boolean;
  accessibilityGranted: boolean | null;
  holdDownKey: string;
  onStart: () => void;
  onStop: () => void;
}

export function useHoldDownToggle({ enabled, initialized, accessibilityGranted, holdDownKey, onStart, onStop }: UseHoldDownToggleProps) {
  const onStartRef = useRef(onStart);
  const onStopRef = useRef(onStop);
  useEffect(() => { onStartRef.current = onStart; }, [onStart]);
  useEffect(() => { onStopRef.current = onStop; }, [onStop]);

  useEffect(() => {
    if (!enabled || !initialized || !accessibilityGranted) return;

    let unlistenStart: (() => void) | null = null;
    let unlistenStop: (() => void) | null = null;
    let unlistenError: (() => void) | null = null;
    let cancelled = false;

    const setup = async () => {
      unlistenStart = await listen('hold-down-start', () => {
        onStartRef.current();
      });
      if (cancelled) { unlistenStart(); return; }

      unlistenStop = await listen('hold-down-stop', () => {
        onStopRef.current();
      });
      if (cancelled) { unlistenStart(); unlistenStop(); return; }

      // If the rdev thread dies, wait briefly then attempt to restart it.
      unlistenError = await listen<string>('keyboard-listener-error', async () => {
        if (cancelled) return;
        await new Promise<void>((r) => setTimeout(r, 2000));
        if (!cancelled) {
          try {
            await invoke('start_keyboard_listener', { hotkey: holdDownKey, mode: 'hold_down' });
          } catch (err) {
            console.error('Failed to restart hold-down listener after error:', err);
          }
        }
      });
      if (cancelled) { unlistenStart(); unlistenStop(); unlistenError(); return; }

      // Start the rdev listener
      try {
        await invoke('start_keyboard_listener', { hotkey: holdDownKey, mode: 'hold_down' });
        if (cancelled) {
          invoke('stop_keyboard_listener').catch(() => {});
        }
      } catch (err) {
        console.error('Failed to start hold-down listener:', err);
      }
    };

    setup();

    return () => {
      cancelled = true;
      unlistenStart?.();
      unlistenStop?.();
      unlistenError?.();
      invoke('stop_keyboard_listener').catch(() => {});
    };
  }, [enabled, initialized, accessibilityGranted, holdDownKey]);
}
