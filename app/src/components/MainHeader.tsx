import type { DoubleTapKey, RecordingMode } from '../lib/settings';
import type { DictationStatus } from '../lib/types';
import type { ReactNode } from 'react';
import { WindowHeader } from './ui/WindowHeader';
import type { MeetingRuntimePhase } from '../lib/meetings';

interface MainHeaderProps {
  status: DictationStatus;
  initialized: boolean;
  recordingDuration: number;
  audioLevel?: number;
  triggerKey: DoubleTapKey;
  recordingMode: RecordingMode;
  onRecord: () => void;
  onStop: () => void;
  onOpenSettings: () => void;
  settingsOpen: boolean;
  updateIndicator?: ReactNode;
  mode?: 'main' | 'settings';
  buildBadge?: string;
  meetingPhase?: MeetingRuntimePhase;
  meetingElapsedMs?: number;
  showRecordControls?: boolean;
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
  audioLevel = 0,
  triggerKey,
  recordingMode,
  onRecord,
  onStop,
  onOpenSettings,
  settingsOpen,
  updateIndicator,
  mode = 'main',
  buildBadge,
  meetingPhase = 'idle',
  meetingElapsedMs = 0,
  showRecordControls = true,
}: MainHeaderProps) {
  const isCapturing = status === 'recording' || status === 'starting';
  const busy = status === 'processing' || status === 'recovering';
  const meetingBusy = meetingPhase !== 'idle' && meetingPhase !== 'failed';
  const label = meetingPhase === 'recording'
    ? `Meeting ${formatTimer(meetingElapsedMs / 1000)}`
    : meetingPhase === 'starting'
      ? 'Meeting connecting'
      : meetingPhase === 'stopping'
        ? 'Meeting stopping'
        : meetingPhase === 'processing'
          ? 'Meeting finishing'
          : statusLabel(status, initialized);
  const normalizedAudioLevel = Math.min(1, Math.max(0, audioLevel) * 16);
  const waveformEnvelope = [0.55, 0.8, 1, 0.8, 0.55];

  return (
    <WindowHeader
      contextLabel={mode === 'settings' ? 'Settings' : undefined}
      className={mode === 'settings' ? 'settings-window-header' : ''}
      showWordmark={mode === 'settings'}
    >
      {mode === 'main' && (
        <div
          data-testid="main-status-chip"
          className={`ui-status-chip ${
            status === 'recording' || meetingPhase === 'recording' ? 'text-error' : 'text-on-surface'
          }`}
          aria-live="polite"
        >
          {meetingPhase === 'starting' || meetingPhase === 'stopping' || meetingPhase === 'processing' || status === 'processing' || status === 'starting' ? (
            <span className="h-2 w-2 animate-spin rounded-full border border-primary/25 border-t-primary" aria-hidden="true" />
          ) : status === 'recording' || meetingPhase === 'recording' ? (
            <span
              data-testid="main-recording-waveform"
              className="flex h-3 w-4 shrink-0 items-center justify-center gap-px"
              aria-hidden="true"
            >
              {waveformEnvelope.map((envelope, index) => (
                <span
                  key={index}
                  className="w-px rounded-full bg-error"
                  style={{
                    height: `${Math.max(2, Math.round((0.12 + normalizedAudioLevel * envelope) * 12))}px`,
                    transition: 'height 50ms ease-out',
                  }}
                />
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
      )}

      {buildBadge && (
        <span
          data-testid="performance-build-badge"
          className="pointer-events-none shrink-0 select-none rounded-full border border-success/30 bg-success/10 px-2.5 py-1 text-[10px] font-bold uppercase tracking-[0.1em] text-success"
        >
          ✓ {buildBadge}
        </span>
      )}

      <div data-tauri-drag-region className="min-w-4 flex-1" />

      {mode === 'settings' ? (
        <button
          type="button"
          onClick={onOpenSettings}
          className="min-w-16 h-[30px] rounded-[var(--ui-radius-pill,999px)] border border-[var(--ui-hairline,var(--murmur-outline-variant))] bg-[var(--ui-tint-raised,var(--murmur-surface-container-lowest))] px-4 text-[length:var(--ui-font-label)] font-semibold text-on-surface shadow-[var(--ui-shadow-1)] transition-colors hover:bg-surface-container-low focus:outline-none focus-visible:ring-2 focus-visible:ring-primary"
        >
          Done
        </button>
      ) : (
        <>
          {updateIndicator}

          {showRecordControls && (
            <>
              <p
                data-testid="hotkey-hint"
                className={`hidden shrink-0 select-none whitespace-nowrap text-xs text-on-surface-variant transition-opacity sm:block ${
                  isCapturing || busy || meetingBusy ? 'pointer-events-none opacity-0' : 'opacity-100'
                }`}
              >
                {hotkeyHint(recordingMode, triggerKey)}
              </p>

              <button
                data-testid="record-pill"
                type="button"
                onClick={() => void (isCapturing ? onStop() : onRecord())}
                disabled={!initialized || busy || meetingBusy}
                aria-label={
                  status === 'recording'
                    ? `Stop recording, ${formatTimer(recordingDuration)}`
                    : status === 'starting'
                      ? 'Cancel recording'
                      : busy || meetingBusy
                        ? label
                        : 'Record'
                }
                className={`ui-record-pill active:scale-[0.98] disabled:cursor-not-allowed disabled:opacity-50 ${
                  isCapturing
                    ? 'border border-error/50 bg-error/10 text-error hover:bg-error/15'
                    : 'bg-[linear-gradient(140deg,var(--murmur-primary),var(--murmur-primary-dim))] text-on-primary shadow-[var(--ui-shadow-accent)] hover:brightness-105'
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
                      : busy || meetingBusy
                        ? 'Wait'
                        : 'Record'}
                </span>
              </button>
            </>
          )}

          <button
            type="button"
            onClick={onOpenSettings}
            aria-label={settingsOpen ? 'Close settings' : 'Open customization and settings'}
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
