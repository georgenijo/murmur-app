import { MAIN_WINDOW_SHORTCUTS } from '../../lib/keyboardShortcuts';
import {
  pasteLastShortcutLabel,
  recordingShortcutHint,
  type Settings,
} from '../../lib/settings';
import { SettingsSection } from './SettingsSection';

export type GlobalShortcutId =
  | 'recording'
  | 'paste-last'
  | 'correction';

export type ShortcutOwnerDestination =
  | { shortcutId: 'recording'; page: 'recording'; target: 'trigger-key' }
  | { shortcutId: 'paste-last'; page: 'delivery'; target: 'paste-last-shortcut' }

interface KeyboardShortcutsSettingsProps {
  settings: Settings;
  activePage: string;
  onOpenOwner: (destination: ShortcutOwnerDestination) => void;
}

interface GlobalShortcutRow {
  destination: ShortcutOwnerDestination;
  label: string;
  binding:
    | { kind: 'on'; label: string }
    | { kind: 'paused'; label: string }
    | { kind: 'off'; label: string | null };
  ownerLabel: string;
}

function configuredBinding(label: string, paused: boolean): GlobalShortcutRow['binding'] {
  return paused ? { kind: 'paused', label } : { kind: 'on', label };
}

function globalShortcutRows(settings: Settings): readonly GlobalShortcutRow[] {
  const pasteLast = pasteLastShortcutLabel(settings.pasteLastShortcut);

  return [
    {
      destination: { shortcutId: 'recording', page: 'recording', target: 'trigger-key' },
      label: 'Recording trigger',
      binding: configuredBinding(
        recordingShortcutHint(settings.recordingMode, settings.doubleTapKey),
        settings.disabled,
      ),
      ownerLabel: 'Recording',
    },
    {
      destination: { shortcutId: 'paste-last', page: 'delivery', target: 'paste-last-shortcut' },
      label: 'Paste last or retry delivery',
      binding: pasteLast === undefined
        ? { kind: 'off', label: null }
        : configuredBinding(pasteLast, false),
      ownerLabel: 'Delivery',
    },
  ];
}

function ShortcutBinding({ label, state = 'on' }: { label: string | null; state?: GlobalShortcutRow['binding']['kind'] }) {
  const hold = label?.startsWith('Hold ');
  return <span className="settings-shortcut-binding">
    {state !== 'on' && <span className="settings-shortcut-state">{state === 'paused' ? 'Paused' : 'Off'}</span>}
    {hold && <span className="settings-shortcut-hold">Hold</span>}
    {label && <kbd className="settings-shortcut-key">{hold ? label.slice(5) : label}</kbd>}
  </span>;
}

export function KeyboardShortcutsSettings({
  settings,
  activePage,
  onOpenOwner,
}: KeyboardShortcutsSettingsProps) {
  const globalRows = globalShortcutRows(settings);

  return (
    <SettingsSection hideHeading
      card={false}
      pageId="shortcuts"
      activePage={activePage}
      title="Keyboard shortcuts"

    >
      <div data-setting-target="shortcuts" className="settings-stack">
        {settings.disabled && (
          <p role="status" className="settings-callout text-[length:var(--ui-font-caption)] leading-relaxed text-on-surface" data-tone="accent">
            Murmur is disabled, so recording shortcuts are paused. Paste last remains available, and every saved binding is shown below.
          </p>
        )}
        <section aria-labelledby="global-shortcuts-title">
          <h2 id="global-shortcuts-title" className="settings-group-heading">Global shortcuts</h2>
          <ul className="settings-card overflow-hidden" aria-label="Global shortcuts">
            {globalRows.map((row) => (
              <li
                key={row.destination.shortcutId}
                data-shortcut-id={row.destination.shortcutId}
                className="settings-shortcut-row"
              >
                <span className="settings-shortcut-label">{row.label}</span>
                <ShortcutBinding label={row.binding.label} state={row.binding.kind} />
                <button
                  type="button"
                  aria-label={`${row.binding.kind === 'off' ? 'Enable' : row.binding.kind === 'paused' ? 'View' : 'Configure'} in ${row.ownerLabel}`}
                  onClick={() => onOpenOwner(row.destination)}
                  className="settings-quiet-btn settings-shortcut-configure"
                >
                  Configure…
                </button>
              </li>
            ))}
          </ul>
        </section>

        <section aria-labelledby="window-shortcuts-title">
          <h2 id="window-shortcuts-title" className="settings-group-heading">Main-window shortcuts</h2>
          <p className="settings-shortcuts-context">Available while the main window is active</p>
          <ul className="settings-card overflow-hidden" aria-label="Main-window shortcuts">
            {MAIN_WINDOW_SHORTCUTS.map((shortcut) => (
              <li
                key={shortcut.action}
                className="settings-shortcut-row"
              >
                <span className="settings-shortcut-label">{shortcut.label}</span>
                <ShortcutBinding label={shortcut.binding} />
              </li>
            ))}
          </ul>
          <p className="mt-2 text-[length:var(--ui-font-caption)] leading-relaxed text-on-surface-variant">
            Control works in place of Command outside text fields.
          </p>
        </section>
      </div>
    </SettingsSection>
  );
}
