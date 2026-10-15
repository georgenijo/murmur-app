import { Pause, Play } from 'lucide-react';
import type { MeetingAudioController } from '../../lib/hooks/useMeetingAudio';
import { formatMeetingTimestamp } from '../../lib/meetings';

interface MeetingAudioPlayerProps {
  audio: MeetingAudioController;
  captureBusy: boolean;
}

const unavailableMessage = (reason: MeetingAudioController['unavailableReason']): string => {
  if (reason === 'notRetained') return 'Audio was not retained for this meeting.';
  if (reason === 'notFinished') return 'Playback will be available after this meeting finishes.';
  return 'No retained audio is available for this meeting.';
};

export function MeetingAudioPlayer({ audio, captureBusy }: MeetingAudioPlayerProps) {
  if (audio.status === 'loading') {
    return <p role="status" className="meeting-quiet-note">Loading retained meeting audio…</p>;
  }
  if (audio.status === 'unavailable') {
    return <p className="meeting-quiet-note">{unavailableMessage(audio.unavailableReason)}</p>;
  }
  if (audio.status === 'error') {
    return (
      <div role="alert" className="meeting-quiet-note border-error/25 bg-error/10 text-error">
        <p>{audio.error ?? 'Retained meeting audio could not be loaded.'}</p>
        <button
          type="button"
          disabled={captureBusy}
          onClick={audio.retry}
          className="meeting-header-action mt-2"
        >
          Retry audio access
        </button>
      </div>
    );
  }

  const controlsDisabled = captureBusy || audio.status === 'buffering';
  const playing = audio.status === 'playing';
  const canPause = playing || audio.status === 'buffering';
  const durationMs = Math.max(0, audio.durationMs);
  const positionMs = Math.min(durationMs, Math.max(0, audio.positionMs));

  const toggleLabel = canPause ? 'Pause' : audio.status === 'paused' ? 'Resume' : 'Play';

  return (
    <section aria-label="Meeting audio playback" className="meeting-audio">
      <button
        type="button"
        disabled={!canPause && captureBusy}
        onClick={canPause ? audio.pause : audio.play}
        aria-label={toggleLabel}
        title={toggleLabel}
        className="meeting-audio-toggle"
      >
        {canPause
          ? <Pause aria-hidden="true" className="h-3.5 w-3.5" fill="currentColor" strokeWidth={0} />
          : <Play aria-hidden="true" className="ml-0.5 h-3.5 w-3.5" fill="currentColor" strokeWidth={0} />}
      </button>
      <input
        type="range"
        min={0}
        max={Math.max(1, durationMs)}
        step={100}
        value={positionMs}
        disabled={controlsDisabled || durationMs === 0}
        aria-label="Playback position"
        aria-valuetext={`${formatMeetingTimestamp(positionMs)} of ${formatMeetingTimestamp(durationMs)}`}
        onChange={(event) => audio.seek(Number(event.target.value))}
        className="h-1.5 min-w-0 flex-1 cursor-pointer accent-primary disabled:cursor-not-allowed disabled:opacity-40"
      />
      <span className="shrink-0 font-mono text-[11px] tabular-nums text-on-surface-variant">
        {formatMeetingTimestamp(positionMs)} / {formatMeetingTimestamp(durationMs)}
      </span>
      <select
        aria-label="Playback channel"
        title="Playback channel"
        value={audio.channel}
        disabled={controlsDisabled}
        onChange={(event) => {
          const channel = event.target.value;
          if (channel === 'all' || channel === 'me' || channel === 'them') {
            audio.setChannel(channel);
          }
        }}
        className="meeting-audio-channel"
      >
        <option value="all">Both channels</option>
        <option value="me">Me only</option>
        <option value="them">Them only</option>
      </select>
      {captureBusy && (
        <p role="status" className="meeting-audio-note">
          Playback is paused while Murmur records or processes a request.
        </p>
      )}
      {audio.status === 'buffering' && (
        <p role="status" className="meeting-audio-note">Buffering retained audio…</p>
      )}
    </section>
  );
}
