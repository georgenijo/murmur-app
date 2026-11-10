import { formatMeetingTimestamp, type MeetingRuntimeStatus } from '../../lib/meetings';

/** Persisted `active` can outlive capture while native transcription drains. */
export function meetingCapturePresentation(sessionId: string, status: MeetingRuntimeStatus) {
  if (status.sessionId !== sessionId) return null;
  const phase = status.phase;
  if (phase !== 'starting' && phase !== 'recording' && phase !== 'stopping' && phase !== 'processing') return null;
  const label = phase === 'starting' ? 'Connecting…'
    : phase === 'recording' ? 'Recording'
    : phase === 'stopping' ? 'Stopping…' : 'Finishing…';
  const footer = phase === 'starting' ? 'Connecting both channels…'
    : phase === 'recording' ? 'Listening…'
    : phase === 'stopping' ? 'Stopping capture…' : 'Finishing pending transcript chunks…';
  return { phase, summary: `${label} · ${formatMeetingTimestamp(status.elapsedMs)}`, footer };
}
