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
        title="Checking for updates"
        className="ui-icon-button ml-auto bg-surface-container-low"
      >
        <span aria-hidden="true" className="h-1.5 w-1.5 animate-pulse rounded-full bg-primary" />
        <span className="sr-only">Checking for updates…</span>
      </span>
    );
  }

  if (status.phase === 'up-to-date') {
    return <span role="status" className="sr-only">Murmur is up to date</span>;
  }

  if (status.phase === 'error') {
    const installFailed = status.stage === 'install';
    return (
      <button
        type="button"
        onClick={installFailed ? onOpen : onRetryCheck}
        title={installFailed ? 'Update needs attention' : 'Update check failed — retry'}
        className="ui-icon-button ml-auto bg-error/10 font-bold text-error hover:bg-error/15 focus:outline-none focus-visible:ring-2 focus-visible:ring-error"
        aria-label={installFailed ? 'Update installation needs attention' : 'Update check failed. Retry'}
      >
        <span aria-hidden="true">!</span>
        <span className="sr-only">
          {installFailed ? 'Update needs attention' : 'Update check failed · Retry'}
        </span>
      </button>
    );
  }

  if (status.phase !== 'available') return null;

  return (
    <button
      type="button"
      onClick={onOpen}
      title={`Murmur ${status.version} is available`}
      className="ui-icon-button ml-auto bg-primary/10 hover:bg-primary/15 focus:outline-none focus-visible:ring-2 focus-visible:ring-primary"
      aria-label={`Murmur ${status.version} is available. View update`}
    >
      <span aria-hidden="true" className="h-1.5 w-1.5 rounded-full bg-primary" />
      <span className="sr-only">Update available · v{status.version.replace(/^v/, '')}</span>
    </button>
  );
}
