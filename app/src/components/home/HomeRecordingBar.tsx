import type { DoubleTapKey, RecordingMode } from '../../lib/settings';
import type { MeetingRuntimePhase } from '../../lib/meetings';
import type { DictationStatus } from '../../lib/types';
import { DashboardAction, DashboardSurface } from '../ui/DashboardPrimitives';

interface HomeRecordingBarProps {
  status: DictationStatus;
  initialized: boolean;
  recordingDuration: number;
  audioLevel: number;
  triggerKey: DoubleTapKey;
  recordingMode: RecordingMode;
  meetingPhase: MeetingRuntimePhase;
  onRecord: () => void;
  onStop: () => void;
  onTranscribeFile: () => void;
}

const KEY_LABELS: Record<DoubleTapKey, string> = {
  shift_l: '⇧ Shift',
  alt_l: '⌥ Option',
  ctrl_r: '⌃ Control',
};

function hotkeyHint(mode: RecordingMode, key: DoubleTapKey): string {
  if (mode === 'double_tap') return `Double-tap ${KEY_LABELS[key]} anywhere to begin`;
  if (mode === 'both') return `Hold or double-tap ${KEY_LABELS[key]} anywhere to begin`;
  return `Hold ${KEY_LABELS[key]} anywhere to begin`;
}

function timer(seconds: number): string {
  const wholeSeconds = Math.max(0, Math.floor(seconds));
  return `${Math.floor(wholeSeconds / 60)}:${String(wholeSeconds % 60).padStart(2, '0')}`;
}

export function HomeRecordingBar({
  status,
  initialized,
  recordingDuration,
  audioLevel,
  triggerKey,
  recordingMode,
  meetingPhase,
  onRecord,
  onStop,
  onTranscribeFile,
}: HomeRecordingBarProps) {
  const isCapturing = status === 'starting' || status === 'recording';
  const busy = status === 'processing' || status === 'recovering';
  const meetingBusy = meetingPhase !== 'idle' && meetingPhase !== 'failed';
  const normalized = Math.min(1, Math.max(0, audioLevel) * 16);
  const envelopes = [0.52, 0.78, 1, 0.78, 0.52];
  const statusTitle = status === 'starting'
    ? 'Connecting to microphone'
    : status === 'recording'
      ? `Recording · ${timer(recordingDuration)}`
      : status === 'processing'
        ? 'Processing locally'
        : status === 'recovering'
          ? 'Recovering microphone'
          : initialized
            ? 'Ready to dictate'
            : 'Initializing';
  const actionLabel = status === 'starting'
    ? 'Cancel'
    : status === 'recording'
      ? 'Stop Recording'
      : status === 'processing'
        ? 'Processing'
        : status === 'recovering'
          ? 'Recovering'
          : 'Start Recording';

  return (
    <DashboardSurface as="section" variant="flat" ariaLabel="Dictation controls">
      <div className="home-recording-bar">
      <DashboardAction
        testId="home-record-button"
        variant="primary"
        tone={isCapturing ? 'danger' : 'default'}
        onActivate={() => void (isCapturing ? onStop() : onRecord())}
        disabled={!initialized || busy || meetingBusy}
        ariaLabel={status === 'recording' ? `Stop recording, ${timer(recordingDuration)}` : status === 'starting' ? 'Cancel recording' : busy ? statusTitle : 'Start recording'}
      >
        <span className="home-record-dot" aria-hidden="true" />
        <span>{actionLabel}</span>
      </DashboardAction>

      <div className="home-record-state" aria-live="polite">
        <div className="home-record-state-line">
          <strong>{statusTitle}</strong>
          {status === 'recording' && (
            <span className="home-record-waveform" aria-hidden="true">
              {envelopes.map((envelope, index) => (
                <span key={index} style={{ height: `${Math.max(3, Math.round((0.15 + normalized * envelope) * 18))}px` }} />
              ))}
            </span>
          )}
        </div>
        <span className="home-record-hint">{hotkeyHint(recordingMode, triggerKey)}</span>
      </div>

      <DashboardAction
        variant="quiet"
        onActivate={onTranscribeFile}
        disabled={isCapturing || busy || meetingBusy}
      >
        Transcribe file…
      </DashboardAction>
      </div>
    </DashboardSurface>
  );
}
