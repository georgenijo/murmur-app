import { useState, useId, useCallback } from 'react';

interface SettingsSectionProps {
  title: string;
  subtitle?: string;
  defaultExpanded?: boolean;
  children: React.ReactNode;
  // Page mode: when `pageId` is set, the section renders as a flat, non-collapsible
  // settings page (static heading + body) and is shown only when activePage === pageId.
  // Used by the two-pane settings layout where the left nav replaces accordions.
  pageId?: string;
  activePage?: string;
}

export function SettingsSection({ title, subtitle, defaultExpanded = true, children, pageId, activePage }: SettingsSectionProps) {
  const [expanded, setExpanded] = useState(defaultExpanded);
  const [overflowVisible, setOverflowVisible] = useState(defaultExpanded);
  const contentId = useId();
  const headerId = useId();

  const handleToggle = useCallback(() => {
    setExpanded((prev) => {
      if (prev) setOverflowVisible(false);
      return !prev;
    });
  }, []);

  const handleTransitionEnd = useCallback(() => {
    if (expanded) setOverflowVisible(true);
  }, [expanded]);

  // Page mode: flat, always-open page gated by the active category. Hooks above
  // still run (so order is stable) but the accordion chrome is skipped.
  if (pageId !== undefined) {
    if (activePage !== undefined && activePage !== pageId) return null;
    return (
      <div>
        <h1 className="text-base font-semibold text-on-surface">{title}</h1>
        {subtitle && (
          <p className="mt-0.5 text-xs text-on-surface-variant">{subtitle}</p>
        )}
        <div className="pt-4 space-y-4">{children}</div>
      </div>
    );
  }

  return (
    <div className="mb-2 last:mb-0">
      <button
        type="button"
        id={headerId}
        aria-expanded={expanded}
        aria-controls={contentId}
        onClick={handleToggle}
        className="flex w-full items-center justify-between py-3 text-sm font-semibold text-on-surface focus:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-1 rounded-sm"
      >
        <span className="text-left">
          {title}
          {subtitle && !expanded && (
            <span className="block text-xs font-normal text-on-surface-variant">{subtitle}</span>
          )}
        </span>
        <svg
          className={`w-4 h-4 text-on-surface-variant shrink-0 transition-transform duration-200 ${
            expanded ? 'rotate-180' : ''
          }`}
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
        >
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
        </svg>
      </button>
      <div
        id={contentId}
        role="region"
        aria-labelledby={headerId}
        className={`grid transition-[grid-template-rows] duration-200 ${
          expanded ? 'grid-rows-[1fr]' : 'grid-rows-[0fr]'
        }`}
        onTransitionEnd={handleTransitionEnd}
      >
        <div className={overflowVisible ? 'overflow-visible' : 'overflow-hidden'}>
          <div className="pb-4 space-y-4">
            {children}
          </div>
        </div>
      </div>
    </div>
  );
}
