import { useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { DEFAULT_SETTINGS, type QueryKey } from '../settings';
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

interface UseQueryFlowProps {
  enabled: boolean;
  initialized: boolean;
  accessibilityGranted: boolean | null;
  queryHotkey: QueryKey | null;
  microphone?: string;
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

export function useQueryFlow({
  enabled,
  initialized,
  accessibilityGranted,
  queryHotkey,
  microphone,
  command,
  onQueryCompleted,
}: UseQueryFlowProps) {
  const activePassRef = useRef<number | null>(null);
  const activeProviderRef = useRef(command.provider);
  const completedPassRef = useRef<number | null>(null);
  const commandRef = useRef(command);
  const microphoneRef = useRef(microphone);
  const onQueryCompletedRef = useRef(onQueryCompleted);
  useEffect(() => { commandRef.current = command; }, [command]);
  useEffect(() => { microphoneRef.current = microphone; }, [microphone]);
  useEffect(() => { onQueryCompletedRef.current = onQueryCompleted; }, [onQueryCompleted]);

  useEffect(() => {
    if (!enabled || !initialized || !accessibilityGranted || !queryHotkey) return;
    let disposed = false;
    let unlistenToggle: (() => void) | null = null;
    let unlistenState: (() => void) | null = null;
    let unlistenHidden: (() => void) | null = null;

    const setup = async () => {
      // Install the completion observer before a toggle can start a pass. A
      // synchronous start failure must still be folded into usage exactly
      // once rather than landing in the listener-registration gap.
      unlistenState = await listen<unknown>('query-state-changed', (event) => {
        if (disposed || !isStatePayload(event.payload)) return;
        const payload = event.payload;
        if (payload.queryPassId !== activePassRef.current) return;
        if (payload.state !== 'ready' && payload.state !== 'failed') return;
        if (completedPassRef.current === payload.queryPassId) return;
        completedPassRef.current = payload.queryPassId;
        onQueryCompletedRef.current?.({
          provider: activeProviderRef.current,
          succeeded: payload.state === 'ready',
          errorCode: payload.errorCode,
          usage: isQueryUsage(payload.usage) ? payload.usage : null,
        });
      });
      if (disposed) { unlistenState(); return; }

      unlistenToggle = await listen<unknown>('query-toggle', (event) => {
        if (disposed || !isTogglePayload(event.payload)) return;
        const { queryPassId, action } = event.payload;
        if (action === 'start') {
          const immutableCommand = commandRef.current;
          activePassRef.current = queryPassId;
          activeProviderRef.current = immutableCommand.provider;
          completedPassRef.current = null;
          const selectedMicrophone = microphoneRef.current;
          void invoke('start_query_capture', {
            queryPassId,
            deviceName: selectedMicrophone && selectedMicrophone !== DEFAULT_SETTINGS.microphone
              ? selectedMicrophone
              : null,
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
      if (disposed) { unlistenToggle(); unlistenState(); return; }

      unlistenHidden = await listen('query-review-hidden', () => {
        activePassRef.current = null;
        completedPassRef.current = null;
      });
      if (disposed) {
        unlistenToggle();
        unlistenState();
        unlistenHidden();
        return;
      }

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
      unlistenState?.();
      unlistenHidden?.();
      void invoke('stop_query_listener').catch(() => {});
      const passId = activePassRef.current;
      activePassRef.current = null;
      if (passId !== null) {
        void invoke('cancel_query', { queryPassId: passId }).catch(() => {});
      }
    };
  }, [
    enabled,
    initialized,
    accessibilityGranted,
    queryHotkey,
    command.provider,
    command.executable,
    command.arguments,
    command.timeoutSeconds,
  ]);
}
