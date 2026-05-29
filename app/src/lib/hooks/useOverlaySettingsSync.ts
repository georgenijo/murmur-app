import { useEffect } from 'react';
import { listen } from '@tauri-apps/api/event';
import { Settings, loadSettings } from '../settings';

/**
 * Listens for the `settings-changed` event emitted by the overlay window's
 * quick-settings controls (auto-paste / global disable) and applies the latest
 * persisted settings via `apply`. The overlay runs in a separate Tauri window
 * with no shared React context, so this event is the bridge back to the main
 * window's settings state and backend configuration.
 */
export function useOverlaySettingsSync(apply: (fresh: Settings) => void) {
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen('settings-changed', () => {
      if (!cancelled) apply(loadSettings());
    }).then((fn) => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    });
    return () => { cancelled = true; unlisten?.(); };
  }, [apply]);
}
