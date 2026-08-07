import type { HTMLAttributes, ReactNode } from 'react';

interface WindowHeaderProps extends HTMLAttributes<HTMLElement> {
  contextLabel?: string;
  children?: ReactNode;
}

/**
 * Shared native-overlay title bar. It owns the macOS traffic-light inset and
 * chrome height so feature screens cannot accidentally create a second row.
 */
export function WindowHeader({
  contextLabel,
  children,
  className = '',
  ...props
}: WindowHeaderProps) {
  return (
    <header
      data-tauri-drag-region
      className={`ui-window-header bg-background/95 backdrop-blur-xl ${className}`}
      {...props}
    >
      <div data-tauri-drag-region className="ui-window-header-content">
        <span data-tauri-drag-region className="ui-window-wordmark">Murmur</span>
        {contextLabel && (
          <span
            data-tauri-drag-region
            className="select-none text-xs font-medium text-on-surface-variant"
          >
            {contextLabel}
          </span>
        )}
        {children}
      </div>
    </header>
  );
}
