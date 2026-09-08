import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { isDictationStatus, type DictationStatus } from '../types';
import { flog } from '../log';

export function useOverlayRecordingStatus() {
  const [status, setStatus] = useState<DictationStatus>('idle');
  const statusRef = useRef<DictationStatus>('idle');

  useEffect(() => {
    let disposed = false;
    let revision = 0;
    let unlisten: (() => void) | undefined;
    const applyStatus = (next: DictationStatus) => {
      statusRef.current = next;
      setStatus(next);
      flog.info('overlay', 'status changed', { status: next });
    };

    void listen<unknown>('recording-status-changed', ({ payload }) => {
      if (disposed || !isDictationStatus(payload)) return;
      revision += 1;
      applyStatus(payload);
    }).then(async (stopListening) => {
      if (disposed) {
        stopListening();
        return;
      }
      unlisten = stopListening;
      // Subscribe before reading so reload cannot miss a transition. Events
      // received during the read supersede its potentially older snapshot.
      const snapshotRevision = revision;
      const snapshot = await invoke<unknown>('get_status');
      if (disposed || revision !== snapshotRevision) return;
      if (typeof snapshot !== 'object' || snapshot === null
        || !('state' in snapshot) || !isDictationStatus(snapshot.state)) {
        throw new Error('Invalid recording status snapshot');
      }
      applyStatus(snapshot.state);
    }).catch((error: unknown) => {
      if (!disposed) {
        flog.warn('overlay', 'recording status synchronization failed', { error: String(error) });
      }
    });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  return { status, statusRef };
}
