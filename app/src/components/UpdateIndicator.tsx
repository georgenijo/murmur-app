import type { UpdateStatus } from '../lib/updater';

interface UpdateIndicatorProps {
  status: UpdateStatus;
  onOpen: () => void;
  onRetryCheck: () => void;
}

export function UpdateIndicator({ status, onOpen, onRetryCheck }: UpdateIndicatorProps) {
  if (status.phase === 'checking') {
    return (
      <span
        role="status"
        className="ml-auto inline-flex items-center gap-2 rounded-full bg-surface-container-low px-3 py-1.5 text-xs font-medium text-on-surface-variant"
      >
        <span aria-hidden="true" className="h-1.5 w-1.5 animate-pulse rounded-full bg-primary" />
        Checking for updates…
      </span>
    );
  }

  if (status.phase === 'up-to-date') {
    return (
      <span
        role="status"
        className="ml-auto inline-flex items-center gap-2 rounded-full bg-success/10 px-3 py-1.5 text-xs font-medium text-success"
      >
        <span aria-hidden="true">✓</span>
        Murmur is up to date
      </span>
    );
  }

  if (status.phase === 'error') {
    return (
      <button
        type="button"
        onClick={onRetryCheck}
        className="ml-auto inline-flex items-center gap-2 rounded-full bg-error/10 px-3 py-1.5 text-xs font-semibold text-error transition-colors hover:bg-error/15 focus:outline-none focus-visible:ring-2 focus-visible:ring-error"
        aria-label="Update check failed. Retry"
      >
        Update check failed · Retry
      </button>
    );
  }

  if (status.phase !== 'available') return null;

  return (
    <button
      type="button"
      onClick={onOpen}
      className="ml-auto inline-flex items-center gap-2 rounded-full bg-primary/10 px-3 py-1.5 text-xs font-semibold text-on-surface transition-colors hover:bg-primary/15 focus:outline-none focus-visible:ring-2 focus-visible:ring-primary"
      aria-label={`Murmur ${status.version} is available. View update`}
    >
      <span aria-hidden="true" className="h-1.5 w-1.5 rounded-full bg-primary" />
      Update available · v{status.version.replace(/^v/, '')}
    </button>
  );
}
