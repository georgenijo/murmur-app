import { useState, type ButtonHTMLAttributes } from 'react';
import { noDefaultButtonKeyDown } from '../../lib/defaultButton';

type Props = Omit<ButtonHTMLAttributes<HTMLButtonElement>, 'onClick' | 'children'> & {
  label: string;
  confirmLabel?: string;
  onConfirm: () => void;
};

/** A deliberate second activation; blur/Escape disarm it. Return never deletes. */
export function ConfirmActionButton({ label, confirmLabel = 'Confirm delete', onConfirm, ...props }: Props) {
  const [armed, setArmed] = useState(false);
  const text = `${(armed ? confirmLabel : label).replace(/…$/, '')}…`;
  return <button {...props} type="button" aria-label={props['aria-label'] ? `${armed ? 'Confirm ' : ''}${props['aria-label'].replace(/…$/, '').replace(/^(Delete|Remove|Clear)/, (word) => armed ? word.toLowerCase() : word)}…` : undefined}
    onBlur={(event) => { setArmed(false); props.onBlur?.(event); }}
    onKeyDown={(event) => { noDefaultButtonKeyDown(event); if (event.key === 'Escape') setArmed(false); }}
    onClick={() => { if (armed) { setArmed(false); onConfirm(); } else setArmed(true); }}>
    {text}
  </button>;
}
