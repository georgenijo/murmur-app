import { useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { cancelRecording } from '../dictation';
import type { DictationStatus } from '../types';

interface UseEscapeCancelProps {
  status: DictationStatus;
  enabled: boolean;
}

type ActiveTransformStatus =
  | 'capturing'
  | 'listening'
  | 'thinking';

function isActiveTransformStatus(status: unknown): status is ActiveTransformStatus {
  return status === 'capturing' || status === 'listening' || status === 'thinking';
}

export function useEscapeCancel({ status, enabled }: UseEscapeCancelProps) {
  const statusRef = useRef(status);
  const cancellingRef = useRef(false);
  useEffect(() => { statusRef.current = status; }, [status]);

  useEffect(() => {
    if (!enabled) return;

    let cancelled = false;
    let unlisten: (() => void) | null = null;

    listen('escape-cancel', async () => {
      if (cancelled) return;
      if (cancellingRef.current) return;
      cancellingRef.current = true;
      try {
        let transformStatus: unknown = 'idle';
        try {
          transformStatus = await invoke<unknown>('transform_status');
        } catch (err) {
          // Preserve dictation Escape cancellation if the independent
          // transform-status query is temporarily unavailable.
          console.error('transform_status failed during Escape cancellation:', err);
        }

        if (isActiveTransformStatus(transformStatus)) {
          await invoke('cancel_transform');
          return;
        }

        // Ready/failed both park at ReviewPending and remain owned by the
        // focusable transform-review webview's Escape handler. Applying has no
        // safe keyboard cancellation action. Never double-dispatch either one
        // through the global event path.
        if (transformStatus === 'review_pending' || transformStatus === 'applying') return;

        if (statusRef.current === 'idle') return;
        await cancelRecording();
      } catch (err) {
        console.error('Escape cancellation failed:', err);
      } finally {
        cancellingRef.current = false;
      }
    }).then((fn) => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [enabled]);
}
