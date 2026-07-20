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

    invoke<unknown>('get_overlay_geometry')
      .then((value) => {
        if (cancelled) return;
        if (isOverlayGeometry(value)) {
          flog.info('overlay', 'geometry loaded', { windowW: value.windowW, collapsedH: value.collapsedH });
          setGeometry(value);
        } else {
          flog.warn('overlay', 'get_overlay_geometry returned invalid payload');
        }
      })
      .catch((e) => flog.warn('overlay', 'get_overlay_geometry failed', { error: String(e) }));

    listen<unknown>('overlay-geometry-changed', (event) => {
      if (isOverlayGeometry(event.payload)) {
        setGeometry(event.payload);
      } else {
        flog.warn('overlay', 'overlay-geometry-changed had invalid payload');
      }
    }).then((fn) => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  return geometry;
}
