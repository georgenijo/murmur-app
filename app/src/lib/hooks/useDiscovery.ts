import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import {
  DISCOVERY_CHANGED_EVENT,
  dismissDiscoveryChecklist,
  dismissDiscoveryHint,
  discoveryEvidence,
  enqueueDiscoveryHint,
  loadDiscoveryState,
  reopenDiscoveryChecklist,
  syncDiscoveryEvidence,
  type DiscoveryHintId,
} from '../discovery';
import type { HistoryEntry } from '../history';
import { SITE_MODE_BROWSERS, type Settings } from '../settings';
import { loadStats } from '../stats';
import type { MeetingRuntimeStatus } from '../meetings';

interface UseDiscoveryOptions {
  enabled: boolean;
  settings: Settings;
  statsVersion: number;
  historyEntries: HistoryEntry[];
}

function browserBundleId(value: unknown): string | null {
  if (!value || typeof value !== 'object' || !('teachingContext' in value)) return null;
  const teachingContext = value.teachingContext;
  if (!teachingContext || typeof teachingContext !== 'object' || !('appBundleId' in teachingContext)) return null;
  return typeof teachingContext.appBundleId === 'string' ? teachingContext.appBundleId : null;
}

function isSecondTapExpired(value: unknown): boolean {
  return value !== null && typeof value === 'object' && 'reason' in value
    && value.reason === 'second_tap_expired';
}

export function useDiscovery({ enabled, settings, statsVersion, historyEntries }: UseDiscoveryOptions) {
  const [state, setState] = useState(loadDiscoveryState);
  const newestHistoryId = historyEntries[historyEntries.length - 1]?.id ?? null;
  const newestHistoryIdRef = useRef(newestHistoryId);
  const browserIds = useMemo(
    () => new Set<string>(SITE_MODE_BROWSERS.map((browser) => browser.bundleId)),
    [],
  );
  const evidence = useMemo(
    () => discoveryEvidence(settings, loadStats()),
    [settings.appProfiles, settings.queryExecutable, settings.queryHotkey, statsVersion],
  );

  useEffect(() => {
    const refresh = () => setState(loadDiscoveryState());
    window.addEventListener(DISCOVERY_CHANGED_EVENT, refresh);
    return () => window.removeEventListener(DISCOVERY_CHANGED_EVENT, refresh);
  }, []);

  useEffect(() => {
    if (!enabled) return;
    setState(syncDiscoveryEvidence(evidence));
  }, [enabled, evidence]);

  useEffect(() => {
    const previousId = newestHistoryIdRef.current;
    newestHistoryIdRef.current = newestHistoryId;
    if (!enabled || newestHistoryId === null || newestHistoryId === previousId) return;
    setState(enqueueDiscoveryHint('correct_and_teach'));
  }, [enabled, newestHistoryId]);

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
      listen<unknown>('transcription-complete', ({ payload }) => {
        const bundleId = browserBundleId(payload);
        if (!disposed && bundleId !== null && browserIds.has(bundleId)) {
          setState(enqueueDiscoveryHint('browser_modes'));
        }
      }),
      listen<MeetingRuntimeStatus>('meeting-status-changed', ({ payload }) => {
        if (!disposed && payload.phase === 'processing' && payload.sessionId !== null) {
          setState(enqueueDiscoveryHint('meeting_summaries'));
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
  }, [browserIds, enabled]);

  const dismissChecklist = useCallback(() => setState(dismissDiscoveryChecklist()), []);
  const reopenChecklist = useCallback(() => setState(reopenDiscoveryChecklist()), []);
  const dismissHint = useCallback((id: DiscoveryHintId) => setState(dismissDiscoveryHint(id)), []);

  return { state, dismissChecklist, reopenChecklist, dismissHint };
}
