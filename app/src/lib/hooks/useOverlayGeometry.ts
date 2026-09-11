import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { flog } from '../log';
import { isOverlayGeometry } from '../overlayGeometry';
import type { OverlayContent, OverlayGeometry } from '../overlayGeometry';

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
export function useOverlayGeometry(content: OverlayContent = 'controls'): OverlayGeometry | null {
  const [geometry, setGeometry] = useState<OverlayGeometry | null>(null);

  useEffect(() => {
    setGeometry(null);
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    let retryTimer: ReturnType<typeof setTimeout> | null = null;
    let eventGeneration = 0;

    const scheduleRetry = (attempt: number, fetchGeneration: number) => {
      if (cancelled || eventGeneration !== fetchGeneration) return;
      const delay = FETCH_RETRY_DELAYS_MS[attempt];
      if (delay === undefined) {
        flog.warn('overlay', 'get_overlay_geometry exhausted retries', { attempts: attempt + 1 });
        return;
      }
      retryTimer = setTimeout(() => {
        retryTimer = null;
        if (!cancelled && eventGeneration === fetchGeneration) {
          void attemptFetch(attempt + 1);
        }
      }, delay);
    };

    // Every fetch is bound to the latest display event. Suggestion mode cannot
    // apply the normal geometry carried by that event, so it fetches its current
    // content profile again instead.
    async function attemptFetch(attempt: number): Promise<void> {
      const fetchGeneration = eventGeneration;
      try {
        const value = await invoke<unknown>('get_overlay_geometry', { content });
        if (cancelled || eventGeneration !== fetchGeneration) return;
        if (isOverlayGeometry(value)) {
          flog.info('overlay', 'geometry loaded', { windowW: value.windowW, collapsedH: value.collapsedH });
          setGeometry(value);
        } else {
          flog.warn('overlay', 'get_overlay_geometry returned invalid payload', { attempt });
          scheduleRetry(attempt, fetchGeneration);
        }
      } catch (e) {
        if (cancelled || eventGeneration !== fetchGeneration) return;
        flog.warn('overlay', 'get_overlay_geometry failed', { attempt, error: String(e) });
        scheduleRetry(attempt, fetchGeneration);
      }
    }

    const initialize = async () => {
      const stopListening = await listen<unknown>('overlay-geometry-changed', (event) => {
        if (cancelled) return;
        if (isOverlayGeometry(event.payload)) {
          eventGeneration += 1;
          if (retryTimer) {
            clearTimeout(retryTimer);
            retryTimer = null;
          }
          if (content === 'controls') {
            setGeometry(event.payload);
          } else {
            setGeometry(null);
            void attemptFetch(0);
          }
        } else {
          flog.warn('overlay', 'overlay-geometry-changed had invalid payload');
        }
      });

      if (cancelled) {
        stopListening();
        return;
      }
      unlisten = stopListening;

      await attemptFetch(0);
    };

    initialize().catch((e) => {
      flog.warn('overlay', 'overlay geometry listener setup failed', { error: String(e) });
    });

    return () => {
      cancelled = true;
      if (retryTimer) clearTimeout(retryTimer);
      unlisten?.();
    };
  }, [content]);

  return geometry;
}
