import { useId, type ReactNode } from 'react';
import { defaultButtonKeyDown } from '../../lib/defaultButton';
import { useDialogFocus } from '../../lib/hooks/useDialogFocus';

export function ConfirmDialog({ title, children, confirmLabel, dangerous = false, disabled = false, onCancel, onConfirm, restoreFocus }: {
  title: string;
  children: ReactNode;
  confirmLabel: string;
  dangerous?: boolean;
  disabled?: boolean;
  onCancel: () => void;
  onConfirm: () => void;
  restoreFocus?: () => HTMLElement | null;
}) {
  const titleId = useId();
  const dialogRef = useDialogFocus(onCancel, restoreFocus);
  return <div className="dialog-backdrop fixed inset-0 z-50 flex items-center justify-center p-6">
    <div ref={dialogRef} tabIndex={-1} role="dialog" aria-modal="true" aria-labelledby={titleId}
      className="dialog-popover w-full max-w-md p-5" onKeyDown={defaultButtonKeyDown}>
      <h2 id={titleId} className="text-base font-semibold text-on-surface">{title}</h2>
      <div className="mt-2 text-sm text-on-surface-variant">{children}</div>
      <div className="mt-5 flex justify-end gap-2">
        <button type="button" onClick={onCancel} className="settings-quiet-btn px-3 py-2 text-xs text-on-surface">Cancel</button>
        <button type="button" data-default-button={!dangerous || undefined} disabled={disabled} onClick={onConfirm}
          className={`rounded-lg px-3 py-2 text-xs font-semibold disabled:opacity-40 ${dangerous ? 'border border-error/30 bg-error/10 text-error' : 'bg-primary text-on-primary'}`}>{confirmLabel}</button>
      </div>
    </div>
  </div>;
}
