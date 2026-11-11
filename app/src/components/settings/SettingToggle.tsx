import { SettingsRow } from './SettingsRow';
import AnimatedSwitch from '../ui/animated-switch/animated-switch';

/** Shared toggle switch used by every settings page. */
export function Toggle({ label, checked, onChange, disabled = false }: {
  label: string;
  checked: boolean;
  onChange: () => void;
  disabled?: boolean;
}) {
  return (
    <AnimatedSwitch
      size="md"
      checked={checked}
      disabled={disabled}
      aria-label={label}
      onCheckedChange={() => onChange()}
    />
  );
}

/** A shared settings row with an optional reset to its declared default. */
export function SettingToggle({ title, description, label = title, checked, onChange, disabled = false, targetId, defaultChecked, compact = false }: {
  title: string;
  description?: string;
  label?: string;
  checked: boolean;
  onChange: () => void;
  disabled?: boolean;
  targetId?: string;
  defaultChecked?: boolean;
  compact?: boolean;
}) {
  return <SettingsRow title={title} description={description} compact={compact} targetId={targetId} disabled={disabled}
    onReset={defaultChecked !== undefined && checked !== defaultChecked ? onChange : undefined}
    control={<Toggle label={label} checked={checked} onChange={onChange} disabled={disabled} />} />;
}
