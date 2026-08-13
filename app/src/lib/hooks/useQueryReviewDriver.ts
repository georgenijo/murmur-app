import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { flog } from '../log';
import { isQueryUsage, type QueryUsage } from '../queryUsage';

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
  replace: boolean;
}

interface QueryContent {
  queryPassId: number | null;
  answer: string;
  errorDetail: string | null;
  provider: 'claude' | 'codex' | 'grok' | 'cursor' | 'custom' | null;
  usage: QueryUsage | null;
  signInFix: string | null;
  contextSummary: string | null;
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
    && typeof payload.text === 'string'
    && typeof payload.replace === 'boolean';
}

export function useQueryReviewDriver() {
  const [state, setState] = useState<QueryReviewState>('idle');
  const [errorCode, setErrorCode] = useState<string | null>(null);
  const [answer, setAnswer] = useState('');
  const [errorDetail, setErrorDetail] = useState<string | null>(null);
  const [usage, setUsage] = useState<QueryUsage | null>(null);
  const [signInFix, setSignInFix] = useState<string | null>(null);
  const [signInStatus, setSignInStatus] = useState<string | null>(null);
  const [signInBusy, setSignInBusy] = useState(false);
  const [contextSummary, setContextSummary] = useState<string | null>(null);
  const passIdRef = useRef<number | null>(null);
  const nextSequenceRef = useRef(0);
  const contentRefreshTicketRef = useRef(0);
  const contextRefreshTicketRef = useRef(0);
  const answerRecoveryRef = useRef(false);
  const terminalPassIdRef = useRef<number | null>(null);
  const terminalAnswerSnapshotRef = useRef(false);
  const signInAttemptRef = useRef(0);
  const copyAttemptRef = useRef(0);

  useEffect(() => {
    let disposed = false;
    let unlistenState: (() => void) | null = null;
    let unlistenChunk: (() => void) | null = null;
    let unlistenContext: (() => void) | null = null;
    let unlistenHidden: (() => void) | null = null;

    const refreshContext = async (expectedPassId: number) => {
      const ticket = contextRefreshTicketRef.current + 1;
      contextRefreshTicketRef.current = ticket;
      try {
        const content = await invoke<QueryContent>('get_query_review_content');
        if (
          !disposed
          && passIdRef.current === expectedPassId
          && content.queryPassId === expectedPassId
          && contextRefreshTicketRef.current === ticket
        ) {
          setContextSummary(typeof content.contextSummary === 'string' ? content.contextSummary : null);
        }
      } catch {
        flog.warn('query-review', 'could not refresh query context');
      }
    };

    const refreshAnswer = async (expectedPassId: number, terminal = false) => {
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
          setUsage(isQueryUsage(content.usage) ? content.usage : null);
          setSignInFix(typeof content.signInFix === 'string' ? content.signInFix : null);
          if (terminal || terminalPassIdRef.current === expectedPassId) {
            terminalAnswerSnapshotRef.current = true;
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
          contextRefreshTicketRef.current += 1;
          answerRecoveryRef.current = false;
          terminalPassIdRef.current = null;
          terminalAnswerSnapshotRef.current = false;
          setAnswer('');
          setErrorDetail(null);
          setUsage(null);
          setSignInFix(null);
          setSignInStatus(null);
          setSignInBusy(false);
          signInAttemptRef.current += 1;
          copyAttemptRef.current += 1;
          setContextSummary(null);
        }
        setState(payload.state);
        setErrorCode(payload.errorCode);
        // Context and answer refreshes have independent ownership. Nonterminal
        // state changes can recover a missed context notification without an
        // older snapshot ever replacing streamed answer text.
        void refreshContext(payload.queryPassId);
        if (payload.state === 'ready' || payload.state === 'failed') {
          terminalPassIdRef.current = payload.queryPassId;
          void refreshAnswer(payload.queryPassId, true);
        }
      });
      if (disposed) { unlistenState(); return; }

      unlistenChunk = await listen<unknown>('query-answer-chunk', (event) => {
        if (disposed || !isChunkPayload(event.payload)) return;
        const payload = event.payload;
        if (payload.queryPassId !== passIdRef.current) return;
        // A terminal gated snapshot is Rust's complete bounded answer. A chunk
        // delivered late across the two event channels is already represented
        // in that snapshot and must not be appended a second time.
        if (terminalAnswerSnapshotRef.current) return;
        if (answerRecoveryRef.current) {
          nextSequenceRef.current = Math.max(nextSequenceRef.current, payload.sequence + 1);
          // Rust appends before emitting each chunk. Reissuing while recovery
          // is active makes only the snapshot requested after the latest seen
          // chunk eligible to replace the incomplete local answer. Recovery is
          // permanent for this pass: a snapshot may already contain queued
          // chunks, so returning to append mode could duplicate their text.
          void refreshAnswer(
            payload.queryPassId,
            terminalPassIdRef.current === payload.queryPassId,
          );
          return;
        }
        if (payload.sequence !== nextSequenceRef.current) {
          answerRecoveryRef.current = true;
          nextSequenceRef.current = payload.sequence + 1;
          void refreshAnswer(
            payload.queryPassId,
            terminalPassIdRef.current === payload.queryPassId,
          );
          return;
        }
        nextSequenceRef.current += 1;
        // Before a terminal state, a snapshot requested before this chunk must
        // not replace newer local stream state. Ready/Failed is different:
        // Rust emits it only after storing every output chunk, so that pending
        // gated snapshot is authoritative and must recover a missed tail event.
        if (terminalPassIdRef.current !== payload.queryPassId) {
          contentRefreshTicketRef.current += 1;
        }
        setAnswer((current) => payload.replace ? payload.text : current + payload.text);
      });
      if (disposed) { unlistenState(); unlistenChunk(); return; }

      unlistenContext = await listen<unknown>('query-context-resolved', (event) => {
        if (disposed || !event.payload || typeof event.payload !== 'object') return;
        const queryPassId = (event.payload as Record<string, unknown>).queryPassId;
        if (!validPassId(queryPassId) || queryPassId !== passIdRef.current) return;
        void refreshContext(queryPassId);
      });
      if (disposed) { unlistenState(); unlistenChunk(); unlistenContext(); return; }

      unlistenHidden = await listen<unknown>('query-review-hidden', (event) => {
        if (disposed || !event.payload || typeof event.payload !== 'object') return;
        const payload = event.payload as Record<string, unknown>;
        if (Object.keys(payload).length !== 1
          || !validPassId(payload.queryPassId)
          || payload.queryPassId !== passIdRef.current) return;
        passIdRef.current = null;
        nextSequenceRef.current = 0;
        contentRefreshTicketRef.current += 1;
        contextRefreshTicketRef.current += 1;
        answerRecoveryRef.current = false;
        terminalPassIdRef.current = null;
        terminalAnswerSnapshotRef.current = false;
        setState('idle');
        setErrorCode(null);
        setAnswer('');
        setErrorDetail(null);
        setUsage(null);
        setSignInFix(null);
        setSignInStatus(null);
        setSignInBusy(false);
        signInAttemptRef.current += 1;
        copyAttemptRef.current += 1;
        setContextSummary(null);
      });
      if (disposed) { unlistenState(); unlistenChunk(); unlistenContext(); unlistenHidden(); }
    };
    void setup();
    return () => {
      disposed = true;
      signInAttemptRef.current += 1;
      copyAttemptRef.current += 1;
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
    const attempt = copyAttemptRef.current + 1;
    copyAttemptRef.current = attempt;
    const ownsAttempt = () => (
      copyAttemptRef.current === attempt && passIdRef.current === queryPassId
    );
    void invoke('copy_query_answer', { queryPassId }).then(() => {
      if (ownsAttempt()) setErrorCode(null);
    }).catch(() => {
      if (ownsAttempt()) {
        setErrorCode('clipboard_unavailable');
        flog.warn('query-review', 'copy failed', { query_pass_id: queryPassId });
      }
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
    usage,
    signInFix,
    signInStatus,
    signInBusy,
    contextSummary,
    cancel,
    copy,
    signIn,
  };
}
