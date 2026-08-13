import type { QueryProviderId } from './settings';

export interface QueryUsage {
  inputTokens: number;
  outputTokens: number;
  reasoningOutputTokens: number;
  cachedInputTokens: number;
  cacheCreationInputTokens: number;
  costUsd: number | null;
}

export const QUERY_PROVIDER_IDS: readonly QueryProviderId[] = [
  'claude',
  'codex',
  'grok',
  'cursor',
  'custom',
];

export function isQueryProviderId(value: unknown): value is QueryProviderId {
  return typeof value === 'string'
    && (QUERY_PROVIDER_IDS as readonly string[]).includes(value);
}

function isTokenCount(value: unknown): value is number {
  return typeof value === 'number'
    && Number.isSafeInteger(value)
    && value >= 0;
}

export function isQueryUsage(value: unknown): value is QueryUsage {
  if (!value || typeof value !== 'object') return false;
  const usage = value as Record<string, unknown>;
  return isTokenCount(usage.inputTokens)
    && isTokenCount(usage.outputTokens)
    && isTokenCount(usage.reasoningOutputTokens)
    && isTokenCount(usage.cachedInputTokens)
    && isTokenCount(usage.cacheCreationInputTokens)
    && (usage.costUsd === null || (
      typeof usage.costUsd === 'number'
      && Number.isFinite(usage.costUsd)
      && usage.costUsd >= 0
    ));
}

export function formatQueryCost(costUsd: number): string {
  if (costUsd === 0) return '$0.00';
  const digits = costUsd >= 1 ? 2 : costUsd >= 0.01 ? 4 : costUsd >= 0.0001 ? 6 : 8;
  const fixed = costUsd.toFixed(digits);
  return `$${fixed.replace(/0+$/, '').replace(/\.$/, '')}`;
}
