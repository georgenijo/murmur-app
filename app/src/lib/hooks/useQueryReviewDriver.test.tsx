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
  detail: string,
  fix: string,
  usage: unknown = null,
) {
  return {
    queryPassId,
    answer,
    errorDetail: detail,
    provider: 'claude',
    usage,
    signInFix: fix,
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

  it('does not let a delayed prior-pass snapshot overwrite any active-pass field', async () => {
    let resolvePrior!: (value: ReturnType<typeof content>) => void;
    const prior = new Promise<ReturnType<typeof content>>((resolve) => { resolvePrior = resolve; });
    mocks.invoke
      .mockImplementationOnce(() => prior)
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

    await act(async () => {
      resolvePrior(content(61, 'stale answer', 'stale detail', 'stale fix', usage(61)));
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(current!.answer).toBe('current answer');
    expect(current!.errorDetail).toBe('current detail');
    expect(current!.signInFix).toBe('current fix');
    expect(current!.usage?.inputTokens).toBe(62);
  });

  it('keeps only the newest same-pass recovery snapshot across a replace gap race', async () => {
    let resolveOlder!: (value: ReturnType<typeof content>) => void;
    let resolveNewer!: (value: ReturnType<typeof content>) => void;
    const older = new Promise<ReturnType<typeof content>>((resolve) => { resolveOlder = resolve; });
    const newer = new Promise<ReturnType<typeof content>>((resolve) => { resolveNewer = resolve; });
    mocks.invoke.mockImplementationOnce(() => older).mockImplementationOnce(() => newer);
    await mount();
    await act(async () => {
      mocks.listeners['query-state-changed']?.({
        payload: { queryPassId: 71, state: 'running', errorCode: null },
      });
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
    mocks.invoke.mockImplementationOnce(() => terminal);
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
      resolveTerminal(content(81, 'streamed prefix plus tail', '', ''));
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
    mocks.invoke.mockResolvedValueOnce({
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
});
