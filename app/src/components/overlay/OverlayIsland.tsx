import { forwardRef, type ComponentPropsWithoutRef } from 'react';
import type { OverlayGeometry } from '../../lib/overlayGeometry';
import { OVERLAY_ISLAND_TRANSITION } from '../../lib/overlayMotion';

/** The native frame is already acknowledged before geometry reaches this
 * component. The full-window wrapper has no pointer handlers/hit target. */
export const OverlayIsland = forwardRef<HTMLDivElement, ComponentPropsWithoutRef<'div'> & {
  geometry: OverlayGeometry; expanded: boolean; active: boolean; tapMissed?: boolean;
}>(function OverlayIsland({ geometry: g, expanded, active, tapMissed, children, ...props }, ref) {
  return <div {...props} ref={ref} className="overlay-island cursor-pointer select-none overflow-hidden"
    data-floating={g.floating || undefined} data-active={active || undefined}
    style={{
      position: 'relative', flexShrink: 0, marginInline: 'auto', width: g.windowW,
      height: g.collapsedH + (expanded ? g.dropdownH : 0),
      borderRadius: g.floating ? g.island.cornerActive
        : `0 0 ${active || expanded ? g.island.cornerActive : g.island.cornerIdle}px ${active || expanded ? g.island.cornerActive : g.island.cornerIdle}px`,
      background: '#000',
      boxShadow: tapMissed ? 'inset 0 -2px 0 rgba(245,158,11,0.9)' : 'none',
      transition: OVERLAY_ISLAND_TRANSITION,
    }}>{children}</div>;
});
