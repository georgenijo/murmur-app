import { DictationStatus } from '../lib/types';

interface StatusHeaderProps {
  status: DictationStatus;
  initialized: boolean;
  recordingDuration: number;
}

function getStatusColor(status: DictationStatus, initialized: boolean): string {
  switch (status) {
    case 'recording':
      return 'text-red-500';
    case 'processing':
      return 'text-yellow-500';
    case 'idle':
      return initialized ? 'text-green-500' : 'text-gray-400';
    default:
      return 'text-gray-400';
  }
}

function getStatusText(status: DictationStatus, initialized: boolean): string {
  if (!initialized) return 'Initializing...';
  switch (status) {
    case 'recording':
      return 'Recording';
    case 'processing':
      return 'Processing...';
    case 'idle':
      return 'Ready';
    default:
      return status;
  }
}

export function StatusHeader({ status, initialized, recordingDuration }: StatusHeaderProps) {
  return (
    <header className="flex-shrink-0 px-4 py-3 border-b border-gray-200 dark:border-gray-700 bg-white/80 dark:bg-gray-800/80 backdrop-blur-sm">
      <div className="flex items-center justify-between">
        <h1 className="text-lg font-semibold text-gray-900 dark:text-white tracking-tight">
          Local Dictation
        </h1>
        <div className="flex items-center gap-2">
          {status === 'processing' ? (
            <svg aria-hidden="true" className="w-5 h-5 animate-spin text-yellow-500" xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24">
              <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4"></circle>
              <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"></path>
            </svg>
          ) : (
            <svg
              aria-hidden="true"
              className={`w-5 h-5 ${status === 'recording' ? 'text-red-500 animate-pulse' : 'text-gray-400'}`}
              fill="currentColor"
              viewBox="0 0 24 24"
            >
              <path d="M12 14c1.66 0 3-1.34 3-3V5c0-1.66-1.34-3-3-3S9 3.34 9 5v6c0 1.66 1.34 3 3 3z"/>
              <path d="M17 11c0 2.76-2.24 5-5 5s-5-2.24-5-5H5c0 3.53 2.61 6.43 6 6.92V21h2v-3.08c3.39-.49 6-3.39 6-6.92h-2z"/>
            </svg>
          )}
          <span className={`text-sm font-medium ${getStatusColor(status, initialized)}`}>
            {status === 'recording' && recordingDuration > 0
              ? `${recordingDuration}s`
              : getStatusText(status, initialized)}
          </span>
        </div>
      </div>
    </header>
  );
}
