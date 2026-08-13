import { beforeEach, describe, expect, it } from 'vitest';
import { STATS_STORE } from './durableUserData';
import { loadStats, saveStats, updateQueryStats } from './stats';

const CLAUDE_USAGE = {
  inputTokens: 120,
  outputTokens: 45,
  reasoningOutputTokens: 3,
  cachedInputTokens: 20,
  cacheCreationInputTokens: 4,
  costUsd: 0.012,
};

describe('Voice Query stats', () => {
  beforeEach(() => localStorage.clear());

  it('migrates a pre-query stats blob with zeroed content-free counters', () => {
    localStorage.setItem(STATS_STORE.storageKey, JSON.stringify({
      totalWords: 10,
      totalRecordings: 2,
      totalDurationSeconds: 4,
      wpmSamples: [150],
      dailyBuckets: {
        '2026-08-12': {
          words: 5,
          recordings: 1,
          recordingSeconds: 2,
          answer: 'SENTINEL_DAILY_ANSWER',
        },
        SENTINEL_DATE: { words: 9, recordings: 1, recordingSeconds: 2 },
      },
    }));

    const stats = loadStats();
    expect(stats.query.queriesRun).toBe(0);
    expect(stats.query.inputTokens).toBe(0);
    expect(stats.query.outputTokens).toBe(0);
    expect(stats.query.byProvider.claude.queriesRun).toBe(0);
    expect(stats.query.failuresByErrorCode).toEqual({});
    expect(stats.dailyBuckets['2026-08-12']).toEqual({
      words: 5,
      recordings: 1,
      recordingSeconds: 2,
    });
    expect(stats.dailyBuckets.SENTINEL_DATE).toBeUndefined();
    expect(JSON.stringify(stats)).not.toContain('SENTINEL');
  });

  it('aggregates token, cost, provider, and stable failure-code counters', () => {
    updateQueryStats({
      provider: 'claude',
      succeeded: true,
      errorCode: null,
      usage: CLAUDE_USAGE,
    });
    updateQueryStats({
      provider: 'claude',
      succeeded: false,
      errorCode: 'timed_out',
      usage: null,
    });
    updateQueryStats({
      provider: 'codex',
      succeeded: false,
      errorCode: 'a raw private error must not become a key',
      usage: {
        ...CLAUDE_USAGE,
        inputTokens: 30,
        outputTokens: 10,
        costUsd: null,
      },
    });
    updateQueryStats({
      provider: 'custom',
      succeeded: true,
      // Clipboard delivery warnings do not turn a Ready pass into a failure.
      errorCode: 'clipboard_unavailable',
      usage: null,
    });

    const query = loadStats().query;
    expect(query).toMatchObject({
      queriesRun: 4,
      successfulQueries: 2,
      failedQueries: 2,
      inputTokens: 150,
      outputTokens: 55,
      reportedCostUsd: 0.012,
      failuresByErrorCode: { timed_out: 1, unknown: 1 },
    });
    expect(query.byProvider.claude).toMatchObject({
      queriesRun: 2,
      successfulQueries: 1,
      failedQueries: 1,
      inputTokens: 120,
      outputTokens: 45,
    });
    expect(query.byProvider.codex).toMatchObject({
      queriesRun: 1,
      successfulQueries: 0,
      failedQueries: 1,
      inputTokens: 30,
      outputTokens: 10,
    });
    expect(query.failuresByErrorCode.clipboard_unavailable).toBeUndefined();
    expect(JSON.stringify(query)).not.toContain('raw private error');
  });

  it('counts typed environment failures but not a successful clipboard warning', () => {
    updateQueryStats({
      provider: 'claude',
      succeeded: false,
      errorCode: 'invalid_environment',
      usage: null,
    });
    updateQueryStats({
      provider: 'codex',
      succeeded: false,
      errorCode: 'environment_unavailable',
      usage: null,
    });
    updateQueryStats({
      provider: 'grok',
      succeeded: false,
      errorCode: 'audio_capture_failed',
      usage: null,
    });
    updateQueryStats({
      provider: 'custom',
      succeeded: true,
      errorCode: 'clipboard_superseded',
      usage: null,
    });

    const query = loadStats().query;
    expect(query.successfulQueries).toBe(1);
    expect(query.failedQueries).toBe(3);
    expect(query.failuresByErrorCode).toEqual({
      invalid_environment: 1,
      environment_unavailable: 1,
      audio_capture_failed: 1,
    });
    expect(query.failuresByErrorCode).not.toHaveProperty('clipboard_superseded');
  });

  it('drops undeclared fields, providers, failure keys, and malformed numbers on load', () => {
    localStorage.setItem(STATS_STORE.storageKey, JSON.stringify({
      totalWords: 5,
      totalRecordings: 1,
      totalDurationSeconds: 2,
      wpmSamples: [],
      dailyBuckets: {},
      question: 'SENTINEL_QUESTION',
      answer: 'SENTINEL_ANSWER',
      query: {
        queriesRun: 2,
        successfulQueries: 1,
        failedQueries: 1,
        inputTokens: -5,
        outputTokens: 'secret',
        reportedCostUsd: Number.POSITIVE_INFINITY,
        byProvider: {
          claude: {
            queriesRun: 2,
            successfulQueries: 1,
            failedQueries: 1,
            inputTokens: 12,
            outputTokens: 3,
            reportedCostUsd: 0.01,
            answer: 'SENTINEL_PROVIDER_ANSWER',
          },
          privateProviderName: { queriesRun: 99 },
        },
        failuresByErrorCode: {
          timed_out: 1,
          SENTINEL_PRIVATE_ERROR: 9,
        },
      },
    }));

    const stats = loadStats();
    expect(stats.query.inputTokens).toBe(0);
    expect(stats.query.outputTokens).toBe(0);
    expect(stats.query.reportedCostUsd).toBe(0);
    expect(stats.query.failuresByErrorCode).toEqual({ timed_out: 1 });
    expect(Object.keys(stats.query.byProvider)).toEqual([
      'claude',
      'codex',
      'grok',
      'cursor',
      'custom',
    ]);
    expect(JSON.stringify(stats)).not.toContain('SENTINEL');
    expect(JSON.stringify(stats)).not.toContain('privateProviderName');

    const structurallyUnsafe = {
      ...stats,
      answer: 'SENTINEL_WRITE_ANSWER',
      query: {
        ...stats.query,
        question: 'SENTINEL_WRITE_QUESTION',
      },
    } as typeof stats;
    saveStats(structurallyUnsafe);
    expect(localStorage.getItem(STATS_STORE.storageKey)).not.toContain('SENTINEL');
  });
});
