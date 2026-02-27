import { DictationStatus } from '../lib/types';

interface StatusHeaderProps {
  status: DictationStatus;
  initialized: boolean;
  recordingDuration: number;
  onSettingsToggle: () => void;
  isSettingsOpen: boolean;
}

function getStatusColor(status: DictationStatus, initialized: boolean): string {
  if (status === 'recording') return 'text-red-500 dark:text-red-400';
  if (status === 'processing') return 'text-amber-600 dark:text-amber-500';
  if (initialized) return 'text-emerald-600 dark:text-emerald-500';
  return 'text-stone-400 dark:text-stone-500';
}

function getStatusText(status: DictationStatus, initialized: boolean, duration: number): string {
  if (status === 'recording') return `Recording ${duration}s`;
  if (status === 'processing') return 'Processing...';
  if (initialized) return 'Ready';
  return 'Initializing...';
}

export function StatusHeader({ status, initialized, recordingDuration, onSettingsToggle, isSettingsOpen }: StatusHeaderProps) {
  const statusColor = getStatusColor(status, initialized);
  const statusText = getStatusText(status, initialized, recordingDuration);

  return (
    <header
      data-tauri-drag-region
      className="shrink-0 flex items-center justify-between px-4 py-3 border-b border-stone-200 dark:border-stone-700 bg-white/80 dark:bg-stone-800/80 backdrop-blur-sm"
    >
      <span className="text-sm font-semibold text-stone-800 dark:text-stone-100">Murmur</span>

      <div className="flex items-center gap-3">
        <div className={`flex items-center gap-1.5 text-sm font-medium ${statusColor}`}>
          {status === 'processing' ? (
            <svg className="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24">
              <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
              <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
            </svg>
          ) : (
            <svg className={`w-4 h-4 ${status === 'recording' ? 'animate-pulse' : ''}`} fill="currentColor" viewBox="0 0 24 24">
              <path d="M12 14c1.66 0 3-1.34 3-3V5c0-1.66-1.34-3-3-3S9 3.34 9 5v6c0 1.66 1.34 3 3 3z"/>
              <path d="M17 11c0 2.76-2.24 5-5 5s-5-2.24-5-5H5c0 3.53 2.61 6.43 6 6.92V21h2v-3.08c3.39-.49 6-3.39 6-6.92h-2z"/>
            </svg>
          )}
          <span>{statusText}</span>
        </div>

        <button
          onClick={onSettingsToggle}
          className={`p-1.5 rounded-md transition-colors ${
            isSettingsOpen
              ? 'text-stone-900 dark:text-stone-100 bg-stone-100 dark:bg-stone-700'
              : 'text-stone-500 dark:text-stone-400 hover:text-stone-800 dark:hover:text-stone-200 hover:bg-stone-100 dark:hover:bg-stone-700'
          }`}
          aria-label="Toggle settings"
          aria-expanded={isSettingsOpen}
        >
          <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z" />
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
          </svg>
        </button>
      </div>
    </header>
  );
}
