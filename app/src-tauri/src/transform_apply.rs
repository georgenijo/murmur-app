//! Applies (and undoes) a reviewed transform result back into the target
//! document, and undoes it (issue #312, PR-B2).
//!
//! This is the write-side counterpart to `selection.rs` (which only reads).
//! Every AX/clipboard/paste call is macOS-only and dispatched onto the main
//! thread, mirroring `injector::inject_text`'s constraint. On non-macOS
//! builds every entry point returns `ApplyError::Unsupported` — the module
//! still compiles cross-platform so the pure decision-table logic below is
//! unit-testable without Accessibility permission, a running AX server, or
//! macOS at all.
//!
//! Privacy: neither the snapshot's original text nor the proposed replacement
//! text is ever logged or attached to an event payload. `transform-apply-failed`
//! carries only `ApplyError::as_str()`.
//!
//! ## Clipboard save/restore policy (reconciled)
//!
//! The house rule is unconditional: **the proposed text is written to the
//! clipboard before anything else happens**, so the user never loses the
//! transform result even if every write path below fails. The user's
//! pre-apply clipboard contents are captured (best-effort) before that write
//! so they can be put back later. This house-rule write REPLACES whatever was
//! on the clipboard, including non-text contents (an image, a file reference)
//! that `read_clipboard_text` cannot capture and therefore cannot restore —
//! accepted as the cost of the clipboard-first guarantee, same as
//! `inject_text`'s.
//!
//! What happens to that saved original afterward depends on whether the
//! proposed text actually reached the document:
//!
//! | Outcome | Original clipboard | Rationale |
//! |---|---|---|
//! | AX set succeeded (confirmed or unverified) | Restored immediately | Text landed in the document — the clipboard no longer needs to be the sole record of it. Synchronous, no delay needed. |
//! | AX set failed, Cmd+V paste succeeded | Restored after ~300ms, off the main thread | Same reasoning, one hop later. The delay lets the synthetic keystroke actually land before we overwrite what the target app may still be reading from the pasteboard; it runs on a background task (`tokio::task::spawn_blocking`), never blocking the main thread the AX/paste work ran on. |
//! | Target gone / selection changed under us | **Not restored** — proposed text stays in the clipboard | We deliberately did not touch the document; the user's only path to the transform result is now a manual paste. |
//! | AX set failed and the paste fallback also failed (or was skipped — target gone right before pasting) | **Not restored** — proposed text stays in the clipboard | Same reasoning: nothing landed, so the clipboard is the fallback delivery mechanism. `transform-apply-failed` fires so the frontend can show the banner. |
//!
//! In short: restore the original clipboard whenever the proposed text is
//! believed to have reached the document by *some* mechanism; leave the
//! proposed text in place whenever it did not. This is captured as a pure
//! function (`decide_apply_outcome`) below and exercised by a decision-table
//! test so every branch is covered without touching AX or the real clipboard.
//!
//! ## Why every AX write failure falls back to paste
//!
//! `selection.rs`'s read-side classifier fails closed on any AX ambiguity
//! (reading is a privacy-sensitive act: an ambiguous status must not be
//! silently treated as "safe to read"). Writing is different: Cmd+V is the
//! same universal fallback auto-paste already relies on, and the clipboard
//! has already been loaded with the proposed text by the house rule above, so
//! attempting it is always safe. Rather than trying to special-case
//! `kAXErrorAttributeUnsupported` (-25205) from every other AX failure, any
//! non-success status from the attribute-set call routes to the paste
//! fallback — the two are handled identically because the recovery action is
//! identical.
//!
//! Before committing to the paste, two more checks run right at that point
//! (not reusing anything computed earlier, since both can go stale in the
//! tens-to-hundreds of milliseconds an AX round-trip takes):
//! - **Already written?** The AX write's own messaging timeout (100ms — see
//!   below) can fire before the target finishes applying it, reporting
//!   failure for a write that lands moments later. Re-reading the selection
//!   right before pasting and finding the replacement already there reports
//!   `AppliedVia::AxUnverified` and skips the paste, avoiding a double
//!   insert. Known gap: this only catches it if the selection is still
//!   there to re-read. Some target apps COLLAPSE the selection to an
//!   insertion-point caret right after a programmatic text replacement
//!   (rather than keeping the new text selected) — `AXSelectedText` then
//!   reads back empty, `already_written` evaluates `false`, and the paste
//!   fallback still fires, producing a double insert. Not closed by this
//!   PR; a future revision could widen the re-read to the surrounding text
//!   range instead of only the current selection.
//! - **Still frontmost?** The initial frontmost check (at the top of
//!   `apply_text_to_target`, after activation) is ~100-150ms stale by the
//!   time the paste fallback is considered — plenty of time for the user to
//!   switch apps. A fresh, lightweight re-check (no re-activation, no sleep)
//!   catches that; a mismatch fails closed as `ApplyError::TargetGone`
//!   instead of posting Cmd+V into whatever is now frontmost.
//!
//! ## Apply vs. undo: the selection range is not interchangeable
//!
//! `snapshot.range` is the range **the original text** occupied at capture
//! time. `apply_transform` restores that range as-is, because the document
//! still holds the original text (untouched) while `TransformStatus` sits at
//! `ReviewPending` — this is also why `apply_transform` re-verifies the
//! restored (or current) selection's *content* against `snapshot.text` before
//! writing (below), not just its position: `ReviewPending` is user-paced, so
//! the document may have been edited since capture.
//!
//! `undo_applied_transform`, however, is restoring the range **the proposed
//! text** now occupies — which is a different length whenever the proposed
//! text isn't exactly as long as the original (in UTF-16 code units, the unit
//! AX ranges use). Reusing `snapshot.range`'s original length for undo would
//! restore a selection sized for text that's no longer there: too short
//! (proposed longer than original) leaves residue after the write; too long
//! (proposed shorter) spills into whatever document text follows and undo
//! then overwrites it. `range_len_for` computes the correct length for each
//! direction instead of assuming the original's.
//!
//! Both directions also require the destination content to match what's
//! expected (`expected_current`) before writing — never overwrite based on
//! range alone.
//!
//! Known gap: some target apps normalize text on the way in — straight
//! quotes rewritten to smart/curly quotes, or Unicode NFC normalization —
//! so what `AXSelectedText` reads back after a write isn't always
//! byte-identical to what was written. `apply_transform`'s own
//! verify-after-write already surfaces this: a normalizing target reports
//! `AppliedVia::AxUnverified` rather than the clean `Ax` match, and that's
//! the leading indicator to watch for. `undo_applied_transform`'s
//! `expected_current` check compares against the proposed text exactly as
//! Murmur wrote it, so on a normalizing target it will not match what's
//! actually in the document — undo then fails closed as
//! `ApplyError::SelectionChanged` rather than risk overwriting text that
//! only differs by normalization. Not solved by this PR.
//!
//! ## Session identity (generation counter)
//!
//! Each `TransformSession` (`start_session`) is stamped with a monotonic
//! `generation`. `apply_transform`/`undo_applied_transform` capture it when
//! they read the session and pass it to `set_applied`, which only mutates the
//! CURRENT session if its generation still matches. This closes a race where
//! a slow in-flight apply/undo (main-thread AX work, or the backgrounded
//! clipboard restore) completes after the user has already started a new
//! transform pass — without the check, that stale completion would flip
//! `applied` on a session it no longer belongs to.

#![allow(dead_code)]

use crate::state::{AppState, TransformStatus};
use crate::MutexExt;

/// A captured selection plus whatever the transform pipeline has produced (or
/// applied) for it so far. Exactly one instance lives in `AppState` at a time.
///
/// `proposed` is filled in by a later PR in the #312 series (the sidecar
/// transform call); this PR only defines the shape and the apply/undo
/// machinery that consumes it.
#[derive(Clone)]
pub struct TransformSession {
    pub snapshot: crate::selection::TransformSnapshot,
    /// The spoken instruction transcribed for this pass (issue #312 PR-C2).
    /// Filled by `finish_transform_instruction` before the sidecar call.
    /// Surfaced ONLY via `get_transform_review_content` (pulled by the popover
    /// window) — never logged or attached to an event payload.
    pub instruction: Option<String>,
    pub proposed: Option<String>,
    pub applied: bool,
    /// Monotonic id stamped at `start_session` time. See the module doc
    /// comment's "Session identity" section.
    pub generation: u64,
}

