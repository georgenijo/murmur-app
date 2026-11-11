// Motion tokens for the overlay expand/collapse choreography.
//
// This is the single source of truth for the durations and easings that used to
// live inline in OverlayWidget's JSX and as hand-tuned constants. Keeping them
// here lets the expansion controller derive dependent timings — notably the
// shrink delay — from the very numbers the CSS transitions use, so the code link
// can never silently drift.
//
// The notch island grows its wings out from the notch centre and reveals the
// dropdown downward; the floating pill grows upward from its bottom edge.
// Width and height share one duration/easing so each opens as one surface.

/** Width/height transition duration for the island element (ms). */
export const OVERLAY_HEIGHT_MS = 360;
/** Spring easing for the width/height transition. */
const OVERLAY_SPRING = 'cubic-bezier(0.34,1.56,0.64,1)';
/** Fade/scale for the floating pill appearing and hiding (ms). */
const OVERLAY_PILL_FADE_MS = 180;

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
 * The composed transition for the centred notch island and its hover dropdown.
 */
export const OVERLAY_ISLAND_TRANSITION =
  `width ${OVERLAY_HEIGHT_MS}ms ${OVERLAY_SPRING}, height ${OVERLAY_HEIGHT_MS}ms ${OVERLAY_SPRING}`;

/** Settling easing for the floating pill's resize. The spring's overshoot
 * reads as a bounce on a free-floating capsule, most visibly on collapse. */
const OVERLAY_PILL_EASE = 'cubic-bezier(0.32,0.72,0,1)';

/**
 * The floating pill: composited opacity/transform for appearing, plus one
 * settling width/height resize (same duration as the notch) when hover grows
 * or collapses it, so both dimensions move as one surface.
 */
export const OVERLAY_PILL_TRANSITION =
  `opacity ${OVERLAY_PILL_FADE_MS}ms ease-out, transform ${OVERLAY_PILL_FADE_MS}ms ease-out, `
  + `width ${OVERLAY_HEIGHT_MS}ms ${OVERLAY_PILL_EASE}, height ${OVERLAY_HEIGHT_MS}ms ${OVERLAY_PILL_EASE}`;

/**
 * Live Words rides the island: its window anchors to the collapsed surface and
 * the card moves by the hover dropdown's height on the island's own curve, so
 * the two never separate while the native window waits to shrink.
 */
export const LIVE_WORDS_LIFT_EVENT = 'overlay-live-words-lift';
/** Sent by a (re)loaded Live Words window once it listens; the overlay replies
 * with the current lift so a missed hover transition never strands the card. */
export const LIVE_WORDS_LIFT_REQUEST_EVENT = 'overlay-live-words-lift-request';
export function liveWordsLiftTransition(floating: boolean): string {
  return `transform ${OVERLAY_HEIGHT_MS}ms ${floating ? OVERLAY_PILL_EASE : OVERLAY_SPRING}`;
}
