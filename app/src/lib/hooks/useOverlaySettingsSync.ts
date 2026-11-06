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
      const { field, value, ownedFieldsRevision } = payload as { field?: unknown; value?: unknown; ownedFieldsRevision?: unknown };
      if ((field === 'autoPaste' || field === 'disabled') && typeof value === 'boolean') {
        apply({ [field]: value, ...(typeof ownedFieldsRevision === 'number' ? { ownedFieldsRevision } : {}) });
      }
    }).then(fn => { if (cancelled) fn(); else unlisten = fn; });
    return () => { cancelled = true; unlisten?.(); };
  }, [apply]);
}
