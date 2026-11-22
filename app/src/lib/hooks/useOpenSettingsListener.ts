import { useEffect } from 'react';
import { listen } from '@tauri-apps/api/event';

/**
 * Listens for the `open-settings` event emitted by the overlay's gear button (no payload: General) or a model cue (`{ page, target }`) and
 * invokes `onOpen` to reveal the Settings panel. Showing/focusing the main window
 * alone is not enough — the panel's open state is local React state in App.tsx.
 */
export interface OpenSettingsDestination { page: string; target?: string }

function destinationFromPayload(payload: unknown): OpenSettingsDestination | undefined {
  if (!payload || typeof payload !== 'object') return undefined;
  const { page, target } = payload as Record<string, unknown>;
  if (typeof page !== 'string' || page.length === 0) return undefined;
  return typeof target === 'string' ? { page, target } : { page };
}

export function useOpenSettingsListener(onOpen: (destination?: OpenSettingsDestination) => void) {
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen<unknown>('open-settings', (event) => {
      if (!cancelled) onOpen(destinationFromPayload(event.payload));
    }).then((fn) => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    });
    return () => { cancelled = true; unlisten?.(); };
  }, [onOpen]);
}
