import { useEffect, useRef } from 'react';
import { registerHotkey, unregisterHotkey, hotkeyToShortcut } from '../hotkey';

interface UseHotkeyToggleProps {
  initialized: boolean;
  hotkey: string;
  onToggle: () => void;
}

export function useHotkeyToggle({ initialized, hotkey, onToggle }: UseHotkeyToggleProps) {
  const initializedRef = useRef(initialized);
  useEffect(() => { initializedRef.current = initialized; }, [initialized]);

  useEffect(() => {
    if (!initialized) return;

    const shortcut = hotkeyToShortcut(hotkey);

    registerHotkey(shortcut, () => {
      if (!initializedRef.current) return;
      onToggle();
    }).catch((err) => {
      console.error('Failed to register hotkey:', err);
    });

    return () => {
      unregisterHotkey().catch((err) => {
        console.warn('Failed to unregister hotkey on cleanup:', err);
      });
    };
  }, [initialized, hotkey, onToggle]);
}