impl TransformSession {
    pub fn new(snapshot: crate::selection::TransformSnapshot, generation: u64) -> Self {
        Self {
            snapshot,
            instruction: None,
            proposed: None,
            applied: false,
            generation,
        }
    }
}

// -- Session setter/getter APIs (internal; C2 fills `proposed` from the
// sidecar in a later PR) --

/// Start a new session for a freshly captured selection, replacing whatever
/// session (if any) was active. There is only ever one active session.
pub fn start_session(app_state: &AppState, snapshot: crate::selection::TransformSnapshot) {
    let generation = app_state.next_transform_session_generation();
    *app_state.transform_session.lock_or_recover() = Some(TransformSession::new(snapshot, generation));
}

/// Fill in (or replace) the proposed replacement text for the active session.
/// Returns `false` if there is no active session.
pub fn set_proposed_text(app_state: &AppState, text: String) -> bool {
    let mut session = app_state.transform_session.lock_or_recover();
    match session.as_mut() {
        Some(active) => {
            active.proposed = Some(text);
            true
        }
        None => false,
    }
}

/// Fill in (or replace) the transcribed instruction for the active session
/// (issue #312 PR-C2). Returns `false` if there is no active session.
pub fn set_instruction(app_state: &AppState, instruction: String) -> bool {
    let mut session = app_state.transform_session.lock_or_recover();
    match session.as_mut() {
        Some(active) => {
            active.instruction = Some(instruction);
            true
        }
        None => false,
    }
}

/// Snapshot (clone) of the active session, if any. Used by tests and by
/// `apply_transform`/`undo_transform` to read the session without holding the
/// lock across the AX/clipboard work below.
pub fn session_snapshot(app_state: &AppState) -> Option<TransformSession> {
    app_state.transform_session.lock_or_recover().clone()
}

/// Clear the active session unconditionally. Called at the two invalidation
/// points B1 already established: the start of a new dictation recording
/// (`commands::recording::start_native_recording`, only once a recording is
/// actually about to start) and the start of a new transform pass (the
/// transform hotkey press in `keyboard.rs`, gated there on `TransformStatus`
/// being `Idle`/`ReviewPending` so a mid-flight pass can't be clobbered).
/// Returns whether a session was actually present to clear.
pub fn clear_session(app_state: &AppState) -> bool {
    app_state.transform_session.lock_or_recover().take().is_some()
}

/// Mark the active session as applied (or not) — but only if `generation`
/// still matches the current session's. A mismatch means the session that
/// was applied/undone has since been replaced or cleared (see the module doc
/// comment's "Session identity" section); mutating it now would corrupt a
/// session this call no longer has anything to do with, so it silently
/// no-ops instead.
fn set_applied(app_state: &AppState, applied: bool, generation: u64) {
    if let Some(active) = app_state.transform_session.lock_or_recover().as_mut() {
        if active.generation == generation {
            active.applied = applied;
        }
    }
}

/// Which of the two write mechanisms actually wrote text into the document,
/// if either. Not an error — both are "applied", just with different
/// confidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppliedVia {
    /// `AXUIElementSetAttributeValue` succeeded and a verify-after-write read
    /// back confirmed the change landed.
    Ax,
    /// `AXUIElementSetAttributeValue` succeeded but verify-after-write did
    /// not confirm it (mismatch, or the attribute isn't readable at all), OR
    /// the set call reported failure but a re-read right before the paste
    /// fallback showed the replacement already in place (its messaging
    /// timeout can fire before the target finishes applying it). Applied but
    /// unconfirmed — deliberately not an error.
    AxUnverified,
    /// The AX set was refused/unsupported by the target, so the existing
    /// Cmd+V paste machinery (`injector::simulate_paste`) was used instead.
    Paste,
}

impl AppliedVia {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ax => "ax",
            Self::AxUnverified => "ax_unverified",
            Self::Paste => "paste",
        }
    }
}

/// Reasons `apply_transform`/`undo_transform` can fail. Every variant is
/// loggable/eventable on its own (no payload) — never attach snapshot or
/// proposed text to any of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyError {
    /// Not running on macOS — there is no AX/clipboard apply path here.
    Unsupported,
    /// `TransformStatus` was not in the state required for the requested
    /// action (`ReviewPending` for apply, `Idle` with `session.applied` for
    /// undo), or a concurrent apply/undo already claimed the transition.
    Busy,
    /// No active `TransformSession`.
    NoSession,
    /// The active session has no proposed replacement text yet.
    NoProposedText,
    /// `apply_transform` called on a session that is already applied.
    AlreadyApplied,
    /// `undo_transform` called on a session that has not been applied (or has
    /// already been undone — double-undo is rejected).
    NotApplied,
    /// Clipboard access failed before the house-rule write — nothing was
    /// attempted past this point.
    ClipboardUnavailable,
    /// The target app is no longer frontmost — either the initial check
    /// (activation failed to bring it back, or a different app took over)
    /// or the fresh re-check taken immediately before the paste fallback.
    /// Fails closed: the document is never touched. Text stays in the
    /// clipboard.
    TargetGone,
    /// Neither restoring the original selection range nor falling back to
    /// the current selection produced content matching what was expected —
    /// fails closed rather than risk overwriting text the user never
    /// selected (or edited since capture, during `ReviewPending`).
    SelectionChanged,
    /// Both the AX write and the Cmd+V paste fallback failed.
    PasteFailed,
}

impl ApplyError {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unsupported => "unsupported",
            Self::Busy => "busy",
            Self::NoSession => "no_session",
            Self::NoProposedText => "no_proposed_text",
            Self::AlreadyApplied => "already_applied",
            Self::NotApplied => "not_applied",
            Self::ClipboardUnavailable => "clipboard_unavailable",
            Self::TargetGone => "target_gone",
            Self::SelectionChanged => "selection_changed",
            Self::PasteFailed => "paste_failed",
        }
    }
}

/// What must happen to the user's pre-apply clipboard contents once the
/// apply attempt is over. See the clipboard policy table in the module doc
/// comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardOutcome {
    /// The proposed text reached the document by some mechanism; put the
    /// user's original clipboard contents back.
    RestoreOriginal,
    /// The proposed text did not confirm as reaching the document; leave it
    /// in the clipboard as the fallback delivery path.
    LeaveProposed,
}

/// Coarse outcome of the primary `AXUIElementSetAttributeValue` attempt, used
/// to route through `decide_apply_outcome`. Pure representation of the native
/// call's result so the decision table is testable without any AX/OS access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxWriteAttempt {
    /// Not attempted at all — the pre-write guard failed first (target gone
    /// or selection changed under us).
    NotAttempted,
    /// The attribute-set call succeeded.
    Succeeded,
    /// The attribute-set call failed. Every non-success `AXError` — including
    /// but not limited to `kAXErrorAttributeUnsupported` — routes here; see
    /// the module doc comment for why they're not distinguished.
    Failed,
}

/// Outcome of the checks run immediately before committing to the Cmd+V
/// paste fallback (only reached when `ax_write == AxWriteAttempt::Failed`).
/// See the module doc comment's "Why every AX write failure falls back to
/// paste" section for what each check catches and why it's re-run fresh
/// rather than reusing anything computed earlier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrePasteCheck {
    /// A re-read of the selection already shows the replacement text in
    /// place — the AX set call actually succeeded despite reporting failure.
    /// Report `AppliedVia::AxUnverified` and skip the paste to avoid a
    /// double insert.
    AlreadyWritten,
    /// The target app is no longer frontmost by the time we're about to
    /// paste. Fails closed.
    TargetGone,
    /// Neither of the above — safe to attempt the paste fallback.
    Proceed,
}

/// Decision table for the pre-paste checks. Pure — no I/O — so it's covered
/// directly by `tests::pre_paste_check_table`.
pub fn decide_pre_paste_check(already_written: bool, still_frontmost: bool) -> PrePasteCheck {
    if already_written {
        PrePasteCheck::AlreadyWritten
    } else if !still_frontmost {
        PrePasteCheck::TargetGone
    } else {
        PrePasteCheck::Proceed
    }
}

