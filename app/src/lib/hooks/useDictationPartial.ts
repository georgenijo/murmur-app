import { useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import type { DictationStatus } from '../types';

interface DictationPartialPayload {
  recordingId: number;
  text: string;
}

function validPayload(value: unknown): value is DictationPartialPayload {
  const payload = value as Partial<DictationPartialPayload> | null;
  return typeof payload?.recordingId === 'number'
    && Number.isSafeInteger(payload.recordingId)
    && payload.recordingId > 0
    && typeof payload.text === 'string'
    && payload.text.length <= 4096;
}

export function useDictationPartial(status: DictationStatus): string {
  const [partial, setPartial] = useState('');
  const recordingIdRef = useRef(0);
  const statusRef = useRef(status);
  useEffect(() => {
    statusRef.current = status;
    if (status !== 'recording') setPartial('');
  }, [status]);

  useEffect(() => {
    let cancelled = false;
    const unlistens: Array<() => void> = [];
    void listen<unknown>('dictation-generation-started', ({ payload }) => {
      const recordingId = (payload as { recordingId?: unknown } | null)?.recordingId;
      if (typeof recordingId === 'number' && Number.isSafeInteger(recordingId) && recordingId > 0) {
        recordingIdRef.current = recordingId;
        setPartial('');
      }
    }).then((unlisten) => cancelled ? unlisten() : unlistens.push(unlisten)).catch(() => {});
    void listen<unknown>('dictation-partial', ({ payload }) => {
      if (!validPayload(payload)
        || payload.recordingId !== recordingIdRef.current
        || statusRef.current !== 'recording') return;
      setPartial(payload.text.trim());
    }).then((unlisten) => cancelled ? unlisten() : unlistens.push(unlisten)).catch(() => {});
    return () => {
      cancelled = true;
      unlistens.forEach((unlisten) => unlisten());
    };
  }, []);
  return partial;
}
