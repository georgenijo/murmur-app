import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { flog } from '../log';
import { isOverlayGeometry } from '../overlayGeometry';
import type { OverlayGeometry } from '../overlayGeometry';

// Backoff schedule (ms) between initial fetch attempts. A single failed fetch
// used to leave the transparent overlay blank until the next display change, so
// retry a few times before giving up.
const FETCH_RETRY_DELAYS_MS = [250, 1000];

/**
 * Owns overlay geometry sourced from Rust. Rust is the single source of truth
 * for every overlay dimension; the frontend only reads it. On mount we fetch
 * the current geometry (retrying on failure), and we subscribe to
 * `overlay-geometry-changed` so the island resizes when the display
 * configuration changes.
 */
export function useOverlayGeometry(): OverlayGeometry | null {
  const [geometry, setGeometry] = useState<OverlayGeometry | null>(null);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    let retryTimer: ReturnType<typeof setTimeout> | null = null;

    // Attempt the initial fetch, retrying with backoff so a transient failure
    // (or a not-yet-ready backend) does not leave the overlay blank forever.
    const attemptFetch = (attempt: number) => {
      invoke<unknown>('get_overlay_geometry')
        .then((value) => {
          if (cancelled) return;
          if (isOverlayGeometry(value)) {
            flog.info('overlay', 'geometry loaded', { windowW: value.windowW, collapsedH: value.collapsedH });
            setGeometry(value);
          } else {
            flog.warn('overlay', 'get_overlay_geometry returned invalid payload', { attempt });
            scheduleRetry(attempt);
          }
        })
        .catch((e) => {
          if (cancelled) return;
          flog.warn('overlay', 'get_overlay_geometry failed', { attempt, error: String(e) });
          scheduleRetry(attempt);
        });
    };

    const scheduleRetry = (attempt: number) => {
      const delay = FETCH_RETRY_DELAYS_MS[attempt];
      if (delay === undefined) {
        flog.warn('overlay', 'get_overlay_geometry exhausted retries', { attempts: attempt + 1 });
        return;
      }
      retryTimer = setTimeout(() => {
        retryTimer = null;
        if (!cancelled) attemptFetch(attempt + 1);
      }, delay);
    };

    attemptFetch(0);

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
      if (retryTimer) clearTimeout(retryTimer);
      unlisten?.();
    };
  }, []);

  return geometry;
}