/// Step 2+3 guard: decide whether it's safe to proceed to the attribute-set
/// call at all, given whether the target app is still frontmost and whether
/// the selection's CONTENT (after whichever range-restore attempt was made)
/// matches what's expected. Pure — no I/O — so it is covered directly by
/// `tests::pre_write_guard_table`.
///
/// `range_restore_succeeded` is retained in the signature for symmetry with
/// how the native caller naturally computes things (and to keep the existing
/// decision-table test's shape), but no longer independently changes the
/// outcome: the content check is unconditional now — even after a
/// successful range restore, the read-back selection must still match
/// `current_selection_matches_expected`, since `ReviewPending` is user-paced
/// and the document may have been edited since capture.
pub fn decide_pre_write_guard(
    target_is_frontmost: bool,
    _range_restore_succeeded: bool,
    current_selection_matches_expected: bool,
) -> Result<(), ApplyError> {
    if !target_is_frontmost {
        return Err(ApplyError::TargetGone);
    }
    if current_selection_matches_expected {
        Ok(())
    } else {
        Err(ApplyError::SelectionChanged)
    }
}

/// Decision table for the clipboard-save/restore + AX-vs-paste routing
/// described in the module doc comment. Pure — no I/O, no AX, no clipboard —
/// so every branch is covered by `tests::apply_decision_table` without
/// needing Accessibility permission, a running AX server, or macOS at all.
pub fn decide_apply_outcome(
    pre_write_guard: Result<(), ApplyError>,
    ax_write: AxWriteAttempt,
    ax_verify_matched: Option<bool>,
    pre_paste_check: PrePasteCheck,
    paste_attempted: bool,
    paste_succeeded: bool,
) -> (Result<AppliedVia, ApplyError>, ClipboardOutcome) {
    if let Err(guard_error) = pre_write_guard {
        return (Err(guard_error), ClipboardOutcome::LeaveProposed);
    }
    match ax_write {
        AxWriteAttempt::Succeeded => {
            let via = match ax_verify_matched {
                Some(true) => AppliedVia::Ax,
                Some(false) | None => AppliedVia::AxUnverified,
            };
            (Ok(via), ClipboardOutcome::RestoreOriginal)
        }
        AxWriteAttempt::Failed => match pre_paste_check {
            PrePasteCheck::AlreadyWritten => {
                (Ok(AppliedVia::AxUnverified), ClipboardOutcome::RestoreOriginal)
            }
            PrePasteCheck::TargetGone => (Err(ApplyError::TargetGone), ClipboardOutcome::LeaveProposed),
            PrePasteCheck::Proceed => {
                if paste_attempted && paste_succeeded {
                    (Ok(AppliedVia::Paste), ClipboardOutcome::RestoreOriginal)
                } else {
                    (Err(ApplyError::PasteFailed), ClipboardOutcome::LeaveProposed)
                }
            }
        },
        AxWriteAttempt::NotAttempted => {
            // The guard was Ok (otherwise we'd have hit the early return
            // above), so reaching here would mean the guard passed but the
            // write was skipped anyway — not a reachable state from
            // `apply_text_to_target`, but treated conservatively as a
            // failure rather than silently reporting success.
            (Err(ApplyError::PasteFailed), ClipboardOutcome::LeaveProposed)
        }
    }
}

/// Length, in UTF-16 code units, of `text` — the unit AX text ranges use
/// (`AXSelectedTextRange`'s `CFRange.length`, exactly as
/// `selection.rs::decode_range` reads it from the native API without
/// reinterpretation). See the module doc comment's "Apply vs. undo" section
/// for why `apply_transform` and `undo_applied_transform` must each pass a
/// DIFFERENT string here rather than reusing `snapshot.range`'s original
/// length for both directions.
pub fn range_len_for(text: &str) -> usize {
    text.encode_utf16().count()
}

/// Delay before restoring the user's original clipboard contents after the
/// Cmd+V paste fallback. Cmd+V is an asynchronous keystroke through the HID
/// event queue and the target app's own event loop; overwriting the
/// clipboard immediately risks racing the app's own paste-read. Runs on a
/// background task (`tokio::task::spawn_blocking`), never on the main thread
/// — `arboard`'s clipboard access needs no particular thread, so there's no
/// reason to block the UI for it. No such delay (or backgrounding) is needed
/// on the AX path — `AXUIElementSetAttributeValue` is synchronous, so that
/// restore happens inline, immediately.
const PASTE_CLIPBOARD_RESTORE_DELAY_MS: u64 = 300;

/// Delay after re-activating the target app before checking whether it
/// became frontmost, mirroring `inject_text`'s default focus-settle delay.
/// Runs on the main thread (activation itself is a main-thread-only AppKit
/// call) but is small relative to the paste-restore delay above, which is
/// why only the latter was moved off it.
const ACTIVATION_SETTLE_MS: u64 = 50;

/// Pure pre-flight validation for `apply_transform`: does a session exist,
/// is it not already applied, and does it have proposed text? Factored out
/// of `apply_transform` so the session-lifecycle rules are unit-testable
/// without an `AppHandle` (real or mock) at all. Returns the text to write,
/// the snapshot to write it against, and the session's generation (for the
/// later `set_applied` call).
fn validate_apply(
    session: &Option<TransformSession>,
) -> Result<(String, crate::selection::TransformSnapshot, u64), ApplyError> {
    let session = session.as_ref().ok_or(ApplyError::NoSession)?;
    if session.applied {
        return Err(ApplyError::AlreadyApplied);
    }
    let text = session.proposed.clone().ok_or(ApplyError::NoProposedText)?;
    Ok((text, session.snapshot.clone(), session.generation))
}

/// Pure pre-flight validation for `undo_transform`: does a session exist and
/// is it currently applied? A second call after a successful undo (which
/// flips `applied` back to `false`) is rejected here — this is what makes
/// double-undo a hard error rather than a silent no-op. Returns the
/// snapshot, the currently-applied proposed text (what's now in the
/// document — see the module doc comment's "Apply vs. undo" section), and
/// the session's generation.
fn validate_undo(
    session: &Option<TransformSession>,
) -> Result<(crate::selection::TransformSnapshot, String, u64), ApplyError> {
    let session = session.as_ref().ok_or(ApplyError::NoSession)?;
    if !session.applied {
        return Err(ApplyError::NotApplied);
    }
    // Invariant: `applied` only ever becomes true after a successful
    // `apply_transform`, which requires `proposed` to be `Some` — but guard
    // defensively rather than trust that invariant across future changes.
    let proposed = session.proposed.clone().ok_or(ApplyError::NoProposedText)?;
    Ok((session.snapshot.clone(), proposed, session.generation))
}

/// Shared engine behind `apply_transform` and `undo_applied_transform`: both
/// are "write `replacement` into the range currently occupied by
/// `expected_current`, of length `range_len` starting at `range_start`" —
/// they differ only in which text is which (see each caller), and in what
/// happens to `session.applied` afterward (handled by the caller, not here).
///
/// Dispatches the AX/clipboard work to the main thread exactly once and
/// returns as soon as it completes; any delayed clipboard restore is
/// scheduled on a background task rather than awaited here, so this future
/// resolves promptly instead of blocking on the ~300ms paste-restore delay.
#[cfg(target_os = "macos")]
async fn run_apply(
    app_handle: &tauri::AppHandle,
    pid: i32,
    range_start: Option<usize>,
    range_len: usize,
    expected_current: String,
    replacement: String,
    apply_epoch: u64,
) -> Result<AppliedVia, ApplyError> {
    let (tx, rx) = tokio::sync::oneshot::channel::<native::NativeApplyOutcome>();
    app_handle
        .run_on_main_thread(move || {
            let outcome =
                native::apply_text_to_target(pid, range_start, range_len, &expected_current, &replacement);
            let _ = tx.send(outcome);
        })
        .map_err(|_| ApplyError::Unsupported)?;
    let outcome = rx.await.map_err(|_| ApplyError::Unsupported)?;

    if let Some(original) = outcome.pending_clipboard_restore {
        // N1 (B2 review): a newer apply/undo/cancel bumps the apply epoch. If
        // one begins inside this ~300ms window it re-writes the clipboard with
        // its own house-rule text; this stale restore must NOT clobber that.
        // Capture the epoch we were scheduled under and only restore if it is
        // still current.
        let app = app_handle.clone();
        tokio::task::spawn_blocking(move || {
            std::thread::sleep(std::time::Duration::from_millis(PASTE_CLIPBOARD_RESTORE_DELAY_MS));
            use tauri::Manager;
            if let Some(state) = app.try_state::<crate::State>() {
                if state.app_state.transform_apply_epoch() != apply_epoch {
                    tracing::info!(
                        target: "transform",
                        "pending clipboard restore skipped — a newer apply/undo/cancel superseded it"
                    );
                    return;
                }
            }
            let _ = crate::injector::write_clipboard_text(&original);
        });
    }

    outcome.result
}

