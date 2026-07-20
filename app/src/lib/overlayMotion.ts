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
