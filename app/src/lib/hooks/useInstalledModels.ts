import { useMemo, useState } from 'react';
import { useModelRuntimeCatalog, type ModelInstallState } from '../modelRuntime';
import { AVAILABLE_MODEL_OPTIONS, type ModelOption } from '../settings';

export type ModelInstallStates = Partial<Record<ModelOption, ModelInstallState>>;

export function useInstalledModels(enabled = true) {
  const [revision, setRevision] = useState(0);
  const { models, error, loaded } = useModelRuntimeCatalog(enabled, revision);

  const installStates = useMemo<ModelInstallStates | null>(() => {
    if (!loaded) return null;
    const states: ModelInstallStates = {};
    for (const { value } of AVAILABLE_MODEL_OPTIONS) states[value] = 'notInstalled';
    for (const model of models) {
      if (AVAILABLE_MODEL_OPTIONS.some(({ value }) => value === model.modelName)) {
        states[model.modelName as ModelOption] = model.installState;
      }
    }
    return states;
  }, [loaded, models]);

  // Boolean projection for callers that only need "is it on disk".
  const installedModels = useMemo(() => installStates === null
    ? null
    : Object.fromEntries(
        AVAILABLE_MODEL_OPTIONS.map(({ value }) => [value, installStates[value] === 'installed']),
      ) as Partial<Record<ModelOption, boolean>>,
  [installStates]);

  return {
    installStates,
    installedModels,
    error: Boolean(error),
    retry: () => setRevision((value) => value + 1),
  };
}

export function preferredInstalledModel(initialModel: ModelOption, installedModels: Partial<Record<ModelOption, boolean>> | null): ModelOption {
  if (!installedModels || installedModels[initialModel]) return initialModel;
  return AVAILABLE_MODEL_OPTIONS.find(({ value }) => installedModels[value])?.value ?? initialModel;
}
