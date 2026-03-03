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
  const cancellingRef = useRef(false);
  useEffect(() => { statusRef.current = status; }, [status]);

  useEffect(() => {
    if (!enabled) return;

    let cancelled = false;
    let unlisten: (() => void) | null = null;

    listen('escape-cancel', async () => {
      if (cancelled) return;
      if (statusRef.current === 'idle') return;
      if (cancellingRef.current) return;
      cancellingRef.current = true;
      try {
        await cancelRecording();
      } catch (err) {
        console.error('cancel_native_recording failed:', err);
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
