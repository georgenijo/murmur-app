// Motion tokens for the overlay expand/collapse choreography.
//
// This is the single source of truth for the durations and easings that used to
// live inline in OverlayWidget's JSX and as hand-tuned constants. Keeping them
// here lets the expansion controller derive dependent timings — notably the
// shrink delay — from the very numbers the CSS transitions use, so the code link
// can never silently drift.
//
// Values are identical to what shipped before this module existed: there is zero
// intended visual or timing change, only a single definition point.

/** Width/position transition duration for the island element (ms). */
export const OVERLAY_WIDTH_MS = 400;
/** Height transition duration for the island element (ms). */
export const OVERLAY_HEIGHT_MS = 360;
/** Spring easing (with overshoot) for the hover dropdown's height growth. */
export const OVERLAY_SPRING = 'cubic-bezier(0.34,1.56,0.64,1)';
/**
 * Easing for the idle↔active width + horizontal-position change (the "in use /
 * not in use" transition). A smooth decelerate with NO overshoot: the active
 * pill grows to exactly the window width, so an overshooting spring would push
 * the pill past the window frame where it is clipped — making the settle read as
 * an abrupt cut. easeOutQuint-style deceleration lands cleanly at full width.
 */
export const OVERLAY_ACTIVE_EASE = 'cubic-bezier(0.22,1,0.36,1)';

/**
 * Duration (ms) for the enable↔disable state morph: the top-bar mic's stroke
 * color crossfade (white↔red), the slashed-out line drawing in/out, and the
 * pill's background tint. Shares `OVERLAY_ACTIVE_EASE` so the whole indicator
 * settles with the same no-overshoot feel as the width/position change. Without
 * it the disable toggle swapped two separate SVG nodes instantly, a hard cut.
 */
export const OVERLAY_STATE_MS = 260;

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
 * The composed `transition` string applied to the island element. Width and
 * horizontal position (`transform: translateX`) animate together with the
 * non-overshoot active ease so the idle↔active change grows and slides as one
 * smooth motion — previously `margin-left` was NOT transitioned, so the pill's
 * left edge snapped while its width sprang, an asymmetric jump. Height keeps the
 * playful spring for the hover dropdown, which grows into open space below the
 * notch and benefits from the overshoot.
 */
export const OVERLAY_ISLAND_TRANSITION =
  `width ${OVERLAY_WIDTH_MS}ms ${OVERLAY_ACTIVE_EASE}, ` +
  `transform ${OVERLAY_WIDTH_MS}ms ${OVERLAY_ACTIVE_EASE}, ` +
  `height ${OVERLAY_HEIGHT_MS}ms ${OVERLAY_SPRING}, ` +
  `background-color ${OVERLAY_STATE_MS}ms ${OVERLAY_ACTIVE_EASE}`;