/// Apply the active session's proposed text to the target document.
///
/// Cross-platform signature; the real implementation is macOS-only (see
/// `native::apply_text_to_target`). On any other platform this is
/// `ApplyError::Unsupported` — there is no AX apply path to attempt.
pub async fn apply_transform(
    app_handle: &tauri::AppHandle,
    app_state: &AppState,
) -> Result<AppliedVia, ApplyError> {
    let session = session_snapshot(app_state);
    let (text, snapshot, generation) = validate_apply(&session)?;

    #[cfg(target_os = "macos")]
    {
        let range_start = snapshot.range.map(|(start, _)| start);
        // Apply: the document still holds the ORIGINAL text (untouched, per
        // `ReviewPending` being user-paced) — the range to restore is sized
        // for `snapshot.text`, not the proposed replacement.
        let range_len = range_len_for(&snapshot.text);
        let expected_current = snapshot.text.clone();
        let apply_epoch = app_state.next_transform_apply_epoch();
        let result = run_apply(app_handle, snapshot.pid, range_start, range_len, expected_current, text, apply_epoch).await;
        if result.is_ok() {
            set_applied(app_state, true, generation);
        }
        result
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app_handle, text, snapshot, generation);
        Err(ApplyError::Unsupported)
    }
}

/// Undo the active session's applied text, restoring `snapshot.text` via the
/// same apply machinery (AX-set-or-paste-fallback, clipboard save/restore).
/// Valid only while `session.applied` is true; sets it back to `false` on
/// success so a second undo is rejected (`ApplyError::NotApplied`).
///
/// Named `undo_applied_transform` (rather than `undo_transform`) to leave
/// that name free for the `#[tauri::command]` wrapper below, matching the
/// two registered command names exactly (`apply_transform_result`,
/// `undo_transform`).
pub async fn undo_applied_transform(
    app_handle: &tauri::AppHandle,
    app_state: &AppState,
) -> Result<(), ApplyError> {
    let session = session_snapshot(app_state);
    let (snapshot, proposed, generation) = validate_undo(&session)?;

    #[cfg(target_os = "macos")]
    {
        let range_start = snapshot.range.map(|(start, _)| start);
        // Undo: the document currently holds the PROPOSED text (that's what
        // `apply_transform` wrote there) — the range to restore must be
        // sized for `proposed`, NOT `snapshot.range`'s original length. See
        // the module doc comment's "Apply vs. undo" section; this is exactly
        // the bug this parametrization fixes.
        let range_len = range_len_for(&proposed);
        let original = snapshot.text.clone();
        let apply_epoch = app_state.next_transform_apply_epoch();
        let result = run_apply(app_handle, snapshot.pid, range_start, range_len, proposed, original, apply_epoch).await;
        result.map(|_| {
            set_applied(app_state, false, generation);
        })
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app_handle, snapshot, proposed, generation);
        Err(ApplyError::Unsupported)
    }
}

/// RAII guard around the `TransformStatus::Applying` window.
///
/// Construction (`try_new`) atomically transitions from a required `from`
/// status to `Applying` under a single lock acquisition (`AppState::
/// try_transition_transform_status`) — closing the check-then-act race two
/// concurrent `apply_transform_result`/`undo_transform` command invocations
/// could otherwise hit (both observing the same starting status before
/// either flips it).
///
/// On drop, the status goes to `Idle` if the operation succeeded
/// (`mark_succeeded`), or back to the ORIGINAL `from` status otherwise. This
/// means a failed `apply_transform_result` lands back on `ReviewPending`
/// rather than `Idle` — deliberately, so the frontend can offer a Retry
/// action instead of the review popover having nowhere to go back to. A
/// failed `undo_transform` lands back on `Idle`, which is also its own
/// precondition, so retrying undo needs no special case.
pub struct ApplyingGuard<'a> {
    app_state: &'a AppState,
    prior_status: TransformStatus,
    succeeded: bool,
}

impl<'a> ApplyingGuard<'a> {
    /// Attempt the `from -> Applying` transition. Returns `None` if the
    /// current status isn't `from` — the caller should treat that as
    /// `ApplyError::Busy` rather than proceeding.
    pub fn try_new(app_state: &'a AppState, from: TransformStatus) -> Option<Self> {
        if app_state.try_transition_transform_status(from, TransformStatus::Applying) {
            Some(Self {
                app_state,
                prior_status: from,
                succeeded: false,
            })
        } else {
            None
        }
    }

    /// Mark the operation as having succeeded — on drop the status goes to
    /// `Idle` instead of back to `prior_status`.
    pub fn mark_succeeded(&mut self) {
        self.succeeded = true;
    }
}

impl Drop for ApplyingGuard<'_> {
    fn drop(&mut self) {
        let target = if self.succeeded {
            TransformStatus::Idle
        } else {
            self.prior_status
        };
        self.app_state.set_transform_status(target);
    }
}

/// Apply the reviewed transform result. Requires `TransformStatus::ReviewPending`.
#[tauri::command]
pub async fn apply_transform_result(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, crate::State>,
) -> Result<String, String> {
    use tauri::Emitter;

    let mut guard = match ApplyingGuard::try_new(&state.app_state, TransformStatus::ReviewPending) {
        Some(guard) => guard,
        None => {
            let _ = app_handle.emit("transform-apply-failed", ApplyError::Busy.as_str());
            return Err(ApplyError::Busy.as_str().to_string());
        }
    };
    match apply_transform(&app_handle, &state.app_state).await {
        Ok(via) => {
            guard.mark_succeeded();
            Ok(via.as_str().to_string())
        }
        Err(error) => {
            let _ = app_handle.emit("transform-apply-failed", error.as_str());
            Err(error.as_str().to_string())
        }
    }
}

/// Undo the most recently applied transform. Requires `TransformStatus::Idle`
/// (apply already returned control there) and `session.applied`.
#[tauri::command]
pub async fn undo_transform(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, crate::State>,
) -> Result<(), String> {
    use tauri::Emitter;

    let mut guard = match ApplyingGuard::try_new(&state.app_state, TransformStatus::Idle) {
        Some(guard) => guard,
        None => {
            let _ = app_handle.emit("transform-apply-failed", ApplyError::Busy.as_str());
            return Err(ApplyError::Busy.as_str().to_string());
        }
    };
    match undo_applied_transform(&app_handle, &state.app_state).await {
        Ok(()) => {
            guard.mark_succeeded();
            Ok(())
        }
        Err(error) => {
            let _ = app_handle.emit("transform-apply-failed", error.as_str());
            Err(error.as_str().to_string())
        }
    }
}

#[cfg(target_os = "macos")]
mod native {
    //! Raw AX/AppKit FFI for the apply/undo write path. Deliberately
    //! self-contained (duplicates a little of `selection.rs`'s and
    //! `injector.rs`'s FFI scaffolding) rather than factoring it into a
    //! shared module, for the same reason `selection.rs` gives: avoids any
    //! risk of regressing already-reviewed read-side/paste-side code for
    //! this PR.

