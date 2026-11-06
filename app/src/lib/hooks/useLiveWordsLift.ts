import { useEffect, useRef } from 'react';
import { emitTo } from '@tauri-apps/api/event';
import { listen } from '../targetedEvents';
import type { OverlayGeometry } from '../overlayGeometry';
import { LIVE_WORDS_LIFT_EVENT, LIVE_WORDS_LIFT_REQUEST_EVENT } from '../overlayMotion';

/** Tell the Live Words window how far the hover dropdown moves the island's
 * edge, as the reveal or collapse starts. A Live Words window that loads late
 * asks for the current value, so it never waits for the next hover change. */
export function liveWordsLift(geometry: OverlayGeometry | null, expanded: boolean): number {
  return geometry && expanded ? geometry.dropdownH : 0;
}

export function useLiveWordsLift(geometry: OverlayGeometry | null, expanded: boolean) {
  const lift = liveWordsLift(geometry, expanded);
  const current = useRef(lift);
  current.current = lift;
  useEffect(() => {
    emitTo('dictation-preview', LIVE_WORDS_LIFT_EVENT, lift).catch(() => {});
  }, [lift]);
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    // Replies carry the latest value, so they can only ever catch the preview up.
    void listen(LIVE_WORDS_LIFT_REQUEST_EVENT, () => {
      if (!disposed) emitTo('dictation-preview', LIVE_WORDS_LIFT_EVENT, current.current).catch(() => {});
    }).then(fn => { if (disposed) fn(); else unlisten = fn; }).catch(() => {});
    return () => { disposed = true; unlisten?.(); };
  }, []);
}
