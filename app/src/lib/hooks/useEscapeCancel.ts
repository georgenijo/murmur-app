import { useEffect, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';
import { cancelRecording } from '../dictation';
import type { DictationStatus } from '../types';

interface UseEscapeCancelProps {
  status: DictationStatus;
  enabled: boolean;
}

export function useEscapeCancel({ status, enabled }: UseEscapeCancelProps) {
  const statusRef = useRef(status);
  useEffect(() => { statusRef.current = status; }, [status]);

  useEffect(() => {
    if (!enabled) return;

    let cancelled = false;
    let unlisten: (() => void) | null = null;
    let cancellationInFlight = false;

    listen<void>('escape-cancel', async () => {
      if (cancelled || cancellationInFlight || statusRef.current === 'idle') return;
      cancellationInFlight = true;
      try {
        await cancelRecording();
      } catch (err) {
        console.error('Escape cancellation failed:', err);
      } finally {
        cancellationInFlight = false;
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
