import type { OverlayGeometry } from '../../lib/overlayGeometry';

function PowerIcon({ stroke }: { stroke: string }) {
  return (
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke={stroke} strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M12 2v10" />
      <path d="M18.4 6.6a9 9 0 1 1-12.8 0" />
    </svg>
  );
}

function ClipboardPasteIcon({ stroke }: { stroke: string }) {
  return (
    <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke={stroke} strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <rect x="8" y="2" width="8" height="4" rx="1" />
      <path d="M16 4h2a2 2 0 0 1 2 2v4" />
      <path d="M8 4H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h5" />
      <path d="M16 14v6" />
      <path d="M13 17h6" />
    </svg>
  );
}

function SlidersIcon({ stroke }: { stroke: string }) {
  return (
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke={stroke} strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M4 7h4" />
      <path d="M12 7h8" />
      <circle cx="10" cy="7" r="2" />
      <path d="M4 17h10" />
      <path d="M18 17h2" />
      <circle cx="16" cy="17" r="2" />
    </svg>
  );
}

interface OverlayDropdownProps {
  geometry: OverlayGeometry;
  expanded: boolean;
  disabled: boolean;
  autoPaste: boolean;
  fileOutputEnabled: boolean;
  onToggleDisabled: (e: React.MouseEvent) => void;
  onToggleAutoPaste: (e: React.MouseEvent) => void;
  onOpenSettings: (e: React.MouseEvent) => void;
}

/** The three quick-settings buttons revealed on hover-expand. */
export function OverlayDropdown({
  geometry,
  expanded,
  disabled,
  autoPaste,
  fileOutputEnabled,
  onToggleDisabled,
  onToggleAutoPaste,
  onOpenSettings,
}: OverlayDropdownProps) {
  const effectiveAutoPaste = autoPaste && !fileOutputEnabled;
  const autoPastePaused = autoPaste && fileOutputEnabled;
  const autoPasteLabel = autoPastePaused
    ? 'Auto-paste paused while saving files'
    : effectiveAutoPaste
      ? 'Disable auto-paste'
      : 'Enable auto-paste';
  const autoPasteColor = effectiveAutoPaste
    ? '#10b981'
    : autoPastePaused
      ? '#f59e0b'
      : 'rgba(255,255,255,0.85)';
  const autoPasteBackground = effectiveAutoPaste
    ? 'rgba(16,185,129,0.16)'
    : autoPastePaused
      ? 'rgba(245,158,11,0.14)'
      : 'rgba(255,255,255,0.06)';

  return (
    <div
      className="flex items-center justify-center gap-3"
      style={{
        height: geometry.dropdownH,
        padding: '0 10px 6px',
        opacity: expanded ? 1 : 0,
        pointerEvents: expanded ? 'auto' : 'none',
        transition: 'opacity 200ms ease',
        transitionDelay: expanded ? '100ms' : '0ms',
      }}
    >
      {/* Global disable */}
      <button
        type="button"
        aria-label={disabled ? 'Enable Murmur' : 'Disable Murmur'}
        onClick={onToggleDisabled}
        className="shrink-0 flex items-center justify-center cursor-pointer rounded-[9px] transition-colors"
        style={{ width: 26, height: 26, background: disabled ? 'rgba(239,68,68,0.12)' : 'rgba(255,255,255,0.06)' }}
      >
        <PowerIcon stroke={disabled ? '#ef4444' : 'rgba(255,255,255,0.85)'} />
      </button>

      {/* Auto-paste */}
      <button
        type="button"
        role="switch"
        aria-checked={effectiveAutoPaste}
        aria-label={autoPasteLabel}
        title={autoPasteLabel}
        onClick={onToggleAutoPaste}
        className="shrink-0 flex items-center justify-center cursor-pointer rounded-[9px] transition-colors"
        style={{ width: 26, height: 26, opacity: disabled ? 0.35 : 1, background: autoPasteBackground }}
      >
        <ClipboardPasteIcon stroke={autoPasteColor} />
      </button>

      {/* Open settings */}
      <button
        type="button"
        aria-label="Open settings"
        onClick={onOpenSettings}
        className="shrink-0 flex items-center justify-center cursor-pointer rounded-[9px] transition-colors"
        style={{ width: 26, height: 26, background: 'rgba(255,255,255,0.06)' }}
      >
        <SlidersIcon stroke="rgba(255,255,255,0.85)" />
      </button>
    </div>
  );
}
