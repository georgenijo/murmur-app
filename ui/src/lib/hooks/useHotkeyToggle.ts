import { useEffect, useRef } from 'react';
import { registerHotkey, unregisterHotkey, hotkeyToShortcut } from '../hotkey';

interface UseHotkeyToggleProps {
  enabled: boolean;
  initialized: boolean;
  hotkey: string;
  onToggle: () => void;
}

export function useHotkeyToggle({ enabled, initialized, hotkey, onToggle }: UseHotkeyToggleProps) {
  const initializedRef = useRef(initialized);
  const onToggleRef = useRef(onToggle);
  useEffect(() => { initializedRef.current = initialized; }, [initialized]);
  useEffect(() => { onToggleRef.current = onToggle; }, [onToggle]);

  useEffect(() => {
    if (!enabled || !initialized) return;

    const shortcut = hotkeyToShortcut(hotkey);
    let cleanedUp = false;

    registerHotkey(shortcut, () => {
      if (!initializedRef.current) return;
      onToggleRef.current();
    }).then(() => {
      if (cleanedUp) {
        unregisterHotkey().catch(() => {});
      }
    }).catch((err) => {
      console.error('Failed to register hotkey:', err);
    });

    return () => {
      cleanedUp = true;
      unregisterHotkey().catch((err) => {
        console.warn('Failed to unregister hotkey on cleanup:', err);
      });
    };
  }, [enabled, initialized, hotkey]);
}
