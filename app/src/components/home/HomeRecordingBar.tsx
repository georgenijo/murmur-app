import { useState, type ReactNode } from 'react';
import { openMicrophoneSettings, type MicPermissionStatus } from '../../lib/dictation';
import { DashboardAction } from '../ui/DashboardPrimitives';
import { Mic, Square } from 'lucide-react';
import {
  dictationKeyLabel,
  type DictationKey,
  type RecordingMode,
} from '../../lib/settings';
import type { MeetingRuntimePhase } from '../../lib/meetings';
import type { DictationStatus } from '../../lib/types';
import {
  smartDestinationLabel,
  type SmartDestinationStatus,
} from '../../lib/smartDestination';

interface HomeRecordingBarProps {
  status: DictationStatus;
  initialized: boolean;
  recordingDuration: number;
  audioLevel: number;
  smartDestinationStatus?: SmartDestinationStatus | null;
  smartDestinationAppName?: string | null;
  triggerKey: DictationKey;
  recordingMode: RecordingMode;
  meetingPhase: MeetingRuntimePhase;
  onRecord: () => void;
  onStop: () => void;
  /** Status strip under the prompt row, like a composer's model/agent bar. */
  footer?: ReactNode;
  microphonePermission?: MicPermissionStatus;
}

function timer(seconds: number): string {
  const wholeSeconds = Math.max(0, Math.floor(seconds));
  return `${Math.floor(wholeSeconds / 60)}:${String(wholeSeconds % 60).padStart(2, '0')}`;
}

function Shortcut({ triggerKey }: { triggerKey: DictationKey }) {
  return <kbd className="home-talk-key">{dictationKeyLabel(triggerKey)}</kbd>;
}

export function HomeRecordingBar({
  status,
  initialized,
  recordingDuration,
  audioLevel,
  smartDestinationStatus = null,
  smartDestinationAppName = null,
  triggerKey,
  recordingMode,
  meetingPhase,
  onRecord,
  onStop,
  footer,
  microphonePermission = 'unknown',
}: HomeRecordingBarProps) {
  const [recoveryError, setRecoveryError] = useState(false);
  const micDenied = microphonePermission === 'denied';
  const isCapturing = status === 'starting' || status === 'recording';
  const busy = status === 'processing' || status === 'recovering';
  const meetingBusy = meetingPhase !== 'idle' && meetingPhase !== 'failed';
  const normalized = Math.min(1, Math.max(0, audioLevel) * 16);
  const envelopes = [0.52, 0.78, 1, 0.78, 0.52];
  const destinationLabel = (status === 'recording' || status === 'processing')
    && smartDestinationStatus
    ? smartDestinationLabel(smartDestinationStatus, smartDestinationAppName)
    : null;

  let title: string;
  let hint: ReactNode;
  if (status === 'starting') {
    title = 'Connecting to your microphone…';
    hint = 'Click to cancel';
  } else if (status === 'recording') {
    title = `Listening · ${timer(recordingDuration)}`;
    hint = 'Click to stop';
  } else if (status === 'processing') {
    title = 'Writing it out…';
    hint = 'Your text will be copied when it’s ready';
  } else if (status === 'recovering') {
    title = 'Reconnecting your microphone…';
    hint = 'This can take a moment';
  } else if (meetingBusy) {
    title = 'Notetaker is running';
    hint = 'Dictation is available when the meeting ends';
  } else if (!initialized) {
    title = 'Getting ready…';
    hint = 'This takes a moment the first time';
  } else {
    title = 'Click to start dictating';
    const gesture = recordingMode === 'double_tap' ? 'double-tap' : recordingMode === 'both' ? 'hold or double-tap' : 'hold';
    hint = <>or {gesture} <Shortcut triggerKey={triggerKey} /> in any app</>;
  }

  const waiting = busy || meetingBusy || !initialized;
  const showMicRecovery = micDenied && !isCapturing && !busy;

  return (
    <section className="home-talk" aria-label="Dictation controls" data-state={isCapturing ? 'capturing' : showMicRecovery || waiting ? 'waiting' : 'ready'}>
      {showMicRecovery ? (
        <div className="home-mic-recovery">
          <button type="button" className="home-talk-action" disabled aria-disabled="true" aria-label="Start dictation — microphone access required">
            <span className="home-talk-button" aria-hidden="true"><Mic className="home-talk-icon" strokeWidth={2.2} /></span>
          </button>
          <div className="home-mic-recovery-copy">
            <p role="status"><span className="home-warning-dot" aria-hidden="true" />Murmur can't hear you. Allow microphone access in System Settings.</p>
            <DashboardAction variant="primary" onActivate={() => {
              setRecoveryError(false);
              void openMicrophoneSettings().catch(() => setRecoveryError(true));
            }}>Open System Settings…</DashboardAction>
            {recoveryError && <p role="alert">Couldn’t open System Settings. Open Privacy &amp; Security → Microphone.</p>}
          </div>
        </div>
      ) : <button
        type="button"
        data-testid="home-record-button"
        className="home-talk-action"
        onClick={() => void (isCapturing ? onStop() : onRecord())}
        disabled={waiting}
        aria-label={isCapturing ? 'Stop dictation' : waiting ? title : 'Start dictation'}
      >
        <span className="home-talk-button" aria-hidden="true">
          {isCapturing
            ? <Square aria-hidden="true" className="home-talk-icon" fill="currentColor" strokeWidth={0} />
            : busy || !initialized
              ? <span aria-hidden="true" className="home-talk-spinner" />
              : <Mic aria-hidden="true" className="home-talk-icon" strokeWidth={2.2} />}
        </span>

        <span className="home-talk-text" aria-live="polite">
          <span className="home-talk-title">
            <strong>{title}</strong>
            {status === 'recording' && (
              <span className="home-record-waveform" aria-hidden="true">
                {envelopes.map((envelope, index) => (
                  <span key={index} style={{ height: `${Math.max(3, Math.round((0.15 + normalized * envelope) * 16))}px` }} />
                ))}
              </span>
            )}
          </span>
          <span className="home-talk-hint">
            {destinationLabel ? (
              <span
                className="home-talk-destination"
                aria-label={`Destination: ${destinationLabel}`}
              >
                {destinationLabel}
              </span>
            ) : hint}
          </span>
        </span>
      </button>}
      {footer && <div className="home-talk-footer">{footer}</div>}
    </section>
  );
}
