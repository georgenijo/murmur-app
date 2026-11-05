import { emitTo } from '@tauri-apps/api/event';
import type { KnowledgeScope } from './knowledge';

export const LOCAL_STATS_COMPLETION = 'local-stats-completion';

export type LocalStatsReceipt =
  | { kind: 'correction_proposed'; proposalId: number }
  | { kind: 'correction_taught'; proposalId: number; scope: KnowledgeScope['kind'] };

function identity(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value > 0;
}

export function parseLocalStatsReceipt(value: unknown): LocalStatsReceipt | null {
  if (!value || typeof value !== 'object' || !('kind' in value)) return null;
  switch (value.kind) {
    case 'correction_proposed':
      return 'proposalId' in value && identity(value.proposalId)
        ? { kind: value.kind, proposalId: value.proposalId } : null;
    case 'correction_taught':
      if (!('proposalId' in value) || !identity(value.proposalId) || !('scope' in value)
        || (value.scope !== 'global' && value.scope !== 'app' && value.scope !== 'project')) return null;
      return { kind: value.kind, proposalId: value.proposalId, scope: value.scope };
    default: return null;
  }
}

export function reportLocalStats(receipt: LocalStatsReceipt): void {
  try {
    // Main is the only writer, including completions from the review webview.
    void emitTo('main', LOCAL_STATS_COMPLETION, receipt).catch(() => {});
  } catch {
    // Counting cannot change a completed action's result.
  }
}