    use super::{
        AppliedVia, ApplyError, AxWriteAttempt, ClipboardOutcome, PrePasteCheck, ACTIVATION_SETTLE_MS,
    };
    use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication, NSWorkspace};
    use std::ffi::{c_char, c_void, CStr, CString};
    use std::thread;
    use std::time::Duration;

    type AXUIElementRef = *const c_void;
    type CFTypeRef = *const c_void;
    type CFIndex = isize;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
        fn AXUIElementCopyAttributeValue(
            element: AXUIElementRef,
            attribute: CFTypeRef,
            value: *mut CFTypeRef,
        ) -> i32;
        fn AXUIElementSetAttributeValue(
            element: AXUIElementRef,
            attribute: CFTypeRef,
            value: CFTypeRef,
        ) -> i32;
        fn AXUIElementSetMessagingTimeout(element: AXUIElementRef, timeout: f32) -> i32;
        fn AXValueCreate(value_type: u32, value_ptr: *const c_void) -> CFTypeRef;
        fn CFStringCreateWithCString(
            allocator: CFTypeRef,
            string: *const c_char,
            encoding: u32,
        ) -> CFTypeRef;
        fn CFStringGetLength(string: CFTypeRef) -> CFIndex;
        fn CFStringGetMaximumSizeForEncoding(length: CFIndex, encoding: u32) -> CFIndex;
        fn CFStringGetCString(
            string: CFTypeRef,
            buffer: *mut c_char,
            buffer_size: CFIndex,
            encoding: u32,
        ) -> bool;
        fn CFRelease(value: CFTypeRef);
    }

    const AX_SUCCESS: i32 = 0;
    /// Messaging timeout for every AX call in this module. Deliberately
    /// higher than `selection.rs`'s 25ms: that figure was tuned for the
    /// fail-closed READ side, where an ambiguous ("did it time out, or does
    /// the element have no value?") status must abort rather than guess. The
    /// write side has no such ambiguity to worry about — a timeout here just
    /// needs the target enough time to actually finish applying the write
    /// before we (wrongly) treat it as failed and fall back to pasting a
    /// second copy (see `decide_pre_paste_check`'s `AlreadyWritten` case,
    /// which also catches whatever timeouts still slip through).
    const AX_WRITE_TIMEOUT_SECONDS: f32 = 0.1;
    const UTF8_ENCODING: u32 = 0x0800_0100;
    // kAXValueCFRangeType from the AXValueType enum (ApplicationServices /
    // HIServices AXValue.h) — same constant `selection.rs` uses to decode.
    const AX_VALUE_CFRANGE_TYPE: u32 = 4;

    #[repr(C)]
    #[derive(Debug, Clone, Copy)]
    struct CFRange {
        location: CFIndex,
        length: CFIndex,
    }

    struct CFGuard(CFTypeRef);
    impl Drop for CFGuard {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CFRelease(self.0) };
            }
        }
    }

    fn cfstring(s: &str) -> Option<CFGuard> {
        let c = CString::new(s).ok()?;
        let raw = unsafe { CFStringCreateWithCString(std::ptr::null(), c.as_ptr(), UTF8_ENCODING) };
        if raw.is_null() {
            return None;
        }
        Some(CFGuard(raw))
    }

    fn cfstring_to_string(value: CFTypeRef) -> Option<String> {
        let length = unsafe { CFStringGetLength(value) };
        let max_size = unsafe { CFStringGetMaximumSizeForEncoding(length, UTF8_ENCODING) };
        if max_size <= 0 {
            return Some(String::new());
        }
        let mut buffer = vec![0 as c_char; (max_size + 1) as usize];
        let converted = unsafe {
            CFStringGetCString(
                value,
                buffer.as_mut_ptr(),
                buffer.len() as CFIndex,
                UTF8_ENCODING,
            )
        };
        if !converted {
            return None;
        }
        Some(
            unsafe { CStr::from_ptr(buffer.as_ptr()) }
                .to_string_lossy()
                .into_owned(),
        )
    }

    fn set_timeout(element: AXUIElementRef) -> bool {
        unsafe { AXUIElementSetMessagingTimeout(element, AX_WRITE_TIMEOUT_SECONDS) == AX_SUCCESS }
    }

    fn copy_attribute(element: AXUIElementRef, name: &str) -> Option<CFGuard> {
        let attr = cfstring(name)?;
        let mut value: CFTypeRef = std::ptr::null();
        let status = unsafe { AXUIElementCopyAttributeValue(element, attr.0, &mut value) };
        if status != AX_SUCCESS || value.is_null() {
            if !value.is_null() {
                unsafe { CFRelease(value) };
            }
            return None;
        }
        Some(CFGuard(value))
    }

    /// Copy `AXFocusedUIElement` of the application identified by `pid`
    /// (NOT necessarily the frontmost app — the caller has already checked
    /// that separately). Returns owned guards for both the application and
    /// focused-element AX references. Called exactly ONCE per
    /// `apply_text_to_target` invocation — every subsequent read/write/range
    /// operation is threaded through the same `focused` reference (the
    /// `_of` functions below) rather than re-resolving
    /// `AXFocusedUIElement`, so the field verified is guaranteed to be the
    /// field written even if focus were to shift mid-call.
    fn focused_element(pid: i32) -> Option<(CFGuard, CFGuard)> {
        let app = unsafe { AXUIElementCreateApplication(pid) };
        if app.is_null() {
            return None;
        }
        let app_guard = CFGuard(app);
        if !set_timeout(app) {
            return None;
        }
        let focused = copy_attribute(app, "AXFocusedUIElement")?;
        if !set_timeout(focused.0) {
            return None;
        }
        Some((app_guard, focused))
    }

    /// Read `AXSelectedText` of an already-resolved focused element.
    fn read_selected_text_of(focused: AXUIElementRef) -> Option<String> {
        let value = copy_attribute(focused, "AXSelectedText")?;
        cfstring_to_string(value.0)
    }

    /// Write `text` into `AXSelectedText` of an already-resolved focused
    /// element. Returns whether the set call reported success.
    fn write_selected_text_of(focused: AXUIElementRef, text: &str) -> bool {
        let Some(value) = cfstring(text) else {
            return false;
        };
        // `attr` must stay alive (bound to a variable) through the call below
        // — a temporary `CFGuard` dropped inline would `CFRelease` the
        // CFString before `AXUIElementSetAttributeValue` reads it.
        let Some(attr) = cfstring("AXSelectedText") else {
            return false;
        };
        let status = unsafe { AXUIElementSetAttributeValue(focused, attr.0, value.0) };
        status == AX_SUCCESS
    }

    /// Restore a selection range on an already-resolved focused element via
    /// `AXSelectedTextRange`. Best-effort — returns whether it succeeded so
    /// the caller can fall back to the "current selection still matches"
    /// check per the module doc comment.
    fn restore_selection_range_of(focused: AXUIElementRef, range: (usize, usize)) -> bool {
        let (start, end) = range;
        let cf_range = CFRange {
            location: start as CFIndex,
            length: (end.saturating_sub(start)) as CFIndex,
        };
        let value = unsafe {
            AXValueCreate(
                AX_VALUE_CFRANGE_TYPE,
                &cf_range as *const CFRange as *const c_void,
            )
        };
        if value.is_null() {
            return false;
        }
        let value_guard = CFGuard(value);
        let Some(attr) = cfstring("AXSelectedTextRange") else {
            return false;
        };
        let status = unsafe { AXUIElementSetAttributeValue(focused, attr.0, value_guard.0) };
        status == AX_SUCCESS
    }

    /// Lightweight frontmost check: no activation attempt, no settle delay.
    /// Used both as the tail of `activate_and_check_frontmost` and, on its
    /// own, for the pre-paste re-check — the earlier
    /// `activate_and_check_frontmost` result is up to ~100-150ms stale by
    /// the time the paste fallback is considered.
    fn is_frontmost(pid: i32) -> bool {
        NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .map(|app| app.processIdentifier() == pid)
            .unwrap_or(false)
    }

    /// Re-activate the app owning `pid` and verify it is frontmost afterward.
    /// `activateWithOptions` with no options is used deliberately —
    /// `ActivateIgnoringOtherApps` is deprecated (no effect) on macOS 14+, and
    /// bringing just the app's main/key windows forward is sufficient here.
    fn activate_and_check_frontmost(pid: i32) -> bool {
        if let Some(target) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) {
            target.activateWithOptions(NSApplicationActivationOptions::empty());
        }
        thread::sleep(Duration::from_millis(ACTIVATION_SETTLE_MS));
        is_frontmost(pid)
    }

    /// Result of `apply_text_to_target`. Carries a pending clipboard restore
    /// separately from `result` so the caller (`run_apply`, off the main
    /// thread) can schedule the ~300ms-delayed restore on a background task
    /// instead of this (main-thread) function sleeping for it — see the
    /// module doc comment's clipboard policy table.
    pub(super) struct NativeApplyOutcome {
        pub result: Result<AppliedVia, ApplyError>,
        /// `Some(original)` only when the Cmd+V paste fallback succeeded and
        /// the caller must restore `original` to the clipboard after the
        /// delay. `None` when there's nothing to restore, or when the
        /// restore already happened inline (AX path — synchronous, no delay
        /// needed).
        pub pending_clipboard_restore: Option<String>,
    }

    /// Native implementation of the apply/undo write path. `replacement` is
    /// whatever should end up in the document; `expected_current` is what
    /// must currently occupy the selection (by content, not just position)
    /// for the write to proceed at all. Must run on the main thread (same
    /// constraint as `inject_text`'s focus query and
    /// `capture_selection_native`).
    pub(super) fn apply_text_to_target(
        pid: i32,
        range_start: Option<usize>,
        range_len: usize,
        expected_current: &str,
        replacement: &str,
    ) -> NativeApplyOutcome {
        // House rule: the replacement text is written to the clipboard
        // FIRST, unconditionally, before anything else is attempted. The
        // user must never lose it regardless of what happens next.
        let original_clipboard = crate::injector::read_clipboard_text().ok();
        if crate::injector::write_clipboard_text(replacement).is_err() {
            return NativeApplyOutcome {
                result: Err(ApplyError::ClipboardUnavailable),
                pending_clipboard_restore: None,
            };
        }

        let target_is_frontmost = activate_and_check_frontmost(pid);

        // Fetch the focused element ONCE (finding: re-fetching per call could
        // let restore/read/write silently operate on different elements if
        // focus shifted mid-call).
        let focused = if target_is_frontmost {
            focused_element(pid)
        } else {
            None
        };

        let range_restore_succeeded = focused
            .as_ref()
            .zip(range_start)
            .map(|((_app, el), start)| restore_selection_range_of(el.0, (start, start + range_len)))
            .unwrap_or(false);

        // Content check is unconditional: even after a successful range
        // restore, the read-back selection must equal `expected_current`
        // before writing (`ReviewPending` is user-paced; the document may
        // have changed since capture).
        let current_selection_matches_expected = focused
            .as_ref()
            .and_then(|(_app, el)| read_selected_text_of(el.0))
            .map(|current| current == expected_current)
            .unwrap_or(false);

        let pre_write_guard = super::decide_pre_write_guard(
            target_is_frontmost,
            range_restore_succeeded,
            current_selection_matches_expected,
        );

        let ax_write = match (&pre_write_guard, focused.as_ref()) {
            (Ok(()), Some((_app, el))) => {
                if write_selected_text_of(el.0, replacement) {
                    AxWriteAttempt::Succeeded
                } else {
                    AxWriteAttempt::Failed
                }
            }
            _ => AxWriteAttempt::NotAttempted,
        };

        let ax_verify_matched = if ax_write == AxWriteAttempt::Succeeded {
            Some(
                focused
                    .as_ref()
                    .and_then(|(_app, el)| read_selected_text_of(el.0))
                    .map(|readback| readback == replacement)
                    .unwrap_or(false),
            )
        } else {
            None
        };

        let (pre_paste_check, paste_attempted, paste_succeeded) = if ax_write == AxWriteAttempt::Failed {
            let already_written = focused
                .as_ref()
                .and_then(|(_app, el)| read_selected_text_of(el.0))
                .map(|current| current == replacement)
                .unwrap_or(false);
            // Fresh, lightweight re-check right before pasting — no
            // re-activation, no sleep; the initial `target_is_frontmost`
            // check above is stale by the time we get here.
            let still_frontmost = is_frontmost(pid);
            let check = super::decide_pre_paste_check(already_written, still_frontmost);
            match check {
                PrePasteCheck::Proceed => (check, true, crate::injector::simulate_paste().is_ok()),
                _ => (check, false, false),
            }
        } else {
            (PrePasteCheck::Proceed, false, false) // unused unless ax_write == Failed
        };

        let (result, clipboard_outcome) = super::decide_apply_outcome(
            pre_write_guard,
            ax_write,
            ax_verify_matched,
            pre_paste_check,
            paste_attempted,
            paste_succeeded,
        );

        match clipboard_outcome {
            ClipboardOutcome::LeaveProposed => NativeApplyOutcome {
                result,
                pending_clipboard_restore: None,
            },
            ClipboardOutcome::RestoreOriginal => {
                if matches!(result, Ok(AppliedVia::Paste)) {
                    // Delayed restore — handed to the caller to run off the
                    // main thread; do NOT sleep here.
                    NativeApplyOutcome {
                        result,
                        pending_clipboard_restore: original_clipboard,
                    }
                } else {
                    // AX path: synchronous, no delay needed — restore inline.
                    if let Some(original) = original_clipboard {
                        let _ = crate::injector::write_clipboard_text(&original);
                    }
                    NativeApplyOutcome {
                        result,
                        pending_clipboard_restore: None,
                    }
                }
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod native {
    //! No AX apply path off macOS; kept as a stub module so
    //! `apply_text_to_target` has a single call shape if a future PR needs
    //! one (currently unused — `apply_transform`/`undo_transform` return
    //! `ApplyError::Unsupported` directly on non-macOS).
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection::TransformSnapshot;
    use std::time::Instant;

    fn snapshot() -> TransformSnapshot {
        TransformSnapshot {
            bundle_id: Some("com.example.app".to_string()),
            pid: 123,
            text: "original text".to_string(),
            range: Some((0, 13)),
            bounds: None,
            captured_at: Instant::now(),
        }
    }

    // -- Session lifecycle --

    #[test]
    fn no_session_by_default() {
        let state = AppState::default();
        assert!(session_snapshot(&state).is_none());
    }

    #[test]
    fn start_session_installs_a_fresh_unapplied_session() {
        let state = AppState::default();
        start_session(&state, snapshot());
        let session = session_snapshot(&state).expect("session should be present");
        assert!(session.proposed.is_none());
        assert!(!session.applied);
    }

    #[test]
    fn start_session_stamps_an_increasing_generation() {
        let state = AppState::default();
        start_session(&state, snapshot());
        let first_generation = session_snapshot(&state).unwrap().generation;

        start_session(&state, snapshot());
        let second_generation = session_snapshot(&state).unwrap().generation;

        assert!(second_generation > first_generation);
    }

    #[test]
    fn set_proposed_text_requires_an_active_session() {
        let state = AppState::default();
        assert!(!set_proposed_text(&state, "hello".to_string()));
        start_session(&state, snapshot());
        assert!(set_proposed_text(&state, "hello".to_string()));
        assert_eq!(
            session_snapshot(&state).unwrap().proposed,
            Some("hello".to_string())
        );
    }

    #[test]
    fn starting_a_new_session_replaces_the_previous_one() {
        let state = AppState::default();
        start_session(&state, snapshot());
        set_proposed_text(&state, "first".to_string());

        let mut second = snapshot();
        second.text = "second original".to_string();
        start_session(&state, second);

        let session = session_snapshot(&state).unwrap();
        assert!(session.proposed.is_none());
        assert_eq!(session.snapshot.text, "second original");
    }

    #[test]
    fn clear_session_removes_it_and_reports_whether_one_was_present() {
        let state = AppState::default();
        assert!(!clear_session(&state));
        start_session(&state, snapshot());
        assert!(clear_session(&state));
        assert!(session_snapshot(&state).is_none());
        assert!(!clear_session(&state));
    }

    #[test]
    fn set_applied_is_a_no_op_without_a_session() {
        let state = AppState::default();
        set_applied(&state, true, 1); // must not panic
        assert!(session_snapshot(&state).is_none());
    }

    #[test]
    fn set_applied_no_ops_when_generation_does_not_match_current_session() {
        // Models a slow in-flight apply/undo completing AFTER the user has
        // already started a new transform pass (issue #312 PR-B2 review,
        // finding #5): the stale completion's `set_applied` call must not
        // corrupt the session that replaced it.
        let state = AppState::default();
        start_session(&state, snapshot());
        let stale_generation = session_snapshot(&state).unwrap().generation;

        // A new session starts (e.g. the user re-pressed the transform
        // hotkey) before the stale call lands.
        start_session(&state, snapshot());
        let current_generation = session_snapshot(&state).unwrap().generation;
        assert_ne!(stale_generation, current_generation);

        set_applied(&state, true, stale_generation);
        assert!(
            !session_snapshot(&state).unwrap().applied,
            "set_applied with a stale generation must not mutate the current session"
        );

        set_applied(&state, true, current_generation);
        assert!(
            session_snapshot(&state).unwrap().applied,
            "set_applied with the current generation must still work"
        );
    }

    // `apply_transform`/`undo_transform` dispatch to `run_on_main_thread` on
    // a real `tauri::AppHandle`, which these unit tests don't have (and
    // don't need — `tauri::test::mock_app()` produces an `AppHandle` typed
    // over `MockRuntime`, not the concrete `Wry` runtime these functions use,
    // so it isn't a drop-in substitute). Session-lifecycle rules are instead
    // covered directly against the pure `validate_apply`/`validate_undo`
    // helpers those functions call before ever touching the AppHandle.

    // `validate_apply`/`validate_undo`'s `Ok` variant carries a
    // `TransformSnapshot`, which deliberately has no `PartialEq` (see
    // `selection.rs`'s doc comment — it isn't meant to be compared
    // wholesale). These assertions match on the `Err` variant with
    // `matches!` rather than `assert_eq!` for that reason; the `Ok` cases are
    // asserted by unpacking the tuple/value and comparing individual fields.

    #[test]
    fn apply_requires_a_session() {
        assert!(matches!(validate_apply(&None), Err(ApplyError::NoSession)));
    }

    #[test]
    fn apply_requires_proposed_text() {
        let session = Some(TransformSession::new(snapshot(), 1));
        assert!(matches!(
            validate_apply(&session),
            Err(ApplyError::NoProposedText)
        ));
    }

    #[test]
    fn apply_rejects_an_already_applied_session() {
        let mut session = TransformSession::new(snapshot(), 1);
        session.proposed = Some("proposed".to_string());
        session.applied = true;
        assert!(matches!(
            validate_apply(&Some(session)),
            Err(ApplyError::AlreadyApplied)
        ));
    }

    #[test]
    fn apply_accepts_a_fresh_session_with_proposed_text() {
        let mut session = TransformSession::new(snapshot(), 7);
        session.proposed = Some("proposed".to_string());
        let (text, resolved_snapshot, generation) =
            validate_apply(&Some(session)).expect("should validate");
        assert_eq!(text, "proposed");
        assert_eq!(resolved_snapshot.text, "original text");
        assert_eq!(generation, 7);
    }

    #[test]
    fn undo_requires_a_session() {
        assert!(matches!(validate_undo(&None), Err(ApplyError::NoSession)));
    }

    #[test]
    fn undo_requires_the_session_to_be_applied() {
        let mut session = TransformSession::new(snapshot(), 1);
        session.proposed = Some("proposed".to_string());
        assert!(matches!(
            validate_undo(&Some(session)),
            Err(ApplyError::NotApplied)
        ));
    }

    #[test]
    fn undo_accepts_an_applied_session() {
        let mut session = TransformSession::new(snapshot(), 9);
        session.proposed = Some("proposed".to_string());
        session.applied = true;
        let (resolved_snapshot, proposed, generation) =
            validate_undo(&Some(session)).expect("should validate");
        assert_eq!(resolved_snapshot.text, "original text");
        assert_eq!(proposed, "proposed");
        assert_eq!(generation, 9);
    }

    #[test]
    fn double_undo_is_rejected() {
        // First undo (modeled directly, since the native write path isn't
        // exercised here) flips `applied` back to false; a second validation
        // against that same post-undo session must reject it.
        let mut session = TransformSession::new(snapshot(), 1);
        session.proposed = Some("proposed".to_string());
        session.applied = true;
        assert!(validate_undo(&Some(session.clone())).is_ok());
        session.applied = false; // what a successful undo does on completion
        assert!(matches!(
            validate_undo(&Some(session)),
            Err(ApplyError::NotApplied)
        ));
    }

    #[test]
    fn recording_start_invalidates_the_session() {
        // Mirrors the guard site in `commands::recording::start_native_recording`:
        // a new recording must never leave a stale transform session behind.
        let state = AppState::default();
        start_session(&state, snapshot());
        set_proposed_text(&state, "proposed".to_string());
        clear_session(&state); // one-line call wired into the guard site
        assert!(session_snapshot(&state).is_none());
    }

    #[test]
    fn new_transform_start_invalidates_the_previous_session() {
        // Mirrors the guard site in `keyboard.rs`'s transform-key-pressed
        // handler: pressing the transform hotkey again must not let a stale
        // proposed/applied session leak into the new pass.
        let state = AppState::default();
        start_session(&state, snapshot());
        set_proposed_text(&state, "proposed".to_string());
        set_applied(&state, true, session_snapshot(&state).unwrap().generation);
        clear_session(&state);
        assert!(session_snapshot(&state).is_none());
    }

    // -- ApplyError / AppliedVia classification --

    #[test]
    fn apply_error_outcome_strings_are_stable_and_content_free() {
        let cases = [
            (ApplyError::Unsupported, "unsupported"),
            (ApplyError::Busy, "busy"),
            (ApplyError::NoSession, "no_session"),
            (ApplyError::NoProposedText, "no_proposed_text"),
            (ApplyError::AlreadyApplied, "already_applied"),
            (ApplyError::NotApplied, "not_applied"),
            (ApplyError::ClipboardUnavailable, "clipboard_unavailable"),
            (ApplyError::TargetGone, "target_gone"),
            (ApplyError::SelectionChanged, "selection_changed"),
            (ApplyError::PasteFailed, "paste_failed"),
        ];
        for (error, expected) in cases {
            assert_eq!(error.as_str(), expected);
        }
    }

    #[test]
    fn applied_via_outcome_strings_are_stable() {
        assert_eq!(AppliedVia::Ax.as_str(), "ax");
        assert_eq!(AppliedVia::AxUnverified.as_str(), "ax_unverified");
        assert_eq!(AppliedVia::Paste.as_str(), "paste");
    }

    // -- ApplyingGuard: atomic transition + restore-prior-status-on-failure --

    #[test]
    fn applying_guard_try_new_requires_the_exact_from_status() {
        let state = AppState::default();
        assert!(state.transform_status() == TransformStatus::Idle);
        assert!(ApplyingGuard::try_new(&state, TransformStatus::ReviewPending).is_none());
        assert_eq!(state.transform_status(), TransformStatus::Idle);
    }

    #[test]
    fn applying_guard_sets_applying_and_restores_prior_status_on_drop_by_default() {
        let state = AppState::default();
        state.set_transform_status(TransformStatus::ReviewPending);
        {
            let guard = ApplyingGuard::try_new(&state, TransformStatus::ReviewPending)
                .expect("should transition");
            assert_eq!(state.transform_status(), TransformStatus::Applying);
            drop(guard);
        }
        // No mark_succeeded() call -- modeling a failed apply -- so drop
        // restores ReviewPending, enabling a Retry.
        assert_eq!(state.transform_status(), TransformStatus::ReviewPending);
    }

    #[test]
    fn applying_guard_lands_idle_when_marked_succeeded() {
        let state = AppState::default();
        state.set_transform_status(TransformStatus::ReviewPending);
        {
            let mut guard = ApplyingGuard::try_new(&state, TransformStatus::ReviewPending)
                .expect("should transition");
            guard.mark_succeeded();
        }
        assert_eq!(state.transform_status(), TransformStatus::Idle);
    }

    #[test]
    fn applying_guard_second_concurrent_attempt_is_rejected() {
        // Two "concurrent" apply_transform_result calls both trying the same
        // ReviewPending -> Applying transition: only one may succeed.
        let state = AppState::default();
        state.set_transform_status(TransformStatus::ReviewPending);
        let first = ApplyingGuard::try_new(&state, TransformStatus::ReviewPending);
        assert!(first.is_some());
        let second = ApplyingGuard::try_new(&state, TransformStatus::ReviewPending);
        assert!(
            second.is_none(),
            "a second concurrent apply must not also claim the transition"
        );
    }

    // -- Pure decision-table tests --

    #[test]
    fn pre_write_guard_table() {
        // (frontmost, range_restore_ok, current_matches) -> expected
        let cases: &[((bool, bool, bool), Result<(), ApplyError>)] = &[
            ((false, false, false), Err(ApplyError::TargetGone)),
            ((false, true, true), Err(ApplyError::TargetGone)), // frontmost check wins even if everything else looks fine
            ((true, true, true), Ok(())),
            // Range restore succeeded, but the read-back content does NOT
            // match what was expected (e.g. the document was edited during
            // `ReviewPending`) -- content-verify is unconditional now, so
            // this fails closed instead of trusting the range restore alone.
            ((true, true, false), Err(ApplyError::SelectionChanged)),
            ((true, false, true), Ok(())),
            ((true, false, false), Err(ApplyError::SelectionChanged)),
        ];
        for ((frontmost, range_ok, matches), expected) in cases.iter().copied() {
            assert_eq!(
                decide_pre_write_guard(frontmost, range_ok, matches),
                expected,
                "frontmost={frontmost} range_ok={range_ok} matches={matches}"
            );
        }
    }

    #[test]
    fn pre_paste_check_table() {
        // (already_written, still_frontmost) -> expected
        let cases: &[((bool, bool), PrePasteCheck)] = &[
            ((true, true), PrePasteCheck::AlreadyWritten),
            ((true, false), PrePasteCheck::AlreadyWritten), // already-written check wins regardless of frontmost
            ((false, false), PrePasteCheck::TargetGone),
            ((false, true), PrePasteCheck::Proceed),
        ];
        for ((already_written, still_frontmost), expected) in cases.iter().copied() {
            assert_eq!(
                decide_pre_paste_check(already_written, still_frontmost),
                expected,
                "already_written={already_written} still_frontmost={still_frontmost}"
            );
        }
    }

    #[test]
    fn apply_decision_table() {
        struct Case {
            name: &'static str,
            guard: Result<(), ApplyError>,
            ax_write: AxWriteAttempt,
            ax_verify_matched: Option<bool>,
            pre_paste_check: PrePasteCheck,
            paste_attempted: bool,
            paste_succeeded: bool,
            expected_result: Result<AppliedVia, ApplyError>,
            expected_clipboard: ClipboardOutcome,
        }

        let cases = [
            Case {
                name: "target gone never attempts a write, clipboard keeps proposed text",
                guard: Err(ApplyError::TargetGone),
                ax_write: AxWriteAttempt::NotAttempted,
                ax_verify_matched: None,
                pre_paste_check: PrePasteCheck::Proceed,
                paste_attempted: false,
                paste_succeeded: false,
                expected_result: Err(ApplyError::TargetGone),
                expected_clipboard: ClipboardOutcome::LeaveProposed,
            },
            Case {
                name: "selection changed never attempts a write, clipboard keeps proposed text",
                guard: Err(ApplyError::SelectionChanged),
                ax_write: AxWriteAttempt::NotAttempted,
                ax_verify_matched: None,
                pre_paste_check: PrePasteCheck::Proceed,
                paste_attempted: false,
                paste_succeeded: false,
                expected_result: Err(ApplyError::SelectionChanged),
                expected_clipboard: ClipboardOutcome::LeaveProposed,
            },
            Case {
                name: "AX set succeeds and verifies -> confirmed, restore original clipboard",
                guard: Ok(()),
                ax_write: AxWriteAttempt::Succeeded,
                ax_verify_matched: Some(true),
                pre_paste_check: PrePasteCheck::Proceed,
                paste_attempted: false,
                paste_succeeded: false,
                expected_result: Ok(AppliedVia::Ax),
                expected_clipboard: ClipboardOutcome::RestoreOriginal,
            },
            Case {
                name: "AX set succeeds but verify mismatches -> unverified, still restore",
                guard: Ok(()),
                ax_write: AxWriteAttempt::Succeeded,
                ax_verify_matched: Some(false),
                pre_paste_check: PrePasteCheck::Proceed,
                paste_attempted: false,
                paste_succeeded: false,
                expected_result: Ok(AppliedVia::AxUnverified),
                expected_clipboard: ClipboardOutcome::RestoreOriginal,
            },
            Case {
                name: "AX set succeeds but the attribute is unreadable (None) -> unverified, still restore",
                guard: Ok(()),
                ax_write: AxWriteAttempt::Succeeded,
                ax_verify_matched: None,
                pre_paste_check: PrePasteCheck::Proceed,
                paste_attempted: false,
                paste_succeeded: false,
                expected_result: Ok(AppliedVia::AxUnverified),
                expected_clipboard: ClipboardOutcome::RestoreOriginal,
            },
            Case {
                name: "AX set fails, paste fallback succeeds -> applied via paste, restore original",
                guard: Ok(()),
                ax_write: AxWriteAttempt::Failed,
                ax_verify_matched: None,
                pre_paste_check: PrePasteCheck::Proceed,
                paste_attempted: true,
                paste_succeeded: true,
                expected_result: Ok(AppliedVia::Paste),
                expected_clipboard: ClipboardOutcome::RestoreOriginal,
            },
            Case {
                name: "AX set fails, paste fallback also fails -> hard failure, leave proposed text",
                guard: Ok(()),
                ax_write: AxWriteAttempt::Failed,
                ax_verify_matched: None,
                pre_paste_check: PrePasteCheck::Proceed,
                paste_attempted: true,
                paste_succeeded: false,
                expected_result: Err(ApplyError::PasteFailed),
                expected_clipboard: ClipboardOutcome::LeaveProposed,
            },
            Case {
                name: "AX set fails, but a re-read shows it already landed -> unverified, no paste (double-apply guard)",
                guard: Ok(()),
                ax_write: AxWriteAttempt::Failed,
                ax_verify_matched: None,
                pre_paste_check: PrePasteCheck::AlreadyWritten,
                paste_attempted: false,
                paste_succeeded: false,
                expected_result: Ok(AppliedVia::AxUnverified),
                expected_clipboard: ClipboardOutcome::RestoreOriginal,
            },
            Case {
                name: "AX set fails, target no longer frontmost right before pasting -> TargetGone, no paste",
                guard: Ok(()),
                ax_write: AxWriteAttempt::Failed,
                ax_verify_matched: None,
                pre_paste_check: PrePasteCheck::TargetGone,
                paste_attempted: false,
                paste_succeeded: false,
                expected_result: Err(ApplyError::TargetGone),
                expected_clipboard: ClipboardOutcome::LeaveProposed,
            },
        ];

        for case in cases {
            let (result, clipboard) = decide_apply_outcome(
                case.guard,
                case.ax_write,
                case.ax_verify_matched,
                case.pre_paste_check,
                case.paste_attempted,
                case.paste_succeeded,
            );
            assert_eq!(result, case.expected_result, "case: {}", case.name);
            assert_eq!(clipboard, case.expected_clipboard, "case: {}", case.name);
        }
    }

    // -- range_len_for: apply-vs-undo length mismatch table --

    #[test]
    fn range_len_for_table() {
        let cases: &[(&str, usize)] = &[
            ("", 0),
            ("hi", 2),
            ("hello there, how are you", 24),
            // Multi-byte / astral: UTF-16 code units, not bytes or `char`s.
            // '\u{e9}' (é) is 1 UTF-16 unit despite being 2 UTF-8 bytes;
            // '\u{1f600}' (an emoji) is a surrogate pair -- 2 UTF-16 units
            // despite being 1 `char`.
            ("caf\u{e9}", 4),
            ("\u{1f600}", 2),
        ];
        for (text, expected) in cases.iter().copied() {
            assert_eq!(range_len_for(text), expected, "text={text:?}");
        }
    }

    #[test]
    fn apply_and_undo_use_different_range_lengths_when_text_length_changes() {
        // This is exactly the bug finding #1 fixes: undo must NOT reuse the
        // original snapshot range's length when the proposed text is a
        // different length -- it needs the length of whatever text is
        // CURRENTLY in the document (the proposed text), not the original.
        let original = "hi";
        let proposed = "hello there, how are you";

        let apply_range_len = range_len_for(original);
        let undo_range_len = range_len_for(proposed);

        assert_eq!(apply_range_len, 2);
        assert_eq!(undo_range_len, 24);
        assert_ne!(
            apply_range_len, undo_range_len,
            "apply and undo must compute range length from DIFFERENT source text"
        );
    }
}
