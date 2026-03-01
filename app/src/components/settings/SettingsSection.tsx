import { useState, useId, useCallback } from 'react';

interface SettingsSectionProps {
  title: string;
  subtitle?: string;
  defaultExpanded?: boolean;
  children: React.ReactNode;
}

export function SettingsSection({ title, subtitle, defaultExpanded = true, children }: SettingsSectionProps) {
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

  return (
    <div className="border-b border-stone-200 dark:border-stone-700">
      <button
        type="button"
        id={headerId}
        aria-expanded={expanded}
        aria-controls={contentId}
        onClick={handleToggle}
        className="flex w-full items-center justify-between py-3 text-sm font-semibold text-stone-900 dark:text-stone-100 focus:outline-none focus-visible:ring-2 focus-visible:ring-stone-500 focus-visible:ring-offset-1 rounded-sm"
      >
        <span className="text-left">
          {title}
          {subtitle && !expanded && (
            <span className="block text-xs font-normal text-stone-400 dark:text-stone-500">{subtitle}</span>
          )}
        </span>
        <svg
          className={`w-4 h-4 text-stone-400 shrink-0 transition-transform duration-200 ${
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
