import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { flog } from '../log';

export type QueryReviewState =
  | 'idle'
  | 'connecting'
  | 'listening'
  | 'transcribing'
  | 'running'
  | 'ready'
  | 'failed';

interface QueryStatePayload {
  queryPassId: number;
  state: QueryReviewState;
  errorCode: string | null;
}

interface QueryChunkPayload {
  queryPassId: number;
  sequence: number;
  text: string;
}

interface QueryPartialPayload {
  queryPassId: number;
  text: string;
}

interface QueryContent {
  queryPassId: number | null;
  answer: string;
}

function validPassId(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value > 0;
}

function isStatePayload(value: unknown): value is QueryStatePayload {
  if (!value || typeof value !== 'object') return false;
  const payload = value as Record<string, unknown>;
  return validPassId(payload.queryPassId)
    && typeof payload.state === 'string'
    && ['idle', 'connecting', 'listening', 'transcribing', 'running', 'ready', 'failed'].includes(payload.state)
    && (payload.errorCode === null || typeof payload.errorCode === 'string');
}

function isChunkPayload(value: unknown): value is QueryChunkPayload {
  if (!value || typeof value !== 'object') return false;
  const payload = value as Record<string, unknown>;
  return validPassId(payload.queryPassId)
    && typeof payload.sequence === 'number'
    && Number.isSafeInteger(payload.sequence)
    && payload.sequence >= 0
    && typeof payload.text === 'string';
}

function isPartialPayload(value: unknown): value is QueryPartialPayload {
  if (!value || typeof value !== 'object') return false;
  const payload = value as Record<string, unknown>;
  return validPassId(payload.queryPassId) && typeof payload.text === 'string';
}

export function useQueryReviewDriver() {
  const [state, setState] = useState<QueryReviewState>('idle');
  const [errorCode, setErrorCode] = useState<string | null>(null);
  const [answer, setAnswer] = useState('');
  const [partial, setPartial] = useState('');
  const passIdRef = useRef<number | null>(null);
  const stateRef = useRef<QueryReviewState>('idle');
  const nextSequenceRef = useRef(0);

  useEffect(() => {
    let disposed = false;
    let unlistenState: (() => void) | null = null;
    let unlistenChunk: (() => void) | null = null;
    let unlistenPartial: (() => void) | null = null;
    let unlistenHidden: (() => void) | null = null;

    const refresh = async (expectedPassId: number) => {
      try {
        const content = await invoke<QueryContent>('get_query_review_content');
        if (!disposed && content.queryPassId === expectedPassId && typeof content.answer === 'string') {
          setAnswer(content.answer);
        }
      } catch {
        flog.warn('query-review', 'could not refresh answer content');
      }
    };

    const setup = async () => {
      unlistenState = await listen<unknown>('query-state-changed', (event) => {
        if (disposed || !isStatePayload(event.payload)) return;
        const payload = event.payload;
        if (passIdRef.current !== payload.queryPassId) {
          passIdRef.current = payload.queryPassId;
          nextSequenceRef.current = 0;
          setAnswer('');
          setPartial('');
        }
        stateRef.current = payload.state;
        setState(payload.state);
        setErrorCode(payload.errorCode);
        if (payload.state !== 'listening') {
          setPartial('');
        }
        if (payload.state === 'ready' || payload.state === 'failed') {
          void refresh(payload.queryPassId);
        }
      });
      if (disposed) { unlistenState(); return; }

      unlistenChunk = await listen<unknown>('query-answer-chunk', (event) => {
        if (disposed || !isChunkPayload(event.payload)) return;
        const payload = event.payload;
        if (payload.queryPassId !== passIdRef.current) return;
        if (payload.sequence !== nextSequenceRef.current) {
          void refresh(payload.queryPassId);
          nextSequenceRef.current = payload.sequence + 1;
          return;
        }
        nextSequenceRef.current += 1;
        setAnswer((current) => current + payload.text);
      });
      if (disposed) { unlistenState(); unlistenChunk(); return; }

      unlistenPartial = await listen<unknown>('query-partial', (event) => {
        if (disposed || !isPartialPayload(event.payload)) return;
        const payload = event.payload;
        if (payload.queryPassId !== passIdRef.current) return;
        if (stateRef.current !== 'listening') return;
        setPartial(payload.text);
      });
      if (disposed) { unlistenState(); unlistenChunk(); unlistenPartial(); return; }

      unlistenHidden = await listen('query-review-hidden', () => {
        passIdRef.current = null;
        nextSequenceRef.current = 0;
        stateRef.current = 'idle';
        setState('idle');
        setErrorCode(null);
        setAnswer('');
        setPartial('');
      });
      if (disposed) { unlistenState(); unlistenChunk(); unlistenPartial(); unlistenHidden(); }
    };
    void setup();
    return () => {
      disposed = true;
      unlistenState?.();
      unlistenChunk?.();
      unlistenPartial?.();
      unlistenHidden?.();
    };
  }, []);

  const cancel = useCallback(() => {
    const queryPassId = passIdRef.current;
    if (queryPassId === null) return;
    void invoke('cancel_query', { queryPassId }).catch(() => {
      flog.warn('query-review', 'cancel failed', { query_pass_id: queryPassId });
    });
  }, []);

  const copy = useCallback(() => {
    const queryPassId = passIdRef.current;
    if (queryPassId === null) return;
    void invoke('copy_query_answer', { queryPassId }).then(() => {
      setErrorCode(null);
    }).catch(() => {
      setErrorCode('clipboard_unavailable');
      flog.warn('query-review', 'copy failed', { query_pass_id: queryPassId });
    });
  }, []);

  return { state, errorCode, answer, partial, cancel, copy };
}
