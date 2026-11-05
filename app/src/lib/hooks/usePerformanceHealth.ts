import { invoke } from '@tauri-apps/api/core';
import { useCallback, useEffect, useState } from 'react';
import { getStatus } from '../dictation';
import {
  deriveCaptureHealth,
  parseCaptureHealthHistory,
  type CaptureHealth,
} from '../captureHealth';
import {
  getModelRuntimeStatus,
  type ModelRuntimeSnapshot,
} from '../modelRuntime';
import type { DictationStatus } from '../types';

export interface PerformanceHealth {
  loading: boolean;
  error: string | null;
  modelName: string | null;
  dictationStatus: DictationStatus | null;
  runtime: ModelRuntimeSnapshot | null;
  capture: CaptureHealth;
}

function isDictationStatus(value: unknown): value is DictationStatus {
  return value === 'idle' || value === 'recording' || value === 'processing';
}

export function usePerformanceHealth(enabled: boolean): PerformanceHealth & { refresh: () => void } {
  const [health, setHealth] = useState<PerformanceHealth>({
    loading: true,
    error: null,
    modelName: null,
    dictationStatus: null,
    runtime: null,
    capture: deriveCaptureHealth([]),
  });

  const refresh = useCallback(() => {
    if (!enabled) return;
    void (async () => {
      try {
        const [status, captureHistoryValue] = await Promise.all([
          getStatus(),
          invoke<unknown>('get_capture_health_history'),
        ]);
        const captureHistory = parseCaptureHealthHistory(captureHistoryValue);
        const modelName = typeof status.model === 'string' ? status.model : null;
        const runtime = modelName ? await getModelRuntimeStatus(modelName) : null;
        setHealth({
          loading: false,
          error: null,
          modelName,
          dictationStatus: isDictationStatus(status.state) ? status.state : null,
          runtime,
          capture: deriveCaptureHealth(captureHistory.observations),
        });
      } catch (reason) {
        setHealth(current => ({
          ...current,
          loading: false,
          error: reason instanceof Error ? reason.message : String(reason),
        }));
      }
    })();
  }, [enabled]);

  useEffect(() => {
    if (!enabled) return;
    refresh();
    const interval = window.setInterval(refresh, 2_000);
    return () => window.clearInterval(interval);
  }, [enabled, refresh]);

  return { ...health, refresh };
}
