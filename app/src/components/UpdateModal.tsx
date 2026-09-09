import { useEffect, useRef } from 'react';
import { exit } from '@tauri-apps/plugin-process';
import { openUrl } from '@tauri-apps/plugin-opener';
import { motion, useReducedMotion } from 'motion/react';
import Markdown from 'react-markdown';
import rehypeSanitize from 'rehype-sanitize';
import type { UpdateStatus } from '../lib/updater';
import { cn } from '../lib/sona-utils';

// Replace with an owned stable redirect (for example, murmur.georgenijo.com/download)
// once one is publicly available.
export const LATEST_RELEASES_URL = 'https://github.com/georgenijo/murmur-app/releases/latest';

interface UpdateModalProps {
  status: UpdateStatus;
  onDownload: () => void;
  onRetryCheck: () => void;
  onSkip: () => void;
  onDismiss: () => void;
}

export function UpdateModal({ status, onDownload, onRetryCheck, onSkip, onDismiss }: UpdateModalProps) {
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
    status.phase === 'error';
  const canDismiss =
    (status.phase === 'available' || status.phase === 'error') && !status.isForced;

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
      if (focusable.length === 0) return;

      event.preventDefault();
      const currentIndex = focusable.indexOf(document.activeElement as HTMLElement);
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
  }, [canDismiss, visible]);

  if (
    status.phase !== 'available' &&
    status.phase !== 'preparing' &&
    status.phase !== 'downloading' &&
    status.phase !== 'ready' &&
    status.phase !== 'error'
  ) {
    return null;
  }

  const isForced = (status.phase === 'available' || status.phase === 'error') && status.isForced;
  const isPreparing = status.phase === 'preparing';
  const isDownloading = status.phase === 'downloading';
  const isReady = status.phase === 'ready';
  const isError = status.phase === 'error';
  const isCheckError = isError && status.stage === 'check';
  const requiresReinstall = isError && status.recovery === 'reinstall';
  const isBusy = isPreparing || isDownloading || isReady;
  const hasReleaseNotes = status.phase === 'available' && status.notes.trim().length > 0;

  const version =
    status.phase === 'available' ? status.version :
    status.phase === 'preparing' ? status.version :
    status.phase === 'downloading' ? status.version :
    status.phase === 'ready' ? status.version : '';

  return (
    <div
      className="dialog-backdrop fixed inset-0 z-50 flex items-center justify-center p-5 backdrop-blur-[2px]"
      onClick={(event) => {
        if (event.target === event.currentTarget && !isForced && !isBusy) onDismiss();
      }}
    >
      <motion.div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby="update-dialog-title"
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
          {/* Close button — shown on non-forced error and non-forced available states */}
          {((isError && !isForced) || (status.phase === 'available' && !isForced)) && (
            <button
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
              <svg className="h-5 w-5 text-on-surface" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4" />
              </svg>
            </div>
            <div className="min-w-0">
              <h2 id="update-dialog-title" className="text-lg font-semibold tracking-[var(--ui-track-title,-0.022em)] text-on-surface">
                {requiresReinstall
                  ? 'Reinstall Murmur to Update'
                  : isForced
                    ? 'Required Update'
                    : 'Update Available'}
              </h2>
              {version && <p className="mt-0.5 text-sm text-on-surface-variant">Version {version}</p>}
            </div>
          </div>
        </div>

        <div className={cn('min-h-0 px-6 py-5', hasReleaseNotes && 'flex-1 overflow-y-auto')}>
          {isForced && (
            <p className="dialog-card mb-4 border-primary/20 bg-[var(--ui-tint-accent-subtle)] px-3 py-2 text-sm text-on-surface">
              This update is required to continue using the app.
            </p>
          )}

          {/* Release notes */}
          {hasReleaseNotes && status.phase === 'available' && (
            <div className="text-sm leading-6 text-on-surface-variant [&_a]:font-medium [&_a]:text-primary [&_a]:underline [&_a]:underline-offset-2 [&_code]:rounded [&_code]:bg-surface-container-high [&_code]:px-1 [&_h1]:mb-2 [&_h1]:mt-5 [&_h1]:text-base [&_h1]:font-semibold [&_h1]:text-on-surface [&_h1:first-child]:mt-0 [&_h2]:mb-2 [&_h2]:mt-5 [&_h2]:text-base [&_h2]:font-semibold [&_h2]:text-on-surface [&_h2:first-child]:mt-0 [&_h3]:mb-1.5 [&_h3]:mt-4 [&_h3]:text-sm [&_h3]:font-semibold [&_h3]:text-on-surface [&_li]:my-1 [&_ol]:my-2 [&_ol]:list-decimal [&_ol]:pl-5 [&_p]:my-2 [&_strong]:font-semibold [&_strong]:text-on-surface [&_ul]:my-2 [&_ul]:list-disc [&_ul]:pl-5">
              <Markdown rehypePlugins={[rehypeSanitize]}>{status.notes}</Markdown>
            </div>
          )}

          {/* Download progress */}
          {isPreparing && (
            <p className="text-sm text-on-surface text-center">
              Preparing update...
            </p>
          )}

          {isDownloading && (
            <div>
              <div className="flex justify-between text-xs text-on-surface-variant mb-1">
                <span>Downloading...</span>
                <span>{status.progress}%</span>
              </div>
              <div className="h-2 w-full overflow-hidden rounded-full bg-[var(--ui-tint-sunken)]">
                <div
                  className="h-full rounded-full bg-[linear-gradient(140deg,var(--murmur-primary),var(--murmur-primary-dim))] transition-all duration-200"
                  style={{ width: `${status.progress}%` }}
                />
              </div>
            </div>
          )}

          {/* Ready / installing state */}
          {isReady && (
            <p className="text-sm text-on-surface text-center">
              Installing and relaunching...
            </p>
          )}

          {/* Error state */}
          {isError && (
            <div className="dialog-card border-error/30 bg-error/10 px-3 py-2">
              <p className="text-sm text-error">{status.message}</p>
            </div>
          )}
        </div>

        {/* Action buttons */}
        <div className="shrink-0 border-t border-[var(--ui-hairline)] bg-[var(--ui-tint-sunken)] px-6 py-4">
            {(status.phase === 'available' || (isError && !requiresReinstall)) && (
              <button
                ref={primaryActionRef}
                type="button"
                onClick={isCheckError ? onRetryCheck : onDownload}
                className={cn(
                  'w-full rounded-[var(--ui-radius-pill)] px-4 py-2.5 text-sm font-semibold text-on-primary transition-colors',
                  'bg-[linear-gradient(140deg,var(--murmur-primary),var(--murmur-primary-dim))]',
                  'shadow-[var(--ui-shadow-accent)]',
                )}
              >
                {isError ? 'Retry' : 'Update Now'}
              </button>
            )}

            {isCheckError && (
              <button
                type="button"
                onClick={() => void openUrl(LATEST_RELEASES_URL)}
                className="dialog-pill-btn mt-2 w-full px-4 py-2 text-sm text-on-surface hover:bg-surface-container"
              >
                Download latest version
              </button>
            )}

            {status.phase === 'available' && !isForced && (
              <div className="mt-2 grid grid-cols-2 gap-2">
                <button
                  type="button"
                  onClick={onSkip}
                  className="dialog-pill-btn w-full px-4 py-2 text-sm text-on-surface hover:bg-surface-container"
                >
                  Skip This Version
                </button>
                <button
                  type="button"
                  onClick={onDismiss}
                  className="dialog-pill-btn w-full px-4 py-2 text-sm text-on-surface-variant hover:bg-surface-container hover:text-primary"
                >
                  Later
                </button>
              </div>
            )}

            {(((status.phase === 'available' || isError) && isForced) || requiresReinstall) && (
              <button
                ref={requiresReinstall ? primaryActionRef : undefined}
                type="button"
                onClick={() => exit(0)}
                className="dialog-pill-btn mt-2 w-full px-4 py-2 text-sm text-error hover:bg-error/10"
              >
                Quit
              </button>
            )}
        </div>
      </motion.div>
    </div>
  );
}
