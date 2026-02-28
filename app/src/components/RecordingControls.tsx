import { DictationStatus } from '../lib/types';

interface RecordingControlsProps {
  status: DictationStatus;
  initialized: boolean;
  onStart: () => void;
  onStop: () => void;
}

export function RecordingControls({ status, initialized, onStart, onStop }: RecordingControlsProps) {
  return (
    <div className="shrink-0 flex justify-center gap-3">
      {status === 'recording' ? (
        <button
          onClick={onStop}
          disabled={!initialized}
          className="flex items-center gap-2 px-6 py-3 bg-red-500 hover:bg-red-600 active:bg-red-700 text-white font-medium rounded-lg shadow-sm transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
        >
          <svg className="w-5 h-5" fill="currentColor" viewBox="0 0 20 20">
            <rect x="5" y="5" width="10" height="10" rx="1" />
          </svg>
          Stop Recording
        </button>
      ) : (
        <button
          onClick={onStart}
          disabled={!initialized || status === 'processing'}
          className="flex items-center gap-2 px-6 py-3 bg-stone-800 hover:bg-stone-900 active:bg-stone-950 dark:bg-stone-100 dark:hover:bg-white dark:text-stone-900 text-white font-medium rounded-lg shadow-sm transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
        >
          <svg className="w-5 h-5" fill="currentColor" viewBox="0 0 20 20">
            <circle cx="10" cy="10" r="6" />
          </svg>
          {status === 'processing' ? 'Processing...' : 'Start Recording'}
        </button>
      )}
    </div>
  );
}
