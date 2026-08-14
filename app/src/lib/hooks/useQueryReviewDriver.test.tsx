import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

type Listener = (event: { payload: unknown }) => void;

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listeners: {} as Record<string, Listener>,
  unlisten: vi.fn(),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (event: string, listener: Listener) => {
    mocks.listeners[event] = listener;
    return mocks.unlisten;
  }),
}));
vi.mock('../log', () => ({
  flog: { info: vi.fn(), warn: vi.fn(), error: vi.fn() },
}));

import { useQueryReviewDriver } from './useQueryReviewDriver';

type Driver = ReturnType<typeof useQueryReviewDriver>;

function content(
  queryPassId: number,
  answer: string,
  detail = '',
  fix = '',
  usage: unknown = null,
  contextSummary: string | null = null,
) {
  return {
    queryPassId,
    answer,
    errorDetail: detail,
    provider: 'claude',
    usage,
    signInFix: fix,
    contextSummary,
  };
}

function usage(inputTokens: number) {
  return {
    inputTokens,
    outputTokens: 8,
    reasoningOutputTokens: 1,
    cachedInputTokens: 2,
    cacheCreationInputTokens: 3,
    costUsd: 0.004,
  };
}

describe('useQueryReviewDriver ownership', () => {
  let container: HTMLDivElement;
  let root: Root;
  let current: Driver | null = null;

  beforeEach(() => {
    vi.useRealTimers();
    vi.clearAllMocks();
    mocks.invoke.mockReset();
    mocks.listeners = {};
    mocks.unlisten.mockReset();
    current = null;
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    vi.useRealTimers();
    await act(async () => root.unmount());
    container.remove();
  });

  async function mount() {
    function Harness() {
      current = useQueryReviewDriver();
      return null;
    }
    await act(async () => {
      root.render(<Harness />);
      await Promise.resolve();
      await Promise.resolve();
    });
  }

  it('refreshes the terminal answer and context snapshot', async () => {
    mocks.invoke.mockResolvedValue(content(
      41,
      'answer',
      '',
      '',
      null,
      'Context: TextEdit',
    ));
    await mount();

    await act(async () => {
      mocks.listeners['query-state-changed']?.({
        payload: { queryPassId: 41, state: 'ready', errorCode: null },
      });
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(current!.contextSummary).toBe('Context: TextEdit');
  });

  it('does not let a delayed prior-pass snapshot overwrite any active-pass field', async () => {
    let resolvePrior!: (value: ReturnType<typeof content>) => void;
    const prior = new Promise<ReturnType<typeof content>>((resolve) => { resolvePrior = resolve; });
    mocks.invoke
      .mockResolvedValueOnce(content(61, '', '', '', null, 'Context: Prior app'))
      .mockImplementationOnce(() => prior)
      .mockResolvedValueOnce(content(62, '', '', '', null, 'Context: Current app'))
      .mockResolvedValueOnce(content(
        62,
        'current answer',
        'current detail',
        'current fix',
        usage(62),
      ));
    await mount();

    await act(async () => {
      mocks.listeners['query-state-changed']?.({
        payload: { queryPassId: 61, state: 'failed', errorCode: 'provider_not_authenticated' },
      });
      await Promise.resolve();
      mocks.listeners['query-state-changed']?.({
        payload: { queryPassId: 62, state: 'failed', errorCode: 'provider_not_authenticated' },
      });
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(current!.answer).toBe('current answer');
    expect(current!.errorDetail).toBe('current detail');
    expect(current!.signInFix).toBe('current fix');
    expect(current!.usage?.inputTokens).toBe(62);
    expect(current!.contextSummary).toBe('Context: Current app');

    await act(async () => {
      resolvePrior(content(61, 'stale answer', 'stale detail', 'stale fix', usage(61)));
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(current!.answer).toBe('current answer');
    expect(current!.errorDetail).toBe('current detail');
    expect(current!.signInFix).toBe('current fix');
    expect(current!.usage?.inputTokens).toBe(62);
    expect(current!.contextSummary).toBe('Context: Current app');
  });

  it('keeps only the newest same-pass recovery snapshot across a replace gap race', async () => {
    let resolveOlder!: (value: ReturnType<typeof content>) => void;
    let resolveNewer!: (value: ReturnType<typeof content>) => void;
    const older = new Promise<ReturnType<typeof content>>((resolve) => { resolveOlder = resolve; });
    const newer = new Promise<ReturnType<typeof content>>((resolve) => { resolveNewer = resolve; });
    mocks.invoke
      .mockResolvedValueOnce(content(71, ''))
      .mockImplementationOnce(() => older)
      .mockImplementationOnce(() => newer);
    await mount();
    await act(async () => {
      mocks.listeners['query-state-changed']?.({
        payload: { queryPassId: 71, state: 'running', errorCode: null },
      });
      await Promise.resolve();
      mocks.listeners['query-answer-chunk']?.({
        payload: { queryPassId: 71, sequence: 1, text: 'gap', replace: false },
      });
      mocks.listeners['query-answer-chunk']?.({
        payload: { queryPassId: 71, sequence: 2, text: 'raw fallback', replace: true },
      });
      await Promise.resolve();
    });
    await act(async () => {
      resolveNewer(content(71, 'complete latest', 'latest detail', 'latest fix', usage(72)));
      await Promise.resolve();
      await Promise.resolve();
    });
    await act(async () => {
      resolveOlder(content(71, 'older partial', 'older detail', 'older fix', usage(71)));
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(current!.answer).toBe('complete latest');
    expect(current!.errorDetail).toBe('latest detail');
    expect(current!.signInFix).toBe('latest fix');
    expect(current!.usage?.inputTokens).toBe(72);
  });

  it('keeps the terminal snapshot authoritative over a concurrent replace event', async () => {
    let resolveTerminal!: (value: ReturnType<typeof content>) => void;
    const terminal = new Promise<ReturnType<typeof content>>((resolve) => { resolveTerminal = resolve; });
    mocks.invoke
      .mockResolvedValueOnce(content(81, ''))
      .mockImplementationOnce(() => terminal);
    await mount();
    await act(async () => {
      mocks.listeners['query-state-changed']?.({
        payload: { queryPassId: 81, state: 'ready', errorCode: null },
      });
      await Promise.resolve();
      mocks.listeners['query-answer-chunk']?.({
        payload: { queryPassId: 81, sequence: 0, text: 'raw fallback', replace: true },
      });
      await Promise.resolve();
    });
    expect(current!.answer).toBe('raw fallback');
    await act(async () => {
      resolveTerminal(content(81, 'streamed prefix plus tail'));
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(current!.answer).toBe('streamed prefix plus tail');
  });

  it('cancels an in-flight sign-in probe when the active pass changes', async () => {
    vi.useFakeTimers();
    let resolveProbe!: (authenticated: boolean) => void;
    const probe = new Promise<boolean>((resolve) => { resolveProbe = resolve; });
    mocks.invoke.mockImplementation((command: string) => {
      if (command === 'get_query_review_content') {
        return Promise.resolve(content(91, '', 'not signed in', 'Run claude /login.'));
      }
      if (command === 'launch_query_sign_in_for_pass') return Promise.resolve();
      if (command === 'probe_query_sign_in_for_pass') return probe;
      return Promise.resolve();
    });
    await mount();
    await act(async () => {
      mocks.listeners['query-state-changed']?.({
        payload: { queryPassId: 91, state: 'failed', errorCode: 'provider_not_authenticated' },
      });
      await Promise.resolve();
      await Promise.resolve();
    });

    let signInPromise!: Promise<void>;
    await act(async () => {
      signInPromise = current!.signIn();
      await Promise.resolve();
    });
    expect(current!.signInBusy).toBe(true);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2000);
      await Promise.resolve();
    });
    expect(mocks.invoke).toHaveBeenCalledWith('probe_query_sign_in_for_pass', { queryPassId: 91 });

    await act(async () => {
      mocks.listeners['query-state-changed']?.({
        payload: { queryPassId: 92, state: 'connecting', errorCode: null },
      });
      await Promise.resolve();
    });
    expect(current!.signInBusy).toBe(false);
    expect(current!.signInStatus).toBeNull();

    await act(async () => {
      resolveProbe(true);
      await signInPromise;
      await Promise.resolve();
    });
    expect(current!.signInStatus).toBeNull();
    expect(current!.signInBusy).toBe(false);
  });

  it('does not let a delayed prior-pass Copy success clear the new pass error', async () => {
    let resolveCopy!: () => void;
    const copy = new Promise<void>((resolve) => { resolveCopy = resolve; });
    let contentPassId = 101;
    mocks.invoke.mockImplementation((command: string) => {
      if (command === 'get_query_review_content') {
        return Promise.resolve(content(contentPassId, 'answer', '', 'Run claude /login.'));
      }
      if (command === 'copy_query_answer') return copy;
      return Promise.resolve();
    });
    await mount();
    await act(async () => {
      mocks.listeners['query-state-changed']?.({
        payload: { queryPassId: 101, state: 'ready', errorCode: 'clipboard_unavailable' },
      });
      await Promise.resolve();
      await Promise.resolve();
      current!.copy();
      await Promise.resolve();
    });

    contentPassId = 102;
    await act(async () => {
      mocks.listeners['query-state-changed']?.({
        payload: { queryPassId: 102, state: 'failed', errorCode: 'provider_not_authenticated' },
      });
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(current!.errorCode).toBe('provider_not_authenticated');

    await act(async () => {
      resolveCopy();
      await copy;
      await Promise.resolve();
    });
    expect(current!.errorCode).toBe('provider_not_authenticated');
  });

  it('marks the current answer copied after an explicit Copy succeeds', async () => {
    mocks.invoke.mockImplementation((command: string) => {
      if (command === 'get_query_review_content') {
        return Promise.resolve(content(103, 'answer'));
      }
      return Promise.resolve();
    });
    await mount();
    await act(async () => {
      mocks.listeners['query-state-changed']?.({
        payload: { queryPassId: 103, state: 'ready', errorCode: 'auto_copy_disabled' },
      });
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(current!.errorCode).toBe('auto_copy_disabled');

    await act(async () => {
      current!.copy();
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(mocks.invoke).toHaveBeenCalledWith('copy_query_answer', { queryPassId: 103 });
    expect(current!.errorCode).toBeNull();
  });

  it('does not let a delayed prior-pass Copy failure overwrite the new pass error', async () => {
    let rejectCopy!: (error: Error) => void;
    const copy = new Promise<void>((_resolve, reject) => { rejectCopy = reject; });
    let contentPassId = 111;
    mocks.invoke.mockImplementation((command: string) => {
      if (command === 'get_query_review_content') {
        return Promise.resolve(content(contentPassId, 'answer', '', 'Run claude /login.'));
      }
      if (command === 'copy_query_answer') return copy;
      return Promise.resolve();
    });
    await mount();
    await act(async () => {
      mocks.listeners['query-state-changed']?.({
        payload: { queryPassId: 111, state: 'ready', errorCode: null },
      });
      await Promise.resolve();
      await Promise.resolve();
      current!.copy();
      await Promise.resolve();
    });

    contentPassId = 112;
    await act(async () => {
      mocks.listeners['query-state-changed']?.({
        payload: { queryPassId: 112, state: 'failed', errorCode: 'provider_not_authenticated' },
      });
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(current!.errorCode).toBe('provider_not_authenticated');

    await act(async () => {
      rejectCopy(new Error('old clipboard failure'));
      await copy.catch(() => undefined);
      await Promise.resolve();
    });
    expect(current!.errorCode).toBe('provider_not_authenticated');
  });

  it('replaces optimistic structured chunks when the adapter falls back to raw', async () => {
    await mount();
    await act(async () => {
      mocks.listeners['query-state-changed']?.({
        payload: { queryPassId: 17, state: 'running', errorCode: null },
      });
      mocks.listeners['query-answer-chunk']?.({
        payload: {
          queryPassId: 17,
          sequence: 0,
          text: 'clean answer',
          replace: false,
        },
      });
    });
    expect(current!.answer).toBe('clean answer');

    await act(async () => {
      mocks.listeners['query-answer-chunk']?.({
        payload: {
          queryPassId: 17,
          sequence: 1,
          text: '{"type":"partial"}\nmalformed\n',
          replace: true,
        },
      });
      mocks.listeners['query-answer-chunk']?.({
        payload: {
          queryPassId: 17,
          sequence: 2,
          text: 'raw tail',
          replace: false,
        },
      });
    });

    expect(current!.answer).toBe('{"type":"partial"}\nmalformed\nraw tail');
  });

  it('rejects chunks that omit the explicit append-or-replace mode', async () => {
    await mount();
    await act(async () => {
      mocks.listeners['query-state-changed']?.({
        payload: { queryPassId: 18, state: 'running', errorCode: null },
      });
      mocks.listeners['query-answer-chunk']?.({
        payload: { queryPassId: 18, sequence: 0, text: 'ambiguous' },
      });
    });
    expect(current!.answer).toBe('');
  });

  it('accepts only typed numeric usage from the exact terminal pass', async () => {
    await mount();
    mocks.invoke.mockResolvedValue({
      queryPassId: 23,
      answer: 'done',
      errorDetail: null,
      provider: 'claude',
      usage: {
        inputTokens: 1234,
        outputTokens: 56,
        reasoningOutputTokens: 0,
        cachedInputTokens: 100,
        cacheCreationInputTokens: 2,
        costUsd: 0.012,
      },
      signInFix: null,
      contextSummary: null,
    });

    await act(async () => {
      mocks.listeners['query-state-changed']?.({
        payload: { queryPassId: 23, state: 'ready', errorCode: null },
      });
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(current!.usage).toEqual({
      inputTokens: 1234,
      outputTokens: 56,
      reasoningOutputTokens: 0,
      cachedInputTokens: 100,
      cacheCreationInputTokens: 2,
      costUsd: 0.012,
    });
  });

  it('pulls the visible context summary through the gated content command', async () => {
    mocks.invoke.mockResolvedValue(content(
      41,
      '',
      '',
      '',
      null,
      'Context: Safari — 1.2 KB selection',
    ));
    await mount();

    await act(async () => {
      mocks.listeners['query-state-changed']?.({
        payload: { queryPassId: 41, state: 'connecting', errorCode: null },
      });
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(current!.contextSummary).toBe('Context: Safari — 1.2 KB selection');

    mocks.invoke.mockClear();
    await act(async () => {
      mocks.listeners['query-context-resolved']?.({ payload: { queryPassId: 41 } });
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(mocks.invoke).toHaveBeenCalledWith('get_query_review_content');
  });

  it('ignores stale context notifications and clears summary when hidden', async () => {
    mocks.invoke.mockResolvedValue(content(
      52,
      'answer',
      '',
      '',
      null,
      'Context: Editor — window title',
    ));
    await mount();
    await act(async () => {
      mocks.listeners['query-state-changed']?.({
        payload: { queryPassId: 52, state: 'ready', errorCode: null },
      });
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(current!.contextSummary).toBe('Context: Editor — window title');

    mocks.invoke.mockClear();
    await act(async () => {
      mocks.listeners['query-context-resolved']?.({ payload: { queryPassId: 51 } });
      await Promise.resolve();
    });
    expect(mocks.invoke).not.toHaveBeenCalled();

    await act(async () => {
      mocks.listeners['query-review-hidden']?.({ payload: { queryPassId: 51 } });
      mocks.listeners['query-review-hidden']?.({
        payload: { queryPassId: 52, extra: 'SENTINEL_CONTENT' },
      });
      await Promise.resolve();
    });
    expect(current!.state).toBe('ready');
    expect(current!.contextSummary).toBe('Context: Editor — window title');

    await act(async () => {
      mocks.listeners['query-review-hidden']?.({ payload: { queryPassId: 52 } });
      await Promise.resolve();
    });
    expect(current!.state).toBe('idle');
    expect(current!.answer).toBe('');
    expect(current!.contextSummary).toBeNull();
  });

  it('does not let a delayed prior-pass refresh overwrite the active pass', async () => {
    let resolvePrior!: (content: {
      queryPassId: number;
      answer: string;
      contextSummary: string;
    }) => void;
    const priorRefresh = new Promise<{
      queryPassId: number;
      answer: string;
      contextSummary: string;
    }>((resolve) => {
      resolvePrior = resolve;
    });
    mocks.invoke
      .mockImplementationOnce(() => priorRefresh)
      .mockResolvedValueOnce({
        queryPassId: 62,
        answer: 'current answer',
        contextSummary: 'Context: Current app',
      })
      .mockResolvedValueOnce({
        queryPassId: 62,
        answer: 'current answer',
        contextSummary: 'Context: Current app',
      });
    await mount();

    await act(async () => {
      mocks.listeners['query-state-changed']?.({
        payload: { queryPassId: 61, state: 'running', errorCode: null },
      });
      await Promise.resolve();
      mocks.listeners['query-state-changed']?.({
        payload: { queryPassId: 62, state: 'ready', errorCode: null },
      });
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(current!.answer).toBe('current answer');
    expect(current!.contextSummary).toBe('Context: Current app');

    await act(async () => {
      resolvePrior({
        queryPassId: 61,
        answer: 'stale answer',
        contextSummary: 'Context: Stale app — secret selection',
      });
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(current!.answer).toBe('current answer');
    expect(current!.contextSummary).toBe('Context: Current app');
  });

  it('keeps only the newest same-pass answer recovery response', async () => {
    let resolveOlder!: (content: {
      queryPassId: number;
      answer: string;
      contextSummary: string | null;
    }) => void;
    let resolveNewer!: typeof resolveOlder;
    const older = new Promise<{
      queryPassId: number;
      answer: string;
      contextSummary: string | null;
    }>((resolve) => { resolveOlder = resolve; });
    const newer = new Promise<{
      queryPassId: number;
      answer: string;
      contextSummary: string | null;
    }>((resolve) => { resolveNewer = resolve; });
    mocks.invoke
      .mockResolvedValueOnce({ queryPassId: 71, answer: '', contextSummary: null })
      .mockImplementationOnce(() => older)
      .mockImplementationOnce(() => newer);
    await mount();

    await act(async () => {
      mocks.listeners['query-state-changed']?.({
        payload: { queryPassId: 71, state: 'running', errorCode: null },
      });
      await Promise.resolve();
      mocks.listeners['query-answer-chunk']?.({
        payload: { queryPassId: 71, sequence: 1, text: 'gap chunk', replace: false },
      });
      mocks.listeners['query-answer-chunk']?.({
        payload: { queryPassId: 71, sequence: 2, text: 'later chunk', replace: false },
      });
      await Promise.resolve();
    });

    await act(async () => {
      resolveNewer({ queryPassId: 71, answer: 'complete latest answer', contextSummary: null });
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(current!.answer).toBe('complete latest answer');

    await act(async () => {
      resolveOlder({ queryPassId: 71, answer: 'older incomplete answer', contextSummary: null });
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(current!.answer).toBe('complete latest answer');
  });

  it('lets the authoritative terminal snapshot recover a missing final chunk event', async () => {
    let resolveSnapshot!: (content: {
      queryPassId: number;
      answer: string;
      contextSummary: string | null;
    }) => void;
    const snapshot = new Promise<{
      queryPassId: number;
      answer: string;
      contextSummary: string | null;
    }>((resolve) => { resolveSnapshot = resolve; });
    mocks.invoke
      .mockResolvedValueOnce({ queryPassId: 81, answer: '', contextSummary: null })
      .mockImplementationOnce(() => snapshot);
    await mount();

    await act(async () => {
      mocks.listeners['query-state-changed']?.({
        payload: { queryPassId: 81, state: 'ready', errorCode: null },
      });
      await Promise.resolve();
      mocks.listeners['query-answer-chunk']?.({
        payload: { queryPassId: 81, sequence: 0, text: 'new streamed text', replace: false },
      });
      await Promise.resolve();
    });
    expect(current!.answer).toBe('new streamed text');

    await act(async () => {
      resolveSnapshot({
        queryPassId: 81,
        answer: 'new streamed text plus missing final tail',
        contextSummary: null,
      });
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(current!.answer).toBe('new streamed text plus missing final tail');
  });

  it('does not duplicate a chunk delivered after the complete terminal snapshot', async () => {
    mocks.invoke
      .mockResolvedValueOnce({ queryPassId: 91, answer: 'complete answer', contextSummary: null })
      .mockResolvedValueOnce({ queryPassId: 91, answer: 'complete answer', contextSummary: null });
    await mount();

    await act(async () => {
      mocks.listeners['query-state-changed']?.({
        payload: { queryPassId: 91, state: 'ready', errorCode: null },
      });
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(current!.answer).toBe('complete answer');

    await act(async () => {
      mocks.listeners['query-answer-chunk']?.({
        payload: { queryPassId: 91, sequence: 0, text: 'complete answer', replace: false },
      });
      await Promise.resolve();
    });
    expect(current!.answer).toBe('complete answer');
  });

  it('stays snapshot-only after a running gap when represented chunks arrive later', async () => {
    let resolveRecovery!: (content: {
      queryPassId: number;
      answer: string;
      contextSummary: string | null;
    }) => void;
    const recovery = new Promise<{
      queryPassId: number;
      answer: string;
      contextSummary: string | null;
    }>((resolve) => { resolveRecovery = resolve; });
    mocks.invoke
      .mockResolvedValueOnce({ queryPassId: 101, answer: '', contextSummary: null })
      .mockImplementationOnce(() => recovery)
      .mockResolvedValueOnce({
        queryPassId: 101,
        answer: 'snapshot through two',
        contextSummary: null,
      })
      .mockResolvedValueOnce({
        queryPassId: 101,
        answer: 'snapshot through three',
        contextSummary: null,
      });
    await mount();

    await act(async () => {
      mocks.listeners['query-state-changed']?.({
        payload: { queryPassId: 101, state: 'running', errorCode: null },
      });
      await Promise.resolve();
      mocks.listeners['query-answer-chunk']?.({
        payload: { queryPassId: 101, sequence: 1, text: 'two', replace: false },
      });
      await Promise.resolve();
    });

    await act(async () => {
      resolveRecovery({
        queryPassId: 101,
        answer: 'snapshot through two',
        contextSummary: null,
      });
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(current!.answer).toBe('snapshot through two');

    await act(async () => {
      mocks.listeners['query-answer-chunk']?.({
        payload: { queryPassId: 101, sequence: 2, text: 'two', replace: false },
      });
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(current!.answer).toBe('snapshot through two');

    await act(async () => {
      mocks.listeners['query-answer-chunk']?.({
        payload: { queryPassId: 101, sequence: 3, text: 'three', replace: false },
      });
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(current!.answer).toBe('snapshot through three');
  });

  it('keeps terminal recovery snapshot-only when a gap invalidates the ready refresh', async () => {
    let resolveTerminal!: (content: {
      queryPassId: number;
      answer: string;
      contextSummary: string | null;
    }) => void;
    const terminal = new Promise<{
      queryPassId: number;
      answer: string;
      contextSummary: string | null;
    }>((resolve) => { resolveTerminal = resolve; });
    mocks.invoke
      .mockResolvedValueOnce({ queryPassId: 111, answer: '', contextSummary: null })
      .mockImplementationOnce(() => terminal)
      .mockResolvedValueOnce({
        queryPassId: 111,
        answer: 'complete recovered answer',
        contextSummary: null,
      });
    await mount();

    await act(async () => {
      mocks.listeners['query-state-changed']?.({
        payload: { queryPassId: 111, state: 'ready', errorCode: null },
      });
      await Promise.resolve();
      mocks.listeners['query-answer-chunk']?.({
        payload: { queryPassId: 111, sequence: 1, text: 'represented gap text', replace: false },
      });
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(current!.answer).toBe('complete recovered answer');

    await act(async () => {
      mocks.listeners['query-answer-chunk']?.({
        payload: { queryPassId: 111, sequence: 1, text: 'represented gap text', replace: false },
      });
      mocks.listeners['query-answer-chunk']?.({
        payload: { queryPassId: 111, sequence: 2, text: 'represented tail', replace: false },
      });
      await Promise.resolve();
    });
    expect(current!.answer).toBe('complete recovered answer');

    await act(async () => {
      resolveTerminal({
        queryPassId: 111,
        answer: 'older terminal snapshot',
        contextSummary: null,
      });
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(current!.answer).toBe('complete recovered answer');
  });

  it('replaces listening partials for the active pass and ignores stale ones', async () => {
    await mount();

    await act(async () => {
      mocks.listeners['query-state-changed']?.({
        payload: { queryPassId: 4, state: 'listening', errorCode: null },
      });
      mocks.listeners['query-partial']?.({
        payload: { queryPassId: 4, text: 'what is' },
      });
      await Promise.resolve();
    });
    expect(current!.state).toBe('listening');
    expect(current!.partial).toBe('what is');

    await act(async () => {
      mocks.listeners['query-partial']?.({
        payload: { queryPassId: 4, text: 'what is the weather' },
      });
      mocks.listeners['query-partial']?.({
        payload: { queryPassId: 3, text: 'stale pass' },
      });
      await Promise.resolve();
    });
    expect(current!.partial).toBe('what is the weather');
  });

  it('clears the partial when the query is sent or the popover hides', async () => {
    await mount();

    await act(async () => {
      mocks.listeners['query-state-changed']?.({
        payload: { queryPassId: 8, state: 'listening', errorCode: null },
      });
      mocks.listeners['query-partial']?.({
        payload: { queryPassId: 8, text: 'summarize this' },
      });
      await Promise.resolve();
    });
    expect(current!.partial).toBe('summarize this');

    await act(async () => {
      mocks.listeners['query-state-changed']?.({
        payload: { queryPassId: 8, state: 'transcribing', errorCode: null },
      });
      mocks.listeners['query-partial']?.({
        payload: { queryPassId: 8, text: 'late partial' },
      });
      await Promise.resolve();
    });
    expect(current!.state).toBe('transcribing');
    expect(current!.partial).toBe('');

    await act(async () => {
      mocks.listeners['query-state-changed']?.({
        payload: { queryPassId: 9, state: 'listening', errorCode: null },
      });
      mocks.listeners['query-partial']?.({
        payload: { queryPassId: 9, text: 'next question' },
      });
      mocks.listeners['query-review-hidden']?.({
        payload: { queryPassId: 9 },
      });
      await Promise.resolve();
    });
    expect(current!.state).toBe('idle');
    expect(current!.partial).toBe('');
    expect(current!.answer).toBe('');
  });
});
