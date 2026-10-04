# Meeting Review Workspace

## Problem

Meeting capture already stores immutable `Me` and `Them` transcript evidence and
can explicitly generate one strict, sourced artifact. The review workspace adds
user-authored labels and edits without rewriting raw segments, weakening source
provenance, or letting regeneration erase reviewed work.

## Usage

Sessions with retained audio expose [audio playback](meeting-audio-playback.md).
Play all follows the original Me/Them timeline. Selecting a segment seeks to its
offset on the canonical channel, regardless of its editable speaker label.
Sessions without retained audio explain why playback is unavailable.

The Notetaker loads one Rust-owned workspace snapshot for the selected meeting.
The user can name the meeting, maintain its attendee list, rename the two capture
channels, edit existing generated claims, select transcript segments to add a
decision, action item, or open question to an existing review draft,
reorder or remove list items, save the review, follow a source to the transcript,
regenerate a separate draft, deliberately restore that draft, and copy or export
the reviewed meeting as Markdown, plain text, or JSON.

The three persistence planes have different write owners:

| Plane | Stored data | Mutable operation |
|---|---|---|
| Evidence | `meeting_segments` | Capture finalization only |
| Generated draft | `meeting_artifacts` | Full validated replacement after explicit generation |
| User review | `meeting_reviews` | Revision-checked full snapshot save after explicit review |

Regeneration writes only the generated plane. A saved review remains the active
document until the user confirms **Replace review with generated draft**.

## Shape

Schema v3 adds a monotonic `revision` to `meeting_artifacts` and one
`meeting_reviews` row per session. The review row contains its own revision,
the generated revision it was based on, bounded `Me` and `Them` display labels,
an optional strict review document, and an update timestamp. It references the
session with `ON DELETE CASCADE`. Default labels are derived when no review row
exists. Schema v5 adds an optional title, a typed `manual`, `calendar`, or
`generated` title source, an ordered attendee list, and a separate title FTS table.
Existing sessions have no title or source and an empty attendee list. Search covers
the current title, finalized raw transcript text, and the active document. Schema
v6 adds `meeting_review_fts` rows for summary, decisions, action items (including
owner and due date), and open questions. A saved review document supersedes the
generated draft; a labels-only review still searches the draft. Upgrades backfill
the index through the existing backup and integrity-check path. Invalid stored
documents are unavailable and are not indexed.

Review saves, draft replacement, and restore update the derived index in the same
transaction as their source. Deletion and pruning clear its rows. Searches retain
the existing literal, tokenized AND query semantics: all query terms must match
one title, transcript segment, summary, decision, action item, or open question.
The paginated response includes deduplicated `searchMatches` field identifiers
for returned sessions only (`title`, `transcript`, `summary`, `decision`,
`action_item`, `open_question`). The meeting list shows these labels without
adding snippets. Queries and results stay local and never enter logs or telemetry.

```rust
pub struct MeetingWorkspace {
    pub session: MeetingSession,
    pub segments: Vec<MeetingSegmentView>,
    pub generated: StoredGeneratedArtifact,
    pub review: StoredMeetingReview,
    pub active_document: ActiveReviewDocument,
}

pub struct MeetingSession {
    pub title: Option<String>,
    pub title_source: Option<MeetingTitleSource>,
    pub attendees: Vec<String>,
}

pub enum MeetingTitleSource {
    Manual,
    Calendar,
    Generated,
}

pub struct SaveMeetingMetadataRequest {
    pub session_id: String,
    pub title: Option<String>,
    pub attendees: Vec<String>,
}

pub struct SaveMeetingReviewRequest {
    pub session_id: String,
    pub expected_review_revision: Option<u64>,
    pub base: ReviewEditBaseInput,
    pub labels: SpeakerLabelsInput,
    pub document: Option<EditableReviewDocumentInput>,
    pub new_claims: Vec<NewReviewClaim>,
}

pub enum ReviewEditBaseInput {
    LabelsOnly,
    Generated { generated_revision: u64 },
    Review { review_revision: u64 },
}

pub struct RestoreMeetingReviewRequest {
    pub session_id: String,
    pub generated_revision: u64,
    pub expected_review_revision: Option<u64>,
}
```

Editable review items carry opaque keys and editable values, but no source IDs.
For a generated base, Rust derives deterministic keys from the artifact revision,
section, and position. For a review base, it loads the persisted keys. A save must
submit the summary key and an ordered subset of each section's keys, with no
duplicates or cross-section moves. Rust rehydrates the original source IDs,
validates all bounds and dates, checks the expected revisions, then commits labels
and the complete review document in one `BEGIN IMMEDIATE` transaction.

Select **Use as source** on one or more transcript rows, then choose **New
decision**, **New action item**, or **New open question**. Enter the claim text
and optional action owner/due date, then **Save review**. The selected sources
are fixed for that new claim; choosing other rows creates a separate claim.
Selection and unsaved claims are transient. **Cancel** discards them, and a failed
save keeps them available to retry. Generate a review draft first if none exists.

New claims use a separate typed `newClaims` field on the same save request; they
have no client-supplied key. Rust assigns deterministic opaque keys from the
session, committed review revision, and creation position, validates the combined
document with the existing allowed-source check, and persists the source mapping.
Every new claim needs non-empty text and at least one finalized, non-empty segment
from this meeting. Later edits carry only the saved key and editable values, and
Rust rehydrates those sources exactly as it does for existing generated claims.

The repository exposes five deep capabilities:

