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
  const shouldReduceMotion = useReducedMotion();
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

  const version =
    status.phase === 'available' ? status.version :
    status.phase === 'preparing' ? status.version :
    status.phase === 'downloading' ? status.version :
    status.phase === 'ready' ? status.version : '';

  return (
    <>
      {/* Backdrop */}
      <div
        className="dialog-backdrop fixed inset-0 z-50"
        onClick={!isForced && !isBusy ? onDismiss : undefined}
      />

      {/* Modal */}
      <div className="fixed inset-0 flex items-center justify-center z-50 pointer-events-none">
        <motion.div
          initial={shouldReduceMotion ? false : { opacity: 0, scale: 0.98 }}
          animate={{ opacity: 1, scale: 1 }}
          transition={{ duration: shouldReduceMotion ? 0 : 0.16, ease: [0.23, 1, 0.32, 1] }}
          className="dialog-popover relative w-96 p-6 pointer-events-auto"
        >
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

          {/* Icon */}
          <div className="mx-auto mb-4 flex h-12 w-12 items-center justify-center rounded-[var(--ui-radius-card)] bg-[var(--ui-tint-accent-subtle)]">
            <svg className="h-6 w-6 text-on-surface" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4" />
            </svg>
          </div>

          <h2 className="text-lg font-semibold tracking-[var(--ui-track-title,-0.022em)] text-on-surface text-center mb-1">
            {requiresReinstall
              ? 'Reinstall Murmur to Update'
              : isForced
                ? 'Required Update'
                : 'Update Available'}
          </h2>

          {version && (
            <p className="text-sm text-on-surface-variant text-center mb-3">
              Version {version}
            </p>
          )}

          {isForced && (
            <p className="text-xs text-primary text-center mb-3">
              This update is required to continue using the app.
            </p>
          )}

          {/* Release notes */}
          {status.phase === 'available' && status.notes && (
            <div className="dialog-card mb-4 max-h-32 overflow-y-auto px-3 py-2 text-xs text-on-surface-variant [&_a]:text-primary [&_a]:underline [&_code]:rounded [&_code]:bg-surface-container-high [&_code]:px-1 [&_h1]:mb-1 [&_h1]:mt-2 [&_h1]:text-sm [&_h1]:font-semibold [&_h2]:mb-1 [&_h2]:mt-2 [&_h2]:text-xs [&_h2]:font-semibold [&_h3]:mb-1 [&_h3]:mt-1 [&_h3]:text-xs [&_h3]:font-medium [&_li]:my-0 [&_ol]:my-1 [&_ol]:list-decimal [&_ol]:pl-4 [&_p]:my-1 [&_ul]:my-1 [&_ul]:list-disc [&_ul]:pl-4">
              <Markdown rehypePlugins={[rehypeSanitize]}>{status.notes}</Markdown>
            </div>
          )}

          {/* Download progress */}
          {isPreparing && (
            <p className="text-sm text-on-surface text-center mb-4">
              Preparing update...
            </p>
          )}

          {isDownloading && (
            <div className="mb-4">
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
            <p className="text-sm text-on-surface text-center mb-4">
              Installing and relaunching...
            </p>
          )}

          {/* Error state */}
          {isError && (
            <div className="dialog-card mb-4 border-error/30 bg-error/10 px-3 py-2">
              <p className="text-xs text-error">{status.message}</p>
            </div>
          )}

          {/* Action buttons */}
          <div className="flex flex-col gap-2">
            {(status.phase === 'available' || (isError && !requiresReinstall)) && (
              <button
                onClick={isCheckError ? onRetryCheck : onDownload}
                className={cn(
                  'w-full rounded-[var(--ui-radius-pill)] px-4 py-2 text-sm font-semibold text-on-primary transition-colors',
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
                className="dialog-pill-btn w-full px-4 py-2 text-sm text-on-surface hover:bg-surface-container"
              >
                Download latest version
              </button>
            )}

            {status.phase === 'available' && !isForced && (
              <>
                <button
                  onClick={onSkip}
                  className="dialog-pill-btn w-full px-4 py-2 text-sm text-on-surface hover:bg-surface-container"
                >
                  Skip This Version
                </button>
                <button
                  onClick={onDismiss}
                  className="w-full px-4 py-2 text-sm text-on-surface-variant transition-colors hover:text-primary"
                >
                  Later
                </button>
              </>
            )}

            {(((status.phase === 'available' || isError) && isForced) || requiresReinstall) && (
              <button
                onClick={() => exit(0)}
                className="dialog-pill-btn w-full px-4 py-2 text-sm text-error hover:bg-error/10"
              >
                Quit
              </button>
            )}
          </div>
        </motion.div>
      </div>
    </>
  );
}
