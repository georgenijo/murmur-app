import { useEffect, useRef, useState } from 'react';
import { registerHotkey, unregisterHotkey } from '../hotkey';

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

  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setError(null);
    if (!enabled || !initialized || !hotkey) return;

    let cleanedUp = false;

    registerHotkey(hotkey, () => {
      if (!initializedRef.current) return;
      onToggleRef.current();
    }).then(() => {
      if (cleanedUp) {
        unregisterHotkey().catch(() => {});
      }
    }).catch((err) => {
      console.error('Failed to register hotkey:', err);
      setError(`Could not register hotkey "${hotkey}". Control shortcuts may conflict with macOS system shortcuts — try a Shift or Option combo instead.`);
    });

    return () => {
      cleanedUp = true;
      unregisterHotkey().catch((err) => {
        console.warn('Failed to unregister hotkey on cleanup:', err);
      });
    };
  }, [enabled, initialized, hotkey]);

  return { error };
}
