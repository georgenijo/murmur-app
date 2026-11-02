import { useEffect } from 'react';
import { emitTo } from '@tauri-apps/api/event';
import type { OverlayGeometry } from '../overlayGeometry';
import { LIVE_WORDS_LIFT_EVENT } from '../overlayMotion';

/** Tell the Live Words window how far the hover dropdown moves the island's
 * edge, as the reveal or collapse starts. The preview webview stays mounted
 * while hidden, so it always holds the current value. */
export function liveWordsLift(geometry: OverlayGeometry | null, expanded: boolean): number {
  return geometry && expanded ? geometry.dropdownH : 0;
}

export function useLiveWordsLift(geometry: OverlayGeometry | null, expanded: boolean) {
  const lift = liveWordsLift(geometry, expanded);
  useEffect(() => {
    emitTo('dictation-preview', LIVE_WORDS_LIFT_EVENT, lift).catch(() => {});
  }, [lift]);
}
