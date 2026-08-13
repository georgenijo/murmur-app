import { beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));

import {
  QUERY_HISTORY_ERROR_CODES,
  clearQueryHistory,
  isQueryHistoryChanged,
  isQueryHistoryEntryV1,
  isQueryHistoryPageV1,
  listQueryHistory,
} from './queryHistory';

const entry = {
  schemaVersion: 1,
  id: '0123456789abcdef0123456789abcdef',
  timestampMs: 1_786_500_000_000,
  provider: 'claude',
  question: 'What changed?',
  answer: 'Only the bounded local store.',
  tokens: {
    inputTokens: 10,
    outputTokens: 8,
    reasoningOutputTokens: 0,
    cachedInputTokens: 2,
    cacheCreationInputTokens: 1,
  },
  durationMs: 1_250,
  errorCode: null,
} as const;

const emptyPage = () => ({
  schemaVersion: 1,
  entries: [],
  total: 0,
  offset: 0,
  hasMore: false,
});

describe('Voice Query history IPC boundary', () => {
  beforeEach(() => mocks.invoke.mockReset());

  it('accepts the exact bounded V1 content shape', () => {
    expect(isQueryHistoryEntryV1(entry)).toBe(true);
    expect(isQueryHistoryPageV1({
      schemaVersion: 1,
      entries: [entry],
      total: 1,
      offset: 0,
      hasMore: false,
    })).toBe(true);
    expect(isQueryHistoryEntryV1({
      ...entry,
      timestampMs: 8_640_000_000_000_000,
    })).toBe(true);
    expect(isQueryHistoryEntryV1({
      ...entry,
      timestampMs: 8_640_000_000_000_001,
    })).toBe(false);
  });

  it('rejects undeclared content and usage cost fields', () => {
    expect(isQueryHistoryEntryV1({ ...entry, context: 'private selection' })).toBe(false);
    expect(isQueryHistoryEntryV1({
      ...entry,
      tokens: { ...entry.tokens, costUsd: 0.25 },
    })).toBe(false);
    expect(isQueryHistoryEntryV1({ ...entry, errorCode: 'raw error: secret' })).toBe(false);
    expect(isQueryHistoryEntryV1({ ...entry, errorCode: 'syntactically_valid_but_unknown' })).toBe(false);
    QUERY_HISTORY_ERROR_CODES.forEach((errorCode) => {
      expect(isQueryHistoryEntryV1({ ...entry, errorCode })).toBe(true);
    });
    expect(isQueryHistoryPageV1({
      schemaVersion: 1,
      entries: [entry],
      total: 1,
      offset: 0,
      hasMore: false,
      stderr: 'secret',
    })).toBe(false);
  });

  it('rejects inconsistent pagination and oversized retention totals', () => {
    expect(isQueryHistoryPageV1({
      schemaVersion: 1,
      entries: [entry],
      total: 201,
      offset: 0,
      hasMore: true,
    })).toBe(false);
    expect(isQueryHistoryPageV1({
      schemaVersion: 1,
      entries: [entry],
      total: 1,
      offset: 0,
      hasMore: true,
    })).toBe(false);
    expect(isQueryHistoryPageV1({
      schemaVersion: 1,
      entries: [],
      total: 0,
      offset: 50,
      hasMore: false,
    })).toBe(false);
    expect(isQueryHistoryPageV1({
      schemaVersion: 1,
      entries: [],
      total: 2,
      offset: 0,
      hasMore: false,
    })).toBe(false);
  });

  it('uses bounded list arguments and a no-content clear command', async () => {
    mocks.invoke.mockResolvedValueOnce({
      schemaVersion: 1,
      entries: [],
      total: 0,
      offset: 0,
      hasMore: false,
    }).mockResolvedValueOnce(undefined);

    await listQueryHistory({ offset: -5, limit: 999, provider: 'codex' });
    await clearQueryHistory();

    expect(mocks.invoke).toHaveBeenNthCalledWith(1, 'list_query_history', {
      offset: 0,
      limit: 50,
      provider: 'codex',
    });
    expect(mocks.invoke).toHaveBeenNthCalledWith(2, 'clear_query_history');
  });

  it('coerces non-finite paging values before IPC', async () => {
    mocks.invoke.mockResolvedValue(emptyPage());
    await listQueryHistory({ offset: Number.NaN, limit: Number.POSITIVE_INFINITY });
    expect(mocks.invoke).toHaveBeenCalledWith('list_query_history', {
      offset: 0,
      limit: 50,
      provider: null,
    });
  });

  it('rejects a page whose returned offset differs from the bounded request', async () => {
    mocks.invoke.mockResolvedValue({
      schemaVersion: 1,
      entries: [],
      total: 0,
      offset: 0,
      hasMore: false,
    });
    await expect(listQueryHistory({ offset: 5 })).rejects.toThrow(
      'inconsistent Voice Query history page',
    );
  });

  it('accepts only exact content-free change notifications', () => {
    expect(isQueryHistoryChanged({ kind: 'inserted' })).toBe(true);
    expect(isQueryHistoryChanged({ kind: 'cleared' })).toBe(true);
    expect(isQueryHistoryChanged({ kind: 'inserted', question: 'secret' })).toBe(false);
  });
});
