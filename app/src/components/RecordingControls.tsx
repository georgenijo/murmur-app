import { DictationStatus } from '../lib/types';
import type { DoubleTapKey } from '../lib/settings';

interface RecordingControlsProps {
  status: DictationStatus;
  initialized: boolean;
  onStart: () => void;
  onStop: () => void;
  triggerKey: DoubleTapKey;
}

const HOTKEY_LABELS: Record<DoubleTapKey, string> = {
  shift_l: '⇧ Shift',
  alt_l: '⌥ Option',
  ctrl_r: '⌃ Control',
};

export function RecordingControls({ status, initialized, onStart, onStop, triggerKey }: RecordingControlsProps) {
  return (
    <div className="shrink-0 flex flex-col items-center gap-2">
      {status === 'recording' || status === 'starting' ? (
        <button
          onClick={onStop}
          disabled={!initialized}
          className="flex items-center gap-2 rounded-xl border-2 border-error bg-surface-container-lowest px-7 py-3 font-semibold text-error shadow-lg shadow-error/20 ring-4 ring-error/10 transition-[background-color,transform] hover:bg-error/10 active:scale-[0.98] disabled:cursor-not-allowed disabled:opacity-50"
        >
          {status === 'recording' ? (
            <svg className="w-5 h-5" fill="currentColor" viewBox="0 0 20 20">
              <rect x="5" y="5" width="10" height="10" rx="1" />
            </svg>
          ) : (
            <span className="h-4 w-4 rounded-full border-2 border-error/30 border-t-error animate-spin" />
          )}
          {status === 'recording' ? 'Stop Recording' : 'Cancel Connection'}
        </button>
      ) : (
        <button
          onClick={onStart}
          disabled={!initialized || status === 'processing' || status === 'recovering'}
          className="flex items-center gap-2 rounded-xl bg-[linear-gradient(135deg,var(--murmur-primary),var(--murmur-primary-dim))] px-7 py-3 font-semibold text-on-primary shadow-lg shadow-primary/20 transition-[filter,transform] hover:brightness-105 active:scale-[0.98] disabled:cursor-not-allowed disabled:opacity-50"
        >
          <svg className="w-5 h-5" fill="currentColor" viewBox="0 0 20 20">
            <circle cx="10" cy="10" r="6" />
          </svg>
          {status === 'processing'
            ? 'Processing...'
            : status === 'recovering'
              ? 'Recovering audio...'
              : 'Start Recording'}
        </button>
      )}
      {status === 'idle' && (
        <p className="text-xs text-on-surface-variant">
          Press <kbd className="font-[inherit] font-semibold text-on-surface">{HOTKEY_LABELS[triggerKey]}</kbd> to begin
        </p>
      )}
    </div>
  );
}
