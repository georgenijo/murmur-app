import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useEffect, useMemo, useState } from 'react';

export interface ModelCapabilities {
  partialResults: boolean;
  initialPrompts: boolean;
  multilingual: boolean;
  translation: boolean;
  timestamps: boolean;
  confidence: boolean;
  punctuationControl: boolean;
}

export type ModelInstallState =
  | 'notInstalled'
  | 'installing'
  | 'validating'
  | 'installed'
  | 'invalid';

export type ModelLifecycleState =
  | 'unloaded'
  | 'loading'
  | 'warming'
  | 'ready'
  | 'unloading'
  | 'failed';

export interface ModelRuntimeSnapshot {
  generation: number;
  modelName: string;
  label: string;
  size: string;
  backend: string;
  accelerator: string;
  capabilities: ModelCapabilities;
  supportedPlatforms: string[];
  supported: boolean;
  unavailableReason: 'unsupportedPlatform' | null;
  installState: ModelInstallState;
  lifecycleState: ModelLifecycleState;
  failurePresent: boolean;
  /** Not installed, but an incomplete cache is on disk. Never set by a failed warm-up alone. */
  cacheIncomplete?: boolean;
}

export function getModelRuntimeCatalog(): Promise<ModelRuntimeSnapshot[]> {
  return invoke('get_model_runtime_catalog');
}

export function getModelRuntimeStatus(modelName: string): Promise<ModelRuntimeSnapshot> {
  return invoke('get_model_runtime_status', { modelName });
}

export function applyRuntimeUpdate(
  catalog: ModelRuntimeSnapshot[],
  update: ModelRuntimeSnapshot,
): ModelRuntimeSnapshot[] {
  const current = catalog.find((model) => model.modelName === update.modelName);
  if (current && current.generation >= update.generation) return catalog;
  if (!current) return [...catalog, update];
  return catalog.map((model) => model.modelName === update.modelName ? update : model);
}

export function useModelRuntimeCatalog(enabled = true) {
  const [models, setModels] = useState<ModelRuntimeSnapshot[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!enabled) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void (async () => {
      try {
        const dispose = await listen<ModelRuntimeSnapshot>(
          'model-runtime-status-changed',
          (event) => {
            if (!disposed) setModels((catalog) => applyRuntimeUpdate(catalog, event.payload));
          },
        );
        if (disposed) dispose();
        else unlisten = dispose;
      } catch (reason: unknown) {
        if (!disposed) setError(String(reason));
      }

      try {
        const catalog = await getModelRuntimeCatalog();
        if (!disposed) {
          // Merge by generation so a transition received after listener setup
          // cannot be overwritten by an older catalog response.
          setModels((current) => catalog.reduce(applyRuntimeUpdate, current));
        }
      } catch (reason: unknown) {
        if (!disposed) setError(String(reason));
      }
    })();
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [enabled]);

  const byName = useMemo(
    () => new Map(models.map((model) => [model.modelName, model])),
    [models],
  );
  return { models, byName, error };
}

/** Human-readable model status for user-facing copy; never show raw enum values. */
export function modelStatusLabel(model: Pick<ModelRuntimeSnapshot, 'installState' | 'failurePresent'>): string {
  if (model.installState === 'installing') return 'Downloading…';
  if (model.installState === 'validating') return 'Verifying…';
  if (model.installState === 'invalid') return 'Needs re-download';
  if (model.installState === 'notInstalled') return 'Not downloaded';
  return model.failurePresent ? 'Couldn’t load' : 'Downloaded';
}

/**
 * Repair copy is for a real on-disk signal (an incomplete cache) or an install
 * that ended invalid. A never-downloaded or cleanly removed model, even after a
 * failed warm-up, is just "Not downloaded".
 */
export function modelNeedsRepair(model: Pick<ModelRuntimeSnapshot, 'installState' | 'cacheIncomplete'>): boolean {
  return model.installState === 'invalid'
    || (model.installState === 'notInstalled' && model.cacheIncomplete === true);
}

/** An install is already running (possibly started before this panel mounted). */
export function modelInstallInProgress(model: Pick<ModelRuntimeSnapshot, 'installState'>): boolean {
  return model.installState === 'installing' || model.installState === 'validating';
}
