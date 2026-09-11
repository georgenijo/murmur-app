import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'react';
import { AVAILABLE_MODEL_OPTIONS, type ModelOption } from './settings';

export type ChipTier = 'legacyAppleSilicon' | 'currentAppleSilicon' | 'unknown';

export interface ModelHardwareGuidance {
  schemaVersion: 1;
  chip: string | null;
  chipTier: ChipTier;
  physicalMemoryGib: number | null;
  modelMemoryBudgetMib: number;
  recommendedModel: ModelOption;
  warnedModels: ModelOption[];
}

export function getModelHardwareGuidance(): Promise<ModelHardwareGuidance> {
  return invoke('get_model_hardware_guidance');
}

export function useModelHardwareGuidance() {
  const [guidance, setGuidance] = useState<ModelHardwareGuidance | null>(null);

  useEffect(() => {
    let disposed = false;
    void getModelHardwareGuidance()
      .then((value) => {
        if (!disposed) setGuidance(value);
      })
      .catch(() => {
        // Guidance is informational. Detection failure must not interrupt the
        // existing model picker or change its selection.
      });
    return () => {
      disposed = true;
    };
  }, []);

  return guidance;
}

export function modelGuidanceSummary(guidance: ModelHardwareGuidance): string {
  const recommendation = AVAILABLE_MODEL_OPTIONS.find(
    (model) => model.value === guidance.recommendedModel,
  )?.label ?? guidance.recommendedModel;
  const facts = [
    guidance.chip,
    guidance.physicalMemoryGib === null ? null : `${guidance.physicalMemoryGib} GB RAM`,
  ].filter((value): value is string => Boolean(value));
  return facts.length > 0
    ? `${recommendation} is recommended for ${facts.join(' with ')}.`
    : `${recommendation} is recommended for this Mac.`;
}

export function modelMemoryWarning(
  guidance: ModelHardwareGuidance,
  model: ModelOption,
): string | null {
  if (!guidance.warnedModels.includes(model)) return null;
  const budget = guidance.modelMemoryBudgetMib >= 1024
    ? `${Number((guidance.modelMemoryBudgetMib / 1024).toFixed(1))} GB`
    : `${guidance.modelMemoryBudgetMib} MB`;
  return `This model may put pressure on this Mac's ${budget} model budget. You can still select and use it.`;
}
