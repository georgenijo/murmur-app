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
      owner?.focus();
    }
    branch.toggleAttribute('inert', !open);
  }, [open, present]);

  useLayoutEffect(() => {
    let frame: number | undefined;
    let settle: ReturnType<typeof setTimeout> | undefined;
    let timeout: ReturnType<typeof setTimeout> | undefined;
    if (open) {
      setPresent(true);
      if (reduced) setSettled(true);
      else settle = setTimeout(() => setSettled(true), SETTINGS_FOLD_OPEN_MS);
      if (unmountOnClose && !present && !reduced) frame = requestAnimationFrame(() => setExpanded(true));
      else setExpanded(true);
    } else {
      setSettled(false);
      setExpanded(false);
      if (reduced) setPresent(false);
      else timeout = setTimeout(() => setPresent(false), SETTINGS_FOLD_CLOSE_MS);
    }
    return () => { if (frame !== undefined) cancelAnimationFrame(frame); if (timeout) clearTimeout(timeout); if (settle) clearTimeout(settle); };
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
