import { useCallback, useEffect, useRef, useState } from 'react';
import {
  getPerformanceStoreHealth,
  recoverPerformanceStore,
  type PerformanceStoreHealthV1,
} from '../performance';

const HEALTH_REFRESH_MS = 2_000;
const HEALTH_ERROR = 'Murmur could not verify the local diagnostics store.';
const RECOVERY_ERROR = 'Diagnostics recovery did not complete. Dictation remains available.';

export interface PerformanceStoreHealthState {
  health: PerformanceStoreHealthV1 | null;
  loading: boolean;
  error: string | null;
  recovering: boolean;
  recoveryError: string | null;
  refresh: () => Promise<void>;
  recover: () => Promise<void>;
}

export function usePerformanceStoreHealth(enabled: boolean): PerformanceStoreHealthState {
  const [health, setHealth] = useState<PerformanceStoreHealthV1 | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [recovering, setRecovering] = useState(false);
  const [recoveryError, setRecoveryError] = useState<string | null>(null);
  const enabledRef = useRef(enabled);
  const mountedRef = useRef(true);
  const requestSequenceRef = useRef(0);
  const refreshRequestRef = useRef<number | null>(null);
  const recoveryInFlightRef = useRef(false);
  enabledRef.current = enabled;

  const refresh = useCallback(async () => {
    if (!enabledRef.current
      || recoveryInFlightRef.current
      || refreshRequestRef.current !== null) return;
    const request = ++requestSequenceRef.current;
    refreshRequestRef.current = request;
    setLoading(true);
    try {
      const nextHealth = await getPerformanceStoreHealth();
      if (!mountedRef.current
        || !enabledRef.current
        || requestSequenceRef.current !== request) return;
      setHealth(nextHealth);
      setError(null);
    } catch {
      if (!mountedRef.current
        || !enabledRef.current
        || requestSequenceRef.current !== request) return;
      setError(HEALTH_ERROR);
    } finally {
      if (refreshRequestRef.current === request) {
        refreshRequestRef.current = null;
        if (mountedRef.current) setLoading(false);
      }
    }
  }, []);

  const recover = useCallback(async () => {
    if (!enabledRef.current || recoveryInFlightRef.current) return;
    recoveryInFlightRef.current = true;
    refreshRequestRef.current = null;
    const request = ++requestSequenceRef.current;
    const allowReinitialize = health?.recommendedAction === 'reinitializeStore';
    setRecovering(true);
    setLoading(false);
    setRecoveryError(null);
    try {
      const nextHealth = await recoverPerformanceStore(allowReinitialize);
      if (!mountedRef.current
        || !enabledRef.current
        || requestSequenceRef.current !== request) return;
      setHealth(nextHealth);
      setError(null);
    } catch {
      if (!mountedRef.current
        || !enabledRef.current
        || requestSequenceRef.current !== request) return;
      setRecoveryError(RECOVERY_ERROR);
    } finally {
      recoveryInFlightRef.current = false;
      if (mountedRef.current) setRecovering(false);
    }
  }, [health?.recommendedAction]);

  useEffect(() => {
    if (!enabled) {
      requestSequenceRef.current += 1;
      refreshRequestRef.current = null;
      setLoading(false);
      return;
    }
    void refresh();
    const interval = window.setInterval(() => void refresh(), HEALTH_REFRESH_MS);
    return () => {
      window.clearInterval(interval);
      requestSequenceRef.current += 1;
      refreshRequestRef.current = null;
    };
  }, [enabled, refresh]);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      requestSequenceRef.current += 1;
      refreshRequestRef.current = null;
    };
  }, []);

  return {
    health,
    loading,
    error,
    recovering,
    recoveryError,
    refresh,
    recover,
  };
}
