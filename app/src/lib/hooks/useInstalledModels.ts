import { useEffect, useState } from 'react';
import { getModelRuntimeCatalog } from '../modelRuntime';
import { AVAILABLE_MODEL_OPTIONS, type ModelOption } from '../settings';

export function useInstalledModels(enabled = true) {
  const [installedModels, setInstalledModels] = useState<Partial<Record<ModelOption, boolean>> | null>(null);
  const [error, setError] = useState(false);
  const [revision, setRevision] = useState(0);

  useEffect(() => {
    if (!enabled) return;
    let stale = false;
    setInstalledModels(null);
    setError(false);
    void getModelRuntimeCatalog().then((catalog) => {
      if (stale) return;
      const installed = new Set(catalog.filter((model) => model.installState === 'installed').map((model) => model.modelName));
      setInstalledModels(Object.fromEntries(AVAILABLE_MODEL_OPTIONS.map(({ value }) => [value, installed.has(value)])));
    }, () => {
      if (!stale) setError(true);
    });
    return () => { stale = true; };
  }, [enabled, revision]);

  return { installedModels, error, retry: () => setRevision((value) => value + 1) };
}

export function preferredInstalledModel(initialModel: ModelOption, installedModels: Partial<Record<ModelOption, boolean>> | null): ModelOption {
  if (!installedModels || installedModels[initialModel]) return initialModel;
  return AVAILABLE_MODEL_OPTIONS.find(({ value }) => installedModels[value])?.value ?? initialModel;
}
