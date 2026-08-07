import type { DoubleTapKey, RecordingMode } from '../lib/settings';
import type { DictationStatus } from '../lib/types';
import type { ReactNode } from 'react';
import { WindowHeader } from './ui/WindowHeader';

interface MainHeaderProps {
  status: DictationStatus;
  initialized: boolean;
  recordingDuration: number;
  triggerKey: DoubleTapKey;
  recordingMode: RecordingMode;
  onRecord: () => void;
  onStop: () => void;
  onOpenSettings: () => void;
  settingsOpen: boolean;
  updateIndicator?: ReactNode;
  mode?: 'main' | 'settings';
}

const KEY_LABELS: Record<DoubleTapKey, string> = {
  shift_l: '⇧ Shift',
  alt_l: '⌥ Option',
  ctrl_r: '⌃ Control',
};

function statusLabel(status: DictationStatus, initialized: boolean): string {
  if (status === 'starting') return 'Connecting';
  if (status === 'recovering') return 'Recovering';
  if (status === 'recording') return 'Recording';
  if (status === 'processing') return 'Processing';
  return initialized ? 'Ready' : 'Initializing';
}

function hotkeyHint(mode: RecordingMode, key: DoubleTapKey): string {
  const label = KEY_LABELS[key];
  if (mode === 'double_tap') return `Double-tap ${label} to dictate`;
  if (mode === 'both') return `Hold or double-tap ${label}`;
  return `Hold ${label} to dictate`;
}

function formatTimer(seconds: number): string {
  const wholeSeconds = Math.max(0, Math.floor(seconds));
  const minutes = Math.floor(wholeSeconds / 60);
  return `${minutes}:${String(wholeSeconds % 60).padStart(2, '0')}`;
}

export function MainHeader({
  status,
  initialized,
  recordingDuration,
  triggerKey,
  recordingMode,
  onRecord,
  onStop,
  onOpenSettings,
  settingsOpen,
  updateIndicator,
  mode = 'main',
}: MainHeaderProps) {
  const isCapturing = status === 'recording' || status === 'starting';
  const busy = status === 'processing' || status === 'recovering';
  const label = statusLabel(status, initialized);

  return (
    <WindowHeader contextLabel={mode === 'settings' ? 'Settings' : undefined}>
      <div
        data-testid="main-status-chip"
        className={`ui-status-chip ${
          status === 'recording' ? 'text-error' : 'text-on-surface'
        }`}
        aria-live="polite"
      >
        {status === 'processing' || status === 'starting' ? (
          <span className="h-2 w-2 animate-spin rounded-full border border-primary/25 border-t-primary" aria-hidden="true" />
        ) : status === 'recording' ? (
          <span className="flex h-3 items-center gap-0.5" aria-hidden="true">
            {[5, 9, 6].map((height, index) => (
              <span key={index} className="w-0.5 animate-pulse rounded-full bg-error" style={{ height }} />
            ))}
          </span>
        ) : (
          <span
            aria-hidden="true"
            className={`h-2 w-2 rounded-full ${
              status === 'recovering' ? 'animate-pulse bg-warning' : initialized ? 'bg-success' : 'bg-outline-variant'
            }`}
          />
        )}
        <span>{label}</span>
      </div>

      <div data-tauri-drag-region className="min-w-4 flex-1" />

      {mode === 'settings' ? (
        <button
          type="button"
          onClick={onOpenSettings}
          className="rounded-md px-3 py-1 text-[length:var(--ui-font-label)] font-semibold text-on-surface transition-colors hover:bg-surface-container-low focus:outline-none focus-visible:ring-2 focus-visible:ring-primary"
        >
          Done
        </button>
      ) : (
        <>
          {updateIndicator}

          <p
            data-testid="hotkey-hint"
            className={`hidden shrink-0 select-none whitespace-nowrap text-xs text-on-surface-variant transition-opacity sm:block ${
              isCapturing || busy ? 'pointer-events-none opacity-0' : 'opacity-100'
            }`}
          >
            {hotkeyHint(recordingMode, triggerKey)}
          </p>

          <button
            data-testid="record-pill"
            type="button"
            onClick={() => void (isCapturing ? onStop() : onRecord())}
            disabled={!initialized || busy}
            aria-label={
              status === 'recording'
                ? `Stop recording, ${formatTimer(recordingDuration)}`
                : status === 'starting'
                  ? 'Cancel recording'
                  : busy
                    ? label
                    : 'Record'
            }
            className={`ui-record-pill active:scale-[0.98] disabled:cursor-not-allowed disabled:opacity-50 ${
              isCapturing
                ? 'border border-error/50 bg-error/10 text-error hover:bg-error/15'
                : 'bg-[linear-gradient(135deg,var(--murmur-primary),var(--murmur-primary-dim))] text-on-primary shadow-[0_2px_8px_color-mix(in_srgb,var(--murmur-primary)_18%,transparent)] hover:brightness-105'
            }`}
          >
            <span
              className={`h-1.5 w-1.5 shrink-0 bg-current ${
                isCapturing ? 'rounded-[2px]' : 'rounded-full'
              }`}
              aria-hidden="true"
            />
            <span>
              {status === 'starting'
                ? 'Cancel'
                : status === 'recording'
                  ? 'Stop'
                  : busy
                    ? 'Wait'
                    : 'Record'}
            </span>
            {' '}
            <span
              aria-hidden={status !== 'recording'}
              className={`overflow-hidden font-mono text-xs ${
                status === 'recording' ? 'visible w-auto' : 'invisible w-0'
              }`}
            >
              {formatTimer(recordingDuration)}
            </span>
          </button>

          <button
            type="button"
            onClick={onOpenSettings}
            aria-label={settingsOpen ? 'Close settings' : 'Open settings'}
            aria-expanded={settingsOpen}
            className={`ui-icon-button focus:outline-none focus-visible:ring-2 focus-visible:ring-primary ${
              settingsOpen
                ? 'bg-surface-container-high text-on-surface'
                : ''
            }`}
          >
            <svg className="h-[15px] w-[15px]" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.8} d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z" />
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.8} d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
            </svg>
          </button>
        </>
      )}
    </WindowHeader>
  );
}
