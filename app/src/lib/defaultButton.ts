/**
 * Return/Space rule for dialogs (docs/features/ui-design-system.md, "Copy and
 * interaction"): Space presses the focused button, Return presses only the
 * dialog's default button. A dialog whose confirmation is irreversible passes
 * `null`, so Return does nothing there instead of pressing whichever button
 * happens to hold focus.
 *
 * Returns true when the key was a plain Return that this rule consumed.
 */
export function returnPressesDefault(
  event: Pick<KeyboardEvent, 'key' | 'shiftKey' | 'metaKey' | 'ctrlKey' | 'altKey' | 'isComposing' | 'target' | 'repeat' | 'preventDefault'>,
  pressDefault: (() => void) | null,
): boolean {
  if (event.key !== 'Enter' || event.isComposing) return false;
  if (event.shiftKey || event.metaKey || event.ctrlKey || event.altKey) return false;
  const target = event.target;
  // Multi-line fields keep Return for new lines; native selects own their keys.
  if (target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement) return false;
  if (target instanceof HTMLElement && target.isContentEditable) return false;
  event.preventDefault();
  if (!event.repeat) pressDefault?.();
  return true;
}


/** Mark only recoverable confirmation surfaces with data-default-scope and
 * their confirming button with data-default-button. Standing pane actions and
 * irreversible confirmations deliberately have no default. */
export function defaultButtonKeyDown(event: {
  nativeEvent: KeyboardEvent;
  defaultPrevented: boolean;
}): void {
  if (event.defaultPrevented || event.nativeEvent.defaultPrevented) return;
  const native = event.nativeEvent;
  const target = native.target;
  if (!(target instanceof HTMLElement)) return;
  // Menus and listboxes own selection keys, rather than a dialog default.
  if (target.closest('a[href], [role="menu"], [role="listbox"], [role="combobox"]')) return;
  const scope = target.closest('[role="dialog"], [data-default-scope]');
  const button = scope?.querySelector<HTMLButtonElement>('button[data-default-button]');
  returnPressesDefault(native, button && !button.disabled && !native.repeat
    ? () => button.click() : null);
}


export function noDefaultButtonKeyDown(event: { nativeEvent: KeyboardEvent }): void {
  returnPressesDefault(event.nativeEvent, null);
}
