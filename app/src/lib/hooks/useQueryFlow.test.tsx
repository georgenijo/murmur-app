import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

type Listener = (event: { payload: unknown }) => void;

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(async () => undefined),
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
};

function Harness({
  command,
  onQueryCompleted,
}: {
  command: QueryCommandConfig;
  onQueryCompleted?: (completion: QueryCompletion) => void;
}) {
  useQueryFlow({
    enabled: true,
    initialized: true,
    accessibilityGranted: true,
    queryHotkey: 'alt_r',
    microphone: 'system_default',
    command,
    onQueryCompleted,
  });
  return null;
}

describe('useQueryFlow', () => {
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

  async function renderFlow(
    onQueryCompleted?: (completion: QueryCompletion) => void,
    command: QueryCommandConfig = DEFAULT_COMMAND,
  ) {
    await act(async () => {
      root.render(<Harness command={command} onQueryCompleted={onQueryCompleted} />);
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
      command: {
        provider: 'custom',
        executable: '/usr/bin/printf',
        arguments: ['%s'],
        timeoutSeconds: 60,
        contextLevel: 'selection',
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
      mocks.listeners.get('query-review-hidden')?.({ payload: null });
      mocks.listeners.get('query-toggle')?.({ payload: { queryPassId: 32, action: 'start' } });
      await Promise.resolve();
    });
    expect(mocks.invoke).toHaveBeenCalledWith('start_query_capture', {
      queryPassId: 32,
      deviceName: null,
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
      mocks.listeners.get('query-review-hidden')?.({ payload: null });
      mocks.listeners.get('query-toggle')?.({ payload: { queryPassId: 42, action: 'start' } });
      await Promise.resolve();
    });
    expect(mocks.invoke).toHaveBeenCalledWith('start_query_capture', {
      queryPassId: 42,
      deviceName: null,
      command: { ...DEFAULT_COMMAND, timeoutSeconds: 120 },
    });
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
