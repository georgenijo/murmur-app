import { useCallback, useEffect, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';

/** How the current recording was started. */
export type RecordingOrigin = 'toggle' | 'hold';

/**
 * Track whether the in-flight recording was started by physically holding the
 * trigger key. `'hold'` spans exactly `hold-down-start` → `hold-down-stop`;
 * everything else — double-tap, the main-window button, the overlay click,
 * locked mode — never emits a hold event, so the `'toggle'` default covers it.
 *
 * `hold-down-cancel` and `double-tap-toggle` also reset to `'toggle'`: the
 * backend's deferred-hold design means a short tap in Both mode currently
 * emits neither, but a cancelled speculative hold must never leave the origin
 * stuck at `'hold'` for the toggle-started recording that follows it.
 */
export function useRecordingOrigin(): () => RecordingOrigin {
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

  return useCallback(() => originRef.current, []);
}
