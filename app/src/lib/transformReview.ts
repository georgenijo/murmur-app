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

// Kept in sync with the Rust code producers in `transform_flow.rs`
// (`selection_error_code` / `transform_error_code` / `apply_error_code`). Any
// code the backend emits that is NOT listed here degrades gracefully to the
// generic "Something went wrong" copy via `normalizeReviewErrorCode` — a
// version-skew safety net, not a bug.
export type ReviewErrorCode =
  | 'model_not_downloaded'
  | 'model_unreadable'
  | 'timeout'
  | 'output_invalid'
  | 'crashed'
  | 'model_verification_timeout'
  | 'model_load_timeout'
  | 'handshake_timeout'
  | 'generation_timeout'
  | 'helper_spawn_failed'
  | 'handshake_protocol_failed'
  | 'process_exit'
  | 'model_verification_failed'
  | 'model_load_failed'
  | 'disabled'
  | 'busy'
  | 'no_instruction'
  | 'no_selection'
  | 'too_large'
  | 'ax_unavailable'
  | 'accessibility_denied'
  | 'target_gone'
  | 'selection_changed'
  | 'clipboard_unavailable'
  | 'paste_failed'
  | 'not_applied';

const REVIEW_STATES: readonly ReviewState[] = [
  'listening', 'thinking', 'ready', 'failed', 'applied',
];

const REVIEW_ERROR_CODES: readonly ReviewErrorCode[] = [
  'model_not_downloaded', 'model_unreadable', 'timeout', 'output_invalid', 'crashed',
  'model_verification_timeout', 'model_load_timeout', 'handshake_timeout',
  'generation_timeout', 'helper_spawn_failed', 'handshake_protocol_failed',
  'process_exit', 'model_verification_failed',
  'model_load_failed',
  'disabled', 'busy', 'no_instruction', 'no_selection', 'too_large', 'ax_unavailable',
  'accessibility_denied', 'target_gone', 'selection_changed',
  'clipboard_unavailable', 'paste_failed', 'not_applied',
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
  model_unreadable: "Model file couldn't be read",
  timeout: 'Timed out',
  output_invalid: 'Model gave no usable output',
  crashed: 'Sidecar crashed — original text untouched',
  model_verification_timeout: 'Model verification timed out',
  model_load_timeout: 'Local model loading timed out',
  handshake_timeout: 'Local helper did not become ready in time',
  generation_timeout: 'Generation timed out',
  helper_spawn_failed: 'Local helper could not start',
  handshake_protocol_failed: 'Local helper identity or protocol check failed',
  process_exit: 'Local helper exited — original text untouched',
  model_verification_failed: 'Local model verification failed',
  model_load_failed: 'Local model or Metal backend failed to load',
  disabled: 'Transform temporarily disabled — try again shortly',
  busy: 'Busy — try again in a moment',
  no_instruction: "Didn't catch an instruction — hold the key and speak",
  no_selection: 'Select some text first',
  too_large: 'Selection is too large to transform',
  ax_unavailable: "Couldn't read the selection",
  accessibility_denied: 'Accessibility permission required',
  target_gone: 'The target app changed — original text untouched',
  selection_changed: 'The selection changed — nothing was overwritten',
  clipboard_unavailable: "Couldn't reach the clipboard — try Undo again",
  paste_failed: "Couldn't undo the change — try Undo again",
  not_applied: 'Nothing to undo',
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

/**
 * Only `state` gates whether this is a well-formed event. An unrecognized
 * `errorCode` (e.g. a newer error code this frontend build doesn't know
 * about yet) is a benign version-skew case, not a malformed event — it must
 * not invalidate the whole `transform-state-changed` event, or the popover
 * gets stuck rendering its prior state forever. Read `errorCode` back out
 * via `normalizeReviewErrorCode`, which coerces anything unrecognized to
 * `null` rather than trusting it directly.
 */
export function isTransformStateChangedEvent(v: unknown): v is TransformStateChangedEvent {
  if (typeof v !== 'object' || v === null) return false;
  const o = v as Record<string, unknown>;
  return isReviewState(o.state);
}

/**
 * Coerce a raw `errorCode` value to a known `ReviewErrorCode`, or `null` if
 * it is absent or unrecognized. Falling back to `null` (rather than
 * rejecting the event) keeps `deriveReviewState`'s "Something went wrong"
 * fallback message reachable instead of leaving the popover stuck.
 */
export function normalizeReviewErrorCode(v: unknown): ReviewErrorCode | null {
  return isReviewErrorCode(v) ? v : null;
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

/**
 * Mirrors Rust's `commands::transform_popover::PopoverBox` — the applied
 * window frame `set_transform_popover_expanded` resolves to and returns as
 * an acknowledgment (same `AppliedSurface`-style contract as
 * `useOverlayExpansion`'s overlay resize path). Exported here so PR-C2's
 * state-machine driver can gate its CSS reveal on the resolved frame instead
 * of guessing the popover's size from the size-class alone.
 */
export interface PopoverBox {
  x: number;
  y: number;
  width: number;
  height: number;
  /** True when the box was placed above the selection to avoid clipping below. */
  flipped: boolean;
}

export function isPopoverBox(v: unknown): v is PopoverBox {
  if (typeof v !== 'object' || v === null) return false;
  const o = v as Record<string, unknown>;
  return (
    typeof o.x === 'number'
    && typeof o.y === 'number'
    && typeof o.width === 'number'
    && typeof o.height === 'number'
    && typeof o.flipped === 'boolean'
  );
}
