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

export function useModelRuntimeCatalog(enabled = true, revision = 0) {
  const [models, setModels] = useState<ModelRuntimeSnapshot[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!enabled) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    let catalogLoaded = false;
    let bufferedEvents: ModelRuntimeSnapshot[] = [];
    setLoaded(false);
    setError(null);
    void (async () => {
      try {
        const dispose = await listen<ModelRuntimeSnapshot>(
          'model-runtime-status-changed',
          (event) => {
            if (disposed) return;
            // Events can arrive while the catalog request is in flight. Hold
            // them so the fresh catalog baseline cannot overwrite a newer
            // transition; replay them once the baseline is established.
            if (!catalogLoaded) {
              bufferedEvents.push(event.payload);
              return;
            }
            setModels((catalog) => applyRuntimeUpdate(catalog, event.payload));
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
          // Every subscription/revision starts from the freshly fetched
          // catalog. Native reads can reflect on-disk changes without bumping
          // generation, so the previous subscription's equal-generation state
          // must not block this baseline.
          catalogLoaded = true;
          const baseline = bufferedEvents.reduce(applyRuntimeUpdate, catalog);
          bufferedEvents = [];
          setModels(baseline);
          setLoaded(true);
        }
      } catch (reason: unknown) {
        if (!disposed) {
          setError(String(reason));
          // The listener can still deliver transitions even though the baseline
          // fetch failed. Leave buffering so later events apply directly, and
          // replay anything that arrived while the request was in flight.
          // `loaded` stays false so the UI can retry from a fresh subscription.
          catalogLoaded = true;
          const buffered = bufferedEvents;
          bufferedEvents = [];
          setModels((current) => buffered.reduce(applyRuntimeUpdate, current));
        }
      }
    })();
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [enabled, revision]);

  const byName = useMemo(
    () => new Map(models.map((model) => [model.modelName, model])),
    [models],
  );
  return { models, byName, error, loaded };
}
