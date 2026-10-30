import { useEffect, useRef } from 'react';

/** The existing modal keyboard pattern: enter, contain Tab, cancel, restore. */
export function useDialogFocus(onCancel: () => void, restoreFocus?: () => HTMLElement | null) {
  const dialogRef = useRef<HTMLDivElement>(null);
  const cancelRef = useRef(onCancel);
  const restoreRef = useRef(restoreFocus);
  cancelRef.current = onCancel;
  restoreRef.current = restoreFocus;

  useEffect(() => {
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const dialog = dialogRef.current;
    const focusable = () => Array.from(dialog?.querySelectorAll<HTMLElement>(
      'a[href], button, input, select, textarea, [tabindex]',
    ) ?? []).filter(element => element.tabIndex >= 0
      && !element.matches(':disabled, [hidden], input[type="hidden"]')
      && !element.closest('[hidden], [inert], [aria-hidden="true"]'));
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.isComposing) return;
      if (event.key === 'Escape') {
        event.preventDefault();
        cancelRef.current();
      } else if (event.key === 'Tab') {
        const elements = focusable();
        const index = elements.indexOf(document.activeElement as HTMLElement);
        event.preventDefault();
        const next = event.shiftKey
          ? (index <= 0 ? elements.length - 1 : index - 1)
          : (index + 1) % elements.length;
        (elements[next] ?? dialog)?.focus();
      }
    };
    document.addEventListener('keydown', onKeyDown);
    (focusable()[0] ?? dialog)?.focus();
    return () => {
      document.removeEventListener('keydown', onKeyDown);
      // Menu items can stay mounted through their exit animation after the
      // dialog has opened. Restore the caller's stable trigger, not that item.
      const restoreOpener = opener?.isConnected && opener !== document.body
        && !opener.closest('[role="menu"], [hidden], [inert], [aria-hidden="true"]');
      (restoreOpener ? opener : restoreRef.current?.())?.focus();
    };
  }, []);
  return dialogRef;
}
