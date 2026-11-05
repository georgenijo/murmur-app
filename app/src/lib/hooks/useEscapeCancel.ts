import { useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { cancelRecording } from '../dictation';
import type { DictationStatus } from '../types';

interface UseEscapeCancelProps {
  status: DictationStatus;
  enabled: boolean;
}

interface EscapeCancelPayload {
  queryPassId: number | null;
}

function isEscapeCancelPayload(value: unknown): value is EscapeCancelPayload {
  if (!value || typeof value !== 'object') return false;
  const queryPassId = (value as Record<string, unknown>).queryPassId;
  const validPassId = (passId: unknown) => passId === null || (
      typeof passId === 'number'
      && Number.isSafeInteger(passId)
      && passId > 0
    );
  return validPassId(queryPassId);
}

const MAX_IN_FLIGHT_ESCAPE_TARGETS = 8;
const DICTATION_ESCAPE_TARGET = 'dictation';

export function useEscapeCancel({ status, enabled }: UseEscapeCancelProps) {
  const statusRef = useRef(status);
  useEffect(() => { statusRef.current = status; }, [status]);

  useEffect(() => {
    if (!enabled) return;

    let cancelled = false;
    let unlisten: (() => void) | null = null;
    const cancellingTargets = new Set<string>();

    listen<unknown>('escape-cancel', async (event) => {
      if (cancelled) return;
      if (!isEscapeCancelPayload(event.payload)) return;

      const queryPassId = event.payload.queryPassId ?? null;
      const target = queryPassId !== null
          ? `query:${queryPassId}`
          : DICTATION_ESCAPE_TARGET;
      if (cancellingTargets.has(target)) return;
      if (cancellingTargets.size >= MAX_IN_FLIGHT_ESCAPE_TARGETS) return;
      cancellingTargets.add(target);
      try {
        if (queryPassId !== null) {
          await invoke('cancel_query', { queryPassId });
          return;
        }

        if (statusRef.current === 'idle') return;
        await cancelRecording();
      } catch (err) {
        console.error('Escape cancellation failed:', err);
      } finally {
        cancellingTargets.delete(target);
      }
    }).then((fn) => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    });

    return () => {
      cancelled = true;
      cancellingTargets.clear();
      unlisten?.();
    };
  }, [enabled]);
}
