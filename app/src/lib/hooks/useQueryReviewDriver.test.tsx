import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

type Listener = (event: { payload: unknown }) => void;

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(async () => ({ queryPassId: null, answer: '' })),
  listeners: new Map<string, Listener>(),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (event: string, listener: Listener) => {
    mocks.listeners.set(event, listener);
    return () => mocks.listeners.delete(event);
  }),
}));

import { useQueryReviewDriver } from './useQueryReviewDriver';

describe('useQueryReviewDriver', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    mocks.invoke.mockClear();
    mocks.listeners.clear();
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  async function renderDriver() {
    function Harness() {
      const driver = useQueryReviewDriver();
      return (
        <div>
          <span data-testid="state">{driver.state}</span>
          <span data-testid="partial">{driver.partial}</span>
          <span data-testid="answer">{driver.answer}</span>
        </div>
      );
    }
    await act(async () => {
      root.render(<Harness />);
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });
  }

  function text(testId: string): string {
    return container.querySelector(`[data-testid="${testId}"]`)?.textContent ?? '';
  }

  it('replaces listening partials for the active pass and ignores stale ones', async () => {
    await renderDriver();

    await act(async () => {
      mocks.listeners.get('query-state-changed')?.({
        payload: { queryPassId: 4, state: 'listening', errorCode: null },
      });
      mocks.listeners.get('query-partial')?.({
        payload: { queryPassId: 4, text: 'what is' },
      });
      await Promise.resolve();
    });
    expect(text('state')).toBe('listening');
    expect(text('partial')).toBe('what is');

    await act(async () => {
      mocks.listeners.get('query-partial')?.({
        payload: { queryPassId: 4, text: 'what is the weather' },
      });
      mocks.listeners.get('query-partial')?.({
        payload: { queryPassId: 3, text: 'stale pass' },
      });
      await Promise.resolve();
    });
    expect(text('partial')).toBe('what is the weather');
  });

  it('clears the partial when the query is sent or the popover hides', async () => {
    await renderDriver();

    await act(async () => {
      mocks.listeners.get('query-state-changed')?.({
        payload: { queryPassId: 8, state: 'listening', errorCode: null },
      });
      mocks.listeners.get('query-partial')?.({
        payload: { queryPassId: 8, text: 'summarize this' },
      });
      await Promise.resolve();
    });
    expect(text('partial')).toBe('summarize this');

    await act(async () => {
      mocks.listeners.get('query-state-changed')?.({
        payload: { queryPassId: 8, state: 'transcribing', errorCode: null },
      });
      mocks.listeners.get('query-partial')?.({
        payload: { queryPassId: 8, text: 'late partial' },
      });
      await Promise.resolve();
    });
    expect(text('state')).toBe('transcribing');
    expect(text('partial')).toBe('');

    await act(async () => {
      mocks.listeners.get('query-state-changed')?.({
        payload: { queryPassId: 9, state: 'listening', errorCode: null },
      });
      mocks.listeners.get('query-partial')?.({
        payload: { queryPassId: 9, text: 'next question' },
      });
      mocks.listeners.get('query-review-hidden')?.({ payload: undefined });
      await Promise.resolve();
    });
    expect(text('state')).toBe('idle');
    expect(text('partial')).toBe('');
    expect(text('answer')).toBe('');
  });
});
