# Retained meeting audio playback

The review workspace can play retained audio from a finished meeting. **Play all**
starts the whole timeline with both Me and Them. Selecting a transcript segment
seeks to its saved offset and selects its canonical channel. The transport can
switch between All, Me, and Them without rewriting the timeline. Display labels
and remote-speaker names do not change audio channel ownership.
All adds the retained channels at their recorded levels. Single-channel playback
also preserves its recorded level; playback does not add a gain or normalization setting.

Unretained sessions show an explanation instead of a player. Active meetings
cannot expose retained audio. Finished failed or interrupted sessions can play
valid retained chunks; pending transcription chunks are excluded until final or
failed. The manifest does not read audio files, so a missing or damaged file is
reported when playback requests its bytes.

## Timeline and memory

Capture saves separate mono, 16 kHz, PCM16 WAVs under its private meeting store.
Each chunk is at most 15.5 seconds. The database already records each channel's
capture-relative start and end. Playback uses those offsets, so simultaneous
Me/Them speech remains simultaneous and gaps remain silent. It never concatenates
clips in transcript-row order.

The backend provides metadata pages of 128 chunks by default, capped at 256.
Pages use a `(startMs, segmentId)` cursor and include chunks overlapping the seek
position. The duration is the latest retained chunk end across both channels;
it excludes any extra time spent finishing transcription after audio capture.
A selected channel can have an empty page while the other channel remains available.

The main webview reads individual WAVs through binary IPC in ranges no larger
than 64 KiB. It retains only the current and next decoded chunk for each selected
channel. Future chunks beyond the small scheduling horizon remain metadata,
including long silent gaps. No whole-session audio buffer or merged WAV is created.

## Native boundary

All playback commands require the main window. The frontend sends session and
segment identifiers, never paths. Each byte request joins those identifiers in
SQLite and checks retained-audio consent and finished-session status. The stored
audio reference must match the exact session, channel, and sequence path generated
by capture.

File opens refuse symlinks in every path component and reject hardlinks,
directories, devices, and other nonregular files. Descriptor metadata caps the
file at 512 KiB before reading. A bounded parser validates the generated RIFF
header, PCM16 mono format, 16 kHz rate, data length, and duration against the
segment interval. It accepts the generated format/data chunks and rejects unknown
chunks or trailing content. Exact EOF returns empty bytes; offsets beyond EOF
and oversized requests fail with stable errors containing no paths or audio.

Manifest and byte-range commands share two admitted background workers. Requests
beyond that limit fail immediately. Every byte request checks capture activity
before and after file work. If the retained-audio revision changes, the request
revalidates its exact session and segment instead of interrupting playback for
an unrelated deletion. Descriptor and current-path checks reject replaced or
unlinked files even when an old open descriptor remains readable.

## Pausing and deletion

Playback pauses when dictation, a meeting, a transform, a query, or a microphone
test becomes active. A native busy snapshot covers initial state and explicit
Play requests; events stop already scheduled sources. A playback generation
prevents delayed reads and decodes from restarting paused or deleted audio.
Calendar access and model installation are unrelated to playback admission.
The shared `transform-capture-starting` event pauses playback before accepted
transform, correction, and retry microphone arms, including slow AX selection.
Terminal `transform-review-hidden` and `transform-capture-failed` notifications
refresh the busy state; explicit Play always checks the native owners again.

Deleting one meeting invalidates its playback. Pruning invalidates only sessions
selected for removal, including retention applied at meeting start. A no-op prune
emits nothing. Delete all invalidates all playback. These events go only to the
main window. The backend also revokes outstanding reads when stored audio is removed.

Single-session deletion unlinks referenced WAVs through an opened, checked
session directory before committing the database deletion. File removal failures
return an error and preserve the session row for retry. Some files may already
be gone after a partial failure; retry treats missing files as removed. Delete all
similarly commits row deletion only after its audio directory is removed and
recreated. None of these paths follows an audio-directory symlink.

## Verification

Synthetic WAV and SQLite tests cover retention and terminal-state gates,
cross-session ownership, cursor pagination, overlaps and long gaps, binary range
round trips, EOF, malformed headers, file limits, symlinks, hardlinks, deletion
revocation, and retryable cleanup failures. Frontend tests cover bounded loading,
seek/channel behavior, pause events, and stale asynchronous work. Native smoke
testing uses an isolated synthetic retained-audio fixture; it does not establish
that microphone or system-audio recording succeeded.
