import { act, StrictMode, useLayoutEffect } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

type Listener = (event: { payload: unknown }) => void;

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(async (..._args: unknown[]) => undefined),
  listeners: new Map<string, Listener>(),
  subscriptions: new Map<string, Map<number, Listener>>(),
  nextSubscriptionId: 0,
  listenFailures: new Map<string, number>(),
  listenWaiters: new Map<string, Promise<void>[]>(),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
vi.mock('@tauri-apps/api/event', () => ({
  emitTo: vi.fn(async () => undefined),
  listen: vi.fn(async (event: string, listener: Listener) => {
    const waiter = mocks.listenWaiters.get(event)?.shift();
    if (waiter) await waiter;
    const failures = mocks.listenFailures.get(event) ?? 0;
    if (failures > 0) {
      mocks.listenFailures.set(event, failures - 1);
      throw new Error(`could not listen for ${event}`);
    }
    const subscriptionId = ++mocks.nextSubscriptionId;
    const subscriptions = mocks.subscriptions.get(event) ?? new Map<number, Listener>();
    subscriptions.set(subscriptionId, listener);
    mocks.subscriptions.set(event, subscriptions);
    mocks.listeners.set(event, listener);
    return () => {
      subscriptions.delete(subscriptionId);
      if (subscriptions.size === 0) {
        mocks.subscriptions.delete(event);
        mocks.listeners.delete(event);
        return;
      }
      const remaining = [...subscriptions.entries()].sort(([left], [right]) => right - left)[0];
      if (remaining) mocks.listeners.set(event, remaining[1]);
    };
  }),
}));

import { useQueryFlow, type QuerySetupStatus } from './useQueryFlow';
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
  onSetupStatusChange,
  startPassOnLayout,
}: {
  enabled?: boolean;
  command: QueryCommandConfig;
  automaticallyCopyAnswers?: boolean;
  onQueryCompleted?: (completion: QueryCompletion) => void;
  onSetupStatusChange?: (status: QuerySetupStatus) => void;
  startPassOnLayout?: number;
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
    onSetupStatusChange,
  });
  useLayoutEffect(() => {
    if (startPassOnLayout === undefined) return;
    mocks.listeners.get('query-toggle')?.({
      payload: { queryPassId: startPassOnLayout, action: 'start' },
    });
  }, [startPassOnLayout]);
  return null;
}

