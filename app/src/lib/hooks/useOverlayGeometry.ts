import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { flog } from '../log';
import { isOverlayGeometry } from '../overlayGeometry';
import type { OverlayGeometry } from '../overlayGeometry';

/**
 * Owns overlay geometry sourced from Rust. Rust is the single source of truth
 * for every overlay dimension; the frontend only reads it. On mount we fetch
 * the current geometry, and we subscribe to `overlay-geometry-changed` so the
 * island resizes when the display configuration changes.
 */
export function useOverlayGeometry(): OverlayGeometry | null {
  const [geometry, setGeometry] = useState<OverlayGeometry | null>(null);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    let eventGeneration = 0;

    const initialize = async () => {
      const stopListening = await listen<unknown>('overlay-geometry-changed', (event) => {
        if (cancelled) return;
        if (isOverlayGeometry(event.payload)) {
          eventGeneration += 1;
          setGeometry(event.payload);
        } else {
          flog.warn('overlay', 'overlay-geometry-changed had invalid payload');
        }
      });

      if (cancelled) {
        stopListening();
        return;
      }
      unlisten = stopListening;

      const fetchGeneration = eventGeneration;
      try {
        const value = await invoke<unknown>('get_overlay_geometry');
        if (cancelled || eventGeneration !== fetchGeneration) return;
        if (isOverlayGeometry(value)) {
          flog.info('overlay', 'geometry loaded', { windowW: value.windowW, collapsedH: value.collapsedH });
          setGeometry(value);
        } else {
          flog.warn('overlay', 'get_overlay_geometry returned invalid payload');
        }
      } catch (e) {
        flog.warn('overlay', 'get_overlay_geometry failed', { error: String(e) });
      }
    };

    initialize().catch((e) => {
      flog.warn('overlay', 'overlay geometry listener setup failed', { error: String(e) });
    });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  return geometry;
}
