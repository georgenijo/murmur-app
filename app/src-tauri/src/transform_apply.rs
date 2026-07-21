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
//! so they can be put back later.
//!
//! What happens to that saved original afterward depends on whether the
//! proposed text actually reached the document:
//!
//! | Outcome | Original clipboard | Rationale |
//! |---|---|---|
//! | AX set succeeded (confirmed or unverified) | Restored | Text landed in the document — the clipboard no longer needs to be the sole record of it. |
//! | AX set failed, Cmd+V paste succeeded | Restored (after ~300ms) | Same reasoning, one hop later. The delay lets the synthetic keystroke actually land before we overwrite what the target app may still be reading from the pasteboard. |
//! | Target gone / selection changed under us | **Not restored** — proposed text stays in the clipboard | We deliberately did not touch the document; the user's only path to the transform result is now a manual paste. |
//! | AX set failed and the paste fallback also failed | **Not restored** — proposed text stays in the clipboard | Same reasoning: nothing landed, so the clipboard is the fallback delivery mechanism. `transform-apply-failed` fires so the frontend can show the banner. |
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
    pub proposed: Option<String>,
    pub applied: bool,
}

impl TransformSession {
    pub fn new(snapshot: crate::selection::TransformSnapshot) -> Self {
        Self {
            snapshot,
            proposed: None,
            applied: false,
        }
    }
}

// -- Session setter/getter APIs (internal; C2 fills `proposed` from the
// sidecar in a later PR) --

