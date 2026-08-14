import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

type Listener = (event: { payload: unknown }) => void;

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(async (..._args: unknown[]) => undefined),
  listeners: new Map<string, Listener>(),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (event: string, listener: Listener) => {
    mocks.listeners.set(event, listener);
    return () => mocks.listeners.delete(event);
  }),
}));

import { useQueryFlow } from './useQueryFlow';
import type { QueryCommandConfig } from '../queryProviders';
import type { QueryCompletion } from '../stats';

const DEFAULT_COMMAND: QueryCommandConfig = {
  provider: 'custom',
  executable: '/usr/bin/printf',
  arguments: ['%s'],
  timeoutSeconds: 60,
  contextLevel: 'selection',
  retainQueryHistory: true,
};

function Harness({
  enabled = true,
  command,
  automaticallyCopyAnswers = true,
  onQueryCompleted,
}: {
  enabled?: boolean;
  command: QueryCommandConfig;
  automaticallyCopyAnswers?: boolean;
  onQueryCompleted?: (completion: QueryCompletion) => void;
}) {
  useQueryFlow({
    enabled,
    initialized: true,
    accessibilityGranted: true,
    queryHotkey: 'alt_r',
    microphone: 'system_default',
    automaticallyCopyAnswers,
    command,
    onQueryCompleted,
  });
  return null;
}

