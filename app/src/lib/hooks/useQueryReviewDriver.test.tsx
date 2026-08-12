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

function content(queryPassId: number, answer: string, detail: string, fix: string) {
  return {
    queryPassId,
    answer,
    errorDetail: detail,
    provider: 'claude',
    signInFix: fix,
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
      .mockResolvedValueOnce(content(62, 'current answer', 'current detail', 'current fix'));
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

    await act(async () => {
      resolvePrior(content(61, 'stale answer', 'stale detail', 'stale fix'));
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(current!.answer).toBe('current answer');
    expect(current!.errorDetail).toBe('current detail');
    expect(current!.signInFix).toBe('current fix');
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
      resolveNewer(content(71, 'complete latest', 'latest detail', 'latest fix'));
      await Promise.resolve();
      await Promise.resolve();
    });
    await act(async () => {
      resolveOlder(content(71, 'older partial', 'older detail', 'older fix'));
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(current!.answer).toBe('complete latest');
    expect(current!.errorDetail).toBe('latest detail');
    expect(current!.signInFix).toBe('latest fix');
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
});
