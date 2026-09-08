# Local remote-speaker implementation contract

The signed main executable has a fixed diarization worker mode. Only that child
initializes FluidAudio diarization. Its network access is denied before native
initialization. An exact SHA-256 manifest pins the 21,599,417-byte model set to
FluidInference/speaker-diarization-coreml revision
1ed7a662fdc7109e36d822db793ee6eebdaf8594, CC BY 4.0.

## Storage and UI boundary

The meeting repository owns schema migration, assignments and speaker labels.
The review layer resolves display/export names. The capture coordinator owns
remote PCM lifetime; the diarization coordinator owns the child, cancellation
and assignment publication. Frontend settings and workspace components use
these typed boundaries.

Required storage methods:

- `apply_remote_speakers(session_id: &str, assignments: &[(i64, u32)]) -> Result<(), String>`.
  Only complete sessions, all assigned rows must be final Them rows belonging
  to this session, speaker IDs 1..=32, no duplicate segment IDs. One immediate
  transaction inserts default Speaker N labels and applies assignments. Never
  change text, status, channel, timing, order, generated/reviewed documents.
  Existing remote assignments reject another pass so saved names stay stable.
- `rename_remote_speaker(session_id: &str, speaker_id: u32, label: &str) -> Result<MeetingWorkspace, String>`.
  Bound labels consistently with existing channel labels; one session only.
- `MeetingSegment.remote_speaker_id: Option<u32>`.
- `MeetingWorkspace.remote_speakers: Vec<RemoteSpeakerLabel>` where
  `RemoteSpeakerLabel { speaker_id: u32, label: String }`, camelCase JSON.
- Additive schema v4: session-scoped label table cascading from sessions,
  nullable remote speaker column restricted to Them, enforcement of same-session
  label ownership. Migration preserves v3 evidence and reviews.
- Every display/export uses a resolved session speaker label where assigned,
  otherwise the existing channel fallback. Mic remains Me.

Frontend contract:

- Setting `meetingDiarization: boolean`, false by default. Start options include
  `diarization: boolean`, frozen at capture start.
- UI explicitly discloses: local remote-speaker labels; temporary system audio
  up to two hours while labels process, deleted afterward; no cross-meeting
  voice profiles; uncertain passages remain Them. Audio retention remains
  independent. Feature is inert if optional model absent.
- Model ID `meeting-diarization-coreml`. `download_model` and
  `check_specific_model_exists` accept this auxiliary model, without putting it
  in selectable ASR models. New `get_diarization_model_status` returns
  `{ supported, installed, installing, bytes }`. `remove_diarization_model`
  cancels/reaps current child before removal. Existing `download-progress`
  payload/event is reused.
- `rename_meeting_remote_speaker { sessionId, speakerId, label }` returns the
  updated workspace. The command returns a refreshed workspace.
- Event `meeting-speakers-updated { sessionId }` refreshes the selected workspace
  with the same selection ticket protections as other meeting updates.

## Ownership and bounds

Remote PCM is streamed at 16 kHz into a private temporary file. Two hours is a
hard cap; exceeding it abandons only speaker refinement and deletes the file.
Failures never stop capture or transcription. Startup removes abandoned files.
A successful capture passes this owned file, session ID, timestamp origin and
immutable generation to the post-session coordinator. Any terminal path drops
and deletes the owned file. No embeddings or raw turns enter the database/logs.

Worker startup shares recording_transition with capture. Foreground dictation
kills and confirms termination before starting heavy work, with a short bounded
wait. Interrupted diarization can retry when idle until its overall deadline.
A child whose termination is unconfirmed remains owned and blocks another
worker. No diarization call holds the ASR model-runtime mutex.

Attribution accepts only one sufficiently confident speaker across a chunk,
requires conservative time coverage, and rejects a second speaker or overlap.
It never splits the text or picks a dominant speaker. Numbering follows first
accepted appearance within the session and is capped at 32 speakers.
