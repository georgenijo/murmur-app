import { invoke } from '@tauri-apps/api/core';
import type { KnowledgeEntry, KnowledgeScope } from './knowledge';
import { reportLocalStats } from './localStatsEvents';

export interface TeachingContext {
  appBundleId?: string | null;
  appLabel?: string | null;
  projectRoot?: string | null;
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

export const proposeLearnedCorrection = async (
  originalText: string,
  correctedText: string,
  teachingContext?: TeachingContext,
) => {
  const outcome = await invoke<CorrectionProposalOutcome>('propose_learned_correction', {
    request: { originalText, correctedText, teachingContext },
  });
  if (outcome?.kind === 'proposal') reportLocalStats({ kind: 'correction_proposed', proposalId: outcome.proposalId });
  return outcome;
};

export const proposeSpecificLearnedCorrection = async (
  originalText: string,
  source: string,
  replacement: string,
  teachingContext?: TeachingContext,
) => {
  const outcome = await invoke<CorrectionProposalOutcome>('propose_specific_learned_correction', {
    request: { originalText, source, replacement, teachingContext },
  });
  if (outcome?.kind === 'proposal') reportLocalStats({ kind: 'correction_proposed', proposalId: outcome.proposalId });
  return outcome;
};

export const confirmLearnedCorrection = async (proposalId: number, scope: KnowledgeScope) => {
  const entry = await invoke<KnowledgeEntry>('confirm_learned_correction', { proposalId, scope });
  reportLocalStats({ kind: 'correction_taught', proposalId, scope: scope.kind });
  return entry;
};

export const discardLearnedCorrectionProposal = (proposalId: number) =>
  invoke<void>('discard_learned_correction_proposal', { proposalId });
