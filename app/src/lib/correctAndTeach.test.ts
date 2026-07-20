import { beforeEach, describe, expect, it, vi } from 'vitest';

const invoke = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

import {
  confirmLearnedCorrection,
  discardLearnedCorrectionProposal,
  proposeLearnedCorrection,
} from './correctAndTeach';

describe('Correct and Teach command boundary', () => {
  beforeEach(() => invoke.mockReset());

  it('keeps proposal, explicit confirmation, and discard as separate calls', async () => {
    invoke.mockResolvedValue(undefined);
    await proposeLearnedCorrection('George Neo', 'George Nijo', {
      appBundleId: 'com.example.Editor',
    });
    await confirmLearnedCorrection(12, { kind: 'app', bundleId: 'com.example.Editor' });
    await discardLearnedCorrectionProposal(13);

    expect(invoke.mock.calls).toEqual([
      ['propose_learned_correction', { request: {
        originalText: 'George Neo', correctedText: 'George Nijo',
        teachingContext: { appBundleId: 'com.example.Editor' },
      } }],
      ['confirm_learned_correction', { proposalId: 12, scope: { kind: 'app', bundleId: 'com.example.Editor' } }],
      ['discard_learned_correction_proposal', { proposalId: 13 }],
    ]);
  });
});
