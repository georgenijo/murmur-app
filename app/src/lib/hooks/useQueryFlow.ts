import { useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { DEFAULT_SETTINGS, type QueryKey } from '../settings';
import { flog } from '../log';
import type { QueryCommandSnapshot } from '../queryProviders';

interface QueryTogglePayload {
  queryPassId: number;
  action: 'start' | 'stop';
}

interface UseQueryFlowProps {
  enabled: boolean;
  initialized: boolean;
  accessibilityGranted: boolean | null;
  queryHotkey: QueryKey | null;
  microphone?: string;
  command: QueryCommandSnapshot;
}

function isTogglePayload(value: unknown): value is QueryTogglePayload {
  if (!value || typeof value !== 'object') return false;
  const payload = value as Record<string, unknown>;
  return typeof payload.queryPassId === 'number'
    && Number.isSafeInteger(payload.queryPassId)
    && payload.queryPassId > 0
    && (payload.action === 'start' || payload.action === 'stop');
}

export function useQueryFlow({
  enabled,
  initialized,
  accessibilityGranted,
  queryHotkey,
  microphone,
  command,
}: UseQueryFlowProps) {
  const activePassRef = useRef<number | null>(null);
  const commandRef = useRef(command);
  const microphoneRef = useRef(microphone);
  useEffect(() => { commandRef.current = command; }, [command]);
  useEffect(() => { microphoneRef.current = microphone; }, [microphone]);

  useEffect(() => {
    if (!enabled || !initialized || !accessibilityGranted || !queryHotkey) return;
    let disposed = false;
    let unlistenToggle: (() => void) | null = null;
    let unlistenHidden: (() => void) | null = null;

    const setup = async () => {
      unlistenToggle = await listen<unknown>('query-toggle', (event) => {
        if (disposed || !isTogglePayload(event.payload)) return;
        const { queryPassId, action } = event.payload;
        if (action === 'start') {
          activePassRef.current = queryPassId;
          const selectedMicrophone = microphoneRef.current;
          void invoke('start_query_capture', {
            queryPassId,
            deviceName: selectedMicrophone && selectedMicrophone !== DEFAULT_SETTINGS.microphone
              ? selectedMicrophone
              : null,
            command: commandRef.current,
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

      unlistenHidden = await listen('query-review-hidden', () => {
        activePassRef.current = null;
      });
      if (disposed) {
        unlistenToggle();
        unlistenHidden();
        return;
      }

      try {
        await invoke('start_query_listener', { hotkey: queryHotkey });
      } catch {
        flog.warn('query', 'could not start voice-query listener');
      }
    };
    void setup();

    return () => {
      disposed = true;
      unlistenToggle?.();
      unlistenHidden?.();
      void invoke('stop_query_listener').catch(() => {});
      const passId = activePassRef.current;
      activePassRef.current = null;
      if (passId !== null) {
        void invoke('cancel_query', { queryPassId: passId }).catch(() => {});
      }
    };
  }, [enabled, initialized, accessibilityGranted, queryHotkey]);
}
