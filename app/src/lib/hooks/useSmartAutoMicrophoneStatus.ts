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
  | { kind: 'expired' }
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

  const refresh = useCallback(async () => {
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
    const requestedAt = Date.now();
    try {
      const status = await getSmartAutoMicrophoneStatus(request);
      if (mountedRef.current && requestGenerationRef.current === generation) {
        if (status.state === 'ready') {
          const validForMs = Math.max(0, status.validForMs - (Date.now() - requestedAt));
          setView(validForMs > 0
            ? { kind: 'resolved', status: { ...status, validForMs } }
            : { kind: 'expired' });
        } else {
          setView({ kind: 'resolved', status });
        }
      }
    } catch (error) {
      if (mountedRef.current && requestGenerationRef.current === generation) {
        setView({ kind: 'unavailable', message: String(error) });
      }
    }
  }, [allowContinuity, approvedDeviceIds, enabled, preferredDeviceIds]);

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
    if (view.kind !== 'resolved' || view.status.state !== 'ready') return;
    const timeout = window.setTimeout(() => {
      if (mountedRef.current) setView({ kind: 'expired' });
    }, view.status.validForMs);
    return () => window.clearTimeout(timeout);
  }, [view]);

  useEffect(() => () => {
    mountedRef.current = false;
    requestGenerationRef.current += 1;
  }, []);

  return { view, refresh };
}
