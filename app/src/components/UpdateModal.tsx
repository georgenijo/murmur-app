import { useEffect, useRef } from 'react';
import { Download, RotateCw } from 'lucide-react';
import { exit } from '@tauri-apps/plugin-process';
import { openUrl } from '@tauri-apps/plugin-opener';
import { motion, useReducedMotion } from 'motion/react';
import Markdown from 'react-markdown';
import rehypeSanitize from 'rehype-sanitize';
import type { UpdateStatus } from '../lib/updater';
import { cn } from '../lib/sona-utils';

export const LATEST_RELEASES_URL = 'https://github.com/georgenijo/murmur-app/releases/latest';

interface UpdateModalProps {
  status: UpdateStatus;
  onDownload: () => void;
  onRestart: () => void;
  onRetryCheck: () => void;
  onDismiss: () => void;
}

export function UpdateModal({
  status,
  onDownload,
  onRestart,
  onRetryCheck,
  onDismiss,
}: UpdateModalProps) {
  const dialogRef = useRef<HTMLDivElement>(null);
  const primaryActionRef = useRef<HTMLButtonElement>(null);
  const onDismissRef = useRef(onDismiss);
  onDismissRef.current = onDismiss;
  const shouldReduceMotion = useReducedMotion();
  const visible =
    status.phase === 'available' ||
    status.phase === 'preparing' ||
    status.phase === 'downloading' ||
    status.phase === 'ready' ||
    status.phase === 'restarting' ||
    status.phase === 'error';
  const canDismiss =
    (status.phase === 'available' || status.phase === 'ready' || status.phase === 'error') &&
    !status.isForced;

  useEffect(() => {
    if (!visible) return;

    const previouslyFocused =
      document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && canDismiss) {
        event.preventDefault();
        onDismissRef.current();
        return;
      }
      if (event.key !== 'Tab') return;

      const focusable = Array.from(
        dialogRef.current?.querySelectorAll<HTMLElement>(
          'a[href], button:not(:disabled), [tabindex]:not([tabindex="-1"])',
        ) ?? [],
      );
      event.preventDefault();
      if (focusable.length === 0) {
        dialogRef.current?.focus();
        return;
      }

      const activeElement = document.activeElement;
      const currentIndex = activeElement instanceof HTMLElement
        ? focusable.indexOf(activeElement)
        : -1;
      const nextIndex = event.shiftKey
        ? (currentIndex <= 0 ? focusable.length - 1 : currentIndex - 1)
        : (currentIndex === focusable.length - 1 ? 0 : currentIndex + 1);
      focusable[nextIndex]?.focus();
    };

    document.addEventListener('keydown', onKeyDown);
    const focusTimer = setTimeout(() => {
      if (primaryActionRef.current) primaryActionRef.current.focus();
      else dialogRef.current?.focus();
    }, 60);

    return () => {
      document.removeEventListener('keydown', onKeyDown);
      clearTimeout(focusTimer);
      previouslyFocused?.focus();
    };
  }, [canDismiss, status.phase, visible]);

  if (
    status.phase !== 'available' &&
    status.phase !== 'preparing' &&
    status.phase !== 'downloading' &&
    status.phase !== 'ready' &&
    status.phase !== 'restarting' &&
    status.phase !== 'error'
  ) {
    return null;
  }

  const isForced =
    (status.phase === 'available' || status.phase === 'ready' || status.phase === 'error') &&
    status.isForced;
  const isBusy =
    status.phase === 'preparing' ||
    status.phase === 'downloading' ||
    status.phase === 'restarting';
  const hasReleaseNotes = status.phase === 'available' && status.notes.trim().length > 0;
  const requiresReinstall = status.phase === 'error' && status.recovery === 'reinstall';
  const hasActions =
    status.phase === 'available' || status.phase === 'ready' || status.phase === 'error';
  const version = status.phase === 'error' ? null : status.version;
  const HeaderIcon = status.phase === 'ready' || status.phase === 'restarting' ? RotateCw : Download;
  const title =
    requiresReinstall
      ? 'Reinstall Murmur to Update'
      : status.phase === 'available'
        ? isForced
          ? 'Required Update'
          : 'Update Available'
        : status.phase === 'preparing'
          ? 'Preparing Update'
          : status.phase === 'downloading'
            ? 'Downloading Update'
            : status.phase === 'ready'
              ? 'Ready to Restart'
              : status.phase === 'restarting'
                ? 'Restarting Murmur'
                : 'Update Failed';

  return (
    <div
      className="dialog-backdrop fixed inset-0 z-50 flex items-center justify-center p-5 backdrop-blur-[2px]"
      onClick={(event) => {
        if (event.target === event.currentTarget && canDismiss) onDismiss();
      }}
    >
      <motion.div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby="update-dialog-title"
        aria-busy={isBusy}
        tabIndex={-1}
        initial={shouldReduceMotion ? false : { opacity: 0, scale: 0.98 }}
        animate={{ opacity: 1, scale: 1 }}
        transition={{ duration: shouldReduceMotion ? 0 : 0.16, ease: [0.23, 1, 0.32, 1] }}
        className={cn(
          'dialog-popover relative flex w-full max-w-[560px] flex-col overflow-hidden',
          hasReleaseNotes
            ? 'h-[min(620px,calc(100vh-2.5rem))]'
            : 'max-h-[calc(100vh-2.5rem)]',
        )}
      >
        <div className="relative shrink-0 border-b border-[var(--ui-hairline)] px-6 py-4">
          {canDismiss && (
            <button
              type="button"
              onClick={onDismiss}
              aria-label="Close update dialog"
              className="absolute right-4 top-4 rounded-[var(--ui-radius-control)] p-1 text-on-surface-variant transition-colors hover:bg-surface-container hover:text-on-surface focus:outline-none focus-visible:ring-2 focus-visible:ring-primary"
            >
              <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
              </svg>
            </button>
          )}

          <div className="flex items-center gap-3 pr-8">
            <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-[var(--ui-radius-card)] bg-[var(--ui-tint-accent-subtle)]">
              <HeaderIcon className="h-5 w-5 text-on-surface" aria-hidden="true" />
            </div>
            <div className="min-w-0">
              <h2 id="update-dialog-title" className="text-lg font-semibold tracking-[var(--ui-track-title,-0.022em)] text-on-surface">
                {title}
              </h2>
              {version && (
                <p className="mt-0.5 text-sm text-on-surface-variant">Version {version}</p>
              )}
            </div>
          </div>
        </div>

        <div className={cn('min-h-0 px-6 py-5', hasReleaseNotes && 'flex-1 overflow-y-auto')}>
          {isForced && (
            <p className="dialog-card mb-4 border-primary/20 bg-[var(--ui-tint-accent-subtle)] px-3 py-2 text-sm text-on-surface">
              This update is required to continue using the app.
            </p>
          )}

          {hasReleaseNotes && status.phase === 'available' && (
            <div className="text-sm leading-6 text-on-surface-variant [&_a]:font-medium [&_a]:text-primary [&_a]:underline [&_a]:underline-offset-2 [&_code]:rounded [&_code]:bg-surface-container-high [&_code]:px-1 [&_h1]:mb-2 [&_h1]:mt-5 [&_h1]:text-base [&_h1]:font-semibold [&_h1]:text-on-surface [&_h1:first-child]:mt-0 [&_h2]:mb-2 [&_h2]:mt-5 [&_h2]:text-base [&_h2]:font-semibold [&_h2]:text-on-surface [&_h2:first-child]:mt-0 [&_h3]:mb-1.5 [&_h3]:mt-4 [&_h3]:text-sm [&_h3]:font-semibold [&_h3]:text-on-surface [&_li]:my-1 [&_ol]:my-2 [&_ol]:list-decimal [&_ol]:pl-5 [&_p]:my-2 [&_strong]:font-semibold [&_strong]:text-on-surface [&_ul]:my-2 [&_ul]:list-disc [&_ul]:pl-5">
              <Markdown rehypePlugins={[rehypeSanitize]}>{status.notes}</Markdown>
            </div>
          )}

          {status.phase === 'preparing' && (
            <p className="text-center text-sm text-on-surface">Preparing the download...</p>
          )}

          {status.phase === 'downloading' && (
            <div>
              <div className="mb-2 flex justify-between text-xs text-on-surface-variant">
                <span>Downloading...</span>
                {status.progress !== null && <span>{Math.round(status.progress)}%</span>}
              </div>
              <div
                role="progressbar"
                aria-label="Download progress"
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuenow={status.progress ?? undefined}
                className="h-2 w-full overflow-hidden rounded-full bg-[var(--ui-tint-sunken)]"
              >
                <div
                  className={cn(
                    'h-full rounded-full bg-[linear-gradient(140deg,var(--murmur-primary),var(--murmur-primary-dim))]',
                    status.progress === null
                      ? 'w-1/3 animate-pulse'
                      : 'transition-[width] duration-200',
                  )}
                  style={status.progress === null ? undefined : { width: `${status.progress}%` }}
                />
              </div>
            </div>
          )}

          {status.phase === 'ready' && (
            <p className="text-center text-sm text-on-surface">
              Restart Murmur to install the downloaded update.
            </p>
          )}

          {status.phase === 'restarting' && (
            <p className="text-center text-sm text-on-surface">
              Installing the update and restarting...
            </p>
          )}

          {status.phase === 'error' && (
            <div className="dialog-card border-error/30 bg-error/10 px-3 py-2">
              <p className="text-sm text-error">{status.message}</p>
              {status.stage === 'check' && (
                <p className="mt-2 text-sm text-on-surface-variant">
                  You can also{' '}
                  <a
                    href={LATEST_RELEASES_URL}
                    onClick={(event) => {
                      event.preventDefault();
                      void openUrl(LATEST_RELEASES_URL);
                    }}
                    className="font-medium text-primary underline underline-offset-2"
                  >
                    download the latest version manually
                  </a>
                  .
                </p>
              )}
            </div>
          )}
        </div>

        {hasActions && (
          <div
            className={cn(
              'grid shrink-0 gap-2 border-t border-[var(--ui-hairline)] bg-[var(--ui-tint-sunken)] px-6 py-4',
              requiresReinstall && isForced ? 'grid-cols-1' : 'grid-cols-2',
            )}
          >
            {requiresReinstall ? (
              <>
                <button
                  ref={primaryActionRef}
                  type="button"
                  onClick={() => exit(0)}
                  className="dialog-pill-btn w-full px-4 py-2.5 text-sm font-semibold text-error hover:bg-error/10"
                >
                  Quit
                </button>
                {!isForced && (
                  <button
                    type="button"
                    onClick={onDismiss}
                    className="dialog-pill-btn w-full px-4 py-2 text-sm text-on-surface-variant hover:bg-surface-container hover:text-primary"
                  >
                    Later
                  </button>
                )}
              </>
            ) : (
              <>
                <button
                  ref={primaryActionRef}
                  type="button"
                  onClick={
                    status.phase === 'available'
                      ? onDownload
                      : status.phase === 'ready'
                        ? onRestart
                        : status.stage === 'check'
                          ? onRetryCheck
                          : status.stage === 'install'
                            ? onDownload
                            : onRestart
                  }
                  className={cn(
                    'w-full rounded-[var(--ui-radius-pill)] px-4 py-2.5 text-sm font-semibold text-on-primary transition-colors',
                    'bg-[linear-gradient(140deg,var(--murmur-primary),var(--murmur-primary-dim))]',
                    'shadow-[var(--ui-shadow-accent)]',
                  )}
                >
                  {status.phase === 'available'
                    ? 'Download Update'
                    : status.phase === 'ready'
                      ? 'Restart Now'
                      : 'Retry'}
                </button>
                <button
                  type="button"
                  onClick={isForced ? () => exit(0) : onDismiss}
                  className={cn(
                    'dialog-pill-btn w-full px-4 py-2 text-sm hover:bg-surface-container',
                    isForced
                      ? 'text-error hover:bg-error/10'
                      : 'text-on-surface-variant hover:text-primary',
                  )}
                >
                  {isForced ? 'Quit' : 'Later'}
                </button>
              </>
            )}
          </div>
        )}
      </motion.div>
    </div>
  );
}
