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

/** Title + description row paired with a `Toggle`, with the shared
 *  settings-target flash treatment used by search/palette navigation. */
export function SettingToggle({ title, description, label = title, checked, onChange, disabled = false, targetId }: {
  title: string;
  description: string;
  label?: string;
  checked: boolean;
  onChange: () => void;
  disabled?: boolean;
  targetId?: string;
}) {
  return (
    <div
      data-setting-target={targetId}
      className="flex min-h-[52px] items-center justify-between gap-6 rounded-lg px-1 transition-shadow [&.settings-target-flash]:ring-2 [&.settings-target-flash]:ring-primary/40"
    >
      <div>
        <p className="text-sm font-medium text-on-surface">{title}</p>
        <p className="mt-0.5 text-xs leading-relaxed text-on-surface-variant">{description}</p>
      </div>
      <Toggle label={label} checked={checked} onChange={onChange} disabled={disabled} />
    </div>
  );
}
