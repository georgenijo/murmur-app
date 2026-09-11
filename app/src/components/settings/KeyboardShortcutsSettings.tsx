import { MAIN_WINDOW_SHORTCUTS } from '../../lib/keyboardShortcuts';
import {
  QUERY_KEY_OPTIONS,
  TRANSFORM_KEY_OPTIONS,
  pasteLastShortcutLabel,
  recordingShortcutHint,
  type Settings,
} from '../../lib/settings';
import { SettingsSection } from './SettingsSection';

export type GlobalShortcutId =
  | 'recording'
  | 'transform'
  | 'voice-query'
  | 'paste-last'
  | 'correction';

export type ShortcutOwnerDestination =
  | { shortcutId: 'recording'; page: 'recording'; target: 'trigger-key' }
  | { shortcutId: 'transform'; page: 'ai-transform'; target: 'transform-shortcut' }
  | { shortcutId: 'voice-query'; page: 'ai-query'; target: 'voice-query-shortcut' }
  | { shortcutId: 'paste-last'; page: 'delivery'; target: 'paste-last-shortcut' }
  | { shortcutId: 'correction'; page: 'ai-transform'; target: 'correction-shortcut' };

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

function keyOptionLabel(
  key: Settings['transformHoldKey'] | Settings['queryHotkey'],
): string | null {
  if (key === null) return null;
  return TRANSFORM_KEY_OPTIONS.find((option) => option.value === key)?.label
    ?? QUERY_KEY_OPTIONS.find((option) => option.value === key)?.label
    ?? null;
}

function globalShortcutRows(settings: Settings): readonly GlobalShortcutRow[] {
  const transformKey = keyOptionLabel(settings.transformHoldKey);
  const queryKey = keyOptionLabel(settings.queryHotkey);
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
      destination: { shortcutId: 'transform', page: 'ai-transform', target: 'transform-shortcut' },
      label: 'Selected-text transform',
      binding: transformKey === null
        ? { kind: 'off', label: null }
        : configuredBinding(`Hold ${transformKey}`, settings.disabled),
      ownerLabel: 'Selected-Text Rewrite',
    },
    {
      destination: { shortcutId: 'voice-query', page: 'ai-query', target: 'voice-query-shortcut' },
      label: 'Voice Query',
      binding: queryKey === null
        ? { kind: 'off', label: null }
        : configuredBinding(`Double-tap ${queryKey}`, settings.disabled),
      ownerLabel: 'Voice Query',
    },
    {
      destination: { shortcutId: 'paste-last', page: 'delivery', target: 'paste-last-shortcut' },
      label: 'Paste Last / Retry Delivery',
      binding: pasteLast === undefined
        ? { kind: 'off', label: null }
        : configuredBinding(pasteLast, false),
      ownerLabel: 'Delivery',
    },
    {
      destination: { shortcutId: 'correction', page: 'ai-transform', target: 'correction-shortcut' },
      label: 'Correct last dictation',
      binding: settings.correctionShortcutEnabled
        ? configuredBinding('⌘⇧E', settings.disabled)
        : { kind: 'off', label: '⌘⇧E' },
      ownerLabel: 'Selected-Text Rewrite',
    },
  ];
}

function StatusBadge({ state }: { state: GlobalShortcutRow['binding']['kind'] }) {
  const label = state === 'on' ? 'On' : state === 'paused' ? 'Paused' : 'Off';
  return (
    <span
      className={`rounded-(--ui-radius-pill) px-2 py-0.5 text-xs font-semibold ${state === 'on' ? 'bg-success/10 text-success' : 'bg-surface-container-high text-on-surface-variant'}`}
    >
      {label}
    </span>
  );
}

export function KeyboardShortcutsSettings({
  settings,
  activePage,
  onOpenOwner,
}: KeyboardShortcutsSettingsProps) {
  const globalRows = globalShortcutRows(settings);

  return (
    <SettingsSection
      card={false}
      pageId="shortcuts"
      activePage={activePage}
      title="Keyboard Shortcuts"
      subtitle="See every Murmur shortcut in one place. Change global shortcuts on their owning settings pages."
    >
      <div data-setting-target="shortcuts" className="settings-stack">
        {settings.disabled && (
          <p role="status" className="settings-callout text-xs leading-relaxed text-on-surface" data-tone="accent">
            Murmur is disabled, so recording, transform, Voice Query, and correction shortcuts are paused. Paste Last remains available, and every saved binding is shown below.
          </p>
        )}
        <section aria-labelledby="global-shortcuts-title">
          <h2 id="global-shortcuts-title" className="mb-2 text-sm font-semibold text-on-surface">Global shortcuts</h2>
          <ul className="settings-card overflow-hidden" aria-label="Global shortcuts">
            {globalRows.map((row) => (
              <li
                key={row.destination.shortcutId}
                data-shortcut-id={row.destination.shortcutId}
                className="flex min-h-16 items-center gap-4 border-b border-outline-variant/15 px-4 py-3 last:border-b-0"
              >
                <span className="min-w-0 flex-1">
                  <span className="flex items-center gap-2">
                    <span className="text-sm font-medium text-on-surface">{row.label}</span>
                    <StatusBadge state={row.binding.kind} />
                  </span>
                  <span className="mt-1 block text-xs text-on-surface-variant">
                    {row.binding.label ?? 'Not assigned'}
                  </span>
                </span>
                <button
                  type="button"
                  onClick={() => onOpenOwner(row.destination)}
                  className="settings-quiet-btn shrink-0 px-3 py-1.5 text-xs font-medium text-on-surface"
                >
                  {row.binding.kind === 'off' ? 'Enable' : row.binding.kind === 'paused' ? 'View' : 'Configure'} in {row.ownerLabel}
                </button>
              </li>
            ))}
          </ul>
        </section>

        <section aria-labelledby="window-shortcuts-title">
          <h2 id="window-shortcuts-title" className="mb-2 text-sm font-semibold text-on-surface">Main-window shortcuts</h2>
          <ul className="settings-card overflow-hidden" aria-label="Main-window shortcuts">
            {MAIN_WINDOW_SHORTCUTS.map((shortcut) => (
              <li
                key={shortcut.action}
                className="flex min-h-14 items-center gap-4 border-b border-outline-variant/15 px-4 py-3 last:border-b-0"
              >
                <span className="min-w-0 flex-1 text-sm font-medium text-on-surface">{shortcut.label}</span>
                <StatusBadge state="on" />
                <kbd className="min-w-12 rounded-md border border-outline-variant/30 bg-surface-container-lowest px-2 py-1 text-center text-xs font-semibold text-on-surface">
                  {shortcut.binding}
                </kbd>
              </li>
            ))}
          </ul>
          <p className="mt-2 text-xs leading-relaxed text-on-surface-variant">
            Control works in place of Command outside text fields. Main-window shortcuts are built in and available while the main window is focused.
          </p>
        </section>
      </div>
    </SettingsSection>
  );
}
