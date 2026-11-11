import { Button as BaseButton } from '@base-ui/react/button';
import { Input as BaseInput } from '@base-ui/react/input';
import { forwardRef } from 'react';

/** Shared control geometry. Colors always come from the active appearance. */
export const Button = forwardRef<HTMLButtonElement, BaseButton.Props & {
  size?: 'default' | 'sm' | 'xs';
  variant?: 'outline' | 'primary' | 'ghost';
}>(({ size = 'default', variant = 'outline', className = '', ...props }, ref) => (
  <BaseButton ref={ref} {...props} data-ui-button data-size={size} data-variant={variant} className={`ui-form-button ${className}`} />
));
Button.displayName = 'Button';

export const Input = forwardRef<HTMLInputElement, BaseInput.Props>(({ className = '', ...props }, ref) => (
  <BaseInput ref={ref} {...props} className={`ui-form-input ${className}`} />
));
Input.displayName = 'Input';
