import { modelInstallInProgress, modelNeedsRepair, modelStatusLabel } from './modelRuntime';
import { describe, expect, it } from 'vitest';
import { applyRuntimeUpdate, type ModelRuntimeSnapshot } from './modelRuntime';

function snapshot(generation: number, lifecycleState: ModelRuntimeSnapshot['lifecycleState']): ModelRuntimeSnapshot {
  return {
    generation,
    modelName: 'fake-model',
    label: 'Fake',
    size: '1 MB',
    backend: 'whisper',
    accelerator: 'CPU',
    capabilities: {
      partialResults: false,
      initialPrompts: false,
      multilingual: false,
      translation: true,
      timestamps: true,
      confidence: true,
      punctuationControl: false,
    },
    supportedPlatforms: ['macos'],
    supported: true,
    unavailableReason: null,
    installState: 'installed',
    lifecycleState,
    failurePresent: false,
  };
}

describe('model runtime updates', () => {
  it('accepts new capabilities without feature-specific branching', () => {
    const updated = applyRuntimeUpdate([], snapshot(1, 'ready'));
    expect(updated[0].capabilities.translation).toBe(true);
    expect(updated[0].capabilities.timestamps).toBe(true);
  });

  it('rejects stale lifecycle events', () => {
    const current = snapshot(4, 'ready');
    expect(applyRuntimeUpdate([current], snapshot(3, 'loading'))).toEqual([current]);
  });
});

it('shows repair copy only for an incomplete cache or an invalid install', () => {
  expect(modelNeedsRepair({ installState: 'notInstalled', cacheIncomplete: true })).toBe(true);
  expect(modelNeedsRepair({ installState: 'invalid', cacheIncomplete: false })).toBe(true);
  // Never downloaded or cleanly removed, even after a failed warm-up: neutral copy.
  expect(modelNeedsRepair({ installState: 'notInstalled', cacheIncomplete: false })).toBe(false);
  expect(modelNeedsRepair({ installState: 'notInstalled' })).toBe(false);
  expect(modelNeedsRepair({ installState: 'installed', cacheIncomplete: true })).toBe(false);
});

it('keeps status copy readable for every install state', () => {
  for (const installState of ['notInstalled', 'installing', 'validating', 'installed', 'invalid'] as const) {
    for (const failurePresent of [true, false]) {
      expect(modelStatusLabel({ installState, failurePresent })).not.toMatch(/notInstalled|validating|invalid|failed/);
    }
  }
  expect(modelInstallInProgress({ installState: 'installing' })).toBe(true);
  expect(modelInstallInProgress({ installState: 'validating' })).toBe(true);
  expect(modelInstallInProgress({ installState: 'notInstalled' })).toBe(false);
});