```rust
fn workspace(id: &MeetingSessionId) -> Result<MeetingWorkspace, MeetingReviewError>;
fn save_metadata(request: SaveMeetingMetadataRequest) -> Result<MeetingWorkspace, String>;
fn save_review(edit: ValidatedReviewEdit) -> Result<MeetingWorkspace, MeetingReviewError>;
fn restore_review_from_generated(request: ValidatedRestoreRequest)
    -> Result<MeetingWorkspace, MeetingReviewError>;
fn replace_generated_artifact(id: &MeetingSessionId, artifact: ValidatedGeneratedArtifact,
    metrics: GenerationMetrics) -> Result<GeneratedArtifactRevision, MeetingReviewError>;
```

Commands parse strict DTOs. The repository owns SQLite representation, revision
checks, provenance reconstruction, active-document precedence, and transactions.
React never parses persisted JSON or decides whether generated or reviewed content
is authoritative.

`save_meeting_metadata` accepts only `sessionId`, `title`, and `attendees` from
the main window. Rust trims and validates every value, limits titles and attendee
names to 200 characters, and accepts at most 100 attendees. A non-null title gets
the `manual` source on the server. Clearing the title also clears its source. The
same immediate transaction updates the session row and title FTS row without
changing transcript evidence or review revisions. [Calendar naming](meeting-calendar.md)
uses a separate confirmed apply command to assign `calendar`. A later manual
rename assigns `manual` again.

## Export contract

All three formats represent one fixed scope named a reviewed meeting:

- optional title, title source, ordered attendees, and both canonical/display
  speaker labels;
- the active reviewed document, or the generated draft when no review exists;
- every ordered transcript segment and explicit failed/pending gaps;
- source references for every artifact claim.

Markdown uses transcript anchors, plain text names segment IDs and timestamps, and
JSON uses `murmur.meeting-review-export.v2`. Markdown, text, and JSON include the
title and attendees. Exports exclude audio, local paths,
prompts, discarded drafts, and hidden runtime metrics. Rust builds one validated
snapshot and renders all formats from it. Clipboard rendering and the existing
atomic `.md`/`.txt`/`.json` sink share the 8 MiB bound and never truncate evidence.

### Meeting captions

Choose **SubRip (.srt)** or **WebVTT (.vtt)** in the Format menu to copy captions
or save a subtitle file. Stop the meeting first. Captions use the saved transcript
and speaker names; they exclude review summaries, action items, and audio.

Each recorded speech segment becomes one cue with its original start and end
times relative to the meeting. These are segment timestamps, not word alignment.
Simultaneous speakers retain overlapping cues, ordered by start time and segment
ID. Named remote speakers use their saved name; uncertain passages keep the Them
channel label. Empty final segments are omitted. Pending and failed segments
appear as **[Transcription pending]** and **[Transcription unavailable]** so a
subtitle file cannot silently hide a gap.

Exports reject zero-length or reversed cue times, active meetings, and meetings
with no transcribed speech. Caption text and labels escape markup and normalize
whitespace to prevent embedded text from creating a new cue. Both formats use the
same 8 MiB ceiling and atomic save path as document exports. Saving requires the
extension that matches the selected format.

Some SubRip readers display escaped `<`, `>`, and `&` as entity codes. Use
WebVTT when the destination needs those characters literally; removing their
escaping can cause subtitle readers to hide text or interpret it as formatting.

## Frontend ownership

`useMeetings` owns a monotonically increasing selection ticket so a slow response
for meeting A cannot replace a later selection B. It exposes operations rather than
state setters. `MeetingsPanel` retains capture/history/deletion duties and composes:

- `MeetingReviewWorkspace` for reviewed/generated views, the controlled edit
  form, transient source selection/new claims, regenerate, restore, copy/export,
  and explicit empty/error states. Its `TranscriptRow` and `SourceLinks` render
  canonical evidence and accessible source focus.

Source references are native buttons with `aria-controls`. Activation scrolls and
focuses a `tabIndex={-1}` transcript article and announces its timestamp, display
label, and canonical channel. Save conflicts, invalid stored documents, generation
failure/cancellation, unavailable meetings, copy/export failure, and empty sections
remain visible and actionable.

## Synthesis decision

Candidate A is the base because its edit DTO cannot forge provenance, its active
document is resolved by Rust, and its revisioned full-snapshot save keeps labels and
review content atomic. Candidate B contributed the separately named, confirmed,
revision-checked restore operation. Client-supplied sources for existing keys, timestamp-only
generation identity, generation-time review seeding, separate label writes, and
frontend-owned export rendering were rejected.

## Tradeoffs accepted

- We store a second strict local prose snapshot so regeneration cannot erase edits.
- We use optimistic revision conflicts instead of silently merging concurrent saves.
- New claims require deliberately selected transcript sources; existing claims'
  provenance remains immutable during editing.
- We export the transcript with the review so citations remain meaningful outside
  Murmur.
- We fail when a complete export exceeds 8 MiB instead of truncating evidence.

## Verification

- Migration tests cover older backups, v3 and v4 changes, v5 metadata defaults,
  deletion, pruning, transcript search, and title search.
- Repository tests cover invalid labels, revision conflicts, key forgery,
  source rehydration, reorder/removal, regeneration preservation, and restore.
- Export goldens prove equivalent scope and provenance across all three formats.
- Frontend tests cover stale selection, edit/save/conflict, source focus, keyboard
  navigation, generation states, copy/export, deletion, and empty/error states.
- Visual fixtures mount the real workspace at normal and narrow window sizes.
- Native smoke uses stored fixture data and does not require microphone capture.
