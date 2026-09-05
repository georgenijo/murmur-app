import { useLayoutEffect, useRef, type ReactNode } from 'react';

/**
 * Reveals controls that only make sense while their owning switch is on.
 * Keeping the branch mounted preserves draft values while the switch is off;
 * visibility removes collapsed controls from the keyboard and accessibility
 * tree while the grid transition keeps the layout change quiet and bounded.
 */
export function SettingsBranch({ open, children, className = '' }: {
  open: boolean;
  children: ReactNode;
  className?: string;
}) {
  const branchRef = useRef<HTMLDivElement>(null);

  useLayoutEffect(() => {
    const branch = branchRef.current;
    if (!branch) return;
    if (!open && branch.contains(document.activeElement)) {
      branch.previousElementSibling?.querySelector<HTMLElement>('[role="switch"]')?.focus();
    }
    if (open) branch.removeAttribute('inert');
    else branch.setAttribute('inert', '');
  }, [open]);

  return (
    <div
      ref={branchRef}
      className={`settings-dependent-branch !border-t-0 !py-0 ${open ? 'settings-dependent-branch-open' : ''} ${className}`.trim()}
      data-expanded={open}
      aria-hidden={!open}
    >
      <div className="settings-dependent-branch-clip">
        <div className="settings-dependent-branch-content">
          {children}
        </div>
      </div>
    </div>
  );
}
