import { useEffect } from 'react';
import { listen } from '@tauri-apps/api/event';

/**
 * Listens for the `open-settings` event emitted by the overlay's gear button and
 * invokes `onOpen` to reveal the Settings panel. Showing/focusing the main window
 * alone is not enough — the panel's open state is local React state in App.tsx.
 */
export function useOpenSettingsListener(onOpen: () => void) {
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen('open-settings', () => {
      if (!cancelled) onOpen();
    }).then((fn) => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    });
    return () => { cancelled = true; unlisten?.(); };
  }, [onOpen]);
}
