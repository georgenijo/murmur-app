// Shared types + validators for the transform review popover's events
// contract. Mirrors `lib/overlayGeometry.ts` / `lib/types.ts`'s runtime-guard
// pattern: never trust a raw event/command payload without checking its shape
// first, so a malformed backend payload degrades gracefully instead of
// crashing the popover's render.
//
// The real backend event flow (Rust emitting `transform-state-changed`) is
// wired in PR-C2. This file locks the contract shape now so both the mock
// driver and the eventual real driver produce/consume the same thing.

export type ReviewState = 'listening' | 'thinking' | 'ready' | 'failed' | 'applied';

export type ReviewErrorCode =
  | 'model_not_downloaded'
  | 'timeout'
  | 'output_invalid'
  | 'crashed';

const REVIEW_STATES: readonly ReviewState[] = [
  'listening', 'thinking', 'ready', 'failed', 'applied',
];

const REVIEW_ERROR_CODES: readonly ReviewErrorCode[] = [
  'model_not_downloaded', 'timeout', 'output_invalid', 'crashed',
];

export function isReviewState(v: unknown): v is ReviewState {
  return typeof v === 'string' && (REVIEW_STATES as readonly string[]).includes(v);
}

export function isReviewErrorCode(v: unknown): v is ReviewErrorCode {
  return typeof v === 'string' && (REVIEW_ERROR_CODES as readonly string[]).includes(v);
}

/** Stable copy for each error code — never render a raw error code to the user. */
export const REVIEW_ERROR_COPY: Record<ReviewErrorCode, string> = {
  model_not_downloaded: 'Model not downloaded',
  timeout: 'Timed out',
  output_invalid: 'Model gave no usable output',
  crashed: 'Sidecar crashed — original text untouched',
};

/**
 * Payload of the `transform-state-changed` event. Text content (instruction/
 * original/proposed) is deliberately NOT part of this event — it is fetched
 * separately via the `get_transform_review_content` command so potentially
 * sensitive text is never broadcast as an event payload.
 */
export interface TransformStateChangedEvent {
  state: ReviewState;
  errorCode?: ReviewErrorCode;
}

export function isTransformStateChangedEvent(v: unknown): v is TransformStateChangedEvent {
  if (typeof v !== 'object' || v === null) return false;
  const o = v as Record<string, unknown>;
  if (!isReviewState(o.state)) return false;
  if ('errorCode' in o && o.errorCode !== undefined && !isReviewErrorCode(o.errorCode)) {
    return false;
  }
  return true;
}

/** Return value of the `get_transform_review_content` command. */
export interface TransformReviewContent {
  instruction: string;
  original: string;
  proposed: string;
}

export function isTransformReviewContent(v: unknown): v is TransformReviewContent {
  if (typeof v !== 'object' || v === null) return false;
  const o = v as Record<string, unknown>;
  return (
    typeof o.instruction === 'string'
    && typeof o.original === 'string'
    && typeof o.proposed === 'string'
  );
}

export const EMPTY_REVIEW_CONTENT: TransformReviewContent = {
  instruction: '',
  original: '',
  proposed: '',
};