describe('useQueryFlow', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    mocks.invoke.mockReset();
    mocks.invoke.mockImplementation(async (..._args: unknown[]) => undefined);
    mocks.listeners.clear();
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  async function renderFlow(
    onQueryCompleted?: (completion: QueryCompletion) => void,
    command: QueryCommandConfig = DEFAULT_COMMAND,
    enabled = true,
    automaticallyCopyAnswers = true,
  ) {
    await act(async () => {
      root.render(
        <Harness
          enabled={enabled}
          command={command}
          automaticallyCopyAnswers={automaticallyCopyAnswers}
          onQueryCompleted={onQueryCompleted}
        />,
      );
      await Promise.resolve();
      await Promise.resolve();
    });
  }

  it('arms the dedicated listener and carries one exact pass through start and stop', async () => {
    await renderFlow();
    expect(mocks.invoke).toHaveBeenCalledWith('validate_query_command', {
      command: {
        provider: 'custom',
        executable: '/usr/bin/printf',
        arguments: ['%s'],
        timeoutSeconds: 60,
        contextLevel: 'selection',
        retainQueryHistory: true,
      },
    });
    expect(mocks.invoke).toHaveBeenCalledWith('start_query_listener', { hotkey: 'alt_r' });

    await act(async () => {
      mocks.listeners.get('query-toggle')?.({ payload: { queryPassId: 17, action: 'start' } });
      await Promise.resolve();
    });
    expect(mocks.invoke).toHaveBeenCalledWith('start_query_capture', {
      queryPassId: 17,
      deviceName: null,
      automaticallyCopyAnswer: true,
      command: {
        provider: 'custom',
        executable: '/usr/bin/printf',
        arguments: ['%s'],
        timeoutSeconds: 60,
        contextLevel: 'selection',
        retainQueryHistory: true,
      },
    });

    await act(async () => {
      mocks.listeners.get('query-toggle')?.({ payload: { queryPassId: 17, action: 'stop' } });
      await Promise.resolve();
    });
    expect(mocks.invoke).toHaveBeenCalledWith('finish_query_capture', { queryPassId: 17 });
  });

  it('ignores malformed and stale stop events', async () => {
    await renderFlow();
    mocks.invoke.mockClear();

    await act(async () => {
      mocks.listeners.get('query-toggle')?.({ payload: { queryPassId: 0, action: 'start' } });
      mocks.listeners.get('query-toggle')?.({ payload: { queryPassId: 9, action: 'stop' } });
      await Promise.resolve();
    });

    expect(mocks.invoke).not.toHaveBeenCalled();
  });

  it('keeps a listening pass alive when context changes and applies it to the next pass', async () => {
    await renderFlow();
    await act(async () => {
      mocks.listeners.get('query-toggle')?.({ payload: { queryPassId: 31, action: 'start' } });
      mocks.listeners.get('query-state-changed')?.({
        payload: { queryPassId: 31, state: 'listening', errorCode: null },
      });
      await Promise.resolve();
    });
    mocks.invoke.mockClear();

    await renderFlow(undefined, { ...DEFAULT_COMMAND, contextLevel: 'none' });

    expect(mocks.invoke).not.toHaveBeenCalledWith('cancel_query', { queryPassId: 31 });
    expect(mocks.invoke).not.toHaveBeenCalledWith('stop_query_listener');

    await act(async () => {
      mocks.listeners.get('query-review-hidden')?.({ payload: { queryPassId: 31 } });
      mocks.listeners.get('query-toggle')?.({ payload: { queryPassId: 32, action: 'start' } });
      await Promise.resolve();
    });
    expect(mocks.invoke).toHaveBeenCalledWith('start_query_capture', {
      queryPassId: 32,
      deviceName: null,
      automaticallyCopyAnswer: true,
      command: { ...DEFAULT_COMMAND, contextLevel: 'none' },
    });
  });

  it('keeps a running pass alive when timeout changes and applies it to the next pass', async () => {
    await renderFlow();
    await act(async () => {
      mocks.listeners.get('query-toggle')?.({ payload: { queryPassId: 41, action: 'start' } });
      mocks.listeners.get('query-state-changed')?.({
        payload: { queryPassId: 41, state: 'running', errorCode: null },
      });
      await Promise.resolve();
    });
    mocks.invoke.mockClear();

    await renderFlow(undefined, { ...DEFAULT_COMMAND, timeoutSeconds: 120 });

    expect(mocks.invoke).not.toHaveBeenCalledWith('cancel_query', { queryPassId: 41 });
    expect(mocks.invoke).not.toHaveBeenCalledWith('stop_query_listener');

    await act(async () => {
      mocks.listeners.get('query-review-hidden')?.({ payload: { queryPassId: 41 } });
      mocks.listeners.get('query-toggle')?.({ payload: { queryPassId: 42, action: 'start' } });
      await Promise.resolve();
    });
    expect(mocks.invoke).toHaveBeenCalledWith('start_query_capture', {
      queryPassId: 42,
      deviceName: null,
      automaticallyCopyAnswer: true,
      command: { ...DEFAULT_COMMAND, timeoutSeconds: 120 },
    });
  });

  it('applies history retention to the next pass without restarting the listener', async () => {
    await renderFlow(undefined, {
      ...DEFAULT_COMMAND,
      contextLevel: 'none',
      retainQueryHistory: false,
    });
    mocks.invoke.mockClear();
    await renderFlow(undefined, {
      ...DEFAULT_COMMAND,
      contextLevel: 'none',
      retainQueryHistory: true,
    });
    expect(mocks.invoke).not.toHaveBeenCalledWith('stop_query_listener');
    expect(mocks.invoke).not.toHaveBeenCalledWith('validate_query_command', expect.anything());

    await act(async () => {
      mocks.listeners.get('query-toggle')?.({ payload: { queryPassId: 19, action: 'start' } });
      await Promise.resolve();
    });
    expect(mocks.invoke).toHaveBeenCalledWith('start_query_capture', expect.objectContaining({
      queryPassId: 19,
      command: expect.objectContaining({ retainQueryHistory: true }),
    }));
  });

  it('snapshots the auto-copy preference for each start without restarting the listener', async () => {
    await renderFlow(undefined, DEFAULT_COMMAND, true, false);
    mocks.invoke.mockClear();

    await act(async () => {
      mocks.listeners.get('query-toggle')?.({ payload: { queryPassId: 71, action: 'start' } });
      await Promise.resolve();
    });
    expect(mocks.invoke).toHaveBeenCalledWith('start_query_capture', expect.objectContaining({
      queryPassId: 71,
      automaticallyCopyAnswer: false,
    }));

    mocks.invoke.mockClear();
    await renderFlow(undefined, DEFAULT_COMMAND, true, true);
    expect(mocks.invoke).not.toHaveBeenCalledWith('stop_query_listener');
    expect(mocks.invoke).not.toHaveBeenCalledWith('cancel_query', { queryPassId: 71 });

    await act(async () => {
      mocks.listeners.get('query-review-hidden')?.({ payload: { queryPassId: 71 } });
      mocks.listeners.get('query-toggle')?.({ payload: { queryPassId: 72, action: 'start' } });
      await Promise.resolve();
    });
    expect(mocks.invoke).toHaveBeenCalledWith('start_query_capture', expect.objectContaining({
      queryPassId: 72,
      automaticallyCopyAnswer: true,
    }));
  });

  it('records one content-free completion for the exact active pass', async () => {
    const onQueryCompleted = vi.fn();
    await renderFlow(onQueryCompleted);

    await act(async () => {
      mocks.listeners.get('query-toggle')?.({
        payload: { queryPassId: 21, action: 'start' },
      });
      const terminal = {
        payload: {
          queryPassId: 21,
          state: 'ready',
          errorCode: null,
          usage: {
            inputTokens: 120,
            outputTokens: 45,
            reasoningOutputTokens: 3,
            cachedInputTokens: 20,
            cacheCreationInputTokens: 4,
            costUsd: 0.012,
          },
        },
      };
      mocks.listeners.get('query-state-changed')?.(terminal);
      mocks.listeners.get('query-state-changed')?.(terminal);
    });

    expect(onQueryCompleted).toHaveBeenCalledTimes(1);
    expect(onQueryCompleted).toHaveBeenCalledWith({
      provider: 'custom',
      succeeded: true,
      errorCode: null,
      usage: {
        inputTokens: 120,
        outputTokens: 45,
        reasoningOutputTokens: 3,
        cachedInputTokens: 20,
        cacheCreationInputTokens: 4,
        costUsd: 0.012,
      },
    });
  });

  it('records an exact cancellation once and ignores malformed or stale hidden events', async () => {
    const onQueryCompleted = vi.fn();
    await renderFlow(onQueryCompleted);

    await act(async () => {
      mocks.listeners.get('query-toggle')?.({
        payload: { queryPassId: 23, action: 'start' },
      });
      mocks.listeners.get('query-review-hidden')?.({ payload: null });
      mocks.listeners.get('query-review-hidden')?.({ payload: { queryPassId: 99 } });
      mocks.listeners.get('query-review-hidden')?.({
        payload: { queryPassId: 23, extra: 'SENTINEL_CONTENT' },
      });
      await Promise.resolve();
    });
    expect(onQueryCompleted).not.toHaveBeenCalled();

    mocks.invoke.mockClear();
    await act(async () => {
      mocks.listeners.get('query-toggle')?.({
        payload: { queryPassId: 23, action: 'stop' },
      });
      await Promise.resolve();
    });
    expect(mocks.invoke).toHaveBeenCalledWith('finish_query_capture', { queryPassId: 23 });

    await act(async () => {
      mocks.listeners.get('query-review-hidden')?.({ payload: { queryPassId: 23 } });
      mocks.listeners.get('query-review-hidden')?.({ payload: { queryPassId: 23 } });
    });
    expect(onQueryCompleted).toHaveBeenCalledTimes(1);
    expect(onQueryCompleted).toHaveBeenCalledWith({
      provider: 'custom',
      succeeded: false,
      errorCode: 'cancelled',
      usage: null,
    });
  });

  it('keeps the start-time provider and records the canonical hidden event after disable', async () => {
    const onQueryCompleted = vi.fn();
    await renderFlow(onQueryCompleted);
    await act(async () => {
      mocks.listeners.get('query-toggle')?.({
        payload: { queryPassId: 51, action: 'start' },
      });
      await Promise.resolve();
    });

    await renderFlow(onQueryCompleted, { ...DEFAULT_COMMAND, provider: 'claude' });
    mocks.invoke.mockClear();
    await renderFlow(onQueryCompleted, { ...DEFAULT_COMMAND, provider: 'claude' }, false);
    expect(mocks.invoke).toHaveBeenCalledWith('cancel_query', { queryPassId: 51 });
    await act(async () => {
      await Promise.resolve();
    });
    expect(onQueryCompleted).not.toHaveBeenCalled();

    await act(async () => {
      mocks.listeners.get('query-review-hidden')?.({ payload: { queryPassId: 51 } });
      mocks.listeners.get('query-review-hidden')?.({ payload: { queryPassId: 51 } });
    });

    expect(onQueryCompleted).toHaveBeenCalledTimes(1);
    expect(onQueryCompleted).toHaveBeenCalledWith({
      provider: 'custom',
      succeeded: false,
      errorCode: 'cancelled',
      usage: null,
    });
  });

  it('lets a Ready event delivered after the disable response remain canonical', async () => {
    const onQueryCompleted = vi.fn();
    await renderFlow(onQueryCompleted);
    await act(async () => {
      mocks.listeners.get('query-toggle')?.({
        payload: { queryPassId: 52, action: 'start' },
      });
      await Promise.resolve();
    });
    const stateListener = mocks.listeners.get('query-state-changed');

    await renderFlow(onQueryCompleted, DEFAULT_COMMAND, false);
    expect(onQueryCompleted).not.toHaveBeenCalled();
    await act(async () => {
      // The cancel command response has already resolved. Cross-channel IPC
      // ordering may deliver the earlier terminal event afterward.
      await Promise.resolve();
      stateListener?.({
        payload: { queryPassId: 52, state: 'ready', errorCode: null, usage: null },
      });
    });

    expect(onQueryCompleted).toHaveBeenCalledTimes(1);
    expect(onQueryCompleted).toHaveBeenCalledWith({
      provider: 'custom',
      succeeded: true,
      errorCode: null,
      usage: null,
    });
  });

  it('records a Failed event delivered after disable even when cancel rejects first', async () => {
    const onQueryCompleted = vi.fn();
    mocks.invoke.mockImplementation(async (command: unknown) => {
      if (command === 'cancel_query') throw new Error('termination not yet confirmed');
    });
    await renderFlow(onQueryCompleted);
    await act(async () => {
      mocks.listeners.get('query-toggle')?.({
        payload: { queryPassId: 53, action: 'start' },
      });
      await Promise.resolve();
    });
    const stateListener = mocks.listeners.get('query-state-changed');

    await renderFlow(onQueryCompleted, DEFAULT_COMMAND, false);
    expect(onQueryCompleted).not.toHaveBeenCalled();
    await act(async () => {
      await Promise.resolve();
      stateListener?.({
        payload: {
          queryPassId: 53,
          state: 'failed',
          errorCode: 'termination_unconfirmed',
          usage: null,
        },
      });
    });

    expect(onQueryCompleted).toHaveBeenCalledTimes(1);
    expect(onQueryCompleted).toHaveBeenCalledWith({
      provider: 'custom',
      succeeded: false,
      errorCode: 'termination_unconfirmed',
      usage: null,
    });
  });

  it('retires a completed predecessor when the next pass starts', async () => {
    const onQueryCompleted = vi.fn();
    await renderFlow(onQueryCompleted);
    await act(async () => {
      mocks.listeners.get('query-toggle')?.({
        payload: { queryPassId: 61, action: 'start' },
      });
      mocks.listeners.get('query-state-changed')?.({
        payload: { queryPassId: 61, state: 'ready', errorCode: null, usage: null },
      });
      mocks.listeners.get('query-toggle')?.({
        payload: { queryPassId: 62, action: 'start' },
      });
      // P1 was retired when P2 became active; a delayed hidden event is inert.
      mocks.listeners.get('query-review-hidden')?.({ payload: { queryPassId: 61 } });
      mocks.listeners.get('query-review-hidden')?.({ payload: { queryPassId: 62 } });
    });

    expect(onQueryCompleted).toHaveBeenCalledTimes(2);
    expect(onQueryCompleted).toHaveBeenNthCalledWith(1, {
      provider: 'custom',
      succeeded: true,
      errorCode: null,
      usage: null,
    });
    expect(onQueryCompleted).toHaveBeenNthCalledWith(2, {
      provider: 'custom',
      succeeded: false,
      errorCode: 'cancelled',
      usage: null,
    });
  });

  it('drops malformed usage content and ignores terminal events for another pass', async () => {
    const onQueryCompleted = vi.fn();
    await renderFlow(onQueryCompleted);
    await act(async () => {
      mocks.listeners.get('query-toggle')?.({
        payload: { queryPassId: 22, action: 'start' },
      });
      mocks.listeners.get('query-state-changed')?.({
        payload: {
          queryPassId: 22,
          state: 'ready',
          errorCode: null,
          usage: { inputTokens: 'private content' },
        },
      });
      mocks.listeners.get('query-state-changed')?.({
        payload: { queryPassId: 99, state: 'failed', errorCode: 'timed_out', usage: null },
      });
      mocks.listeners.get('query-state-changed')?.({
        payload: { queryPassId: 22, state: 'failed', errorCode: 'timed_out', usage: null },
      });
    });

    expect(onQueryCompleted).toHaveBeenCalledTimes(1);
    expect(onQueryCompleted).toHaveBeenCalledWith({
      provider: 'custom',
      succeeded: true,
      errorCode: null,
      usage: null,
    });
  });
});
