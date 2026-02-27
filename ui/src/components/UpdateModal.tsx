import { exit } from '@tauri-apps/plugin-process';
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

  const isForced = status.phase === 'available' && status.isForced;
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
        className="fixed inset-0 bg-stone-900/50 z-50"
        onClick={!isForced && !isBusy ? onDismiss : undefined}
      />

      {/* Modal */}
      <div className="fixed inset-0 flex items-center justify-center z-50 pointer-events-none">
        <div className="bg-white dark:bg-stone-800 rounded-2xl shadow-xl p-6 w-96 pointer-events-auto">
          {/* Icon */}
          <div className="w-12 h-12 mx-auto mb-4 bg-blue-100 dark:bg-blue-900/30 rounded-xl flex items-center justify-center">
            <svg className="w-6 h-6 text-blue-600 dark:text-blue-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4" />
            </svg>
          </div>

          <h2 className="text-lg font-semibold text-stone-900 dark:text-stone-100 text-center mb-1">
            {isForced ? 'Required Update' : 'Update Available'}
          </h2>

          {version && (
            <p className="text-sm text-stone-500 dark:text-stone-400 text-center mb-3">
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
            <div className="mb-4 max-h-32 overflow-y-auto px-3 py-2 bg-stone-50 dark:bg-stone-900 rounded-lg text-xs text-stone-600 dark:text-stone-400 whitespace-pre-wrap">
              {status.notes}
            </div>
          )}

          {/* Download progress */}
          {isDownloading && (
            <div className="mb-4">
              <div className="flex justify-between text-xs text-stone-500 dark:text-stone-400 mb-1">
                <span>Downloading...</span>
                <span>{status.progress}%</span>
              </div>
              <div className="w-full h-2 bg-stone-200 dark:bg-stone-700 rounded-full overflow-hidden">
                <div
                  className="h-full bg-blue-500 rounded-full transition-all duration-200"
                  style={{ width: `${status.progress}%` }}
                />
              </div>
            </div>
          )}

          {/* Ready / installing state */}
          {isReady && (
            <p className="text-sm text-stone-600 dark:text-stone-300 text-center mb-4">
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
                className="w-full py-2 px-4 bg-blue-600 hover:bg-blue-700 text-white text-sm font-medium rounded-lg transition-colors"
              >
                {isError ? 'Retry' : 'Update Now'}
              </button>
            )}

            {status.phase === 'available' && !isForced && (
              <>
                <button
                  onClick={onSkip}
                  className="w-full py-2 px-4 border border-stone-300 dark:border-stone-600 bg-white dark:bg-stone-700 text-stone-700 dark:text-stone-300 text-sm font-medium rounded-lg hover:bg-stone-50 dark:hover:bg-stone-600 transition-colors"
                >
                  Skip This Version
                </button>
                <button
                  onClick={onDismiss}
                  className="w-full py-2 px-4 text-stone-500 dark:text-stone-400 text-sm hover:text-stone-700 dark:hover:text-stone-200 transition-colors"
                >
                  Later
                </button>
              </>
            )}

            {status.phase === 'available' && isForced && (
              <button
                onClick={() => exit(0)}
                className="w-full py-2 px-4 border border-red-300 dark:border-red-700 bg-white dark:bg-stone-700 text-red-600 dark:text-red-400 text-sm font-medium rounded-lg hover:bg-red-50 dark:hover:bg-red-900/20 transition-colors"
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
