import { DictationStatus } from '../lib/types';

interface StatusHeaderProps {
  status: DictationStatus;
  initialized: boolean;
  recordingDuration: number;
  onSettingsToggle: () => void;
  isSettingsOpen: boolean;
}

function getStatusDotColor(status: DictationStatus, initialized: boolean): string {
  if (status === 'recording') return 'bg-error';
  if (status === 'processing') return 'bg-amber-500';
  if (initialized) return 'bg-emerald-500';
  return 'bg-outline-variant';
}

function getStatusText(status: DictationStatus, initialized: boolean, duration: number): string {
  if (status === 'recording') return `Recording ${duration}s`;
  if (status === 'processing') return 'Processing...';
  if (initialized) return 'Ready';
  return 'Initializing...';
}

export function StatusHeader({ status, initialized, recordingDuration, onSettingsToggle, isSettingsOpen }: StatusHeaderProps) {
  const statusDotColor = getStatusDotColor(status, initialized);
  const statusText = getStatusText(status, initialized, recordingDuration);

  return (
    <header
      data-tauri-drag-region
      className="flex shrink-0 items-center justify-between bg-background/90 px-4 py-3 backdrop-blur-sm"
    >
      <span className="text-sm font-bold tracking-tight text-primary">Murmur</span>

      <div className="flex items-center gap-3">
        <div className="flex items-center gap-2 rounded-full bg-surface-container-low px-3 py-1.5 text-xs font-semibold text-on-surface">
          <span
            aria-hidden="true"
            className={`h-2 w-2 rounded-full ${statusDotColor} ${status === 'recording' || status === 'processing' ? 'animate-pulse' : ''}`}
          />
          <span>{statusText}</span>
        </div>

        <button
          onClick={onSettingsToggle}
          className={`p-1.5 rounded-lg transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-primary ${
            isSettingsOpen
              ? 'text-on-surface bg-surface-container-high'
              : 'text-on-surface-variant hover:bg-surface-container-high hover:text-on-surface'
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
