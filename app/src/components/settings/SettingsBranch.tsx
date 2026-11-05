import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react';

export const SETTINGS_FOLD_OPEN_MS = 300;
export const SETTINGS_FOLD_CLOSE_MS = 450;

export function useSettingsReducedMotion() {
  const [reduced, setReduced] = useState(() => window.matchMedia?.('(prefers-reduced-motion: reduce)').matches ?? false);
  useEffect(() => {
    const media = window.matchMedia?.('(prefers-reduced-motion: reduce)');
    if (!media) return;
    const change = () => setReduced(media.matches);
    media.addEventListener('change', change);
    return () => media.removeEventListener('change', change);
  }, []);
  return reduced;
}

/** Dependent rows keep drafts mounted. Rare Advanced rows opt into unmounting
 * after the exit clip; both become inert/AX-hidden as soon as closing starts.
 * Behavior follows Hex controls.rs folds(); measurements remain T3's (#848). */
export function SettingsBranch({ open, children, className = '', unmountOnClose = false, restoreFocus, id }: {
  open: boolean;
  children: ReactNode;
  className?: string;
  unmountOnClose?: boolean;
  restoreFocus?: () => HTMLElement | null;
  id?: string;
}) {
  const branchRef = useRef<HTMLDivElement>(null);
  const [present, setPresent] = useState(open);
  const [expanded, setExpanded] = useState(open);
  const [settled, setSettled] = useState(open);
  const reduced = useSettingsReducedMotion();
  const restoreRef = useRef(restoreFocus);
  restoreRef.current = restoreFocus;

  useLayoutEffect(() => {
    const branch = branchRef.current;
    if (!branch) return;
    if (!open && branch.contains(document.activeElement)) {
      const previous = branch.previousElementSibling;
      const owner = restoreRef.current?.() ?? (previous instanceof HTMLElement
        ? previous.matches('button') ? previous : previous.querySelector<HTMLElement>('[role="switch"]') ?? previous.querySelector<HTMLElement>('button, input, select')
        : null);
      owner?.focus({ preventScroll: true });
    }
    branch.toggleAttribute('inert', !open);
  }, [open, present]);

  useLayoutEffect(() => {
    let frame: number | undefined;
    let scrollFrame: number | undefined;
    let scrollSpace: HTMLDivElement | undefined;
    let settle: ReturnType<typeof setTimeout> | undefined;
    let timeout: ReturnType<typeof setTimeout> | undefined;
    const branch = branchRef.current;
    const scroller = branch?.closest('.settings-content-container')?.parentElement;
    const stopScroll = () => {
      if (scrollFrame !== undefined) cancelAnimationFrame(scrollFrame);
      scrollSpace?.remove();
    };
    if (open) {
      setPresent(true);
      if (reduced) setSettled(true);
      else settle = setTimeout(() => setSettled(true), SETTINGS_FOLD_OPEN_MS);
      if (unmountOnClose && !present && !reduced) frame = requestAnimationFrame(() => setExpanded(true));
      else setExpanded(true);
    } else {
      // A bottom-scrolled pane can keep anchoring to a disappearing child until
      // it is clipped away, then jump to its new scroll limit. Follow the actual
      // fold height instead, using the same motion and no per-frame React work.
      const height = branch?.getBoundingClientRect().height ?? 0;
      if (branch && scroller && height > 0 && !reduced) {
        const from = scroller.scrollTop;
        const collapsedLimit = scroller.scrollHeight - scroller.clientHeight - height;
        const to = Math.min(from, Math.max(0, collapsedLimit));
        if (to < from) {
          // If the collapsed page is shorter than the viewport, its native
          // limit reaches zero before the fold does. Reserve that shortfall
          // until scrolling reaches zero too, then remove it without a clamp.
          if (collapsedLimit < 0) {
            scrollSpace = document.createElement('div');
            scrollSpace.setAttribute('aria-hidden', 'true');
            scroller.append(scrollSpace);
          }
          const followCollapse = () => {
            const remaining = Math.min(height, branch.getBoundingClientRect().height);
            if (scrollSpace) scrollSpace.style.height = `${-collapsedLimit * (1 - remaining / height)}px`;
            scroller.scrollTop = to + (from - to) * remaining / height;
            if (remaining > 0) scrollFrame = requestAnimationFrame(followCollapse);
            else stopScroll();
          };
          scrollFrame = requestAnimationFrame(followCollapse);
          // A user's next gesture takes over from automatic scroll adjustment.
          scroller.addEventListener('wheel', stopScroll, { passive: true });
          scroller.addEventListener('touchstart', stopScroll, { passive: true });
          scroller.addEventListener('keydown', stopScroll);
          scroller.addEventListener('pointerdown', stopScroll);
        }
      }
      setSettled(false);
      setExpanded(false);
      if (reduced) setPresent(false);
      else timeout = setTimeout(() => setPresent(false), SETTINGS_FOLD_CLOSE_MS);
    }
    return () => {
      if (frame !== undefined) cancelAnimationFrame(frame);
      stopScroll();
      scroller?.removeEventListener('wheel', stopScroll);
      scroller?.removeEventListener('touchstart', stopScroll);
      scroller?.removeEventListener('keydown', stopScroll);
      scroller?.removeEventListener('pointerdown', stopScroll);
      if (timeout) clearTimeout(timeout);
      if (settle) clearTimeout(settle);
    };
  // present intentionally isn't a dependency: mounting must not cancel entry's frame.
  }, [open, reduced, unmountOnClose]);

  return <div ref={branchRef} id={id}
    className={`settings-dependent-branch ${expanded ? 'settings-dependent-branch-open' : ''} ${className}`.trim()}
    data-expanded={open} aria-hidden={!open}>
    <div className={`settings-dependent-branch-clip ${open && settled ? 'settings-dependent-branch-settled' : ''}`}>
      <div className="settings-dependent-branch-content settings-stack">
        {(!unmountOnClose || open || present) && children}
      </div>
    </div>
  </div>;
}
