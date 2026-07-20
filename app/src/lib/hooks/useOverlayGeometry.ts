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
    let eventGeneration = 0;

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

    // Attempt the initial fetch, retrying with backoff so a transient failure
    // (or a not-yet-ready backend) does not leave the overlay blank forever. A
    // display-change event arriving mid-fetch bumps eventGeneration and voids the
    // stale fetch result so it cannot clobber the fresher event payload — and,
    // since geometry already arrived, no retry is scheduled either.
    const attemptFetch = (attempt: number) => {
      const fetchGeneration = eventGeneration;
      invoke<unknown>('get_overlay_geometry')
        .then((value) => {
          if (cancelled || eventGeneration !== fetchGeneration) return;
          if (isOverlayGeometry(value)) {
            flog.info('overlay', 'geometry loaded', { windowW: value.windowW, collapsedH: value.collapsedH });
            setGeometry(value);
          } else {
            flog.warn('overlay', 'get_overlay_geometry returned invalid payload', { attempt });
            scheduleRetry(attempt);
          }
        })
        .catch((e) => {
          if (cancelled || eventGeneration !== fetchGeneration) return;
          flog.warn('overlay', 'get_overlay_geometry failed', { attempt, error: String(e) });
          scheduleRetry(attempt);
        });
    };

    const initialize = async () => {
      // Register the display-change listener before fetching so an event that
      // fires during the fetch is never missed.
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

      attemptFetch(0);
    };

    initialize().catch((e) => {
      flog.warn('overlay', 'overlay geometry listener setup failed', { error: String(e) });
    });

    return () => {
      cancelled = true;
      if (retryTimer) clearTimeout(retryTimer);
      unlisten?.();
    };
  }, []);

  return geometry;
}
