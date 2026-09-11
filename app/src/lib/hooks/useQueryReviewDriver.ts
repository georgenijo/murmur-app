import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { flog } from '../log';
import { isQueryUsage, type QueryUsage } from '../queryUsage';
import { pollQuerySignIn } from '../queryProviders';
import {
  isHiddenPayload,
  isQueryStatePayload,
  isValidPassId,
  type QueryContent,
  type QueryReviewState,
} from '../queryReview';

interface QueryChunkPayload {
  queryPassId: number;
  sequence: number;
  text: string;
  replace: boolean;
}

interface QueryPartialPayload {
  queryPassId: number;
  text: string;
}

function isChunkPayload(value: unknown): value is QueryChunkPayload {
  if (!value || typeof value !== 'object') return false;
  const payload = value as Record<string, unknown>;
  return isValidPassId(payload.queryPassId)
    && typeof payload.sequence === 'number'
    && Number.isSafeInteger(payload.sequence)
    && payload.sequence >= 0
    && typeof payload.text === 'string'
    && typeof payload.replace === 'boolean';
}

function isPartialPayload(value: unknown): value is QueryPartialPayload {
  if (!value || typeof value !== 'object') return false;
  const payload = value as Record<string, unknown>;
  return isValidPassId(payload.queryPassId) && typeof payload.text === 'string';
}

