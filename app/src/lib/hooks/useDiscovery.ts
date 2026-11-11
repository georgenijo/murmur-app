import { useCallback, useEffect, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import {
  DISCOVERY_CHANGED_EVENT,
  dismissDiscoveryHint,
  enqueueDiscoveryHint,
  loadDiscoveryState,
  type DiscoveryHintId,
} from '../discovery';

interface UseDiscoveryOptions {
  enabled: boolean;
}

function isSecondTapExpired(value: unknown): boolean {
  return value !== null && typeof value === 'object' && 'reason' in value
    && value.reason === 'second_tap_expired';
}

export function useDiscovery({ enabled }: UseDiscoveryOptions) {
  const [state, setState] = useState(loadDiscoveryState);

  useEffect(() => {
    const refresh = () => setState(loadDiscoveryState());
    window.addEventListener(DISCOVERY_CHANGED_EVENT, refresh);
    return () => window.removeEventListener(DISCOVERY_CHANGED_EVENT, refresh);
  }, []);

  useEffect(() => {
    if (!enabled) return;
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    const subscriptions = [
      listen<unknown>('hotkey-tap-rejected', ({ payload }) => {
        if (!disposed && isSecondTapExpired(payload)) {
          setState(enqueueDiscoveryHint('double_tap_timing'));
        }
      }),
    ];
    for (const subscription of subscriptions) {
      void subscription.then((unlisten) => {
        if (disposed) unlisten();
        else unlisteners.push(unlisten);
      }).catch(() => {});
    }
    return () => {
      disposed = true;
      unlisteners.forEach((unlisten) => unlisten());
    };
  }, [enabled]);

  const dismissHint = useCallback((id: DiscoveryHintId) => setState(dismissDiscoveryHint(id)), []);

  return { state, dismissHint };
}
