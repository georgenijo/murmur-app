import { describe, expect, it } from 'vitest';
import { deriveReviewState } from './deriveReviewState';
import type { ReviewStateInput } from './deriveReviewState';

function input(overrides: Partial<ReviewStateInput> = {}): ReviewStateInput {
  return {
    state: 'listening',
    errorCode: null,
    instruction: '',
    original: '',
    proposed: '',
    thinkingElapsedMs: 0,
    ...overrides,
  };
}

describe('deriveReviewState', () => {
  it('listening: shows the placeholder chip, waveform, and release hint', () => {
    const vm = deriveReviewState(input({ state: 'listening' }));
    expect(vm.chipText).toBe('Listening…');
    expect(vm.showWaveform).toBe(true);
    expect(vm.subText).toBe('Release key when done');
    expect(vm.cancelEnabled).toBe(false);
    expect(vm.keyboardActionsActive).toBe(false);
  });

  it('thinking: shows the instruction text and enables cancel, no hint before 5s', () => {
    const vm = deriveReviewState(input({
      state: 'thinking', instruction: 'make this shorter', thinkingElapsedMs: 1000,
    }));
    expect(vm.chipText).toBe('make this shorter');
    expect(vm.statusText).toBe('Transforming…');
    expect(vm.cancelEnabled).toBe(true);
    expect(vm.showStillWorkingHint).toBe(false);
  });

  it('thinking: shows the "still working" hint at/after 5s', () => {
    const vm = deriveReviewState(input({ state: 'thinking', thinkingElapsedMs: 5000 }));
    expect(vm.showStillWorkingHint).toBe(true);
    const vmJustBefore = deriveReviewState(input({ state: 'thinking', thinkingElapsedMs: 4999 }));
    expect(vmJustBefore.showStillWorkingHint).toBe(false);
  });

  it('ready: shows the diff and all three actions', () => {
    const vm = deriveReviewState(input({
      state: 'ready', instruction: 'make this shorter', original: 'a b c', proposed: 'a c',
    }));
    expect(vm.showDiff).toBe(true);
    expect(vm.cancelEnabled).toBe(true);
    expect(vm.retryEnabled).toBe(true);
    expect(vm.approveEnabled).toBe(true);
    expect(vm.keyboardActionsActive).toBe(true);
  });

  it('failed: maps each stable error code to its copy', () => {
    const cases: Array<[NonNullable<ReviewStateInput['errorCode']>, string]> = [
      ['model_not_downloaded', 'Model not downloaded'],
      ['timeout', 'Timed out'],
      ['output_invalid', 'Model gave no usable output'],
      ['crashed', 'Sidecar crashed — original text untouched'],
    ];
    for (const [errorCode, message] of cases) {
      const vm = deriveReviewState(input({ state: 'failed', errorCode }));
      expect(vm.errorMessage).toBe(message);
    }
    expect(deriveReviewState(input({ state: 'failed', errorCode: undefined })).errorMessage)
      .toBe('Something went wrong');
  });

  it('failed: enables retry/cancel but not approve', () => {
    const vm = deriveReviewState(input({ state: 'failed', errorCode: 'timeout' }));
    expect(vm.retryEnabled).toBe(true);
    expect(vm.cancelEnabled).toBe(true);
    expect(vm.approveEnabled).toBe(false);
    expect(vm.keyboardActionsActive).toBe(true);
  });

  it('applied: shows undo and disables every action', () => {
    const vm = deriveReviewState(input({ state: 'applied', instruction: 'make this shorter' }));
    expect(vm.showUndo).toBe(true);
    expect(vm.chipText).toBe('make this shorter');
    expect(vm.cancelEnabled).toBe(false);
    expect(vm.retryEnabled).toBe(false);
    expect(vm.approveEnabled).toBe(false);
    expect(vm.keyboardActionsActive).toBe(false);
  });

  it('every state shows the on-device badge', () => {
    const states: ReviewStateInput['state'][] = ['listening', 'thinking', 'ready', 'failed', 'applied'];
    for (const state of states) {
      expect(deriveReviewState(input({ state })).showOnDeviceBadge).toBe(true);
    }
  });
});
