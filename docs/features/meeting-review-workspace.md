# Meeting Review Workspace

## Problem

Meeting capture already stores immutable `Me` and `Them` transcript evidence and
can explicitly generate one strict, sourced artifact. The review workspace adds
user-authored labels and edits without rewriting raw segments, weakening source
provenance, or letting regeneration erase reviewed work.

## Usage

The Notetaker loads one Rust-owned workspace snapshot for the selected meeting.
The user can rename the two capture channels, edit existing generated claims,
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
exists. Search remains limited to finalized raw transcript text.

```rust
pub struct MeetingWorkspace {
    pub session: MeetingSession,
    pub segments: Vec<MeetingSegmentView>,
    pub generated: StoredGeneratedArtifact,
    pub review: StoredMeetingReview,
    pub active_document: ActiveReviewDocument,
}

pub struct SaveMeetingReviewRequest {
    pub session_id: String,
    pub base: ReviewEditBaseInput,
    pub labels: SpeakerLabelsInput,
    pub document: Option<EditableReviewDocumentInput>,
}

pub enum ReviewEditBaseInput {
    LabelsOnly { expected_review_revision: Option<u64> },
    Generated {
        generated_revision: u64,
        expected_review_revision: Option<u64>,
    },
    Review { review_revision: u64 },
}

pub struct RestoreMeetingReviewRequest {
    pub session_id: String,
    pub generated_revision: u64,
    pub review_revision: u64,
}
```

Editable review items carry opaque keys and editable values, but no source IDs.
For a generated base, Rust derives deterministic keys from the artifact revision,
section, and position. For a review base, it loads the persisted keys. A save must
submit the summary key and an ordered subset of each section's keys, with no
duplicates or cross-section moves. Rust rehydrates the original source IDs,
validates all bounds and dates, checks the expected revisions, then commits labels
and the complete review document in one `BEGIN IMMEDIATE` transaction. New claims
are out of scope until a source-selection workflow exists.

The repository exposes four deep capabilities:

```rust
fn workspace(id: &MeetingSessionId) -> Result<MeetingWorkspace, MeetingReviewError>;
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

## Export contract

All three formats represent one fixed scope named a reviewed meeting:

- bounded session metadata and both canonical/display speaker labels;
- the active reviewed document, or the generated draft when no review exists;
- every ordered transcript segment and explicit failed/pending gaps;
- source references for every artifact claim.

Markdown uses transcript anchors, plain text names segment IDs and timestamps, and
JSON uses `murmur.meeting-review-export.v1`. Exports exclude audio, local paths,
prompts, discarded drafts, and hidden runtime metrics. Rust builds one validated
snapshot and renders all formats from it. Clipboard rendering and the existing
atomic `.md`/`.txt`/`.json` sink share the 8 MiB bound and never truncate evidence.

## Frontend ownership

`useMeetings` owns a monotonically increasing selection ticket so a slow response
for meeting A cannot replace a later selection B. It exposes operations rather than
state setters. `MeetingsPanel` retains capture/history/deletion duties and composes:

- `MeetingReviewWorkspace` for reviewed/generated views, edit, regenerate,
  restore, copy/export, and explicit empty/error states;
- `MeetingArtifactEditor` for the controlled accessible form;
- `MeetingTranscript` for canonical evidence rows and source focus/highlight.

Source references are native buttons with `aria-controls`. Activation scrolls and
focuses a `tabIndex={-1}` transcript article and announces its timestamp, display
label, and canonical channel. Save conflicts, invalid stored documents, generation
failure/cancellation, unavailable meetings, copy/export failure, and empty sections
remain visible and actionable.

## Synthesis decision

Candidate A is the base because its edit DTO cannot forge provenance, its active
document is resolved by Rust, and its revisioned full-snapshot save keeps labels and
review content atomic. Candidate B contributed the separately named, confirmed,
revision-checked restore operation. Client-supplied source IDs, timestamp-only
generation identity, generation-time review seeding, separate label writes, and
frontend-owned export rendering were rejected.

## Tradeoffs accepted

- We store a second strict local prose snapshot so regeneration cannot erase edits.
- We use optimistic revision conflicts instead of silently merging concurrent saves.
- We do not allow new claims until the UI can bind them to deliberately selected
  transcript sources.
- We export the transcript with the review so citations remain meaningful outside
  Murmur.
- We fail when a complete export exceeds 8 MiB instead of truncating evidence.

## Verification

- Migration tests cover v2 backup, v3 columns/foreign key, deletion, pruning, and
  raw-only search.
- Repository tests cover invalid labels, revision conflicts, key forgery,
  source rehydration, reorder/removal, regeneration preservation, and restore.
- Export goldens prove equivalent scope and provenance across all three formats.
- Frontend tests cover stale selection, edit/save/conflict, source focus, keyboard
  navigation, generation states, copy/export, deletion, and empty/error states.
- Visual fixtures mount the real workspace at normal and narrow window sizes.
- Native smoke uses stored fixture data and does not require microphone capture.
