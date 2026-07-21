import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { flog } from '../log';
import {
  EMPTY_REVIEW_CONTENT,
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

/**
 * The real (non-mock) driver: subscribes to the `transform-state-changed`
 * event and fetches text content via `get_transform_review_content` whenever
 * the state changes. `errorCode` is carried on the event itself; instruction/
 * original/proposed text is fetched separately by command so it is never
 * broadcast as an event payload (it may be sensitive selected text).
 *
 * PR-C2 wires the real backend to emit `transform-state-changed` and to
 * populate `get_transform_review_content`; it will also add the real
 * cancel/retry/approve/undo commands. For PR-C1, `cancel` hides the popover
 * (the one native effect already wired) and `retry`/`approve`/`undo` are
 * no-ops — there is no backend transform pipeline yet to drive.
 *
 * Always call this hook (gate its *effects* on `enabled`, per the Rules of
 * Hooks) — same pattern as `useHoldDownToggle`/`useDoubleTapToggle`.
 */
export function useTransformReviewDriver(enabled: boolean): ReviewDriverResult {
  const [state, setState] = useState<ReviewState>('listening');
  const [errorCode, setErrorCode] = useState<ReviewErrorCode | null>(null);
  const [content, setContent] = useState<TransformReviewContent>(EMPTY_REVIEW_CONTENT);
  const [thinkingElapsedMs, setThinkingElapsedMs] = useState(0);

  useEffect(() => {
    if (!enabled) return;
    let cancelled = false;
    let unlisten: (() => void) | null = null;

    listen<unknown>('transform-state-changed', (event) => {
      if (cancelled) return;
      if (!isTransformStateChangedEvent(event.payload)) {
        flog.warn('transform-review', 'transform-state-changed had invalid payload');
        return;
      }
      setState(event.payload.state);
      setErrorCode(normalizeReviewErrorCode(event.payload.errorCode));

      invoke<unknown>('get_transform_review_content')
        .then((value) => {
          if (cancelled) return;
          if (isTransformReviewContent(value)) {
            setContent(value);
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

    return () => {
      cancelled = true;
      unlisten?.();
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
    invoke('hide_transform_popover').catch((e) => {
      flog.warn('transform-review', 'hide_transform_popover failed', { error: String(e) });
    });
  }, []);
  // PR-C2: wire these to real transform commands once the sidecar exists.
  const retry = useCallback(() => {}, []);
  const approve = useCallback(() => {}, []);
  const undo = useCallback(() => {}, []);

  return { state, errorCode, content, thinkingElapsedMs, cancel, retry, approve, undo };
}
