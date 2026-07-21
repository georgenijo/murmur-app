import { useCallback, useEffect, useRef, useState } from 'react';
import { isReviewState } from '../transformReview';
import type { ReviewErrorCode, ReviewState, TransformReviewContent } from '../transformReview';
import { REVIEW_APPLIED_AUTO_DISMISS_MS } from '../overlayMotion';
import type { ReviewDriverResult } from './useTransformReviewDriver';

/**
 * Dev-only demo driver, reachable via `?mock=1` (or `?mock=<state>`) on the
 * transform-review window URL, so the whole review UI is demoable without a
 * backend. Gated on `import.meta.env.DEV` by the caller (`isMockReviewEnabled`)
 * — this module never ships in a production build's active code path.
 */
export function isMockReviewEnabled(): boolean {
  if (!import.meta.env.DEV) return false;
  if (typeof window === 'undefined') return false;
  return new URLSearchParams(window.location.search).has('mock');
}

const SAMPLE_CONTENT: TransformReviewContent = {
  instruction: 'Make this more concise',
  original:
    'I wanted to reach out and let you know that I think we should probably go ahead '
    + 'and schedule a meeting sometime next week to discuss the project timeline in more detail.',
  proposed: "Let's schedule a meeting next week to discuss the project timeline.",
};

/** `?mock=1` (or any value that isn't a known state name) auto-runs this intro cycle once. */
const AUTO_CYCLE_LISTENING_MS = 1800;
const AUTO_CYCLE_THINKING_MS = 2200;
/** Retry/re-thinking duration used by the interactive Retry action below. */
const MOCK_RETRY_THINKING_MS = 1500;

/**
 * Mock state machine: auto-plays listening -> thinking -> ready once on
 * mount (or jumps straight to `?mock=<state>` for screenshotting a specific
 * state), then stays interactive — Approve/Retry/Cancel/Undo drive real state
 * transitions with sample data, exactly like the eventual real driver, so the
 * whole review surface (including its keyboard shortcuts) is demoable.
 *
 * Always call this hook (gate its *effects* on `enabled`) — mirrors
 * `useTransformReviewDriver`'s rule-of-hooks discipline.
 */
export function useMockReviewDriver(enabled: boolean): ReviewDriverResult {
  const [state, setState] = useState<ReviewState>('listening');
  const [errorCode, setErrorCode] = useState<ReviewErrorCode | null>(null);
  const [thinkingElapsedMs, setThinkingElapsedMs] = useState(0);
  // Alternates failed/ready on repeated Retry so a demo session can reach
  // every state without needing a real sidecar to actually fail.
  const nextRetryFailsRef = useRef(false);

  useEffect(() => {
    if (!enabled || typeof window === 'undefined') return;
    const requested = new URLSearchParams(window.location.search).get('mock');
    if (requested && isReviewState(requested)) {
      setState(requested);
      if (requested === 'failed') setErrorCode('timeout');
      return;
    }

    setState('listening');
    const toThinking = window.setTimeout(() => setState('thinking'), AUTO_CYCLE_LISTENING_MS);
    const toReady = window.setTimeout(
      () => setState('ready'),
      AUTO_CYCLE_LISTENING_MS + AUTO_CYCLE_THINKING_MS,
    );
    return () => {
      window.clearTimeout(toThinking);
      window.clearTimeout(toReady);
    };
  }, [enabled]);

  const thinkingStartRef = useRef<number | null>(null);
  useEffect(() => {
    if (!enabled || state !== 'thinking') {
      thinkingStartRef.current = null;
      setThinkingElapsedMs(0);
      return;
    }
    thinkingStartRef.current = Date.now();
    const id = window.setInterval(() => {
      const start = thinkingStartRef.current;
      if (start !== null) setThinkingElapsedMs(Date.now() - start);
    }, 250);
    return () => window.clearInterval(id);
  }, [enabled, state]);

  const cancel = useCallback(() => {
    setErrorCode(null);
    setState('listening');
  }, []);

  const retry = useCallback(() => {
    setState('thinking');
    const willFail = nextRetryFailsRef.current;
    nextRetryFailsRef.current = !willFail;
    window.setTimeout(() => {
      if (willFail) {
        setErrorCode('timeout');
        setState('failed');
      } else {
        setErrorCode(null);
        setState('ready');
      }
    }, MOCK_RETRY_THINKING_MS);
  }, []);

  const approve = useCallback(() => {
    setState('applied');
    window.setTimeout(() => setState('listening'), REVIEW_APPLIED_AUTO_DISMISS_MS);
  }, []);

  const undo = useCallback(() => {
    setState('listening');
  }, []);

  return { state, errorCode, content: SAMPLE_CONTENT, thinkingElapsedMs, cancel, retry, approve, undo };
}
