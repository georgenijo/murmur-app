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

export interface QuerySignIn {
  provider: string;
  hint: string;
}

interface QueryContent {
  queryPassId: number | null;
  answer: string;
  context: QueryContextDisplay | null;
  /** Bounded stderr tail from the failed run, requester-gated to this window. */
  errorDetail: string | null;
  signIn: QuerySignIn | null;
}

export interface QueryContextDisplay {
  status: 'included' | 'excluded' | 'unavailable';
  appName: string | null;
  windowTitle: string | null;
  selectionBytes: number | null;
  selectionTruncated: boolean;
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

export function useQueryReviewDriver() {
  const [state, setState] = useState<QueryReviewState>('idle');
  const [errorCode, setErrorCode] = useState<string | null>(null);
  const [answer, setAnswer] = useState('');
  const [context, setContext] = useState<QueryContextDisplay | null>(null);
  const [errorDetail, setErrorDetail] = useState<string | null>(null);
  const [signIn, setSignIn] = useState<QuerySignIn | null>(null);
  const passIdRef = useRef<number | null>(null);
  const nextSequenceRef = useRef(0);

  useEffect(() => {
    let disposed = false;
    let unlistenState: (() => void) | null = null;
    let unlistenChunk: (() => void) | null = null;
    let unlistenContext: (() => void) | null = null;
    let unlistenHidden: (() => void) | null = null;

    const refresh = async (expectedPassId: number) => {
      try {
        const content = await invoke<QueryContent>('get_query_review_content');
        if (disposed || content.queryPassId !== expectedPassId) return;
        if (typeof content.answer === 'string') {
          setAnswer(content.answer);
          setContext(content.context ?? null);
        }
        setErrorDetail(typeof content.errorDetail === 'string' && content.errorDetail ? content.errorDetail : null);
        setSignIn(
          content.signIn && typeof content.signIn.provider === 'string' && typeof content.signIn.hint === 'string'
            ? content.signIn
            : null,
        );
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
          setContext(null);
          setErrorDetail(null);
          setSignIn(null);
        }
        setState(payload.state);
        setErrorCode(payload.errorCode);
        void refresh(payload.queryPassId);
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

      unlistenContext = await listen<unknown>('query-context-changed', (event) => {
        if (disposed || !event.payload || typeof event.payload !== 'object') return;
        const queryPassId = (event.payload as Record<string, unknown>).queryPassId;
        if (!validPassId(queryPassId) || queryPassId !== passIdRef.current) return;
        void refresh(queryPassId);
      });
      if (disposed) { unlistenState(); unlistenChunk(); unlistenContext(); return; }

      unlistenHidden = await listen('query-review-hidden', () => {
        passIdRef.current = null;
        nextSequenceRef.current = 0;
        setState('idle');
        setErrorCode(null);
        setAnswer('');
        setContext(null);
        setErrorDetail(null);
        setSignIn(null);
      });
      if (disposed) { unlistenState(); unlistenChunk(); unlistenContext(); unlistenHidden(); }
    };
    void setup();
    return () => {
      disposed = true;
      unlistenState?.();
      unlistenChunk?.();
      unlistenContext?.();
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

  /**
   * Hand the pass off to the provider's own login. Murmur never sees the
   * credential — the vendor CLI opens in Terminal and the user finishes there.
   */
  const startSignIn = useCallback(() => {
    const queryPassId = passIdRef.current;
    if (queryPassId === null) return;
    void invoke('launch_query_pass_login', { queryPassId }).catch(() => {
      flog.warn('query-review', 'sign-in launch failed', { query_pass_id: queryPassId });
    });
  }, []);

  return { state, errorCode, answer, context, errorDetail, signIn, cancel, copy, startSignIn };
}
