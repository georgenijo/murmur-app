import { Check, Download, LoaderCircle, RefreshCw, RotateCw, TriangleAlert } from 'lucide-react';
import type { UpdateStatus } from '../lib/updater';
import { cn } from '../lib/sona-utils';

interface UpdateIndicatorProps {
  status: UpdateStatus;
  onOpen: () => void;
  onRetryCheck: () => void;
  onDownload: () => void;
  onRestart: () => void;
  variant?: 'compact' | 'settings';
}

export function UpdateIndicator({ status, onOpen, onRetryCheck, onDownload, onRestart, variant = 'compact' }: UpdateIndicatorProps) {
  const compact = variant === 'compact';
  if (compact && status.phase === 'idle') return null;
  if (compact && status.phase === 'up-to-date') {
    return <span role="status" className="ui-visually-hidden">Murmur is up to date</span>;
  }

  const busy = status.phase === 'checking' || status.phase === 'preparing' || status.phase === 'downloading' || status.phase === 'restarting';
  const progress = status.phase === 'downloading' ? status.progress : null;
  const available = status.phase === 'available';
  const ready = status.phase === 'ready';
  const failed = status.phase === 'error';
  const label = available ? 'Download Update'
    : ready ? 'Restart Now'
    : status.phase === 'checking' ? 'Checking for Updates…'
    : status.phase === 'preparing' ? 'Preparing Update…'
    : status.phase === 'downloading' ? (progress === null ? 'Downloading Update…' : `Downloading Update… ${progress}%`)
    : status.phase === 'restarting' ? 'Restarting Murmur…'
    : failed ? (status.stage === 'check' ? 'Retry Update Check' : 'Update Needs Attention')
    : 'Check for Updates';
  const detail = available ? `Murmur ${status.version} is available`
    : ready ? `Murmur ${status.version} is ready to install`
    : status.phase === 'up-to-date' ? 'You’re up to date.'
    : failed ? (status.stage === 'check' ? 'Couldn’t check for updates. Try again.' : 'Open the update dialog to continue.')
    : null;
  const action = available ? onDownload : ready ? onRestart : failed && status.stage !== 'check' ? onOpen : onRetryCheck;
  const Icon = ready || status.phase === 'restarting' ? RotateCw
    : failed ? TriangleAlert
    : busy && status.phase !== 'downloading' ? LoaderCircle
    : available || status.phase === 'downloading' ? Download : RefreshCw;

  return (
    <div className={compact ? 'ml-auto' : 'settings-field'}>
      <button
        data-testid="update-indicator"
        type="button"
        onClick={action}
        disabled={busy}
        title={detail ? `${label}. ${detail}` : label}
        aria-label={compact && detail ? `${label}. ${detail}` : label}
        className={cn(
          'relative flex items-center justify-center gap-2 focus:outline-none focus-visible:ring-2 focus-visible:ring-primary disabled:cursor-wait',
          compact ? 'ui-icon-button bg-surface-container-low hover:bg-surface-container'
            : 'w-full rounded-lg border border-outline-variant/30 bg-surface-container-lowest px-3 py-2 text-xs font-medium transition-colors hover:bg-surface-container',
          failed ? 'text-error' : available || ready ? 'text-primary' : 'text-on-surface-variant',
        )}
      >
        <span className="relative flex h-4 w-4 shrink-0 items-center justify-center" aria-hidden="true">
          <Icon className={cn('h-4 w-4', (status.phase === 'checking' || status.phase === 'preparing' || status.phase === 'restarting') && 'motion-safe:animate-spin')} />
          {available && <span className="absolute -right-1 -top-1 h-1.5 w-1.5 rounded-full bg-primary ring-2 ring-surface-container-low" />}
          {ready && <Check className="absolute -bottom-1 -right-1 h-2.5 w-2.5 rounded-full bg-primary p-px text-on-primary" />}
        </span>
        {!compact && <span>{label}</span>}
        {compact && status.phase === 'downloading' && (
          <svg className={cn('pointer-events-none absolute inset-0 h-full w-full -rotate-90 text-primary', progress === null && 'motion-safe:animate-spin')} viewBox="0 0 28 28" fill="none" aria-hidden="true">
            <circle cx="14" cy="14" r="12" stroke="currentColor" strokeWidth="1.5" opacity="0.2" />
            <circle cx="14" cy="14" r="12" stroke="currentColor" strokeWidth="1.5" pathLength="100" strokeDasharray={`${progress ?? 25} 100`} strokeLinecap="round" />
          </svg>
        )}
      </button>
      {busy && <span className="ui-visually-hidden" role="status">{label}</span>}
      {!compact && detail && <p role="status" className={cn('text-xs', failed ? 'text-error' : status.phase === 'up-to-date' ? 'text-success' : 'text-on-surface-variant')}>{detail}</p>}
    </div>
  );
}
