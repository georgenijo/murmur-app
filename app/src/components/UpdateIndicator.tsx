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
        data-testid="update-indicator"
        role="status"
        aria-label="Checking for updates"
        title="Checking for updates"
        className="ui-icon-button ml-auto bg-surface-container-low"
      >
        <span aria-hidden="true" className="h-1.5 w-1.5 animate-pulse rounded-full bg-primary" />
      </span>
    );
  }

  if (status.phase === 'up-to-date') {
    return <span role="status" className="ui-visually-hidden">Murmur is up to date</span>;
  }

  if (status.phase === 'error') {
    const installFailed = status.stage === 'install';
    return (
      <button
        data-testid="update-indicator"
        type="button"
        onClick={installFailed ? onOpen : onRetryCheck}
        title={installFailed ? 'Update needs attention' : 'Update check failed — retry'}
        className="ui-icon-button ml-auto bg-error/10 font-bold text-error hover:bg-error/15 focus:outline-none focus-visible:ring-2 focus-visible:ring-error"
        aria-label={installFailed ? 'Update installation needs attention' : 'Update check failed. Retry'}
      >
        <span aria-hidden="true">!</span>
      </button>
    );
  }

  if (status.phase !== 'available') return null;

  return (
    <button
      data-testid="update-indicator"
      type="button"
      onClick={onOpen}
      title={`Murmur ${status.version} is available`}
      className="ui-icon-button ml-auto bg-primary/10 hover:bg-primary/15 focus:outline-none focus-visible:ring-2 focus-visible:ring-primary"
      aria-label={`Murmur ${status.version} is available. View update`}
    >
      <span aria-hidden="true" className="h-1.5 w-1.5 rounded-full bg-primary" />
    </button>
  );
}
