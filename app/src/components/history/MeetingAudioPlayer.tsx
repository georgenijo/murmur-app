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
    return <p role="status" className="dialog-card mb-3 p-3 text-xs text-on-surface-variant">Loading retained meeting audio…</p>;
  }
  if (audio.status === 'unavailable') {
    return <p className="dialog-card mb-3 p-3 text-xs text-on-surface-variant">{unavailableMessage(audio.unavailableReason)}</p>;
  }
  if (audio.status === 'error') {
    return (
      <div role="alert" className="dialog-card mb-3 border-error/25 bg-error/10 p-3 text-xs text-error">
        <p>{audio.error ?? 'Retained meeting audio could not be loaded.'}</p>
        <button
          type="button"
          disabled={captureBusy}
          onClick={audio.retry}
          className="dialog-pill-btn mt-2 px-3 py-1.5 text-xs font-semibold disabled:opacity-40"
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

  return (
    <section aria-label="Meeting audio playback" className="dialog-card mb-3 space-y-3 p-3">
      <div className="flex flex-wrap items-center gap-2">
        <button
          type="button"
          disabled={controlsDisabled}
          onClick={audio.playAll}
          className="rounded-[var(--ui-radius-pill)] bg-primary px-3 py-1.5 text-xs font-semibold text-on-primary disabled:opacity-40"
        >
          Play all
        </button>
        <button
          type="button"
          disabled={!canPause && captureBusy}
          onClick={canPause ? audio.pause : audio.play}
          className="dialog-pill-btn px-3 py-1.5 text-xs font-semibold disabled:opacity-40"
        >
          {canPause ? 'Pause' : audio.status === 'paused' ? 'Resume' : 'Play'}
        </button>
        <label className="ml-auto text-[11px] font-semibold text-on-surface-variant">
          Channel
          <select
            aria-label="Playback channel"
            value={audio.channel}
            disabled={controlsDisabled}
            onChange={(event) => {
              const channel = event.target.value;
              if (channel === 'all' || channel === 'me' || channel === 'them') {
                audio.setChannel(channel);
              }
            }}
            className="ml-2 rounded-[var(--ui-radius-control)] border border-[var(--ui-hairline)] bg-surface-container-lowest px-2 py-1 text-xs text-on-surface disabled:opacity-40"
          >
            <option value="all">All</option>
            <option value="me">Me</option>
            <option value="them">Them</option>
          </select>
        </label>
      </div>
      <div className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-3">
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
          className="h-1.5 w-full cursor-pointer accent-primary disabled:cursor-not-allowed disabled:opacity-40"
        />
        <span className="font-mono text-[11px] tabular-nums text-on-surface-variant">
          {formatMeetingTimestamp(positionMs)} / {formatMeetingTimestamp(durationMs)}
        </span>
      </div>
      {captureBusy && (
        <p role="status" className="text-[11px] text-on-surface-variant">
          Playback is paused while Murmur records or processes a request.
        </p>
      )}
      {audio.status === 'buffering' && (
        <p role="status" className="text-[11px] text-on-surface-variant">Buffering retained audio…</p>
      )}
    </section>
  );
}
