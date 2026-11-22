import { modelNeedsRepair, modelStatusLabel } from './modelRuntime';
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

it('flags partial or failed Core ML installs for repair and keeps status copy readable', () => {
  expect(modelNeedsRepair({ installState: 'notInstalled', failurePresent: true })).toBe(true);
  expect(modelNeedsRepair({ installState: 'invalid', failurePresent: false })).toBe(true);
  expect(modelNeedsRepair({ installState: 'notInstalled', failurePresent: false })).toBe(false);
  expect(modelNeedsRepair({ installState: 'installed', failurePresent: false })).toBe(false);
  for (const installState of ['notInstalled', 'installing', 'validating', 'installed', 'invalid'] as const) {
    for (const failurePresent of [true, false]) {
      expect(modelStatusLabel({ installState, failurePresent })).not.toMatch(/notInstalled|validating|invalid|failed/);
    }
  }
});
