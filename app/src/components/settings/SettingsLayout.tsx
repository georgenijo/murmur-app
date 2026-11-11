import { ChevronDown } from 'lucide-react';
import { createContext, useContext, useId, useRef, type ReactNode } from 'react';
import { useSettingsAdvanced } from '../../lib/hooks/useSettingsAdvanced';
import { SettingsCard } from './SettingsRow';
import { SettingsBranch } from './SettingsBranch';

export const SettingsDisclosureContext = createContext<{ open: boolean; setOpen: (open: boolean) => void } | null>(null);

/**
 * Layout primitives for settings cards. Spacing is owned by the containers in
 * `styles/settings.css` (see "settings rhythm"), never by the settings inside
 * them, so these components add no outer margin, padding, or dividers.
 */

const FLASH_RING = 'rounded-lg transition-shadow [&.settings-target-flash]:ring-2 [&.settings-target-flash]:ring-primary/40';

/** A named group with the same heading and card treatment on every page. */
export function SettingsGroup({ title, children }: { title: string; children: ReactNode }) {
  const id = useId();
  return <section aria-labelledby={id}>
    <h2 id={id} className="settings-group-heading">{title}</h2>
    <SettingsCard>{children}</SettingsCard>
  </section>;
}

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
export function SettingsDisclosure({ title, description, layout, targetId, children, pane, open: controlledOpen, onOpenChange }: {
  title: string;
  description?: string;
  layout: 'rows' | 'stack';
  pane?: string;
  targetId?: string;
  children: ReactNode;
  /** Use local UI state for compact disclosures that do not belong to a pane's Advanced state. */
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
}) {
  const context = useContext(SettingsDisclosureContext);
  const [ownOpen, setOwnOpen] = useSettingsAdvanced(pane ?? '');
  const state = controlledOpen !== undefined && onOpenChange
    ? { open: controlledOpen, setOpen: onOpenChange }
    : context ?? { open: ownOpen, setOpen: setOwnOpen };
  const buttonRef = useRef<HTMLButtonElement>(null);
  const contentId = useId();
  const open = state?.open ?? false;
  return (
    <div data-setting-target={targetId} className="settings-advanced">
      <button ref={buttonRef} type="button" aria-expanded={open} aria-controls={contentId}
        className="settings-advanced-toggle" onClick={() => state?.setOpen(!open)}>
        <span className="settings-advanced-copy"><span>{title}</span>
          {description && <span className="settings-advanced-description">{description}</span>}
        </span>
        <ChevronDown aria-hidden="true" className="settings-advanced-chevron" />
      </button>
      <SettingsBranch id={contentId} open={open} unmountOnClose restoreFocus={() => buttonRef.current}
        className={layout === 'rows' ? 'settings-advanced-rows' : 'settings-advanced-stack settings-branch-stack'}>
        {children}
      </SettingsBranch>
    </div>
  );
}
