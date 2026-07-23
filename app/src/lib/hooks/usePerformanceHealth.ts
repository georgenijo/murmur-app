import { invoke } from '@tauri-apps/api/core';
import { useCallback, useEffect, useState } from 'react';
import { getStatus } from '../dictation';
import {
  getModelRuntimeStatus,
  type ModelRuntimeSnapshot,
} from '../modelRuntime';
import type { DictationStatus } from '../types';

export type TransformPipelineStatus =
  | 'idle'
  | 'capturing'
  | 'listening'
  | 'thinking'
  | 'review_pending'
  | 'applying';

export interface PerformanceHealth {
  loading: boolean;
  error: string | null;
  modelName: string | null;
  dictationStatus: DictationStatus | null;
  transformStatus: TransformPipelineStatus | null;
  runtime: ModelRuntimeSnapshot | null;
}

function isDictationStatus(value: unknown): value is DictationStatus {
  return value === 'idle' || value === 'recording' || value === 'processing';
}

function isTransformStatus(value: unknown): value is TransformPipelineStatus {
  return value === 'idle'
    || value === 'capturing'
    || value === 'listening'
    || value === 'thinking'
    || value === 'review_pending'
    || value === 'applying';
}

export function usePerformanceHealth(enabled: boolean): PerformanceHealth & { refresh: () => void } {
  const [health, setHealth] = useState<PerformanceHealth>({
    loading: true,
    error: null,
    modelName: null,
    dictationStatus: null,
    transformStatus: null,
    runtime: null,
  });

  const refresh = useCallback(() => {
    if (!enabled) return;
    void (async () => {
      try {
        const [status, transformStatus] = await Promise.all([
          getStatus(),
          invoke<unknown>('transform_status'),
        ]);
        const modelName = typeof status.model === 'string' ? status.model : null;
        const runtime = modelName ? await getModelRuntimeStatus(modelName) : null;
        setHealth({
          loading: false,
          error: null,
          modelName,
          dictationStatus: isDictationStatus(status.state) ? status.state : null,
          transformStatus: isTransformStatus(transformStatus) ? transformStatus : null,
          runtime,
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
