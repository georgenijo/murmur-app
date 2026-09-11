import type { ReactNode } from 'react';
import { Mic, Square } from 'lucide-react';
import {
  dictationKeyLabel,
  type DictationKey,
  type RecordingMode,
} from '../../lib/settings';
import type { MeetingRuntimePhase } from '../../lib/meetings';
import type { DictationStatus } from '../../lib/types';

interface HomeRecordingBarProps {
  status: DictationStatus;
  initialized: boolean;
  recordingDuration: number;
  audioLevel: number;
  triggerKey: DictationKey;
  recordingMode: RecordingMode;
  meetingPhase: MeetingRuntimePhase;
  onRecord: () => void;
  onStop: () => void;
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
  triggerKey,
  recordingMode,
  meetingPhase,
  onRecord,
  onStop,
}: HomeRecordingBarProps) {
  const isCapturing = status === 'starting' || status === 'recording';
  const busy = status === 'processing' || status === 'recovering';
  const meetingBusy = meetingPhase !== 'idle' && meetingPhase !== 'failed';
  const normalized = Math.min(1, Math.max(0, audioLevel) * 16);
  const envelopes = [0.52, 0.78, 1, 0.78, 0.52];

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
    title = 'Click to start talking';
    const gesture = recordingMode === 'double_tap' ? 'double-tap' : recordingMode === 'both' ? 'hold or double-tap' : 'hold';
    hint = <>or {gesture} <Shortcut triggerKey={triggerKey} /> in any app</>;
  }

  const waiting = busy || meetingBusy || !initialized;

  return (
    <section className="home-talk" aria-label="Dictation controls" data-state={isCapturing ? 'capturing' : waiting ? 'waiting' : 'ready'}>
      <button
        type="button"
        data-testid="home-record-button"
        className="home-talk-action"
        onClick={() => void (isCapturing ? onStop() : onRecord())}
        disabled={waiting}
        aria-label={status === 'recording' ? `Click to stop recording, ${timer(recordingDuration)}` : status === 'starting' ? 'Click to cancel recording' : title}
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
          <span className="home-talk-hint">{hint}</span>
        </span>
      </button>
    </section>
  );
}
