import { exit } from '@tauri-apps/plugin-process';
import Markdown from 'react-markdown';
import rehypeSanitize from 'rehype-sanitize';
import type { UpdateStatus } from '../lib/updater';

interface UpdateModalProps {
  status: UpdateStatus;
  onDownload: () => void;
  onSkip: () => void;
  onDismiss: () => void;
}

export function UpdateModal({ status, onDownload, onSkip, onDismiss }: UpdateModalProps) {
  if (
    status.phase !== 'available' &&
    status.phase !== 'downloading' &&
    status.phase !== 'ready' &&
    status.phase !== 'error'
  ) {
    return null;
  }

  const isForced = (status.phase === 'available' || status.phase === 'error') && status.isForced;
  const isDownloading = status.phase === 'downloading';
  const isReady = status.phase === 'ready';
  const isError = status.phase === 'error';
  const isBusy = isDownloading || isReady;

  const version =
    status.phase === 'available' ? status.version :
    status.phase === 'downloading' ? status.version :
    status.phase === 'ready' ? status.version : '';

  return (
    <>
      {/* Backdrop */}
      <div
        className="fixed inset-0 z-50 bg-black/50"
        onClick={!isForced && !isBusy ? onDismiss : undefined}
      />

      {/* Modal */}
      <div className="fixed inset-0 flex items-center justify-center z-50 pointer-events-none">
        <div className="bg-surface-container-lowest rounded-2xl shadow-xl p-6 w-96 pointer-events-auto relative">
          {/* Close button — shown on non-forced error and non-forced available states */}
          {((isError && !isForced) || (status.phase === 'available' && !isForced)) && (
            <button
              onClick={onDismiss}
              aria-label="Close update dialog"
              className="absolute right-4 top-4 rounded-md p-1 text-on-surface-variant transition-colors hover:bg-surface-container hover:text-on-surface focus:outline-none focus-visible:ring-2 focus-visible:ring-primary"
            >
              <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
              </svg>
            </button>
          )}

          {/* Icon */}
          <div className="mx-auto mb-4 flex h-12 w-12 items-center justify-center rounded-xl bg-primary/10">
            <svg className="h-6 w-6 text-primary" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4" />
            </svg>
          </div>

          <h2 className="text-lg font-semibold text-on-surface text-center mb-1">
            {isForced ? 'Required Update' : 'Update Available'}
          </h2>

          {version && (
            <p className="text-sm text-on-surface-variant text-center mb-3">
              Version {version}
            </p>
          )}

          {isForced && (
            <p className="text-xs text-amber-600 dark:text-amber-400 text-center mb-3">
              This update is required to continue using the app.
            </p>
          )}

          {/* Release notes */}
          {status.phase === 'available' && status.notes && (
            <div className="mb-4 max-h-32 overflow-y-auto rounded-lg bg-background px-3 py-2 text-xs text-on-surface-variant [&_a]:text-primary [&_a]:underline [&_code]:rounded [&_code]:bg-surface-container-high [&_code]:px-1 [&_h1]:mb-1 [&_h1]:mt-2 [&_h1]:text-sm [&_h1]:font-semibold [&_h2]:mb-1 [&_h2]:mt-2 [&_h2]:text-xs [&_h2]:font-semibold [&_h3]:mb-1 [&_h3]:mt-1 [&_h3]:text-xs [&_h3]:font-medium [&_li]:my-0 [&_ol]:my-1 [&_ol]:list-decimal [&_ol]:pl-4 [&_p]:my-1 [&_ul]:my-1 [&_ul]:list-disc [&_ul]:pl-4">
              <Markdown rehypePlugins={[rehypeSanitize]}>{status.notes}</Markdown>
            </div>
          )}

          {/* Download progress */}
          {isDownloading && (
            <div className="mb-4">
              <div className="flex justify-between text-xs text-on-surface-variant mb-1">
                <span>Downloading...</span>
                <span>{status.progress}%</span>
              </div>
              <div className="h-2 w-full overflow-hidden rounded-full bg-surface-container-highest">
                <div
                  className="h-full bg-primary rounded-full transition-all duration-200"
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
            <div className="mb-4 px-3 py-2 bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 rounded-lg">
              <p className="text-xs text-red-600 dark:text-red-400">{status.message}</p>
            </div>
          )}

          {/* Action buttons */}
          <div className="flex flex-col gap-2">
            {(status.phase === 'available' || isError) && (
              <button
                onClick={onDownload}
                className="w-full py-2 px-4 bg-primary hover:bg-primary-dim text-on-primary text-sm font-medium rounded-lg transition-colors"
              >
                {isError ? 'Retry' : 'Update Now'}
              </button>
            )}

            {status.phase === 'available' && !isForced && (
              <>
                <button
                  onClick={onSkip}
                  className="w-full rounded-lg border border-outline-variant/20 bg-surface-container-lowest px-4 py-2 text-sm font-medium text-on-surface transition-colors hover:bg-surface-container"
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

            {(status.phase === 'available' || isError) && isForced && (
              <button
                onClick={() => exit(0)}
                className="w-full rounded-lg border border-error/30 bg-surface-container-lowest px-4 py-2 text-sm font-medium text-error transition-colors hover:bg-error/10"
              >
                Quit
              </button>
            )}
          </div>
        </div>
      </div>
    </>
  );
}
