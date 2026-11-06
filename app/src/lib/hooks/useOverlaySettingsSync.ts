import { useEffect } from 'react';
import { listen } from '../targetedEvents';
import type { Settings } from '../settings';

/** Host-committed quick controls own one field, never a full cached snapshot. */
export function useOverlaySettingsSync(apply: (fresh: Partial<Pick<Settings, 'autoPaste' | 'disabled' | 'ownedFieldsRevision'>>) => void) {
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void listen<unknown>('overlay-owned-setting-committed', ({ payload }) => {
      if (cancelled || !payload || typeof payload !== 'object') return;
      const { field, value, ownedFieldsRevision, autoPaste, disabled } = payload as Record<string, unknown>;
      if ((field === 'autoPaste' || field === 'disabled') && typeof value === 'boolean') {
        if (typeof ownedFieldsRevision === 'number') {
          if (!Number.isSafeInteger(ownedFieldsRevision) || ownedFieldsRevision < 0 || typeof autoPaste !== 'boolean' || typeof disabled !== 'boolean') return;
          // A revision describes both owned fields. Carry the complete committed
          // pair so a missed earlier event cannot stamp a stale value as current.
          apply({ autoPaste, disabled, ownedFieldsRevision });
        } else apply({ [field]: value });
      }
    }).then(fn => { if (cancelled) fn(); else unlisten = fn; });
    return () => { cancelled = true; unlisten?.(); };
  }, [apply]);
}
