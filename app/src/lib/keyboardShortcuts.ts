/**
 * Main-window keyboard shortcuts.
 *
 * Kept as a pure event→action mapping so the bindings (and everything they
 * deliberately refuse to swallow) are testable without a DOM.
 */

export type MainWindowShortcut = 'palette' | 'search' | 'settings' | 'logs';

/** The subset of `KeyboardEvent` the mapping reads. */
export interface ShortcutEvent {
  key: string;
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
}

/** True when the event came from somewhere the user is typing. */
export function isEditableTarget(target: EventTarget | null): boolean {
  if (!target || !(target instanceof Element)) return false;
  return target.closest('input, textarea, select, [contenteditable=""], [contenteditable="true"]') !== null;
}

/**
 * Map a keydown to a main-window action, or `null` when it should pass through.
 *
 * Command (or Control, for a keyboard-only setup) plus a single letter. Adding
 * Option or Shift always passes through: those combinations belong to the
 * focused control, and Murmur must not shadow text-editing shortcuts.
 *
 * `inEditableField` narrows this further. macOS text fields bind Control+letter
 * to emacs-style editing (⌃F forward, ⌃K kill-to-end-of-line), so while the
 * user is typing only the Command form is accepted — otherwise the shortcuts
 * would eat those bindings inside the search box, settings fields, and the
 * knowledge editor.
 */
export function mainWindowShortcut(
  event: ShortcutEvent,
  inEditableField = false,
): MainWindowShortcut | null {
  if (!(event.metaKey || event.ctrlKey)) return null;
  if (event.altKey || event.shiftKey) return null;
  if (inEditableField && !event.metaKey) return null;
  switch (event.key.toLowerCase()) {
    case 'k': return 'palette';
    case 'f': return 'search';
    case ',': return 'settings';
    case 'l': return 'logs';
    default: return null;
  }
}
