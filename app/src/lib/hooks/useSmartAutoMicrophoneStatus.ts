import { useCallback, useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import {
  getSmartAutoMicrophoneStatus,
  type SmartAutoMicrophoneStatus,
} from '../smartAutoMicrophone';
import type { SmartAutoMicrophoneRequest } from '../settings';

export type SmartAutoMicrophoneStatusView =
  | { kind: 'inactive' }
  | { kind: 'loading' }
  | { kind: 'resolved'; status: SmartAutoMicrophoneStatus }
  | { kind: 'unavailable'; message: string };

const REFRESH_EVENTS = [
  'smart-auto-microphone-changed',
  'audio-input-inventory-changed',
  'recording-initialization-failed',
] as const;

export function useSmartAutoMicrophoneStatus(
  smartAuto: SmartAutoMicrophoneRequest | null,
  enabled = true,
): { view: SmartAutoMicrophoneStatusView; refresh: () => Promise<void> } {
  const [view, setView] = useState<SmartAutoMicrophoneStatusView>(
    smartAuto && enabled ? { kind: 'loading' } : { kind: 'inactive' },
  );
  const mountedRef = useRef(true);
  const requestGenerationRef = useRef(0);

  const approvedDeviceIds = smartAuto?.approvedDeviceIds;
  const preferredDeviceIds = smartAuto?.preferredDeviceIds;
  const allowContinuity = smartAuto?.allowContinuity;

  const refreshWithCause = useCallback(async (cause: 'external' | 'deadline') => {
    const generation = ++requestGenerationRef.current;
    if (!enabled || !approvedDeviceIds || !preferredDeviceIds || allowContinuity === undefined) {
      if (mountedRef.current) setView({ kind: 'inactive' });
      return;
    }
    const request: SmartAutoMicrophoneRequest = {
      approvedDeviceIds,
      preferredDeviceIds,
      allowContinuity,
    };
    setView({ kind: 'loading' });
    const requestedAt = performance.now();
    try {
      const status = await getSmartAutoMicrophoneStatus(request);
      if (mountedRef.current && requestGenerationRef.current === generation) {
        const elapsedMs = Math.max(0, performance.now() - requestedAt);
        if (status.state === 'ready') {
          const validForMs = Math.max(0, status.validForMs - elapsedMs);
          if (validForMs === 0) {
            if (cause === 'external') void refreshWithCause('deadline');
            else setView({ kind: 'unavailable', message: 'Smart Auto status expired before Murmur could confirm it.' });
          } else {
            setView({ kind: 'resolved', status: { ...status, validForMs } });
          }
        } else {
          const retryAfterMs = status.retryAfterMs === null
            ? null
            : cause === 'deadline'
              ? status.retryAfterMs
              : Math.max(0, status.retryAfterMs - elapsedMs);
          if (retryAfterMs === 0) {
            void refreshWithCause('deadline');
          } else {
            setView({ kind: 'resolved', status: { ...status, retryAfterMs } });
          }
        }
      }
    } catch (error) {
      if (mountedRef.current && requestGenerationRef.current === generation) {
        setView({ kind: 'unavailable', message: String(error) });
      }
    }
  }, [allowContinuity, approvedDeviceIds, enabled, preferredDeviceIds]);
  const refresh = useCallback(() => refreshWithCause('external'), [refreshWithCause]);

  useEffect(() => {
    mountedRef.current = true;
    if (!enabled || !approvedDeviceIds || !preferredDeviceIds || allowContinuity === undefined) {
      requestGenerationRef.current += 1;
      setView({ kind: 'inactive' });
      return;
    }

    let disposed = false;
    const unlistens: Array<() => void> = [];
    void Promise.all(REFRESH_EVENTS.map((eventName) => (
      listen<unknown>(eventName, () => {
        if (!disposed) void refresh();
      }).catch(() => null)
    ))).then((listeners) => {
      for (const unlisten of listeners) {
        if (!unlisten) continue;
        if (disposed) unlisten();
        else unlistens.push(unlisten);
      }
      // Subscribe before the initial snapshot so an invalidation cannot land
      // in the gap and leave an old ready result visible.
      if (!disposed) void refresh();
    });
    return () => {
      disposed = true;
      requestGenerationRef.current += 1;
      for (const unlisten of unlistens) unlisten();
    };
  }, [allowContinuity, approvedDeviceIds, enabled, preferredDeviceIds, refresh]);

  useEffect(() => {
    if (view.kind !== 'resolved') return;
    const deadlineMs = view.status.state === 'ready'
      ? view.status.validForMs
      : view.status.retryAfterMs;
    if (deadlineMs === null) return;
    const timeout = window.setTimeout(() => {
      if (mountedRef.current) void refreshWithCause('deadline');
    }, deadlineMs);
    return () => window.clearTimeout(timeout);
  }, [refreshWithCause, view]);

  useEffect(() => () => {
    mountedRef.current = false;
    requestGenerationRef.current += 1;
  }, []);

  return { view, refresh };
}