describe('useQueryFlow', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    mocks.invoke.mockReset();
    mocks.invoke.mockImplementation(async (..._args: unknown[]) => undefined);
    mocks.listeners.clear();
    mocks.subscriptions.clear();
    mocks.nextSubscriptionId = 0;
    mocks.listenFailures.clear();
    mocks.listenWaiters.clear();
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

  it('starts a follow-up with a fresh ID and the current immutable settings', async () => {
    const completed = vi.fn();
    await renderFlow(completed);
    const command = { ...DEFAULT_COMMAND, provider: 'grok' as const, arguments: ['--new-preset'], retainQueryHistory: false };
    await renderFlow(completed, command, true, false);
    mocks.invoke.mockImplementation(async (name: unknown) => name === 'allocate_query_follow_up' ? 72 as unknown as undefined : undefined);
    await act(async () => {
      mocks.listeners.get('query-toggle')?.({ payload: { queryPassId: 71, action: 'follow_up' } });
    });
    expect(mocks.invoke).toHaveBeenCalledWith('allocate_query_follow_up', { queryPassId: 71 });
    expect(mocks.invoke).toHaveBeenCalledWith('start_query_capture', {
      queryPassId: 72, deviceName: null, automaticallyCopyAnswer: false, command,
    });
    await act(async () => {
      mocks.listeners.get('query-toggle')?.({ payload: { queryPassId: 72, action: 'stop' } });
      mocks.listeners.get('query-state-changed')?.({ payload: { queryPassId: 72, state: 'ready', errorCode: null, usage: null } });
    });
    expect(mocks.invoke).toHaveBeenCalledWith('finish_query_capture', { queryPassId: 72 });
    expect(completed).toHaveBeenCalledExactlyOnceWith({ provider: 'grok', succeeded: true, errorCode: null, usage: null });
  });

  it('cancels the exact follow-up reservation when disabling during allocation', async () => {
    await renderFlow();
    let resolveAllocation!: (value: unknown) => void;
    mocks.invoke.mockImplementation((name: unknown) => name === 'allocate_query_follow_up'
      ? new Promise<undefined>((resolve) => { resolveAllocation = resolve as (value: unknown) => void; }) : Promise.resolve(undefined));
    await act(async () => {
      mocks.listeners.get('query-toggle')?.({ payload: { queryPassId: 81, action: 'follow_up' } });
    });
    await renderFlow(undefined, DEFAULT_COMMAND, false);
    await act(async () => { resolveAllocation(82); });
    expect(mocks.invoke).toHaveBeenCalledWith('cancel_query', { queryPassId: 82 });
    expect(mocks.invoke.mock.calls.some(([name]) => name === 'start_query_capture')).toBe(false);
  });

  for (const interruption of ['stop', 'hidden']) {
    it(`does not start capture when ${interruption} beats the follow-up allocation response`, async () => {
      await renderFlow();
      let resolveAllocation!: (value: unknown) => void;
      mocks.invoke.mockImplementation((name: unknown) => name === 'allocate_query_follow_up'
        ? new Promise<undefined>((resolve) => { resolveAllocation = resolve as (value: unknown) => void; }) : Promise.resolve(undefined));
      await act(async () => {
        mocks.listeners.get('query-toggle')?.({ payload: { queryPassId: 81, action: 'follow_up' } });
        if (interruption === 'stop') {
          mocks.listeners.get('query-toggle')?.({ payload: { queryPassId: 82, action: 'stop' } });
        } else {
          mocks.listeners.get('query-review-hidden')?.({ payload: { queryPassId: 82 } });
        }
        resolveAllocation(82);
      });
      expect(mocks.invoke).toHaveBeenCalledWith('cancel_query', { queryPassId: 82 });
      expect(mocks.invoke.mock.calls.some(([name]) => name === 'start_query_capture')).toBe(false);
    });
  }

  it('keeps the accepted follow-up settings frozen through an allocation await', async () => {
    await renderFlow();
    let resolveAllocation!: (value: unknown) => void;
    mocks.invoke.mockImplementation((name: unknown) => name === 'allocate_query_follow_up'
      ? new Promise<undefined>((resolve) => { resolveAllocation = resolve as (value: unknown) => void; }) : Promise.resolve(undefined));
    await act(async () => {
      mocks.listeners.get('query-toggle')?.({ payload: { queryPassId: 91, action: 'follow_up' } });
    });
    await renderFlow(undefined, { ...DEFAULT_COMMAND, arguments: ['changed-after-start'] }, true, false);
    await act(async () => { resolveAllocation(92); });
    expect(mocks.invoke).toHaveBeenCalledWith('start_query_capture', {
      queryPassId: 92, deviceName: null, automaticallyCopyAnswer: true, command: DEFAULT_COMMAND,
    });
  });

  it('does not capture after a stale or dismissed follow-up allocation fails', async () => {
    await renderFlow();
    mocks.invoke.mockImplementation(async (name: unknown) => {
      if (name === 'allocate_query_follow_up') throw new Error('stale');
      return undefined;
    });
    await act(async () => {
      mocks.listeners.get('query-toggle')?.({ payload: { queryPassId: 101, action: 'follow_up' } });
    });
    expect(mocks.invoke.mock.calls.some(([name]) => name === 'start_query_capture')).toBe(false);
    expect(mocks.invoke.mock.calls.some(([name]) => name === 'cancel_query')).toBe(false);
  });

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

  it('uses the newly committed provider when a start arrives before passive effects', async () => {
    const claudeCommand: QueryCommandConfig = {
      ...DEFAULT_COMMAND,
      provider: 'claude',
      executable: '/usr/local/bin/claude',
      arguments: ['--print'],
    };
    const codexCommand: QueryCommandConfig = {
      ...DEFAULT_COMMAND,
      provider: 'codex',
      executable: '/opt/homebrew/bin/codex',
      arguments: ['exec', '--json'],
    };
    await renderFlow(undefined, claudeCommand);
    mocks.invoke.mockClear();

    await act(async () => {
      root.render(
        <Harness
          command={codexCommand}
          startPassOnLayout={61}
        />,
      );
      await Promise.resolve();
    });

    expect(mocks.invoke).toHaveBeenCalledWith('start_query_capture', expect.objectContaining({
      queryPassId: 61,
      command: codexCommand,
    }));
  });

  it('ignores a queued start after Voice Query is synchronously disabled', async () => {
    await renderFlow();
    mocks.invoke.mockClear();

    await act(async () => {
      root.render(
        <Harness
          enabled={false}
          command={DEFAULT_COMMAND}
          startPassOnLayout={62}
        />,
      );
      await Promise.resolve();
    });

    expect(mocks.invoke).not.toHaveBeenCalledWith(
      'start_query_capture',
      expect.objectContaining({ queryPassId: 62 }),
    );
  });

  it('reports a saved-command failure and becomes ready after an explicit retry', async () => {
    const setupStatuses: QuerySetupStatus[] = [];
    let validationAttempt = 0;
    mocks.invoke.mockImplementation(async (command: unknown) => {
      if (command === 'validate_query_command' && validationAttempt++ === 0) {
        throw new Error('invalid_executable');
      }
    });

    await act(async () => {
      root.render(
        <Harness
          command={DEFAULT_COMMAND}
          onSetupStatusChange={(status) => setupStatuses.push(status)}
        />,
      );
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(setupStatuses).toEqual([expect.objectContaining({
      state: 'failed',
      phase: 'command_validation',
    })]);
    expect(setupStatuses[0]?.state === 'failed' ? setupStatuses[0].message : '').toContain(
      'Review the provider, choose Test, then enable it again.',
    );
    expect(mocks.invoke).not.toHaveBeenCalledWith('start_query_listener', expect.anything());
    expect(mocks.listeners.has('query-toggle')).toBe(false);

    await act(async () => {
      root.render(
        <Harness
          enabled={false}
          command={DEFAULT_COMMAND}
          onSetupStatusChange={(status) => setupStatuses.push(status)}
        />,
      );
      await Promise.resolve();
    });
    await act(async () => {
      root.render(
        <Harness
          command={DEFAULT_COMMAND}
          onSetupStatusChange={(status) => setupStatuses.push(status)}
        />,
      );
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(setupStatuses[setupStatuses.length - 1]).toEqual({ state: 'ready' });
    expect(mocks.invoke).toHaveBeenCalledWith('start_query_listener', { hotkey: 'alt_r' });
  });

  it('reports a shortcut registration failure and removes its event callback', async () => {
    const setupStatuses: QuerySetupStatus[] = [];
    mocks.invoke.mockImplementation(async (command: unknown) => {
      if (command === 'start_query_listener') throw new Error('listener unavailable');
    });

    await act(async () => {
      root.render(
        <Harness
          command={DEFAULT_COMMAND}
          onSetupStatusChange={(status) => setupStatuses.push(status)}
        />,
      );
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(setupStatuses).toEqual([expect.objectContaining({
      state: 'failed',
      phase: 'listener_start',
    })]);
    expect(mocks.listeners.has('query-toggle')).toBe(false);
  });

  it.each(['query-state-changed', 'query-toggle'])(
    'recovers when %s listener registration succeeds on retry',
    async (eventName) => {
      const setupStatuses: QuerySetupStatus[] = [];
      mocks.listenFailures.set(eventName, 1);

      await act(async () => {
        root.render(
          <Harness
            command={DEFAULT_COMMAND}
            onSetupStatusChange={(status) => setupStatuses.push(status)}
          />,
        );
        await Promise.resolve();
        await Promise.resolve();
      });
      expect(setupStatuses).toEqual([expect.objectContaining({
        state: 'failed',
        phase: 'listener_start',
      })]);

      await act(async () => {
        root.render(
          <Harness
            enabled={false}
            command={DEFAULT_COMMAND}
            onSetupStatusChange={(status) => setupStatuses.push(status)}
          />,
        );
        await Promise.resolve();
      });
      await act(async () => {
        root.render(
          <Harness
            command={DEFAULT_COMMAND}
            onSetupStatusChange={(status) => setupStatuses.push(status)}
          />,
        );
        await Promise.resolve();
        await Promise.resolve();
      });

      expect(setupStatuses[setupStatuses.length - 1]).toEqual({ state: 'ready' });
      expect(mocks.listeners.has('query-state-changed')).toBe(true);
      expect(mocks.listeners.has('query-review-hidden')).toBe(true);
      expect(mocks.listeners.has('query-toggle')).toBe(true);
    },
  );

  it('keeps one terminal subscription when StrictMode remounts during registration', async () => {
    let releaseFirstRegistration!: () => void;
    const firstRegistration = new Promise<void>((resolve) => {
      releaseFirstRegistration = resolve;
    });
    mocks.listenWaiters.set('query-state-changed', [firstRegistration]);

    await act(async () => {
      root.render(
        <StrictMode>
          <Harness command={DEFAULT_COMMAND} />
        </StrictMode>,
      );
      await Promise.resolve();
      await Promise.resolve();
    });
    await act(async () => {
      releaseFirstRegistration();
      await firstRegistration;
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(mocks.subscriptions.get('query-state-changed')?.size).toBe(1);
    expect(mocks.subscriptions.get('query-review-hidden')?.size).toBe(1);
    expect(mocks.subscriptions.get('query-toggle')?.size).toBe(1);

    mocks.invoke.mockClear();
    await act(async () => {
      mocks.listeners.get('query-toggle')?.({
        payload: { queryPassId: 73, action: 'start' },
      });
      await Promise.resolve();
    });
    expect(mocks.invoke).toHaveBeenCalledWith(
      'start_query_capture',
      expect.objectContaining({ queryPassId: 73 }),
    );
  });

  it('ignores an old validation failure after a newer provider replaces it', async () => {
    let rejectClaude!: (error: Error) => void;
    const claudeValidation = new Promise<undefined>((_resolve, reject) => { rejectClaude = reject; });
    const setupStatuses: QuerySetupStatus[] = [];
    mocks.invoke.mockImplementation(async (...args: unknown[]) => {
      const [invokedCommand, payload] = args;
      const command = payload && typeof payload === 'object' && 'command' in payload
        ? payload.command
        : null;
      const provider = command && typeof command === 'object' && 'provider' in command
        ? command.provider
        : null;
      if (invokedCommand === 'validate_query_command' && provider === 'claude') {
        return claudeValidation;
      }
    });
    const claudeCommand: QueryCommandConfig = {
      ...DEFAULT_COMMAND,
      provider: 'claude',
      executable: '/usr/local/bin/claude',
      arguments: ['--print'],
    };
    const codexCommand: QueryCommandConfig = {
      ...DEFAULT_COMMAND,
      provider: 'codex',
      executable: '/opt/homebrew/bin/codex',
      arguments: ['exec', '--json'],
    };

    await act(async () => {
      root.render(
        <Harness
          command={claudeCommand}
          onSetupStatusChange={(status) => setupStatuses.push(status)}
        />,
      );
      await Promise.resolve();
    });
    await act(async () => {
      root.render(
        <Harness
          enabled={false}
          command={codexCommand}
          onSetupStatusChange={(status) => setupStatuses.push(status)}
        />,
      );
      await Promise.resolve();
    });
    await act(async () => {
      rejectClaude(new Error('invalid_executable'));
      await claudeValidation.catch(() => {});
      await Promise.resolve();
    });
    await act(async () => {
      root.render(
        <Harness
          command={codexCommand}
          onSetupStatusChange={(status) => setupStatuses.push(status)}
        />,
      );
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(setupStatuses).toEqual([{ state: 'ready' }]);
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
