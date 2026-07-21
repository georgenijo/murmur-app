// Motion tokens for the overlay expand/collapse choreography.
//
// This is the single source of truth for the durations and easings that used to
// live inline in OverlayWidget's JSX and as hand-tuned constants. Keeping them
// here lets the expansion controller derive dependent timings — notably the
// shrink delay — from the very numbers the CSS transitions use, so the code link
// can never silently drift.
//
// The island is one constant width in every state (see `geometry_for` in
// commands/overlay.rs), so expansion is HEIGHT-ONLY — width never animates and
// there is no width token.

/** Height transition duration for the island element (ms). */
export const OVERLAY_HEIGHT_MS = 360;
/** Spring easing for the height transition. */
export const OVERLAY_SPRING = 'cubic-bezier(0.34,1.56,0.64,1)';

/** Sustained hover (ms) required on the island before the card opens. */
export const HOVER_OPEN_DWELL_MS = 150;
/** Delay (ms) after the cursor leaves before the card begins closing. */
export const COLLAPSE_DELAY_MS = 300;

/**
 * Delay (ms) between the card starting to close and the native window shrinking.
 *
 * The window must stay tall until the height transition has finished, otherwise
 * the dropdown is clipped mid-close. Derive it from the height transition (plus a
 * small guard) instead of hand-tuning a free-standing 380 — the two can no longer
 * disagree.
 */
export const SHRINK_DELAY_MS = OVERLAY_HEIGHT_MS + 20;

/**
 * The composed `transition` string applied to the island element. Height-only:
 * the island's width is constant across every state, so only height animates.
 */
export const OVERLAY_ISLAND_TRANSITION =
  `height ${OVERLAY_HEIGHT_MS}ms ${OVERLAY_SPRING}`;

// --- Transform review popover motion tokens ---------------------------------
//
// The review popover (`app/src/components/transform-review/`) is a separate
// window from the overlay, but shares its motion language: compact scale/fade
// entrance, soft dismiss, and a short commit pulse on approve. Values live
// here — the one motion-token source for both windows — rather than as
// component-local constants.

/** Entrance: scale 0.96 -> 1 + fade, on popover show. */
export const REVIEW_ENTRANCE_MS = 160;
export const REVIEW_ENTRANCE_EASE = 'ease-out';
export const REVIEW_ENTRANCE_FROM_SCALE = 0.96;

/** Dismiss: fade + scale down slightly, on cancel/fail-clear/auto-dismiss. */
export const REVIEW_DISMISS_MS = 120;
export const REVIEW_DISMISS_EASE = 'ease-in';
export const REVIEW_DISMISS_TO_SCALE = 0.98;

/** Short commit pulse on the diff area when Approve is pressed. */
export const REVIEW_APPROVE_PULSE_MS = 90;
/** Peak scale of the approve pulse (a barely-there "commit" nudge). */
export const REVIEW_APPROVE_PULSE_SCALE = 1.01;

/** How long the transient "applied" state stays up before auto-dismissing. */
export const REVIEW_APPLIED_AUTO_DISMISS_MS = 4000;

/** How long `thinking` must run before the "Still working…" hint appears. */
export const REVIEW_STILL_WORKING_HINT_MS = 5000;

/**
 * Composed `transition` string for the popover's entrance/dismiss (transform +
 * opacity). `prefers-reduced-motion` handling mirrors the overlay: consumers
 * should read `matchMedia('(prefers-reduced-motion: reduce)')` and skip the
 * transition (snap directly to the end state) rather than hardcode a second
 * duration here.
 */
export const REVIEW_ENTRANCE_TRANSITION =
  `transform ${REVIEW_ENTRANCE_MS}ms ${REVIEW_ENTRANCE_EASE}, opacity ${REVIEW_ENTRANCE_MS}ms ${REVIEW_ENTRANCE_EASE}`;
export const REVIEW_DISMISS_TRANSITION =
  `transform ${REVIEW_DISMISS_MS}ms ${REVIEW_DISMISS_EASE}, opacity ${REVIEW_DISMISS_MS}ms ${REVIEW_DISMISS_EASE}`;
/**
 * Approve pulse: `transform`-only (no `opacity` term) so composing it
 * alongside `REVIEW_ENTRANCE_TRANSITION`/`REVIEW_DISMISS_TRANSITION` never
 * needs to redeclare the same property twice — CSS's "last transition for a
 * given property wins" rule previously made the entrance's 160ms `transform`
 * silently lose to this pulse's 90ms whenever both were listed together.
 * Callers pick ONE of these three transition strings per render, never
 * concatenate them.
 */
export const REVIEW_APPROVE_PULSE_TRANSITION =
  `transform ${REVIEW_APPROVE_PULSE_MS}ms ${REVIEW_ENTRANCE_EASE}`;
