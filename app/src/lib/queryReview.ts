import type { QueryProviderId } from './settings';
import type { QueryUsage } from './queryUsage';

/**
 * Canonical Voice Query state contract shared by `useQueryFlow` (arms the
 * global shortcut and tracks usage accounting) and `useQueryReviewDriver`
 * (drives the review popover UI). Both previously duplicated this contract;
 * see docs/features and issue tracking for "F9" cleanup context.
 */
export const QUERY_STATES = [
  'idle',
  'connecting',
  'listening',
  'transcribing',
  'running',
  'ready',
  'failed',
] as const;

export type QueryState = typeof QUERY_STATES[number];

/** Alias kept for call sites that name the review-popover state explicitly. */
export type QueryReviewState = QueryState;

export interface QueryStatePayload {
  queryPassId: number;
  state: QueryState;
  errorCode: string | null;
  usage?: unknown;
}

export interface QueryHiddenPayload {
  queryPassId: number;
}

export interface QueryContent {
  queryPassId: number | null;
  answer: string;
  errorDetail: string | null;
  provider: QueryProviderId | null;
  usage: QueryUsage | null;
  signInFix: string | null;
  contextSummary: string | null;
  capabilitySummary: string | null;
}

export function isValidPassId(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value > 0;
}

export function isQueryStatePayload(value: unknown): value is QueryStatePayload {
  if (!value || typeof value !== 'object') return false;
  const payload = value as Record<string, unknown>;
  return isValidPassId(payload.queryPassId)
    && typeof payload.state === 'string'
    && (QUERY_STATES as readonly string[]).includes(payload.state)
    && (payload.errorCode === null || typeof payload.errorCode === 'string');
}

export function isHiddenPayload(value: unknown): value is QueryHiddenPayload {
  if (!value || typeof value !== 'object') return false;
  const payload = value as Record<string, unknown>;
  return Object.keys(payload).length === 1 && isValidPassId(payload.queryPassId);
}
