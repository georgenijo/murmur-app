import { useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { flog } from '../log';
import type { OverlayGeometry } from '../overlayGeometry';

/**
 * Initialize native calibration from the durable setting, then write again
 * only when that durable value changes. Geometry refreshes are native-owned and
 * must not replay the saved value over an active calibration draft.
 */
export function useApplySavedOverlayOffset(
  geometry: OverlayGeometry | null,
  savedOffset: number,
): void {
  const appliedSavedOffsetRef = useRef<number | null>(null);

  useEffect(() => {
    if (!geometry || appliedSavedOffsetRef.current === savedOffset) return;
    appliedSavedOffsetRef.current = savedOffset;
    void invoke('set_overlay_vertical_offset', { offset: savedOffset }).catch((error) => {
      if (appliedSavedOffsetRef.current === savedOffset) {
        appliedSavedOffsetRef.current = null;
      }
      flog.warn('overlay', 'could not apply calibrated offset', { error: String(error) });
    });
  }, [geometry, savedOffset]);
}