/// Start a new session for a freshly captured selection, replacing whatever
/// session (if any) was active. There is only ever one active session.
pub fn start_session(app_state: &AppState, snapshot: crate::selection::TransformSnapshot) {
    *app_state.transform_session.lock_or_recover() = Some(TransformSession::new(snapshot));
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

/// Snapshot (clone) of the active session, if any. Used by tests and by
/// `apply_transform`/`undo_transform` to read the session without holding the
/// lock across the AX/clipboard work below.
pub fn session_snapshot(app_state: &AppState) -> Option<TransformSession> {
    app_state.transform_session.lock_or_recover().clone()
}

/// Clear the active session unconditionally. Called at the two invalidation
/// points B1 already established: the start of a new dictation recording
/// (`commands::recording::start_native_recording`) and the start of a new
/// transform pass (the transform hotkey press in `keyboard.rs`). Returns
/// whether a session was actually present to clear.
pub fn clear_session(app_state: &AppState) -> bool {
    app_state.transform_session.lock_or_recover().take().is_some()
}

/// Mark the active session as applied (or not). Internal — used by
/// `apply_transform`/`undo_transform` after a successful write.
fn set_applied(app_state: &AppState, applied: bool) {
    if let Some(active) = app_state.transform_session.lock_or_recover().as_mut() {
        active.applied = applied;
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
    /// not confirm it (mismatch, or the attribute isn't readable at all).
    /// Applied but unconfirmed — deliberately not an error (see step 4 of the
    /// module doc comment).
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
    /// undo).
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
    /// The target app is no longer frontmost (activation failed to bring it
    /// back, or a different app took over). Fails closed: the document is
    /// never touched. Text stays in the clipboard.
    TargetGone,
    /// The original selection range could not be restored and the current
    /// selection no longer matches what was captured — fails closed rather
    /// than risk overwriting text the user never selected.
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

/// Step 2+3 guard: decide whether it's safe to proceed to the attribute-set
/// call at all, given whether the target app is still frontmost and whether
/// the original selection range/content could be re-established. Pure — no
/// I/O — so it is covered directly by `tests::pre_write_guard_table`.
pub fn decide_pre_write_guard(
    target_is_frontmost: bool,
    range_restore_succeeded: bool,
    current_selection_matches_snapshot: bool,
) -> Result<(), ApplyError> {
    if !target_is_frontmost {
        return Err(ApplyError::TargetGone);
    }
    if range_restore_succeeded {
        return Ok(());
    }
    if current_selection_matches_snapshot {
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
        AxWriteAttempt::Failed => {
            if paste_attempted && paste_succeeded {
                (Ok(AppliedVia::Paste), ClipboardOutcome::RestoreOriginal)
            } else {
                (Err(ApplyError::PasteFailed), ClipboardOutcome::LeaveProposed)
            }
        }
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

/// Delay before restoring the user's original clipboard contents after the
/// Cmd+V paste fallback. Cmd+V is an asynchronous keystroke through the HID
/// event queue and the target app's own event loop; overwriting the
/// clipboard immediately risks racing the app's own paste-read. No such
/// delay is needed on the AX path — `AXUIElementSetAttributeValue` is
/// synchronous.
const PASTE_CLIPBOARD_RESTORE_DELAY_MS: u64 = 300;

/// Delay after re-activating the target app before attempting the paste
/// fallback, mirroring `inject_text`'s default focus-settle delay.
const ACTIVATION_SETTLE_MS: u64 = 50;

/// Pure pre-flight validation for `apply_transform`: does a session exist,
/// is it not already applied, and does it have proposed text? Factored out
/// of `apply_transform` so the session-lifecycle rules are unit-testable
/// without an `AppHandle` (real or mock) at all. Returns the text to write
/// and the snapshot to write it against.
fn validate_apply(session: &Option<TransformSession>) -> Result<(String, crate::selection::TransformSnapshot), ApplyError> {
    let session = session.as_ref().ok_or(ApplyError::NoSession)?;
    if session.applied {
        return Err(ApplyError::AlreadyApplied);
    }
    let text = session.proposed.clone().ok_or(ApplyError::NoProposedText)?;
    Ok((text, session.snapshot.clone()))
}

/// Pure pre-flight validation for `undo_transform`: does a session exist and
/// is it currently applied? A second call after a successful undo (which
/// flips `applied` back to `false`) is rejected here — this is what makes
/// double-undo a hard error rather than a silent no-op.
fn validate_undo(session: &Option<TransformSession>) -> Result<crate::selection::TransformSnapshot, ApplyError> {
    let session = session.as_ref().ok_or(ApplyError::NoSession)?;
    if !session.applied {
        return Err(ApplyError::NotApplied);
    }
    Ok(session.snapshot.clone())
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
    let (text, snapshot) = validate_apply(&session)?;

    #[cfg(target_os = "macos")]
    {
        let (tx, rx) = tokio::sync::oneshot::channel::<Result<AppliedVia, ApplyError>>();
        app_handle
            .run_on_main_thread(move || {
                let result = native::apply_text_to_target(&snapshot, &text);
                let _ = tx.send(result);
            })
            .map_err(|_| ApplyError::Unsupported)?;
        let result = rx.await.unwrap_or(Err(ApplyError::Unsupported));
        if result.is_ok() {
            set_applied(app_state, true);
        }
        result
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app_handle, text, snapshot);
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
    let snapshot = validate_undo(&session)?;
    let original = snapshot.text.clone();

    #[cfg(target_os = "macos")]
    {
        let (tx, rx) = tokio::sync::oneshot::channel::<Result<AppliedVia, ApplyError>>();
        app_handle
            .run_on_main_thread(move || {
                let result = native::apply_text_to_target(&snapshot, &original);
                let _ = tx.send(result);
            })
            .map_err(|_| ApplyError::Unsupported)?;
        let result = rx.await.unwrap_or(Err(ApplyError::Unsupported));
        result.map(|_| {
            set_applied(app_state, false);
        })
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app_handle, snapshot, original);
        Err(ApplyError::Unsupported)
    }
}

/// RAII guard: sets `TransformStatus::Applying` for its lifetime and always
/// resets to `Idle` on drop (both the success and the error path go back to
/// Idle — unlike `IdleGuard` in `commands/recording.rs`, there is no
/// "disarm" case here, since neither apply nor undo ever hands the pipeline
/// off to something else that would keep it busy).
pub struct ApplyingGuard<'a> {
    app_state: &'a AppState,
}

impl<'a> ApplyingGuard<'a> {
    pub fn new(app_state: &'a AppState) -> Self {
        app_state.set_transform_status(TransformStatus::Applying);
        Self { app_state }
    }
}

impl Drop for ApplyingGuard<'_> {
    fn drop(&mut self) {
        self.app_state.set_transform_status(TransformStatus::Idle);
    }
}

/// Apply the reviewed transform result. Requires `TransformStatus::ReviewPending`.
#[tauri::command]
pub async fn apply_transform_result(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, crate::State>,
) -> Result<String, String> {
    use tauri::Emitter;

    if state.app_state.transform_status() != TransformStatus::ReviewPending {
        return Err("No transform result is awaiting review.".to_string());
    }
    let _guard = ApplyingGuard::new(&state.app_state);
    match apply_transform(&app_handle, &state.app_state).await {
        Ok(via) => Ok(via.as_str().to_string()),
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

    if state.app_state.transform_status() != TransformStatus::Idle {
        return Err("Cannot undo while another transform action is in progress.".to_string());
    }
    let _guard = ApplyingGuard::new(&state.app_state);
    match undo_applied_transform(&app_handle, &state.app_state).await {
        Ok(()) => Ok(()),
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
        AppliedVia, ApplyError, AxWriteAttempt, ClipboardOutcome, ACTIVATION_SETTLE_MS,
        PASTE_CLIPBOARD_RESTORE_DELAY_MS,
    };
    use crate::selection::TransformSnapshot;
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
    const AX_QUERY_TIMEOUT_SECONDS: f32 = 0.025;
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
        unsafe { AXUIElementSetMessagingTimeout(element, AX_QUERY_TIMEOUT_SECONDS) == AX_SUCCESS }
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
    /// focused-element AX references so callers can chain further calls
    /// before either is released.
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

    /// Read `AXSelectedText` of the currently focused element of `pid`.
    fn read_selected_text(pid: i32) -> Option<String> {
        let (_app, focused) = focused_element(pid)?;
        let value = copy_attribute(focused.0, "AXSelectedText")?;
        cfstring_to_string(value.0)
    }

    /// Write `text` into `AXSelectedText` of the currently focused element of
    /// `pid`. Returns whether the set call reported success.
    fn write_selected_text(pid: i32, text: &str) -> bool {
        let Some((_app, focused)) = focused_element(pid) else {
            return false;
        };
        let Some(value) = cfstring(text) else {
            return false;
        };
        // `attr` must stay alive (bound to a variable) through the call below
        // — a temporary `CFGuard` dropped inline would `CFRelease` the
        // CFString before `AXUIElementSetAttributeValue` reads it.
        let Some(attr) = cfstring("AXSelectedText") else {
            return false;
        };
        let status = unsafe { AXUIElementSetAttributeValue(focused.0, attr.0, value.0) };
        status == AX_SUCCESS
    }

    /// Restore the originally captured selection range via
    /// `AXSelectedTextRange`. Best-effort — returns whether it succeeded so
    /// the caller can fall back to the "current selection still matches"
    /// check per the module doc comment.
    fn restore_selection_range(pid: i32, range: (usize, usize)) -> bool {
        let Some((_app, focused)) = focused_element(pid) else {
            return false;
        };
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
        let status =
            unsafe { AXUIElementSetAttributeValue(focused.0, attr.0, value_guard.0) };
        status == AX_SUCCESS
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
        NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .map(|app| app.processIdentifier() == pid)
            .unwrap_or(false)
    }

    /// Native implementation of the apply/undo write path. `text` is whatever
    /// should end up in the document — the proposed replacement for
    /// `apply_transform`, or `snapshot.text` (the original) for
    /// `undo_transform`. Must run on the main thread (same constraint as
    /// `inject_text`'s focus query and `capture_selection_native`).
    pub(super) fn apply_text_to_target(
        snapshot: &TransformSnapshot,
        text: &str,
    ) -> Result<AppliedVia, ApplyError> {
        // House rule: the proposed/undo text is written to the clipboard
        // FIRST, unconditionally, before anything else is attempted. The
        // user must never lose it regardless of what happens next.
        let original_clipboard = crate::injector::read_clipboard_text().ok();
        crate::injector::write_clipboard_text(text).map_err(|_| ApplyError::ClipboardUnavailable)?;

        let target_is_frontmost = activate_and_check_frontmost(snapshot.pid);
        let range_restore_succeeded = target_is_frontmost
            && snapshot
                .range
                .map(|range| restore_selection_range(snapshot.pid, range))
                .unwrap_or(false);
        let current_selection_matches_snapshot = target_is_frontmost
            && !range_restore_succeeded
            && read_selected_text(snapshot.pid)
                .map(|current| current == snapshot.text)
                .unwrap_or(false);

        let pre_write_guard = super::decide_pre_write_guard(
            target_is_frontmost,
            range_restore_succeeded,
            current_selection_matches_snapshot,
        );

        let ax_write = match pre_write_guard {
            Err(_) => AxWriteAttempt::NotAttempted,
            Ok(()) => {
                if write_selected_text(snapshot.pid, text) {
                    AxWriteAttempt::Succeeded
                } else {
                    AxWriteAttempt::Failed
                }
            }
        };

        let ax_verify_matched = if ax_write == AxWriteAttempt::Succeeded {
            Some(
                read_selected_text(snapshot.pid)
                    .map(|readback| readback == text)
                    .unwrap_or(false),
            )
        } else {
            None
        };

        let (paste_attempted, paste_succeeded) = if ax_write == AxWriteAttempt::Failed {
            (true, crate::injector::simulate_paste().is_ok())
        } else {
            (false, false)
        };

        let (result, clipboard_outcome) = super::decide_apply_outcome(
            pre_write_guard,
            ax_write,
            ax_verify_matched,
            paste_attempted,
            paste_succeeded,
        );

        if clipboard_outcome == ClipboardOutcome::RestoreOriginal {
            if matches!(result, Ok(AppliedVia::Paste)) {
                thread::sleep(Duration::from_millis(PASTE_CLIPBOARD_RESTORE_DELAY_MS));
            }
            if let Some(original) = original_clipboard {
                let _ = crate::injector::write_clipboard_text(&original);
            }
        }

        result
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
        set_applied(&state, true); // must not panic
        assert!(session_snapshot(&state).is_none());
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
        let session = Some(TransformSession::new(snapshot()));
        assert!(matches!(
            validate_apply(&session),
            Err(ApplyError::NoProposedText)
        ));
    }

    #[test]
    fn apply_rejects_an_already_applied_session() {
        let mut session = TransformSession::new(snapshot());
        session.proposed = Some("proposed".to_string());
        session.applied = true;
        assert!(matches!(
            validate_apply(&Some(session)),
            Err(ApplyError::AlreadyApplied)
        ));
    }

    #[test]
    fn apply_accepts_a_fresh_session_with_proposed_text() {
        let mut session = TransformSession::new(snapshot());
        session.proposed = Some("proposed".to_string());
        let (text, resolved_snapshot) = validate_apply(&Some(session)).expect("should validate");
        assert_eq!(text, "proposed");
        assert_eq!(resolved_snapshot.text, "original text");
    }

    #[test]
    fn undo_requires_a_session() {
        assert!(matches!(validate_undo(&None), Err(ApplyError::NoSession)));
    }

    #[test]
    fn undo_requires_the_session_to_be_applied() {
        let mut session = TransformSession::new(snapshot());
        session.proposed = Some("proposed".to_string());
        assert!(matches!(
            validate_undo(&Some(session)),
            Err(ApplyError::NotApplied)
        ));
    }

    #[test]
    fn undo_accepts_an_applied_session() {
        let mut session = TransformSession::new(snapshot());
        session.proposed = Some("proposed".to_string());
        session.applied = true;
        assert!(validate_undo(&Some(session)).is_ok());
    }

    #[test]
    fn double_undo_is_rejected() {
        // First undo (modeled directly, since the native write path isn't
        // exercised here) flips `applied` back to false; a second validation
        // against that same post-undo session must reject it.
        let mut session = TransformSession::new(snapshot());
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
        set_applied(&state, true);
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

    // -- Pure decision-table tests --

    #[test]
    fn pre_write_guard_table() {
        // (frontmost, range_restore_ok, current_matches) -> expected
        let cases: &[((bool, bool, bool), Result<(), ApplyError>)] = &[
            ((false, false, false), Err(ApplyError::TargetGone)),
            ((false, true, true), Err(ApplyError::TargetGone)), // frontmost check wins even if everything else looks fine
            ((true, true, false), Ok(())),
            ((true, true, true), Ok(())),
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
    fn apply_decision_table() {
        struct Case {
            name: &'static str,
            guard: Result<(), ApplyError>,
            ax_write: AxWriteAttempt,
            ax_verify_matched: Option<bool>,
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
                paste_attempted: true,
                paste_succeeded: false,
                expected_result: Err(ApplyError::PasteFailed),
                expected_clipboard: ClipboardOutcome::LeaveProposed,
            },
        ];

        for case in cases {
            let (result, clipboard) = decide_apply_outcome(
                case.guard,
                case.ax_write,
                case.ax_verify_matched,
                case.paste_attempted,
                case.paste_succeeded,
            );
            assert_eq!(result, case.expected_result, "case: {}", case.name);
            assert_eq!(clipboard, case.expected_clipboard, "case: {}", case.name);
        }
    }
}
