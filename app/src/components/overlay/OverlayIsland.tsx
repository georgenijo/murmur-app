import { forwardRef, type ComponentPropsWithoutRef } from 'react';
import type { OverlayGeometry } from '../../lib/overlayGeometry';
import { OVERLAY_ISLAND_TRANSITION, OVERLAY_PILL_TRANSITION } from '../../lib/overlayMotion';

/** The native frame is already acknowledged before geometry reaches this
 * component. The full-window wrapper has no pointer handlers/hit target.
 *
 * Notch: a dark island hanging from the top edge whose wings stay inside the
 * menu bar; hover grows it downward. Floating pill: a dark-glass capsule at the
 * bottom of its fixed window that fades in for activity and grows upward on
 * hover, so recording never waits on a native resize.
 *
 * The pill's own width follows `expanded`, not the native window: the window
 * grows before the reveal and shrinks after the collapse, so width and height
 * animate together inside it. Its children sit in a frame fixed at the window
 * width, centred, so they never reflow while the capsule resizes around them. */
const PILL_BORDER = 1;
export const OverlayIsland = forwardRef<HTMLDivElement, ComponentPropsWithoutRef<'div'> & {
  geometry: OverlayGeometry; expanded: boolean; active: boolean; tapMissed?: boolean;
}>(function OverlayIsland({ geometry: g, expanded, active, tapMissed, children, ...props }, ref) {
  const open = active || expanded;
  const corner = open ? g.island.cornerActive : g.island.cornerIdle;
  const width = g.floating && !expanded ? g.island.pillW : g.windowW;
  return <div {...props} ref={ref} className="overlay-island cursor-pointer select-none overflow-hidden"
    data-floating={g.floating || undefined} data-active={active || undefined}
    data-shown={!g.floating || open || undefined}
    style={{
      position: 'relative', flexShrink: 0, marginInline: 'auto', width,
      height: g.collapsedH + (expanded ? g.dropdownH : 0),
      ...(g.floating ? {
        display: 'flex', justifyContent: 'center', alignItems: 'flex-end',
        borderRadius: expanded ? g.island.cornerActive + 4 : g.collapsedH / 2,
        border: `${PILL_BORDER}px solid rgba(255, 255, 255, 0.14)`,
        boxSizing: 'border-box',
        opacity: open ? 1 : 0,
        transform: open ? 'none' : 'translateY(6px) scale(0.92)',
        transformOrigin: '50% 100%',
        transition: OVERLAY_PILL_TRANSITION,
      } : {
        borderRadius: `0 0 ${corner}px ${corner}px`,
        transition: OVERLAY_ISLAND_TRANSITION,
      }),
      // No backdrop-filter: the transparent webview has nothing behind the
      // island to blur, so it would only add an offscreen pass per frame.
      backgroundColor: g.floating ? 'rgba(12, 12, 14, 0.92)' : 'rgba(20, 20, 20, 0.92)',
      // Inset only: the native window is exactly the island's size, so an
      // outer shadow would be clipped to a hard rectangle at its corners.
      boxShadow: tapMissed ? 'inset 0 -2px 0 rgba(245,158,11,0.9)' : 'none',
    }}>
      {/* Always mounted, layout-neutral for the notch: a placement switch must
          not remount the rows and reset their state (e.g. the elapsed timer). */}
      <div className="overlay-pill-frame" style={g.floating ? {
        display: 'flex', flexDirection: 'column-reverse', flexShrink: 0, width: g.windowW - 2 * PILL_BORDER,
      } : { display: 'contents' }}>{children}</div>
    </div>;
});
