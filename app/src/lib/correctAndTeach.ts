import { invoke } from '@tauri-apps/api/core';
import type { KnowledgeEntry, KnowledgeScope } from './knowledge';

export interface TeachingContext {
  appBundleId?: string;
  appLabel?: string;
  projectRoot?: string;
}

export interface CorrectionScopeOption {
  scope: KnowledgeScope;
  label: string;
}

export type CorrectionProposalOutcome =
  | {
      kind: 'proposal';
      proposalId: number;
      source: string;
      replacement: string;
      occurrenceCount: number;
      originalText: string;
      correctedText: string;
      scopeOptions: CorrectionScopeOption[];
    }
  | { kind: 'unsafe'; reason: string };

export const proposeLearnedCorrection = (
  originalText: string,
  correctedText: string,
  teachingContext?: TeachingContext,
) => invoke<CorrectionProposalOutcome>('propose_learned_correction', {
  request: { originalText, correctedText, teachingContext },
});

export const confirmLearnedCorrection = (proposalId: number, scope: KnowledgeScope) =>
  invoke<KnowledgeEntry>('confirm_learned_correction', { proposalId, scope });

export const discardLearnedCorrectionProposal = (proposalId: number) =>
  invoke<void>('discard_learned_correction_proposal', { proposalId });
