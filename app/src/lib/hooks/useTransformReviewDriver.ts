import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { DEFAULT_SETTINGS, loadSettings } from '../settings';
import { flog } from '../log';
import {
  EMPTY_REVIEW_CONTENT,
  isTransformProposalChunkEvent,
  isTransformReviewContent,
  isTransformStateChangedEvent,
  normalizeReviewErrorCode,
} from '../transformReview';
import type { ReviewErrorCode, ReviewState, TransformReviewContent } from '../transformReview';

export interface ReviewDriverResult {
  state: ReviewState;
  errorCode: ReviewErrorCode | null;
  content: TransformReviewContent;
  /** Elapsed ms since entering `thinking` (0 outside that state). */
  thinkingElapsedMs: number;
  cancel: () => void;
  retry: () => void;
  approve: () => void;
  undo: () => void;
}

function deviceNameArg(): string | null {
  try {
    const mic = loadSettings().microphone;
    return mic && mic !== DEFAULT_SETTINGS.microphone ? mic : null;
  } catch {
    return null;
  }
}

/**
 * The real (non-mock) driver: subscribes to the `transform-state-changed`
 * event and fetches text content via `get_transform_review_content` whenever
 * the state changes. `errorCode` is carried on the event itself; instruction/
 * original/proposed text is fetched separately by command so it is never
 * broadcast as an event payload (it may be sensitive selected text).
 *
 * Always call this hook (gate its *effects* on `enabled`, per the Rules of
 * Hooks) — same pattern as `useHoldDownToggle`/`useDoubleTapToggle`.
 */
export function useTransformReviewDriver(enabled: boolean): ReviewDriverResult {
  const [state, setState] = useState<ReviewState>('listening');
  const [errorCode, setErrorCode] = useState<ReviewErrorCode | null>(null);
  const [content, setContent] = useState<TransformReviewContent>(EMPTY_REVIEW_CONTENT);
  const [thinkingElapsedMs, setThinkingElapsedMs] = useState(0);
  const [transformPassId, setTransformPassId] = useState<number | null>(null);
  const activePassRef = useRef<number | null>(null);
  const activeStateRef = useRef<ReviewState>('listening');
  const previewSequenceRef = useRef(-1);

  useEffect(() => {
    if (!enabled) return;
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    let unlistenHidden: (() => void) | null = null;
    let unlistenChunk: (() => void) | null = null;

    // A backend-initiated hide (short-tap cancel, linger auto-hide, capture
    // aborts) emits this content-free signal so we drop any stale review
    // content — otherwise the hidden webview keeps the old diff and can flash
    // it on the next show (item 13). Payload is intentionally empty.
    listen('transform-review-hidden', () => {
      if (cancelled) return;
      setContent(EMPTY_REVIEW_CONTENT);
      setErrorCode(null);
      setState('listening');
      setTransformPassId(null);
      activePassRef.current = null;
      activeStateRef.current = 'listening';
      previewSequenceRef.current = -1;
    })
      .then((fn) => {
        if (cancelled) fn();
        else unlistenHidden = fn;
      })
      .catch((e) => {
        flog.error('transform-review', 'listen(transform-review-hidden) failed', { error: String(e) });
      });

    listen<unknown>('transform-state-changed', (event) => {
      if (cancelled) return;
      if (!isTransformStateChangedEvent(event.payload)) {
        flog.warn('transform-review', 'transform-state-changed had invalid payload');
        return;
      }
      const payload = event.payload;
      setState(payload.state);
      setErrorCode(normalizeReviewErrorCode(payload.errorCode));
      setTransformPassId(payload.transformPassId);
      activeStateRef.current = payload.state;
      if (
        activePassRef.current !== payload.transformPassId
        || payload.state !== 'thinking'
      ) {
        previewSequenceRef.current = -1;
      }
      activePassRef.current = payload.transformPassId;
      if (payload.state === 'thinking') {
        setContent((current) => ({ ...current, proposed: '' }));
      }

      invoke<unknown>('get_transform_review_content')
        .then((value) => {
          if (cancelled) return;
          if (
            activePassRef.current !== payload.transformPassId
            || activeStateRef.current !== payload.state
          ) {
            return;
          }
          if (isTransformReviewContent(value)) {
            setContent((current) => (
              payload.state === 'thinking'
                ? { ...value, proposed: current.proposed }
                : value
            ));
          } else {
            flog.warn('transform-review', 'get_transform_review_content returned invalid payload');
          }
        })
        .catch((e) => {
          if (!cancelled) {
            flog.warn('transform-review', 'get_transform_review_content failed', { error: String(e) });
          }
        });
    })
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch((e) => {
        flog.error('transform-review', 'listen(transform-state-changed) failed', { error: String(e) });
      });

    // Unlike the broadcast state event, proposal text is emitted only to this
    // review webview. Payloads are cumulative and monotonically sequenced, so
    // a dropped event cannot create a torn preview.
    listen<unknown>('transform-proposal-chunk', (event) => {
      if (cancelled || !isTransformProposalChunkEvent(event.payload)) return;
      const payload = event.payload;
      if (
        activeStateRef.current !== 'thinking'
        || activePassRef.current !== payload.transformPassId
        || payload.sequence <= previewSequenceRef.current
      ) {
        return;
      }
      previewSequenceRef.current = payload.sequence;
      setContent((current) => ({ ...current, proposed: payload.text }));
    })
      .then((fn) => {
        if (cancelled) fn();
        else unlistenChunk = fn;
      })
      .catch((e) => {
        flog.error('transform-review', 'listen(transform-proposal-chunk) failed', { error: String(e) });
      });

    return () => {
      cancelled = true;
      unlisten?.();
      unlistenHidden?.();
      unlistenChunk?.();
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

  // PR-C2: wire the popover actions to the real transform-flow commands. The
  // backend owns the state machine and emits the follow-up `transform-state-
  // changed` events; these calls never carry any review text.
  const cancel = useCallback(() => {
    // Clear local content immediately so a subsequent show cannot flash stale
    // selection text before the next get_transform_review_content resolves.
    setContent(EMPTY_REVIEW_CONTENT);
    if (transformPassId === null) return;
    invoke('cancel_transform', { transformPassId }).catch((e) => {
      flog.warn('transform-review', 'cancel_transform failed', { error: String(e) });
    });
  }, [transformPassId]);
  const retry = useCallback(() => {
    invoke('retry_transform_instruction', { deviceName: deviceNameArg() }).catch((e) => {
      flog.warn('transform-review', 'retry_transform_instruction failed', { error: String(e) });
    });
  }, []);
  const approve = useCallback(() => {
    invoke('approve_transform').catch((e) => {
      flog.warn('transform-review', 'approve_transform failed', { error: String(e) });
    });
  }, []);
  const undo = useCallback(() => {
    // Flow-level undo: hides + clears session on success WITHOUT a second
    // epoch bump (chaining cancel_transform would clobber paste-fallback
    // clipboard restore inside the 300ms window — C2 finding 4).
    invoke('undo_transform_and_close')
      .then(() => {
        setContent(EMPTY_REVIEW_CONTENT);
      })
      .catch((e) => {
        flog.warn('transform-review', 'undo_transform_and_close failed', { error: String(e) });
      });
  }, []);

  return { state, errorCode, content, thinkingElapsedMs, cancel, retry, approve, undo };
}
