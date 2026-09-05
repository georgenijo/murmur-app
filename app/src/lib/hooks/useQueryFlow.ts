import { useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { DEFAULT_SETTINGS, type QueryKey, type SmartAutoMicrophoneRequest } from '../settings';
import { validateQueryCommand, type QueryCommandConfig } from '../queryProviders';
import { isQueryUsage } from '../queryUsage';
import type { QueryCompletion } from '../stats';
import { flog } from '../log';
interface QueryTogglePayload {
  queryPassId: number;
  action: 'start' | 'stop';
}

interface QueryStatePayload {
  queryPassId: number;
  state: 'idle' | 'connecting' | 'listening' | 'transcribing' | 'running' | 'ready' | 'failed';
  errorCode: string | null;
  usage?: unknown;
}

interface QueryHiddenPayload {
  queryPassId: number;
}

interface TrackedQueryPass {
  provider: QueryCommandConfig['provider'];
  completed: boolean;
}

interface UseQueryFlowProps {
  enabled: boolean;
  initialized: boolean;
  accessibilityGranted: boolean | null;
  queryHotkey: QueryKey | null;
  microphone?: string;
  smartAuto?: SmartAutoMicrophoneRequest | null;
  automaticallyCopyAnswers: boolean;
  command: QueryCommandConfig;
  onQueryCompleted?: (completion: QueryCompletion) => void;
}

function isTogglePayload(value: unknown): value is QueryTogglePayload {
  if (!value || typeof value !== 'object') return false;
  const payload = value as Record<string, unknown>;
  return typeof payload.queryPassId === 'number'
    && Number.isSafeInteger(payload.queryPassId)
    && payload.queryPassId > 0
    && (payload.action === 'start' || payload.action === 'stop');
}

function isStatePayload(value: unknown): value is QueryStatePayload {
  if (!value || typeof value !== 'object') return false;
  const payload = value as Record<string, unknown>;
  return typeof payload.queryPassId === 'number'
    && Number.isSafeInteger(payload.queryPassId)
    && payload.queryPassId > 0
    && typeof payload.state === 'string'
    && ['idle', 'connecting', 'listening', 'transcribing', 'running', 'ready', 'failed'].includes(payload.state)
    && (payload.errorCode === null || typeof payload.errorCode === 'string');
}

function isHiddenPayload(value: unknown): value is QueryHiddenPayload {
  if (!value || typeof value !== 'object') return false;
  const payload = value as Record<string, unknown>;
  return Object.keys(payload).length === 1
    && typeof payload.queryPassId === 'number'
    && Number.isSafeInteger(payload.queryPassId)
    && payload.queryPassId > 0;
}

export function useQueryFlow({
  enabled,
  initialized,
  accessibilityGranted,
  queryHotkey,
  microphone,
  smartAuto = null,
  automaticallyCopyAnswers,
  command,
  onQueryCompleted,
}: UseQueryFlowProps) {
  const activePassRef = useRef<number | null>(null);
  const trackedPassesRef = useRef(new Map<number, TrackedQueryPass>());
  const commandRef = useRef(command);
  const microphoneRef = useRef(microphone);
  const smartAutoRef = useRef(smartAuto);
  const automaticallyCopyAnswersRef = useRef(automaticallyCopyAnswers);
  const onQueryCompletedRef = useRef(onQueryCompleted);
  const terminalListenersReadyRef = useRef<Promise<void>>(Promise.resolve());
  useEffect(() => { commandRef.current = command; }, [command]);
  useEffect(() => { microphoneRef.current = microphone; }, [microphone]);
  useEffect(() => { smartAutoRef.current = smartAuto; }, [smartAuto]);
  useEffect(() => { automaticallyCopyAnswersRef.current = automaticallyCopyAnswers; }, [automaticallyCopyAnswers]);
  useEffect(() => { onQueryCompletedRef.current = onQueryCompleted; }, [onQueryCompleted]);

  const completeTrackedPass = (
    queryPassId: number,
    completion: Omit<QueryCompletion, 'provider'>,
  ) => {
    const tracked = trackedPassesRef.current.get(queryPassId);
    if (!tracked || tracked.completed) return false;
    tracked.completed = true;
    onQueryCompletedRef.current?.({ provider: tracked.provider, ...completion });
    return true;
  };

  const releaseTrackedPass = (queryPassId: number) => {
    if (activePassRef.current === queryPassId) activePassRef.current = null;
    trackedPassesRef.current.delete(queryPassId);
  };

  // Terminal accounting outlives the native-shortcut lifecycle. Disabling or
  // reconfiguring Voice Query cancels the current Rust pass, whose canonical
  // Ready/Failed/hidden event may arrive after that lifecycle effect cleans
  // up. Keeping these listeners mounted prevents command-response ordering
  // from turning an already-terminal pass into a synthetic cancellation.
  useEffect(() => {
    let disposed = false;
    let unlistenState: (() => void) | null = null;
    let unlistenHidden: (() => void) | null = null;

    terminalListenersReadyRef.current = (async () => {
      unlistenState = await listen<unknown>('query-state-changed', (event) => {
        if (disposed || !isStatePayload(event.payload)) return;
        const payload = event.payload;
        if (payload.state !== 'ready' && payload.state !== 'failed') return;
        const completed = completeTrackedPass(payload.queryPassId, {
          succeeded: payload.state === 'ready',
          errorCode: payload.errorCode,
          usage: isQueryUsage(payload.usage) ? payload.usage : null,
        });
        if (completed && activePassRef.current !== payload.queryPassId) {
          trackedPassesRef.current.delete(payload.queryPassId);
        }
      });
      if (disposed) {
        unlistenState();
        unlistenState = null;
        return;
      }

      unlistenHidden = await listen<unknown>('query-review-hidden', (event) => {
        if (disposed || !isHiddenPayload(event.payload)) return;
        const { queryPassId } = event.payload;
        if (!trackedPassesRef.current.has(queryPassId)) return;
        completeTrackedPass(queryPassId, {
          succeeded: false,
          errorCode: 'cancelled',
          usage: null,
        });
        releaseTrackedPass(queryPassId);
      });
      if (disposed) {
        unlistenHidden();
        unlistenHidden = null;
      }
    })();

    return () => {
      disposed = true;
      unlistenState?.();
      unlistenHidden?.();
      activePassRef.current = null;
      trackedPassesRef.current.clear();
    };
  }, []);

  useEffect(() => {
    if (!enabled || !initialized || !accessibilityGranted || !queryHotkey) return;
    let disposed = false;
    let unlistenToggle: (() => void) | null = null;

    const setup = async () => {
      // Install the completion observer before a toggle can start a pass. A
      // synchronous start failure must still be folded into usage exactly
      // once rather than landing in the listener-registration gap.
      await terminalListenersReadyRef.current;
      if (disposed) return;

      unlistenToggle = await listen<unknown>('query-toggle', (event) => {
        if (disposed || !isTogglePayload(event.payload)) return;
        const { queryPassId, action } = event.payload;
        if (action === 'start') {
          const immutableCommand = commandRef.current;
          for (const [trackedPassId, tracked] of trackedPassesRef.current) {
            if (tracked.completed && trackedPassId !== queryPassId) {
              trackedPassesRef.current.delete(trackedPassId);
            }
          }
          if (trackedPassesRef.current.has(queryPassId)) return;
          activePassRef.current = queryPassId;
          trackedPassesRef.current.set(queryPassId, {
            provider: immutableCommand.provider,
            completed: false,
          });
          const selectedMicrophone = microphoneRef.current;
          void invoke('start_query_capture', {
            queryPassId,
            deviceName: smartAutoRef.current ? null : selectedMicrophone && selectedMicrophone !== DEFAULT_SETTINGS.microphone
              ? selectedMicrophone
              : null,
            ...(smartAutoRef.current ? { smartAuto: smartAutoRef.current } : {}),
            automaticallyCopyAnswer: automaticallyCopyAnswersRef.current,
            command: immutableCommand,
          }).catch(() => {
            flog.warn('query', 'start command failed', { query_pass_id: queryPassId });
            void invoke('cancel_query', { queryPassId }).catch(() => {});
          });
          return;
        }
        if (activePassRef.current !== queryPassId) return;
        void invoke('finish_query_capture', { queryPassId }).catch(() => {
          flog.warn('query', 'finish command failed', { query_pass_id: queryPassId });
          void invoke('cancel_query', { queryPassId }).catch(() => {});
        });
      });
      if (disposed) { unlistenToggle(); return; }

      try {
        // Preflight the exact provider, executable, argv, timeout, and
        // Rust-owned declared environment before arming the global shortcut.
        // A bad configuration therefore cannot wait until the first keypress
        // to fail.
        await validateQueryCommand(commandRef.current);
        if (disposed) return;
        await invoke('start_query_listener', { hotkey: queryHotkey });
      } catch {
        flog.warn('query', 'voice-query preflight or listener setup failed');
      }
    };
    void setup();

    return () => {
      disposed = true;
      unlistenToggle?.();
      void invoke('stop_query_listener').catch(() => {});
      const passId = activePassRef.current;
      if (passId !== null) {
        // Rust owns the terminal outcome. The stable listeners above wait for
        // its pass-correlated Ready/Failed/hidden event even if this command
        // response settles first or rejects.
        void invoke('cancel_query', { queryPassId: passId }).catch(() => {});
      }
    };
  // Command changes are intentionally excluded: the start event snapshots
  // commandRef for the next pass, while an active pass keeps its Rust-owned
  // immutable command and must not be cancelled by a Settings rerender.
  }, [
    enabled,
    initialized,
    accessibilityGranted,
    queryHotkey,
  ]);
}
