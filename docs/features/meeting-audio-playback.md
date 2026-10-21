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

## ZIP export with audio

**Export with audio…** in the review's export row saves one `.zip` containing the
selected review document and every retained WAV. Choose Markdown, plain text, or
JSON first; SubRip and WebVTT are not offered for this action. The file name is
chosen in the native save dialog and must end in `.zip`.

Archive contents:

- `meeting-review.md`, `.txt`, or `.json`: exactly the bytes `render_export`
  returns for that format, the same as Copy review and Export…. The document
  excludes audio and local paths and is subject to its own 8 MiB bound.
- `audio/me-00000000.wav`, `audio/them-00000000.wav`, and so on: one entry per
  retained final or failed chunk, in timeline order (chunks still pending
  transcription are not included; see the notice below). Names come from the
  channel and capture sequence, never from a stored path. Entries are the saved
  WAV bytes, stored uncompressed: no transcoding, mixing, trimming, gap filling,
  or synthesis. The archive does not carry
  timeline offsets; the saved transcript timestamps are in the review document.
- `audio/MISSING-AUDIO.txt`, only when needed: lists, in separate sections, each
  retained segment whose file was absent ("Not found on this Mac") and each
  segment still pending transcription ("Still waiting for transcription, so the
  audio was not read"), with name, segment ID, and recorded start/end in
  milliseconds. The UI repeats both counts after the save. Nothing is
  substituted for these files.

Behavior and refusals:

- A session with Keep Meeting Audio off, an active meeting, or no retained audio
  chunks is refused; the button is disabled with an explanation in the same
  status line and the backend returns a clear error if called anyway. The action
  is also disabled while dictation, a meeting, a transform, a query, or a
  microphone test is active.
- Audio goes through the same session-scoped reader as playback: SQLite join,
  the exact generated reference checked by `MeetingRepository::audio_path`,
  no-follow opens that reject symlinks and hardlinks, bounded RIFF/PCM16 mono
  16 kHz validation, and identity revalidation after each read. A forged or
  traversing database reference, a symlink, a hardlink, or a malformed or
  oversized file fails closed: the whole export is refused and no archive is left.
  Only a file that is simply not found counts as missing. If every file is
  missing the export is refused rather than saved as a document-only ZIP.
- Pending chunks keep the playback restriction: their audio is never read, but
  they are counted in the summary (`pendingFiles`) and listed in the notice, so
  a finished or interrupted session with unfinished transcription never exports
  silently short. If every exportable chunk is missing or pending, the export is
  refused.
- Capture is checked before dispatch, before every segment read, and again at
  publication; starting a recording, dictation, transform, query, or microphone
  test mid-export refuses it and removes the temp file.
- Deleting a meeting (or all history, or pruning) holds the same lock that
  publication holds, from revocation through removal. An export samples its
  starting audio revision under that lock, so it waits out a delete in progress.
  Before the rename it always revalidates the session (an equal revision proves
  nothing, since a delete bumps it before removing anything): the session, its
  segment list, and every exported file must still be present and unchanged,
  otherwise the export fails with "changed or was
  deleted" and publishes nothing. Deleting an unrelated meeting does not block
  an export. Delete is not blocked in the UI during an export; whichever side
  takes the lock first wins.
- The export holds one of the two shared audio read slots. If both are in use it
  fails immediately and can be retried.

Size contract, checked before any file is created and again while writing: at
most 8,192 chunks, at most 1 GiB of WAV data (about nine hours of one channel at
16 kHz mono PCM16), each file at most 512 KiB as for playback. Larger sessions
are refused with "too large to export"; Murmur does not split, sample, or
shorten audio. Memory use is bounded by one chunk (512 KiB) plus the document;
audio is streamed into the archive. The archive stays within classic ZIP limits
(no ZIP64). Export is not cancellable once the save dialog is confirmed, and ZIP
entry timestamps are fixed rather than copied from the files.

The archive is built in an exclusively created (`O_EXCL`, no-follow, mode 0600),
uniquely named hidden sibling and renamed into place after a flush, so a failure
never leaves a partial `.zip` or replaces an existing one. Pre-existing files or
links at predictable temp names are never opened, and overlapping exports to one
destination each write their own file; the last rename wins with a complete
archive. Logs record only counts and byte totals.

## Verification

Synthetic WAV and SQLite tests cover retention and terminal-state gates,
cross-session ownership, cursor pagination, overlaps and long gaps, binary range
round trips, EOF, malformed headers, file limits, symlinks, hardlinks, deletion
revocation, and retryable cleanup failures. Frontend tests cover bounded loading,
seek/channel behavior, pause events, and stale asynchronous work. Native smoke
testing uses an isolated synthetic retained-audio fixture; it does not establish
that microphone or system-audio recording succeeded.

Export tests build real generated-format WAVs and cover byte-for-byte documents
in all three formats, every retained entry, unretained/active/empty refusals,
missing files (partial and total), symlink/hardlink/malformed files, forged and
traversing references, both size bounds, atomic failure with an existing
destination, and the `.zip`-only path allow-list. Race regressions are
deterministic: mixed final/pending sessions, planted temp symlinks and
hardlinks, an overlapping export completing mid-build, a session deleted or a
WAV removed between the last read and publication, an export starting while a
delete is paused after revocation, revalidation after a delete with an
unchanged revision, an unrelated deletion, a delete confirmed to be waiting on
the lock while the export holds the publish lock, and capture starting
mid-read or at publication. Frontend tests cover the enabled, disabled, busy,
cancelled, missing-file, and pending-file states.
