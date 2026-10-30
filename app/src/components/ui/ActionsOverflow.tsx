import { Children, isValidElement, type ButtonHTMLAttributes, type ReactNode } from 'react';
import { Menu } from '@base-ui/react/menu';
import { AnimatedDropdown, AnimatedDropdownContent, AnimatedDropdownTrigger } from './animated-dropdown/animated-dropdown';

/** Secondary detail actions share the existing menu, including arrow-key focus. */
export function ActionsOverflow({ label, children, keepOpen = false }: { label: string; children: ReactNode; keepOpen?: boolean }) {
  return <AnimatedDropdown>
    <AnimatedDropdownTrigger aria-label={label} className="settings-quiet-btn px-2 py-1.5 text-xs">···</AnimatedDropdownTrigger>
    <AnimatedDropdownContent align="end" className="flex min-w-40 flex-col items-stretch gap-1 p-2">
      {Children.map(children, (child) => isValidElement<ButtonHTMLAttributes<HTMLButtonElement>>(child)
        ? <Menu.Item render={child} nativeButton disabled={child.props.disabled} closeOnClick={!keepOpen}
          onKeyDownCapture={(event) => {
            if (keepOpen && event.key === 'Enter') { event.preventDefault(); event.stopPropagation(); }
          }} />
        : child)}
    </AnimatedDropdownContent>
  </AnimatedDropdown>;
}
