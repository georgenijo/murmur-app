import { useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { LOCAL_STATS_COMPLETION, parseLocalStatsReceipt } from '../localStatsEvents';
import { STATS_CHANGED_EVENT, updateActivityStats } from '../stats';
import type { MeetingRuntimeStatus } from '../meetings';

/** Mount once in main, where all synchronous stats mutations are serialized. */
export function useLocalStats(): number {
  const [version, setVersion] = useState(0);
  const corrections = useRef(new Set<string>());
  const meetings = useRef(new Set<number>());
  const lastMeeting = useRef<MeetingRuntimeStatus | null>(null);

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    const changed = () => setVersion((value) => value + 1);
    window.addEventListener(STATS_CHANGED_EVENT, changed);
    const subscriptions = [
      listen<unknown>(LOCAL_STATS_COMPLETION, ({ payload }) => {
        if (disposed) return;
        const receipt = parseLocalStatsReceipt(payload);
        if (!receipt) return;
          const key = `${receipt.kind}:${receipt.proposalId}`;
          if (corrections.current.has(key)) return;
          corrections.current.add(key);
          updateActivityStats(receipt.kind === 'correction_taught'
            ? { kind: receipt.kind, scope: receipt.scope }
            : { kind: receipt.kind });
      }),
      listen<MeetingRuntimeStatus>('meeting-status-changed', ({ payload }) => {
        if (disposed) return;
        const previous = lastMeeting.current;
        if (previous && payload.generation < previous.generation) return;
        lastMeeting.current = payload;
        if (payload.phase !== 'idle' || !previous || previous.phase !== 'processing'
          || previous.generation !== payload.generation || !previous.sessionId
          || meetings.current.has(payload.generation)) return;
        meetings.current.add(payload.generation);
        updateActivityStats({ kind: 'meeting', durationMs: previous.elapsedMs });
      }),
      listen<unknown>('delivery-retry-feedback', ({ payload }) => {
        if (disposed || !payload || typeof payload !== 'object' || !('kind' in payload)) return;
        if (payload.kind === 'auto_pasted' || payload.kind === 'clipboard_only') {
          updateActivityStats({ kind: 'paste_last' });
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
      window.removeEventListener(STATS_CHANGED_EVENT, changed);
    };
  }, []);
  return version;
}
