import { useCallback, useEffect, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';

/** How the current recording was started. */
export type RecordingOrigin = 'toggle' | 'hold';

export interface RecordingOriginTracker {
  getOrigin: () => RecordingOrigin;
  /** Reset to `'toggle'` — called when the recording the origin described has
   *  ended, so a `'hold'` mark can never outlive its recording. */
  resetOrigin: () => void;
}

/**
 * Track whether the in-flight recording was started by physically holding the
 * trigger key. `'hold'` spans exactly `hold-down-start` → `hold-down-stop`;
 * everything else — double-tap, the main-window button, the overlay click,
 * locked mode — never emits a hold event, so the `'toggle'` default covers it.
 *
 * `double-tap-toggle` also resets to `'toggle'`, and `hold-down-cancel` is
 * handled defensively even though the backend's deferred-hold design currently
 * never emits it (a short tap in Both mode emits nothing at all).
 *
 * A missed `hold-down-stop` must not strand the origin at `'hold'` — Escape
 * cancels a hold recording while suppressing the release's stop event, and a
 * dead rdev thread mid-hold loses the release entirely. The consumer therefore
 * calls `resetOrigin` whenever recording status leaves `'recording'`: the
 * origin describes one recording and is cleared when that recording ends.
 */
export function useRecordingOrigin(): RecordingOriginTracker {
  const originRef = useRef<RecordingOrigin>('toggle');

  useEffect(() => {
    let cancelled = false;
    const unlistens: (() => void)[] = [];
    const subscribe = (event: string, origin: RecordingOrigin) => {
      listen(event, () => { originRef.current = origin; }).then((fn) => {
        if (cancelled) { fn(); } else { unlistens.push(fn); }
      }).catch(() => {});
    };
    subscribe('hold-down-start', 'hold');
    subscribe('hold-down-stop', 'toggle');
    subscribe('hold-down-cancel', 'toggle');
    subscribe('double-tap-toggle', 'toggle');
    return () => {
      cancelled = true;
      unlistens.forEach((fn) => fn());
      originRef.current = 'toggle';
    };
  }, []);

  return {
    getOrigin: useCallback(() => originRef.current, []),
    resetOrigin: useCallback(() => { originRef.current = 'toggle'; }, []),
  };
}