export function useQueryReviewDriver() {
  const [state, setState] = useState<QueryReviewState>('idle');
  const [errorCode, setErrorCode] = useState<string | null>(null);
  const [answer, setAnswer] = useState('');
  const [followUpBusy, setFollowUpBusy] = useState(false);
  const [followUpError, setFollowUpError] = useState<string | null>(null);
  const followUpAttemptRef = useRef(0);
  const followUpPendingRef = useRef(false);
  const followUpSourceRef = useRef<number | null>(null);
  const [partial, setPartial] = useState('');
  const [errorDetail, setErrorDetail] = useState<string | null>(null);
  const [usage, setUsage] = useState<QueryUsage | null>(null);
  const [signInFix, setSignInFix] = useState<string | null>(null);
  const [signInStatus, setSignInStatus] = useState<string | null>(null);
  const [signInBusy, setSignInBusy] = useState(false);
  const [contextSummary, setContextSummary] = useState<string | null>(null);
  const [capabilitySummary, setCapabilitySummary] = useState<string | null>(null);
  const passIdRef = useRef<number | null>(null);
  const stateRef = useRef<QueryReviewState>('idle');
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
    let unlistenPartial: (() => void) | null = null;
    let unlistenContext: (() => void) | null = null;
    let unlistenHidden: (() => void) | null = null;
    let unlistenFollowUp: (() => void) | null = null;

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
          setCapabilitySummary(typeof content.capabilitySummary === 'string' ? content.capabilitySummary : null);
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
        if (disposed || !isQueryStatePayload(event.payload)) return;
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
          followUpAttemptRef.current += 1;
          followUpSourceRef.current = null;
          followUpPendingRef.current = false;
          setFollowUpBusy(false);
          setFollowUpError(null);
          setPartial('');
          setErrorDetail(null);
          setUsage(null);
          setSignInFix(null);
          setSignInStatus(null);
          setSignInBusy(false);
          signInAttemptRef.current += 1;
          copyAttemptRef.current += 1;
          setContextSummary(null);
          setCapabilitySummary(null);
        }
        stateRef.current = payload.state;
        setState(payload.state);
        setErrorCode(payload.errorCode);
        if (payload.state !== 'listening') {
          setPartial('');
        }
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

      unlistenPartial = await listen<unknown>('query-partial', (event) => {
        if (disposed || !isPartialPayload(event.payload)) return;
        const payload = event.payload;
        if (payload.queryPassId !== passIdRef.current) return;
        if (stateRef.current !== 'listening') return;
        setPartial(payload.text);
      });
      if (disposed) { unlistenState(); unlistenChunk(); unlistenPartial(); return; }

      unlistenContext = await listen<unknown>('query-context-resolved', (event) => {
        if (disposed || !event.payload || typeof event.payload !== 'object') return;
        const queryPassId = (event.payload as Record<string, unknown>).queryPassId;
        if (!isValidPassId(queryPassId) || queryPassId !== passIdRef.current) return;
        void refreshContext(queryPassId);
      });
      if (disposed) { unlistenState(); unlistenChunk(); unlistenPartial(); unlistenContext(); return; }

      unlistenHidden = await listen<unknown>('query-review-hidden', (event) => {
        if (disposed || !isHiddenPayload(event.payload)) return;
        const payload = event.payload;
        if (payload.queryPassId !== passIdRef.current) return;
        followUpSourceRef.current = null;
        passIdRef.current = null;
        nextSequenceRef.current = 0;
        contentRefreshTicketRef.current += 1;
        contextRefreshTicketRef.current += 1;
        answerRecoveryRef.current = false;
        terminalPassIdRef.current = null;
        terminalAnswerSnapshotRef.current = false;
        stateRef.current = 'idle';
        setState('idle');
        followUpAttemptRef.current += 1;
        followUpPendingRef.current = false;
        setFollowUpBusy(false);
        setFollowUpError(null);
        setErrorCode(null);
        setAnswer('');
        setPartial('');
        setErrorDetail(null);
        setUsage(null);
        setSignInFix(null);
        setSignInStatus(null);
        setSignInBusy(false);
        signInAttemptRef.current += 1;
        copyAttemptRef.current += 1;
        setContextSummary(null);
        setCapabilitySummary(null);
      });
      if (disposed) { unlistenState(); unlistenChunk(); unlistenPartial(); unlistenContext(); unlistenHidden(); return; }
      unlistenFollowUp = await listen<unknown>('query-follow-up-unavailable', (event) => {
        if (disposed || !isHiddenPayload(event.payload)) return;
        if (event.payload.queryPassId === followUpSourceRef.current) {
          followUpSourceRef.current = null;
          followUpPendingRef.current = false;
          setFollowUpBusy(false);
        }
        if (event.payload.queryPassId !== passIdRef.current || stateRef.current !== 'ready') return;
        setFollowUpError('Could not start a follow-up. Try again or ask a new query.');
      });
      if (disposed) unlistenFollowUp();
    };
    void setup();
    return () => {
      disposed = true;
      signInAttemptRef.current += 1;
      copyAttemptRef.current += 1;
      unlistenState?.();
      unlistenChunk?.();
      unlistenPartial?.();
      unlistenContext?.();
      unlistenHidden?.();
      unlistenFollowUp?.();
      followUpAttemptRef.current += 1;
    };
  }, []);

  const cancel = useCallback(() => {
    const queryPassId = passIdRef.current;
    if (queryPassId === null) return;
    void invoke('cancel_query', {
      queryPassId,
      ...(followUpSourceRef.current !== null ? { followUpFromPassId: followUpSourceRef.current } : {}),
    }).catch(() => {
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

  const followUp = useCallback(async () => {
    const queryPassId = passIdRef.current;
    if (queryPassId === null || stateRef.current !== 'ready' || followUpPendingRef.current) return;
    const attempt = ++followUpAttemptRef.current;
    const ownsAttempt = () => followUpAttemptRef.current === attempt && passIdRef.current === queryPassId;
    followUpPendingRef.current = true;
    followUpSourceRef.current = queryPassId;
    setFollowUpBusy(true);
    setFollowUpError(null);
    try {
      await invoke('request_query_follow_up', { queryPassId });
    } catch {
      if (followUpSourceRef.current === queryPassId) followUpSourceRef.current = null;
      if (ownsAttempt()) {
        followUpPendingRef.current = false;
        setFollowUpBusy(false);
        setFollowUpError('Could not start a follow-up. Check that Voice Query is enabled, then try again.');
      }
    }
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
      await pollQuerySignIn({
        launch: () => invoke('launch_query_sign_in_for_pass', { queryPassId }),
        onLaunched: () => setSignInStatus('Terminal opened. Waiting for sign-in…'),
        probe: () => invoke<boolean>('probe_query_sign_in_for_pass', { queryPassId }),
        isSignedIn: (authenticated) => authenticated,
        onSignedIn: () => setSignInStatus('Signed in. Ask the query again.'),
        onPending: () => setSignInStatus('Sign-in is still pending. Finish in Terminal, then try again.'),
        ownsAttempt,
      });
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
    partial,
    errorDetail,
    usage,
    signInFix,
    signInStatus,
    signInBusy,
    contextSummary,
    capabilitySummary,
    followUp,
    followUpBusy,
    followUpError,
    cancel,
    copy,
    signIn,
  };
}
