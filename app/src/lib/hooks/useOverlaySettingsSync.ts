import { useEffect } from 'react';
import { listen } from '@tauri-apps/api/event';
import type { Settings } from '../settings';

/** Host-committed quick controls own one field, never a full cached snapshot. */
export function useOverlaySettingsSync(apply: (fresh: Partial<Pick<Settings, 'autoPaste' | 'disabled'>>) => void) {
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void listen<unknown>('overlay-owned-setting-committed', ({ payload }) => {
      if (cancelled || !payload || typeof payload !== 'object') return;
      const { field, value } = payload as { field?: unknown; value?: unknown };
      if ((field === 'autoPaste' || field === 'disabled') && typeof value === 'boolean') {
        apply({ [field]: value });
      }
    }).then(fn => { if (cancelled) fn(); else unlisten = fn; });
    return () => { cancelled = true; unlisten?.(); };
  }, [apply]);
}
