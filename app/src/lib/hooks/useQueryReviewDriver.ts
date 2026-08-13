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

interface QueryContent {
  queryPassId: number | null;
  answer: string;
  errorDetail: string | null;
  provider: 'claude' | 'codex' | 'grok' | 'cursor' | 'custom' | null;
  signInFix: string | null;
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
  const [errorDetail, setErrorDetail] = useState<string | null>(null);
  const [signInFix, setSignInFix] = useState<string | null>(null);
  const [signInStatus, setSignInStatus] = useState<string | null>(null);
  const [signInBusy, setSignInBusy] = useState(false);
  const passIdRef = useRef<number | null>(null);
  const nextSequenceRef = useRef(0);
  const contentRefreshTicketRef = useRef(0);
  const answerRecoveryRef = useRef(false);
  const terminalPassIdRef = useRef<number | null>(null);
  const terminalContentSnapshotRef = useRef(false);
  const signInAttemptRef = useRef(0);

  useEffect(() => {
    let disposed = false;
    let unlistenState: (() => void) | null = null;
    let unlistenChunk: (() => void) | null = null;
    let unlistenHidden: (() => void) | null = null;

    const refresh = async (expectedPassId: number, terminal = false) => {
      const ticket = contentRefreshTicketRef.current + 1;
      contentRefreshTicketRef.current = ticket;
      try {
        const content = await invoke<QueryContent>('get_query_review_content');
        if (
          !disposed
          && passIdRef.current === expectedPassId
          && content.queryPassId === expectedPassId
          && contentRefreshTicketRef.current === ticket
          && typeof content.answer === 'string'
        ) {
          setAnswer(content.answer);
          setErrorDetail(typeof content.errorDetail === 'string' ? content.errorDetail : null);
          setSignInFix(typeof content.signInFix === 'string' ? content.signInFix : null);
          if (terminal || terminalPassIdRef.current === expectedPassId) {
            terminalContentSnapshotRef.current = true;
          }
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
          contentRefreshTicketRef.current += 1;
          answerRecoveryRef.current = false;
          terminalPassIdRef.current = null;
          terminalContentSnapshotRef.current = false;
          setAnswer('');
          setErrorDetail(null);
          setSignInFix(null);
          setSignInStatus(null);
          setSignInBusy(false);
          signInAttemptRef.current += 1;
        }
        setState(payload.state);
        setErrorCode(payload.errorCode);
        if (payload.state === 'ready' || payload.state === 'failed') {
          terminalPassIdRef.current = payload.queryPassId;
          void refresh(payload.queryPassId, true);
        }
      });
      if (disposed) { unlistenState(); return; }

      unlistenChunk = await listen<unknown>('query-answer-chunk', (event) => {
        if (disposed || !isChunkPayload(event.payload)) return;
        const payload = event.payload;
        if (payload.queryPassId !== passIdRef.current) return;
        if (terminalContentSnapshotRef.current) return;
        if (answerRecoveryRef.current) {
          nextSequenceRef.current = Math.max(nextSequenceRef.current, payload.sequence + 1);
          void refresh(
            payload.queryPassId,
            terminalPassIdRef.current === payload.queryPassId,
          );
          return;
        }
        if (payload.sequence !== nextSequenceRef.current) {
          answerRecoveryRef.current = true;
          nextSequenceRef.current = payload.sequence + 1;
          void refresh(
            payload.queryPassId,
            terminalPassIdRef.current === payload.queryPassId,
          );
          return;
        }
        nextSequenceRef.current += 1;
        if (terminalPassIdRef.current !== payload.queryPassId) {
          contentRefreshTicketRef.current += 1;
        }
        setAnswer((current) => current + payload.text);
      });
      if (disposed) { unlistenState(); unlistenChunk(); return; }

      unlistenHidden = await listen('query-review-hidden', () => {
        passIdRef.current = null;
        nextSequenceRef.current = 0;
        contentRefreshTicketRef.current += 1;
        answerRecoveryRef.current = false;
        terminalPassIdRef.current = null;
        terminalContentSnapshotRef.current = false;
        setState('idle');
        setErrorCode(null);
        setAnswer('');
        setErrorDetail(null);
        setSignInFix(null);
        setSignInStatus(null);
        setSignInBusy(false);
        signInAttemptRef.current += 1;
      });
      if (disposed) { unlistenState(); unlistenChunk(); unlistenHidden(); }
    };
    void setup();
    return () => {
      disposed = true;
      signInAttemptRef.current += 1;
      unlistenState?.();
      unlistenChunk?.();
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

  const signIn = useCallback(async () => {
    const queryPassId = passIdRef.current;
    if (queryPassId === null || errorCode !== 'provider_not_authenticated') return;
    const attempt = signInAttemptRef.current + 1;
    signInAttemptRef.current = attempt;
    const ownsAttempt = () => (
      signInAttemptRef.current === attempt && passIdRef.current === queryPassId
    );
    setSignInBusy(true);
    setSignInStatus('Opening Terminal…');
    try {
      await invoke('launch_query_sign_in_for_pass', { queryPassId });
      if (!ownsAttempt()) return;
      setSignInStatus('Terminal opened. Waiting for sign-in…');
      const deadline = Date.now() + 60_000;
      while (ownsAttempt() && Date.now() < deadline) {
        await new Promise((resolve) => window.setTimeout(resolve, 2000));
        if (!ownsAttempt()) return;
        const authenticated = await invoke<boolean>('probe_query_sign_in_for_pass', {
          queryPassId,
        });
        if (!ownsAttempt()) return;
        if (authenticated) {
          setSignInStatus('Signed in. Ask the query again.');
          return;
        }
      }
      if (ownsAttempt()) {
        setSignInStatus('Sign-in is still pending. Finish in Terminal, then try again.');
      }
    } catch {
      if (ownsAttempt()) {
        setSignInStatus('Murmur could not complete provider sign-in.');
      }
    } finally {
      if (ownsAttempt()) setSignInBusy(false);
    }
  }, [errorCode]);

  return {
    state,
    errorCode,
    answer,
    errorDetail,
    signInFix,
    signInStatus,
    signInBusy,
    cancel,
    copy,
    signIn,
  };
}
