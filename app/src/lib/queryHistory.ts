import { invoke } from '@tauri-apps/api/core';
import type { QueryProviderId } from './settings';
import { isQueryProviderId } from './queryUsage';

export interface QueryHistoryTokenCountsV1 {
  inputTokens: number;
  outputTokens: number;
  reasoningOutputTokens: number;
  cachedInputTokens: number;
  cacheCreationInputTokens: number;
}

export const QUERY_HISTORY_ERROR_CODES = [
  'not_configured',
  'invalid_executable',
  'invalid_arguments',
  'invalid_timeout',
  'invalid_environment',
  'environment_unavailable',
  'busy',
  'audio_start_failed',
  'audio_not_ready',
  'audio_capture_failed',
  'audio_recovering',
  'audio_recovery_stalled',
  'no_speech',
  'transcription_failed',
  'empty_query',
  'query_too_large',
  'spawn_failed',
  'process_failed',
  'termination_unconfirmed',
  'timed_out',
  'output_too_large',
  'provider_not_authenticated',
  'provider_error',
  'exit_nonzero',
  'empty_answer',
  'clipboard_superseded',
  'clipboard_unavailable',
  'cancelled',
] as const;

export type QueryHistoryErrorCodeV1 = typeof QUERY_HISTORY_ERROR_CODES[number];

export interface QueryHistoryEntryV1 {
  schemaVersion: 1;
  id: string;
  timestampMs: number;
  provider: QueryProviderId;
  question: string;
  answer: string;
  tokens: QueryHistoryTokenCountsV1 | null;
  durationMs: number;
  errorCode: QueryHistoryErrorCodeV1 | null;
}

export interface QueryHistoryPageV1 {
  schemaVersion: 1;
  entries: QueryHistoryEntryV1[];
  total: number;
  offset: number;
  hasMore: boolean;
}

export interface QueryHistoryListOptions {
  offset?: number;
  limit?: number;
  provider?: QueryProviderId | null;
}

export type QueryHistoryChanged = { kind: 'inserted' | 'cleared' };

export const QUERY_HISTORY_PAGE_SIZE = 50;
export const QUERY_HISTORY_MAX_ENTRIES = 200;

const MAX_QUESTION_BYTES = 32 * 1024;
const MAX_ANSWER_BYTES = 256 * 1024;
const ECMASCRIPT_DATE_MAX_MS = 8_640_000_000_000_000;
const queryHistoryErrorCodes = new Set<string>(QUERY_HISTORY_ERROR_CODES);

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function isSafeNonNegativeInteger(value: unknown): value is number {
  return typeof value === 'number'
    && Number.isSafeInteger(value)
    && value >= 0;
}

function utf8LengthWithin(value: string, maximum: number): boolean {
  return new TextEncoder().encode(value).length <= maximum;
}

function hasExactKeys(value: Record<string, unknown>, keys: readonly string[]): boolean {
  const actual = Object.keys(value).sort();
  const expected = [...keys].sort();
  return actual.length === expected.length
    && actual.every((key, index) => key === expected[index]);
}

function isTokenCount(value: unknown): value is number {
  return isSafeNonNegativeInteger(value);
}

function isHistoryTokenCounts(value: unknown): value is QueryHistoryTokenCountsV1 {
  return isRecord(value)
    && hasExactKeys(value, [
      'inputTokens',
      'outputTokens',
      'reasoningOutputTokens',
      'cachedInputTokens',
      'cacheCreationInputTokens',
    ])
    && isTokenCount(value.inputTokens)
    && isTokenCount(value.outputTokens)
    && isTokenCount(value.reasoningOutputTokens)
    && isTokenCount(value.cachedInputTokens)
    && isTokenCount(value.cacheCreationInputTokens);
}

export function isQueryHistoryEntryV1(value: unknown): value is QueryHistoryEntryV1 {
  if (!isRecord(value)
    || !hasExactKeys(value, [
      'schemaVersion', 'id', 'timestampMs', 'provider', 'question', 'answer',
      'tokens', 'durationMs', 'errorCode',
    ])
    || value.schemaVersion !== 1
    || typeof value.id !== 'string'
    || !/^[a-f0-9]{32}$/.test(value.id)
    || !isSafeNonNegativeInteger(value.timestampMs)
    || value.timestampMs > ECMASCRIPT_DATE_MAX_MS
    || !isQueryProviderId(value.provider)
    || typeof value.question !== 'string'
    || value.question.length === 0
    || value.question.includes('\0')
    || !utf8LengthWithin(value.question, MAX_QUESTION_BYTES)
    || typeof value.answer !== 'string'
    || value.answer.includes('\0')
    || !utf8LengthWithin(value.answer, MAX_ANSWER_BYTES)
    || !(value.tokens === null || isHistoryTokenCounts(value.tokens))
    || !isSafeNonNegativeInteger(value.durationMs)) {
    return false;
  }
  return value.errorCode === null || (
    typeof value.errorCode === 'string'
    && queryHistoryErrorCodes.has(value.errorCode)
  );
}

export function isQueryHistoryPageV1(value: unknown): value is QueryHistoryPageV1 {
  if (!isRecord(value)
    || !hasExactKeys(value, ['schemaVersion', 'entries', 'total', 'offset', 'hasMore'])
    || value.schemaVersion !== 1
    || !Array.isArray(value.entries)
    || value.entries.length > QUERY_HISTORY_PAGE_SIZE
    || !value.entries.every(isQueryHistoryEntryV1)
    || !isSafeNonNegativeInteger(value.total)
    || value.total > QUERY_HISTORY_MAX_ENTRIES
    || !isSafeNonNegativeInteger(value.offset)
    || typeof value.hasMore !== 'boolean') {
    return false;
  }
  return value.offset <= value.total
    && value.entries.length <= value.total - value.offset
    && value.hasMore === (value.offset + value.entries.length < value.total);
}

export function isQueryHistoryChanged(value: unknown): value is QueryHistoryChanged {
  return isRecord(value)
    && Object.keys(value).length === 1
    && (value.kind === 'inserted' || value.kind === 'cleared');
}

export async function listQueryHistory({
  offset = 0,
  limit = QUERY_HISTORY_PAGE_SIZE,
  provider = null,
}: QueryHistoryListOptions = {}): Promise<QueryHistoryPageV1> {
  const boundedOffset = Number.isFinite(offset)
    ? Math.max(0, Math.min(QUERY_HISTORY_MAX_ENTRIES, Math.trunc(offset)))
    : 0;
  const boundedLimit = Number.isFinite(limit)
    ? Math.max(1, Math.min(QUERY_HISTORY_PAGE_SIZE, Math.trunc(limit)))
    : QUERY_HISTORY_PAGE_SIZE;
  const value = await invoke<unknown>('list_query_history', {
    offset: boundedOffset,
    limit: boundedLimit,
    provider,
  });
  if (!isQueryHistoryPageV1(value)) {
    throw new Error('Murmur returned an unsupported Voice Query history schema.');
  }
  if (value.offset !== boundedOffset) {
    throw new Error('Murmur returned an inconsistent Voice Query history page.');
  }
  return value;
}

export async function clearQueryHistory(): Promise<void> {
  await invoke('clear_query_history');
}
