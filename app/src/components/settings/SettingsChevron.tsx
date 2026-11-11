import { ChevronDown, ChevronRight } from 'lucide-react';

/** The one chevron every Settings row uses, so navigation rows and Advanced
 *  disclosures render at the same size (`.settings-chevron`, 16px). */
export function SettingsChevron({ direction, className = '' }: { direction: 'right' | 'down'; className?: string }) {
  const Icon = direction === 'right' ? ChevronRight : ChevronDown;
  return <Icon aria-hidden="true" className={`settings-chevron ${className}`.trim()} />;
}
