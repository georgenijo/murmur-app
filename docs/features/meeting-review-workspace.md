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

## Notetaker layout and actions

Notetaker has one page title and capture action. Its status area keeps a stable
57px height (75px at compact window widths), with a neutral idle dot. Microphone,
Mac audio, and unchecked access use a consistent series of source markers.
Check access is an explicit permission request, with a 24px visual control and
28px pointer target. Denied Mac audio access names the macOS permission pane,
provides Open System Settings as the primary recovery action, and keeps Start
meeting available for retry. No microphone-only capture is implied.

Saved meetings are searchable and grouped by day. Unnamed rows use the start
time; the detail heading uses the start date and time. Activating the heading
opens the existing title and attendee editor. The detail header's Copy transcript,
Export, and More actions share the player and transcript's right edge. Compact
windows put those actions on their own row and show Copy as an icon with an
accessible name. The meeting list and detail continue scrolling independently.

Copy transcript copies Markdown. Export offers the five supported formats and,
when audio was retained, Export with audio. More contains Rename, Calendar
naming, speaker editing, Copy as (a format submenu), and the existing two-step
Delete action. Caption menu items stay unavailable during capture; ordinary
transcript copy/export remain available. Bundle export still requires capture
to finish and presents the existing size, format choice, and missing-audio disclosure.

The 40px player preserves native channel selection, playback/seek handlers,
and loading, buffering, error, and capture-busy messages. Its played track is
derived from the actual playback position. Transcript timestamps use tabular
sans text with a reserved play-glyph slot: pointer hover and keyboard focus
reveal the glyph without shifting the time or transcript columns. Speakers
continues opening the real session-label editor.

An active meeting shows its live session, elapsed time, finalized segments, and
Listening status. The existing timer formatter already emits `4:32`; it is
unchanged. Naming and More are unavailable while capture is busy. With no saved
meetings, one centered first-meeting message and Start meeting action replace
the split list/detail layout. Empty search results retain the search controls.

Visual fixtures support `meetings-many`, `meetings-empty`, `meetings-recording`,
`meetings-permission-denied`, `meetings-export-menu`, and `meetings-more-menu` in
both appearances. Menu fixtures activate the actual production triggers, and
row hover/focus uses the production playback control. The older
`state=meetings-many&proposal=export-menu` / `more-menu` capture URLs remain
supported. No proposal CSS or DOM rewriting is needed.

Native verification: pending for the combined redesign build.
