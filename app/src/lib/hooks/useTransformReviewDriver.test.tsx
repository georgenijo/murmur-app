import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { TransformReviewContent } from '../transformReview';

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

import { useTransformReviewDriver, type ReviewDriverResult } from './useTransformReviewDriver';

const CONTENT: TransformReviewContent = {
  instruction: 'Make it concise',
  original: 'the original selected text',
  proposed: 'concise text',
};

describe('useTransformReviewDriver (real driver)', () => {
  let container: HTMLDivElement;
  let root: Root;
  let current: ReviewDriverResult | null = null;

  beforeEach(() => {
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
    await act(async () => root.unmount());
    container.remove();
  });

  function contentCalls() {
    return mocks.invoke.mock.calls.filter((c) => c[0] === 'get_transform_review_content').length;
  }

  it('pulls review content via command when the state changes to ready', async () => {
    mocks.invoke.mockResolvedValue(CONTENT);

    function Harness() {
      current = useTransformReviewDriver(true);
      return null;
    }

    await act(async () => {
      root.render(<Harness />);
      await Promise.resolve();
      await Promise.resolve();
    });

    // No content is broadcast in the event payload — it must be pulled.
    expect(mocks.listeners['transform-state-changed']).toBeDefined();
    expect(contentCalls()).toBe(0);

    await act(async () => {
      mocks.listeners['transform-state-changed']?.({ payload: { state: 'ready' } });
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(current!.state).toBe('ready');
    // The real (non-mock) driver fetched the text content by command.
    expect(mocks.invoke).toHaveBeenCalledWith('get_transform_review_content');
    expect(current!.content).toEqual(CONTENT);
  });

  it('carries the errorCode from the event on a failed state', async () => {
    mocks.invoke.mockResolvedValue(CONTENT);

    function Harness() {
      current = useTransformReviewDriver(true);
      return null;
    }

    await act(async () => {
      root.render(<Harness />);
      await Promise.resolve();
      await Promise.resolve();
    });

    await act(async () => {
      mocks.listeners['transform-state-changed']?.({
        payload: { state: 'failed', errorCode: 'model_not_downloaded' },
      });
      await Promise.resolve();
    });

    expect(current!.state).toBe('failed');
    expect(current!.errorCode).toBe('model_not_downloaded');
  });

  it('approve/cancel/retry invoke the real transform-flow commands', async () => {
    mocks.invoke.mockResolvedValue(CONTENT);

    function Harness() {
      current = useTransformReviewDriver(true);
      return null;
    }

    await act(async () => {
      root.render(<Harness />);
      await Promise.resolve();
    });

    await act(async () => { current!.approve(); });
    await act(async () => { current!.retry(); });
    await act(async () => { current!.cancel(); });

    const names = mocks.invoke.mock.calls.map((c) => c[0]);
    expect(names).toContain('approve_transform');
    expect(names).toContain('retry_transform_instruction');
    expect(names).toContain('cancel_transform');
  });
});
