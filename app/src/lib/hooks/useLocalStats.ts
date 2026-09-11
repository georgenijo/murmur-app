import { useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { monthKey } from '../activityStats';
import { LOCAL_STATS_COMPLETION, parseLocalStatsReceipt } from '../localStatsEvents';
import { STATS_CHANGED_EVENT, STATS_RESET_EVENT, updateActivityStats } from '../stats';
import type { MeetingRuntimeStatus, MeetingSummaryStatus } from '../meetings';

interface TransformCompletionState {
  month: string;
  presetName: string | null;
  received: Set<'runs' | 'approved' | 'undone'>;
  counted: Set<'runs' | 'approved' | 'undone'>;
  reset: boolean;
}

/** Mount once in main, where all synchronous stats mutations are serialized. */
export function useLocalStats(): number {
  const [version, setVersion] = useState(0);
  const transforms = useRef(new Map<string, TransformCompletionState>());
  const corrections = useRef(new Set<string>());
  const summaries = useRef(new Set<number>());
  const meetings = useRef(new Set<number>());
  const lastMeeting = useRef<MeetingRuntimeStatus | null>(null);

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    const changed = () => setVersion((value) => value + 1);
    const reset = () => {
      for (const completion of transforms.current.values()) completion.reset = true;
    };
    window.addEventListener(STATS_CHANGED_EVENT, changed);
    window.addEventListener(STATS_RESET_EVENT, reset);
    const subscriptions = [
      listen<unknown>(LOCAL_STATS_COMPLETION, ({ payload }) => {
        if (disposed) return;
        const receipt = parseLocalStatsReceipt(payload);
        if (!receipt) return;
        if (receipt.kind === 'transform') {
          const key = `${receipt.passId}:${receipt.attempt}`;
          let completion = transforms.current.get(key);
          if (!completion) {
            completion = {
              month: monthKey(), presetName: receipt.presetName,
              received: new Set(), counted: new Set(), reset: false,
            };
            transforms.current.set(key, completion);
          }
          if (completion.reset) return;
          completion.received.add(receipt.outcome);
          if (receipt.outcome === 'runs' && !completion.counted.has('runs')) {
            completion.month = monthKey();
            completion.presetName = receipt.presetName;
          }
          // An approval can arrive before its run receipt. Its approval rate
          // belongs to the run's month, including approvals after month-end.
          if (!completion.received.has('runs')) return;
          for (const outcome of ['runs', 'approved', 'undone'] as const) {
            if (!completion.received.has(outcome) || completion.counted.has(outcome)) continue;
            if (outcome === 'undone' && !completion.counted.has('approved')) continue;
            completion.counted.add(outcome);
            updateActivityStats({ kind: 'transform', outcome, presetName: completion.presetName }, completion.month);
          }
        } else {
          const key = `${receipt.kind}:${receipt.proposalId}`;
          if (corrections.current.has(key)) return;
          corrections.current.add(key);
          updateActivityStats(receipt.kind === 'correction_taught'
            ? { kind: receipt.kind, scope: receipt.scope }
            : { kind: receipt.kind });
        }
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
      listen<MeetingSummaryStatus>('meeting-summary-status-changed', ({ payload }) => {
        if (disposed || payload.phase !== 'complete' || !payload.sessionId
          || summaries.current.has(payload.generation)) return;
        summaries.current.add(payload.generation);
        updateActivityStats({ kind: 'meeting_summary' });
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
      window.removeEventListener(STATS_RESET_EVENT, reset);
    };
  }, []);
  return version;
}
