import type { ReactNode } from 'react';

/**
 * Layout primitives for settings cards. Spacing is owned by the containers in
 * `styles/settings.css` (see "settings rhythm"), never by the settings inside
 * them, so these components add no outer margin, padding, or dividers.
 */

const FLASH_RING = 'rounded-lg transition-shadow [&.settings-target-flash]:ring-2 [&.settings-target-flash]:ring-primary/40';

/** A tinted note inside a settings card. Tint only: the card's dividers are
 *  the only lines, so a callout never adds a border of its own. */
export function SettingsCallout({ title, children, tone = 'neutral', targetId }: {
  title: string;
  children: ReactNode;
  tone?: 'neutral' | 'accent' | 'warning';
  targetId?: string;
}) {
  return (
    <div data-setting-target={targetId} className={FLASH_RING}>
      <div className="settings-callout settings-title-description" data-tone={tone}>
        <p className="text-sm font-medium text-on-surface">{title}</p>
        <p className="mt-0.5 text-xs leading-relaxed text-on-surface">{children}</p>
      </div>
    </div>
  );
}

/** A collapsed "Advanced" group. `rows` lays its children out like card rows
 *  (divider between each); `stack` spaces them without dividers. */
export function SettingsDisclosure({ title, description, layout, targetId, children }: {
  title: string;
  description?: string;
  layout: 'rows' | 'stack';
  targetId?: string;
  children: ReactNode;
}) {
  return (
    <details data-setting-target={targetId} className={`group ${FLASH_RING}`}>
      <summary className="flex cursor-pointer list-none items-center justify-between rounded-lg text-sm font-semibold text-on-surface focus:outline-none focus-visible:ring-2 focus-visible:ring-primary">
        {title}
        <span aria-hidden="true" className="text-on-surface-variant transition-transform group-open:rotate-180">⌄</span>
      </summary>
      {description && <p className="mt-0.5 text-xs leading-relaxed text-on-surface-variant">{description}</p>}
      <div className={layout === 'rows' ? 'settings-rows settings-rows-nested' : 'settings-stack mt-3'}>
        {children}
      </div>
    </details>
  );
}
