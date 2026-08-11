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
  transformPassId: number | null;
  queryPassId?: number | null;
}

function isEscapeCancelPayload(value: unknown): value is EscapeCancelPayload {
  if (!value || typeof value !== 'object') return false;
  const transformPassId = (value as Record<string, unknown>).transformPassId;
  const queryPassId = (value as Record<string, unknown>).queryPassId;
  const validPassId = (passId: unknown) => passId === null || (
      typeof passId === 'number'
      && Number.isSafeInteger(passId)
      && passId > 0
    );
  return validPassId(transformPassId) && (queryPassId === undefined || validPassId(queryPassId));
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

      const transformPassId = event.payload.transformPassId;
      const queryPassId = event.payload.queryPassId ?? null;
      const target = transformPassId !== null
        ? `transform:${transformPassId}`
        : queryPassId !== null
          ? `query:${queryPassId}`
          : DICTATION_ESCAPE_TARGET;
      if (cancellingTargets.has(target)) return;
      if (cancellingTargets.size >= MAX_IN_FLIGHT_ESCAPE_TARGETS) return;
      cancellingTargets.add(target);
      try {
        if (transformPassId !== null) {
          await invoke('cancel_transform', { transformPassId });
          return;
        }
        if (queryPassId !== null) {
          await invoke('cancel_query', { queryPassId });
          return;
        }

        // Rust emits a null pass only when physical Escape did not target an
        // active/queued transform. ReviewPending is also globally correlated
        // across the brief transition-before-focus handoff; Applying has no
        // global Escape action. A null target can therefore only fall back to
        // dictation cancellation.
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
