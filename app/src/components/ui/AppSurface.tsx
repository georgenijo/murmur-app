import type { HTMLAttributes } from 'react';

/** Shared font owner for the three production workspace roots. Child code
 * blocks can still choose mono; a root utility cannot replace the app stack. */
export function AppSurface({ surface, className = '', style, ...props }:
  HTMLAttributes<HTMLDivElement> & { surface: 'main' | 'onboarding' | 'diagnostics' }) {
  return <div {...props} data-app-surface={surface} className={`font-sans ${className}`}
    style={{ ...style, fontFamily: 'var(--ui-font-sans)' }} />;
}
