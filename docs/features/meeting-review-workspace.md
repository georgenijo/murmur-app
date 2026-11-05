# Meeting transcript workspace

Notetaker lists saved meetings and opens their ordered Me/Them transcripts.
Capture, transcription, speaker refinement, and retained-audio playback remain
local. Search matches meeting titles and transcript segments.

Each meeting supports manual naming, Calendar naming, attendee metadata, Me/Them
channel labels, and session-scoped remote speaker names. Canonical channel
identity and transcript evidence remain unchanged when a label is edited.

## Meeting captions

Copy and export support Markdown, plain text, JSON, SRT, and VTT. Captions require
a completed capture.

## Export with audio

Export with audio bundles the transcript, retained WAVs,
and a manifest; unavailable audio is disclosed before saving. Playback and
capture share native admission checks. The audio manifest and WAV readers keep
all filesystem path resolution in Rust.

Existing database schemas remain supported. Legacy derived documents and their
search rows are retained but ignored. Reading a transcript does not deserialize
that content, and speaker-label updates preserve its original bytes. No migration
deletes user data. New captures produce transcripts and speaker labels only.
