import { Button as BaseButton } from '@base-ui/react/button';
import { Input as BaseInput } from '@base-ui/react/input';

/** Shared control geometry. Colors always come from the active appearance. */
export function Button({ size = 'default', variant = 'outline', className = '', ...props }: BaseButton.Props & {
  size?: 'default' | 'sm' | 'xs';
  variant?: 'outline' | 'primary' | 'ghost';
}) {
  return <BaseButton {...props} data-ui-button data-size={size} data-variant={variant} className={`ui-form-button ${className}`} />;
}

export function Input({ className = '', ...props }: BaseInput.Props) {
  return <BaseInput {...props} className={`ui-form-input ${className}`} />;
}
