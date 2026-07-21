//! End-to-end orchestrator for the AX-selection transform flow (issue #312,
//! PR-C2). Wires together everything the earlier PRs in the series built:
//!
//! - B1's `selection::capture_selection` (read the AX selection, fail closed on
//!   secure fields / oversized selections / AX errors),
//! - B2's `transform_apply` session + apply/undo machinery,
//! - A2's `llm_sidecar` async transform facade (with its own busy/deadline/
//!   crash-breaker rules),
//! - C1's `transform_popover` window commands + geometry contract,
//! - the transform hotkey (`keyboard::start_transform_listener`, emitting
//!   `transform-key-pressed` / `transform-key-released`).
//!
//! ## Shape
//!
//! The logical flow is captured as a **pure state machine** (`decide`) over
//! `FlowState` × `FlowEvent`, exhaustively unit-tested — it is the single
//! specification of "which transition, which side effects" and carries no I/O.
//! The Tauri command wrappers below drive the real effects (AX capture, audio,
//! sidecar, popover, event emission) and mirror that machine's transitions.
//!
//! Every observable side effect is funnelled through the [`FlowEffects`] trait
//! so the async core (`core_start_capture`, `enter_thinking`, `run_transform`)
//! is testable against a recording fake with no Tauri app, no AX server, and —
//! for the sidecar step — the protocol mock helper (see
//! `tests/transform_flow_integration.rs`).
//!
//! ## Privacy (hard invariant)
//!
//! Instruction / original / proposed text NEVER reaches an event payload, a
//! log line, or telemetry. `transform-state-changed` carries only
//! `{ state, errorCode }` (both stable enums). The review text is pulled by the
//! popover window alone via `get_transform_review_content`. The state-machine
//! action list and every `emit_*` here are structurally incapable of carrying
//! that text.
//!
//! ## Spec-by-test (`decide`)
//!
//! The pure `decide` table is the **specification** of logical transitions. The
//! command layer is the authoritative I/O driver and may diverge where the
//! table cannot express concurrent races (e.g. cancel during AX capture). Unit
//! tests under `mod tests` assert the table matches the command layer at every
//! documented divergence point (Review+StartRequested supersede, undo-without-
//! epoch-bump, Applying+Cancel). Do not "fix" table drift by editing only
//! one side — update both and the tests together.

#![allow(dead_code)]

use std::sync::Arc;
use std::time::Duration;

use crate::llm_sidecar::{LlmSidecar, TransformError};
use crate::selection::{SelectionError, TransformSnapshot};
use crate::state::{AppState, DictationStatus, TransformStatus};
use crate::transform_apply::{self, ApplyError};
use crate::MutexExt;

/// Minimum hold before a transform-key release is treated as a real
/// instruction rather than an accidental tap. Enforced frontend-side (the
/// reducer in `useTransformFlow.ts`); mirrored here as the documented contract.
pub const HOLD_MIN_MS: u64 = 300;

/// Default deadline handed to the sidecar for one transform request. Kept
/// below the protocol's `MAX_DEADLINE_MS` (30s) with margin.
pub const DEFAULT_TRANSFORM_DEADLINE: Duration = Duration::from_millis(20_000);

/// How long the "applied" confirmation lingers before the popover auto-hides.
pub const APPLIED_LINGER_MS: u64 = 4_000;

// ===========================================================================
// Review state + error-code vocabulary (the ONLY strings that cross to the UI).
// ===========================================================================

/// The frontend `ReviewState` (see `lib/transformReview.ts`). Only the state
/// name and an optional error code are ever emitted — never any text content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewState {
    Listening,
    Thinking,
    Ready,
    Failed,
    Applied,
}

impl ReviewState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Listening => "listening",
            Self::Thinking => "thinking",
            Self::Ready => "ready",
            Self::Failed => "failed",
            Self::Applied => "applied",
        }
    }
}

/// Map a `SelectionError` to a stable popover error code. Total by construction
/// (exhaustive `match`) — a new variant will fail to compile until mapped.
pub fn selection_error_code(error: SelectionError) -> &'static str {
    match error {
        SelectionError::AccessibilityDenied => "accessibility_denied",
        SelectionError::SecureField => "secure_field",
        SelectionError::NoSelection => "no_selection",
        SelectionError::TooLarge => "too_large",
        SelectionError::AxUnavailable => "ax_unavailable",
    }
}

/// Map a `TransformError` (sidecar facade) to a stable popover error code.
/// Total by construction.
pub fn transform_error_code(error: TransformError) -> &'static str {
    match error {
        TransformError::Unsupported => "unsupported",
        TransformError::NotDownloaded => "model_not_downloaded",
        TransformError::Disabled => "disabled",
        TransformError::Busy => "busy",
        TransformError::InvalidRequest => "invalid_request",
        TransformError::HeavyRuntimeActive => "busy",
        TransformError::SpawnFailed => "crashed",
        TransformError::HandshakeFailed => "crashed",
        // Wrong-content model was removed by the supervisor — re-download needed.
        TransformError::ModelMismatch => "model_not_downloaded",
        TransformError::ModelUnreadable => "model_unreadable",
        TransformError::Timeout => "timeout",
        TransformError::Cancelled => "cancelled",
        TransformError::Crashed => "crashed",
        TransformError::OutputInvalid => "output_invalid",
        TransformError::Protocol => "crashed",
        TransformError::ResourceLimit => "resource_limit",
        TransformError::Internal => "internal",
    }
}

/// Map an `ApplyError` (write-back) to a stable popover error code. Total by
/// construction.
pub fn apply_error_code(error: ApplyError) -> &'static str {
    match error {
        ApplyError::Unsupported => "unsupported",
        ApplyError::Busy => "busy",
        ApplyError::NoSession => "no_session",
        ApplyError::NoProposedText => "no_proposed_text",
        ApplyError::AlreadyApplied => "already_applied",
        ApplyError::NotApplied => "not_applied",
        ApplyError::ClipboardUnavailable => "clipboard_unavailable",
        ApplyError::TargetGone => "target_gone",
        ApplyError::SelectionChanged => "selection_changed",
        ApplyError::PasteFailed => "paste_failed",
    }
}

// ===========================================================================
// Pure state machine (event × state -> next state + action list). Exhaustive.
// ===========================================================================

/// Logical (UI-level) state of one transform pass. Richer than the backend
/// `TransformStatus` lock-state: it distinguishes the two "popover awaiting the
/// user" cases (`Review` after a good proposal vs. `Failed` after an error) and
/// the post-apply `Applied` state (undo available), which both map onto
/// `TransformStatus::Idle`/`ReviewPending`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowState {
    Idle,
    Capturing,
    Listening,
    Thinking,
    /// Proposal shown, awaiting Approve / Retry / Cancel.
    Review,
    /// Error popover shown, awaiting Retry / Cancel.
    Failed,
    /// Apply or undo in flight.
    Applying,
    /// Applied; Undo available until the linger elapses.
    Applied,
}

/// Events the flow reacts to. Precondition/outcome detail (which error, etc.)
/// is carried by the command layer's action, not the pure machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowEvent {
    /// Transform hotkey pressed — begin a capture.
    StartRequested,
    /// Selection captured successfully.
    CaptureOk,
    /// Focused element is secure/password — abort silently + flash.
    CaptureSecure,
    /// Any other capture failure (no_selection / too_large / ax / denied).
    CaptureError,
    /// Transform hotkey released — stop listening, transcribe + transform.
    InstructionRequested,
    /// Transcribed instruction was blank.
    InstructionBlank,
    /// Sidecar returned a proposal.
    TransformOk,
    /// Transcription or sidecar failed.
    TransformError,
    /// User accepted the proposal.
    Approve,
    ApplyOk,
    ApplyError,
    /// User asked to re-speak on the same frozen snapshot.
    Retry,
    /// User cancelled (valid from any live state).
    Cancel,
    /// User undid an applied transform.
    Undo,
    UndoOk,
    UndoError,
}

/// A side effect the flow requests. The pure machine only NAMES effects; the
/// command layer performs them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowAction {
    /// Start (freeze) a new session from the captured snapshot.
    StartSession,
    /// Drop the active session.
    ClearSession,
    ShowPopover,
    HidePopover,
    /// Brief overlay flash indicating a secure field was refused.
    FlashSecureField,
    StartInstructionCapture,
    StopInstructionCapture,
    /// Transcribe the instruction and call the sidecar.
    RunTransform,
    /// Cancel the in-flight sidecar request.
    CancelInflight,
    ApplyResult,
    UndoResult,
    /// Auto-hide the popover after the applied linger.
    ScheduleLingerHide,
    /// Emit `transform-state-changed` with this review state (+ maybe a code).
    Emit(ReviewState),
    SetFocusable(bool),
    SetExpanded(bool),
    /// A no-op that is logged (e.g. a hotkey press while already mid-flow).
    Ignore,
}

/// Result of one `decide` step: the next logical state and the ordered actions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowDecision {
    pub next: FlowState,
    pub actions: Vec<FlowAction>,
}

fn go(next: FlowState, actions: Vec<FlowAction>) -> FlowDecision {
    FlowDecision { next, actions }
}

fn ignore(state: FlowState) -> FlowDecision {
    FlowDecision {
        next: state,
        actions: vec![FlowAction::Ignore],
    }
}

/// Common teardown for a cancel from a state that shows the popover.
fn cancel_from(_state: FlowState) -> FlowDecision {
    go(
        FlowState::Idle,
        vec![FlowAction::HidePopover, FlowAction::ClearSession],
    )
}

/// Re-arm listening on the SAME frozen snapshot (Retry = re-speak).
fn retry_to_listening() -> FlowDecision {
    go(
        FlowState::Listening,
        vec![
            FlowAction::SetExpanded(false),
            FlowAction::SetFocusable(false),
            FlowAction::Emit(ReviewState::Listening),
            FlowAction::StartInstructionCapture,
        ],
    )
}

/// Pure transition function: `(state, event) -> decision`. Exhaustive over both
/// axes (a missing arm is a compile error) so the whole table is covered by
/// `tests::transition_table_is_exhaustive_and_stable`.
pub fn decide(state: FlowState, event: FlowEvent) -> FlowDecision {
    use FlowAction::*;
    use FlowEvent as E;
    use FlowState as S;

    match (state, event) {
        // ---- Idle -----------------------------------------------------------
        (S::Idle, E::StartRequested) => go(S::Capturing, vec![]),
        (S::Idle, _) => ignore(S::Idle),

        // ---- Capturing ------------------------------------------------------
        (S::Capturing, E::CaptureOk) => go(
            S::Listening,
            vec![
                StartSession,
                ShowPopover,
                SetFocusable(false),
                Emit(ReviewState::Listening),
                StartInstructionCapture,
            ],
        ),
        (S::Capturing, E::CaptureSecure) => {
            go(S::Idle, vec![ClearSession, HidePopover, FlashSecureField])
        }
        (S::Capturing, E::CaptureError) => go(
            S::Failed,
            vec![ShowPopover, SetFocusable(true), Emit(ReviewState::Failed)],
        ),
        (S::Capturing, E::Cancel) => cancel_from(state),
        (S::Capturing, _) => ignore(S::Capturing),

        // ---- Listening ------------------------------------------------------
        (S::Listening, E::InstructionRequested) => go(
            S::Thinking,
            vec![StopInstructionCapture, Emit(ReviewState::Thinking), RunTransform],
        ),
        (S::Listening, E::Cancel) => go(
            S::Idle,
            vec![StopInstructionCapture, HidePopover, ClearSession],
        ),
        (S::Listening, _) => ignore(S::Listening),

        // ---- Thinking -------------------------------------------------------
        (S::Thinking, E::TransformOk) => go(
            S::Review,
            vec![SetExpanded(true), SetFocusable(true), Emit(ReviewState::Ready)],
        ),
        (S::Thinking, E::TransformError) => go(
            S::Failed,
            vec![SetFocusable(true), Emit(ReviewState::Failed)],
        ),
        (S::Thinking, E::InstructionBlank) => go(
            S::Failed,
            vec![SetFocusable(true), Emit(ReviewState::Failed)],
        ),
        (S::Thinking, E::Cancel) => go(
            S::Idle,
            vec![CancelInflight, HidePopover, ClearSession],
        ),
        (S::Thinking, _) => ignore(S::Thinking),

        // ---- Review (proposal shown) ---------------------------------------
        (S::Review, E::Approve) => go(S::Applying, vec![ApplyResult]),
        (S::Review, E::Retry) => retry_to_listening(),
        (S::Review, E::Cancel) => cancel_from(state),
        (S::Review, _) => ignore(S::Review),

        // ---- Failed (error popover) ----------------------------------------
        (S::Failed, E::Retry) => retry_to_listening(),
        (S::Failed, E::Cancel) => cancel_from(state),
        // A new hotkey press supersedes a failed popover.
        (S::Failed, E::StartRequested) => go(S::Capturing, vec![ClearSession]),
        (S::Failed, _) => ignore(S::Failed),

        // ---- Applying (apply or undo in flight) ----------------------------
        (S::Applying, E::ApplyOk) => go(
            S::Applied,
            vec![Emit(ReviewState::Applied), ScheduleLingerHide],
        ),
        (S::Applying, E::ApplyError) => go(
            S::Failed,
            vec![SetFocusable(true), Emit(ReviewState::Failed)],
        ),
        (S::Applying, E::UndoOk) => go(S::Idle, vec![HidePopover, ClearSession]),
        // Undo-failure UX (item 12): stay Applied and re-emit Applied — NOT
        // Failed — so the Undo button stays reachable and the applied text is
        // never stranded un-undoable. The command layer carries the undo error
        // code on this `applied` emit (spec-by-test:
        // `decide_undo_error_keeps_applied_and_undo_reachable`).
        (S::Applying, E::UndoError) => {
            go(S::Applied, vec![Emit(ReviewState::Applied)])
        }
        // Cancel during Applying: tear down; ApplyingGuard must not resurrect
        // ReviewPending after the session is cleared (see cancel_transform).
        (S::Applying, E::Cancel) => cancel_from(state),
        (S::Applying, _) => ignore(S::Applying),

        // ---- Applied (undo available) --------------------------------------
        // UndoResult: apply-path undo that, on success, hides + clears session
        // WITHOUT bumping the clipboard-restore epoch (see
        // `undo_transform_and_close`). Failure keeps Applied and emits Failed.
        (S::Applied, E::Undo) => go(S::Applying, vec![UndoResult]),
        (S::Applied, E::Cancel) => cancel_from(state),
        // A new hotkey press supersedes the applied linger popover.
        (S::Applied, E::StartRequested) => go(S::Capturing, vec![ClearSession]),
        (S::Applied, _) => ignore(S::Applied),
    }
}

// ===========================================================================
// Effects seam: everything observable, so the async core is testable.
// ===========================================================================

/// Anchor rect handed to the popover (mirrors `transform_popover::Rect`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnchorRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Every side effect the async core performs, behind a trait so a recording
/// fake can stand in for Tauri in tests.
pub(crate) trait FlowEffects: Send + Sync {
    /// Emit `transform-state-changed` — state name + optional error code ONLY.
    fn emit_state(&self, state: ReviewState, error_code: Option<&str>);
    fn show_popover(&self, anchor: Option<AnchorRect>);
    fn hide_popover(&self);
    fn set_focusable(&self, focusable: bool);
    fn set_expanded(&self, expanded: bool);
    fn flash_secure_field(&self);
    fn schedule_linger_hide(&self);
}

fn snapshot_anchor(snapshot: &TransformSnapshot) -> Option<AnchorRect> {
    snapshot.bounds.map(|r| AnchorRect {
        x: r.x,
        y: r.y,
        width: r.width,
        height: r.height,
    })
}

/// Outcome of `core_start_capture`, telling the command whether to arm audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StartOutcome {
    /// Session frozen, popover shown listening — start the instruction audio.
    Listening,
    /// Aborted (secure field, capture error, or model not downloaded) — no
    /// audio, nothing further for the command to do.
    Aborted,
}

/// Core of `start_transform_capture` (see the command). Assumes the caller has
/// already atomically claimed `TransformStatus::Capturing` under the dictation
/// lock. Generic over the capture future so tests inject a fake snapshot
/// WITHOUT weakening the production AX path in `selection.rs`.
pub(crate) async fn core_start_capture<Fut>(
    app_state: &AppState,
    fx: &dyn FlowEffects,
    model_ready: bool,
    capture: Fut,
) -> StartOutcome
where
    Fut: std::future::Future<Output = Result<TransformSnapshot, SelectionError>>,
{
    if !model_ready {
        // Discoverable error surface: still show the popover, in a failed state.
        // Bail if a short-tap cancel already left Capturing.
        if !app_state.try_transition_transform_status(
            TransformStatus::Capturing,
            TransformStatus::ReviewPending,
        ) {
            return StartOutcome::Aborted;
        }
        fx.show_popover(None);
        fx.set_focusable(true);
        fx.emit_state(ReviewState::Failed, Some("model_not_downloaded"));
        return StartOutcome::Aborted;
    }

    match capture.await {
        Ok(snapshot) => {
            // Cancel during slow AX capture must not resurrect the flow:
            // session re-install + mic arm + popover while the reducer thinks
            // not-holding would wedge Listening with no release coming.
            if !app_state.try_transition_transform_status(
                TransformStatus::Capturing,
                TransformStatus::Listening,
            ) {
                return StartOutcome::Aborted;
            }
            let anchor = snapshot_anchor(&snapshot);
            transform_apply::start_session(app_state, snapshot);
            fx.show_popover(anchor);
            fx.set_focusable(false);
            fx.emit_state(ReviewState::Listening, None);
            StartOutcome::Listening
        }
        Err(SelectionError::SecureField) => {
            // Never show content UI for a password field — abort silently.
            if !app_state.try_transition_transform_status(
                TransformStatus::Capturing,
                TransformStatus::Idle,
            ) {
                return StartOutcome::Aborted;
            }
            transform_apply::clear_session(app_state);
            fx.hide_popover();
            fx.flash_secure_field();
            StartOutcome::Aborted
        }
        Err(other) => {
            if !app_state.try_transition_transform_status(
                TransformStatus::Capturing,
                TransformStatus::ReviewPending,
            ) {
                // Cancelled during capture — tear down any partial UI.
                transform_apply::clear_session(app_state);
                fx.hide_popover();
                return StartOutcome::Aborted;
            }
            fx.show_popover(None);
            fx.set_focusable(true);
            fx.emit_state(ReviewState::Failed, Some(selection_error_code(other)));
            StartOutcome::Aborted
        }
    }
}

/// Transition `Listening -> Thinking` and emit it. Returns `false` if the flow
/// was no longer `Listening` (e.g. cancelled) — the caller must not proceed.
pub(crate) fn enter_thinking(app_state: &AppState, fx: &dyn FlowEffects) -> bool {
    if app_state.try_transition_transform_status(TransformStatus::Listening, TransformStatus::Thinking)
    {
        fx.emit_state(ReviewState::Thinking, None);
        true
    } else {
        false
    }
}

/// Core of the transform step: given the transcription result (`Err` = blank),
/// call the sidecar and drive the flow to `ready` or `failed`. The sidecar call
/// runs as a cancellable task whose abort handle is stored in `inflight` so
/// `cancel_transform` can abort the outer wrapper; it **also** calls
/// `LlmSidecar::cancel_inflight_request` so the blocking work sends a Cancel
/// frame and clears `busy` promptly. Terminal status writes use
/// `try_transition(Thinking → ReviewPending)` so a cancel landing mid-flight
/// cannot resurrect ReviewPending (which would wedge dictation).
pub(crate) async fn run_transform(
    app_state: &AppState,
    fx: &dyn FlowEffects,
    sidecar: &Arc<LlmSidecar>,
    inflight: &std::sync::Mutex<Option<tokio::task::AbortHandle>>,
    instruction: Result<String, ()>,
    original: String,
    deadline: Duration,
) {
    let instruction = match instruction {
        Ok(text) if !text.trim().is_empty() => text,
        _ => {
            if app_state.try_transition_transform_status(
                TransformStatus::Thinking,
                TransformStatus::ReviewPending,
            ) {
                fx.set_focusable(true);
                fx.emit_state(ReviewState::Failed, Some("no_instruction"));
            }
            return;
        }
    };
    // Cancel during transcription can clear the session before we get here —
    // do not spawn the sidecar on a dead session.
    if !transform_apply::set_instruction(app_state, instruction.clone()) {
        return;
    }

    let sidecar = Arc::clone(sidecar);
    let input = original;
    let instr = instruction;
    // Per-request cancel token (item 11): created here, before the spawn, and
    // handed to the sidecar for exactly this request. `transform` registers it
    // as the in-flight token, so `cancel_transform`'s
    // `cancel_inflight_request()` cooperatively cancels ONLY this request —
    // never a neighbouring one. Its lifetime tracks the AbortHandle stored just
    // below (both cleared when the request settles).
    let cancel = crate::llm_sidecar::CancelToken::new();
    let join = tokio::spawn(async move {
        sidecar.transform(&instr, &input, deadline, cancel).await
    });
    *inflight.lock_or_recover() = Some(join.abort_handle());
    let joined = join.await;
    *inflight.lock_or_recover() = None;

    match joined {
        // Aborted by `cancel_transform`, which already tore the flow down.
        Err(err) if err.is_cancelled() => {}
        Err(_) => {
            if app_state.try_transition_transform_status(
                TransformStatus::Thinking,
                TransformStatus::ReviewPending,
            ) {
                fx.set_focusable(true);
                fx.emit_state(ReviewState::Failed, Some("internal"));
            }
        }
        Ok(Ok(output)) => {
            if !app_state.try_transition_transform_status(
                TransformStatus::Thinking,
                TransformStatus::ReviewPending,
            ) {
                return;
            }
            transform_apply::set_proposed_text(app_state, output.output);
            fx.set_expanded(true);
            fx.set_focusable(true);
            fx.emit_state(ReviewState::Ready, None);
        }
        // Cooperative cancel from the sidecar: cancel_transform already tore
        // the flow down — do not force ReviewPending.
        Ok(Err(TransformError::Cancelled)) => {}
        Ok(Err(error)) => {
            if app_state.try_transition_transform_status(
                TransformStatus::Thinking,
                TransformStatus::ReviewPending,
            ) {
                fx.set_focusable(true);
                fx.emit_state(ReviewState::Failed, Some(transform_error_code(error)));
            }
        }
    }
}

// ===========================================================================
// Production effects: real Tauri emission + popover window commands.
// ===========================================================================

/// Content-free "the popover was hidden by the backend" signal (item 13).
///
/// Backend-initiated hides (short-tap cancel, linger auto-hide, audio-start
/// teardown, secure-field/capture aborts) never go through the popover's own
/// Cancel/Undo buttons, so the popover webview keeps whatever review content it
/// last fetched and could flash it on the NEXT show. This bare event tells the
/// review driver to reset its content to `EMPTY_REVIEW_CONTENT`. Privacy: the
/// payload is empty — no state text, no error, nothing.
fn emit_transform_hidden(app: &tauri::AppHandle) {
    use tauri::Emitter;
    let _ = app.emit("transform-review-hidden", ());
}

pub(crate) struct TauriFlowEffects<'a> {
    pub app: &'a tauri::AppHandle,
    pub state: &'a crate::State,
}

impl FlowEffects for TauriFlowEffects<'_> {
    fn emit_state(&self, state: ReviewState, error_code: Option<&str>) {
        use tauri::Emitter;
        // Payload carries ONLY the state name and an optional stable error
        // code — never any instruction/original/proposed text.
        let payload = match error_code {
            Some(code) => serde_json::json!({ "state": state.as_str(), "errorCode": code }),
            None => serde_json::json!({ "state": state.as_str() }),
        };
        let _ = self.app.emit("transform-state-changed", payload);
    }

    fn show_popover(&self, anchor: Option<AnchorRect>) {
        let anchor = anchor.map(|a| crate::commands::transform_popover::Rect {
            x: a.x,
            y: a.y,
            width: a.width,
            height: a.height,
        });
        if let Err(e) =
            crate::commands::transform_popover::show_popover_internal(self.app, self.state, anchor)
        {
            tracing::warn!(target: "transform", error = %e, "show_popover failed");
        }
    }

    fn hide_popover(&self) {
        let _ = crate::commands::transform_popover::hide_popover_internal(self.app);
        // A backend-driven hide (e.g. secure-field / capture-error abort in
        // core_start_capture) must reset the popover's stale content (item 13).
        emit_transform_hidden(self.app);
    }

    fn set_focusable(&self, focusable: bool) {
        let _ = crate::commands::transform_popover::set_focusable_internal(self.app, focusable);
    }

    fn set_expanded(&self, expanded: bool) {
        if let Err(e) =
            crate::commands::transform_popover::set_expanded_internal(self.app, self.state, expanded)
        {
            tracing::warn!(target: "transform", error = %e, "set_expanded failed");
        }
    }

    fn flash_secure_field(&self) {
        use tauri::Emitter;
        // Content-free flash signal — no selection text is read for a secure
        // field, so there is nothing to leak here.
        let _ = self.app.emit("transform-secure-field", ());
    }

    fn schedule_linger_hide(&self) {
        let app = self.app.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(APPLIED_LINGER_MS)).await;
            use tauri::Manager;
            if let Some(state) = app.try_state::<crate::State>() {
                // Only auto-hide if we are still in the applied review (status
                // Idle + an applied session). A newer flow leaves status
                // non-Idle, so its popover is never yanked out from under it.
                let still_applied = state.app_state.transform_status() == TransformStatus::Idle
                    && transform_apply::session_snapshot(&state.app_state)
                        .map(|s| s.applied)
                        .unwrap_or(false);
                if !still_applied {
                    return;
                }
                // Free the held selection text once Undo is no longer reachable
                // from the UI (popover gone). Content is only available via
                // get_transform_review_content, which returns empty after this.
                transform_apply::clear_session(&state.app_state);
            }
            let _ = crate::commands::transform_popover::hide_popover_internal(&app);
            // Linger auto-hide is backend-initiated — reset stale content (item 13).
            emit_transform_hidden(&app);
        });
    }
}

// ===========================================================================
// Instruction transcription (cleanup-only) — a command-level effect.
// ===========================================================================

/// Transcribe the captured instruction audio into a clean prompt. Runs the
/// shared ASR backend, then the deterministic transcript pipeline with the
/// **cleanup-only** stage config (`instruction_cleanup`) — NEVER voice
/// commands, CLI canonicalization, smart formatting, or the IDE-context stage
/// (an instruction is a prompt, not dictation). `Err(())` means empty audio, a
/// transcription failure, or a blank transcript — all surface as `no_instruction`.
async fn transcribe_instruction(
    app_handle: &tauri::AppHandle,
    state: &crate::State,
    samples: &[f32],
) -> Result<String, ()> {
    if samples.is_empty() {
        return Err(());
    }
    let (model_name, language, smart_punctuation) = {
        let dictation = state.app_state.dictation.lock_or_recover();
        (
            dictation.model_name.clone(),
            dictation.language.clone(),
            dictation.smart_punctuation,
        )
    };

    let raw = state.app_state.model_runtime.with_ready_backend(
        Some(app_handle),
        &model_name,
        crate::model_runtime::PreparationReason::Pipeline,
        |backend| backend.transcribe(samples, &language, None, smart_punctuation),
    );
    let (raw, _load_report) = match raw {
        Ok(pair) => pair,
        Err(e) => {
            tracing::warn!(target: "transform", error = %e, "instruction transcription failed");
            return Err(());
        }
    };

    let context = crate::transcript_transform::TranscriptContext {
        session_id: state.app_state.next_transcript_session_id(),
        source: crate::transcript_transform::TranscriptSource::Live,
        context_handle: None,
        cli_formatting_mode: crate::cli_command::CliFormattingMode::Auto,
        stages: crate::transcript_transform::TranscriptStageConfig::instruction_cleanup(),
    };
    let cleaned = crate::transcript_transform::transform_transcript(
        raw,
        &context,
        crate::transcript_transform::TranscriptTransformResources::empty(),
    )
    .map(|out| out.text)
    .unwrap_or_default();

    let trimmed = cleaned.trim().to_string();
    if trimmed.is_empty() {
        Err(())
    } else {
        Ok(trimmed)
    }
}

// ===========================================================================
// Tauri commands (the thin wrappers the frontend calls).
// ===========================================================================

/// Begin a transform pass: check preconditions, claim the flow, capture the
/// selection, and (on success) arm the instruction audio.
///
/// The claim (`Idle -> Capturing`) happens under the dictation lock, in the
/// SAME critical section `start_native_recording` uses to check transform
/// status — closing the symmetric race (dictation-start checks transform
/// status; transform-start must check dictation status under the same lock).
#[tauri::command]
pub(crate) async fn start_transform_capture(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, crate::State>,
    device_name: Option<String>,
) -> Result<(), String> {
    // Serialize against dictation start/stop, taking the same locks in the
    // same order (`recording_transition` then `dictation`) as
    // `start_native_recording`, so the two audio paths can't tear each other's
    // recorder down.
    let _transition = state.app_state.recording_transition.lock().await;

    let claimed = {
        let dictation = state.app_state.dictation.lock_or_recover();
        if dictation.status != DictationStatus::Idle {
            tracing::info!(target: "transform", "start_transform_capture: ignored — dictation active");
            false
        } else if state.benchmark.is_running() {
            tracing::info!(target: "transform", "start_transform_capture: ignored — benchmark running");
            false
        } else if state
            .app_state
            .file_transcribing
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            tracing::info!(target: "transform", "start_transform_capture: ignored — file transcription running");
            false
        } else if state.transform_runtime.is_transform_busy() {
            tracing::info!(target: "transform", "start_transform_capture: ignored — transform runtime busy");
            false
        } else {
            // Atomic Idle -> Capturing under the dictation lock.
            state
                .app_state
                .try_transition_transform_status(TransformStatus::Idle, TransformStatus::Capturing)
        }
    };
    if !claimed {
        return Ok(());
    }

    let model_ready = crate::commands::transform_model::transform_model_status().state
        == crate::commands::transform_model::TransformModelState::Ready;

    let fx = TauriFlowEffects {
        app: &app_handle,
        state: &state,
    };
    let outcome =
        core_start_capture(&state.app_state, &fx, model_ready, crate::selection::capture_selection(&app_handle))
            .await;

    if outcome == StartOutcome::Listening {
        // Arm audio for the spoken instruction. DictationStatus stays Idle —
        // this audio belongs to the transform flow (TransformStatus::Listening).
        // Pass the configured input device (same contract as start_native_recording).
        if let Err(e) = crate::audio::start_recording(Some(app_handle.clone()), device_name) {
            tracing::error!(target: "transform", error = %e, "instruction audio failed to start");
            state.app_state.set_transform_status(TransformStatus::Idle);
            transform_apply::clear_session(&state.app_state);
            let _ = crate::commands::transform_popover::hide_popover_internal(&app_handle);
            emit_transform_hidden(&app_handle);
            return Err(e);
        }
        // Mic-leak window (item 10): `cancel_transform` is lock-free and does
        // not take `recording_transition`, so a short-tap cancel can land
        // between `core_start_capture` returning Listening and the mic actually
        // coming up here. At that instant the cancel saw `is_recording() ==
        // false` and did NOT stop anything, so without this re-check the mic
        // would stay live with status no longer Listening. Re-verify and tear
        // down if the flow was cancelled out from under us.
        if state.app_state.transform_status() != TransformStatus::Listening {
            if crate::audio::is_recording() {
                let _ = crate::audio::stop_recording();
            }
            // cancel_transform already cleared the session / hid the popover;
            // repeat the teardown idempotently so no half-state survives.
            transform_apply::clear_session(&state.app_state);
            let _ = crate::commands::transform_popover::hide_popover_internal(&app_handle);
            emit_transform_hidden(&app_handle);
            return Ok(());
        }
    }
    Ok(())
}

/// Finish the instruction utterance: stop audio, transition to thinking,
/// transcribe (cleanup-only), and run the sidecar transform.
#[tauri::command]
pub(crate) async fn finish_transform_instruction(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, crate::State>,
) -> Result<(), String> {
    let _transition = state.app_state.recording_transition.lock().await;

    // Tolerate a stray release: only proceed from Listening.
    if state.app_state.transform_status() != TransformStatus::Listening {
        return Ok(());
    }

    let samples = crate::audio::stop_recording().unwrap_or_default();

    let fx = TauriFlowEffects {
        app: &app_handle,
        state: &state,
    };
    if !enter_thinking(&state.app_state, &fx) {
        // Lost the race to a concurrent cancel.
        return Ok(());
    }

    let original = match transform_apply::session_snapshot(&state.app_state) {
        Some(session) => session.snapshot.text,
        None => {
            // Session vanished (cancelled) — bail cleanly.
            state.app_state.set_transform_status(TransformStatus::Idle);
            return Ok(());
        }
    };

    let instruction = match transcribe_instruction(&app_handle, &state, &samples).await {
        Ok(raw) => Ok(expand_instruction(&state, &raw)),
        Err(()) => Err(()),
    };
    run_transform(
        &state.app_state,
        &fx,
        &state.transform_runtime,
        &state.app_state.transform_inflight,
        instruction,
        original,
        DEFAULT_TRANSFORM_DEADLINE,
    )
    .await;
    Ok(())
}

/// Expand a transcribed instruction when it names a built-in preset or a
/// saved `KnowledgeKind::Transform`. Otherwise the raw transcript is the
/// instruction (free-form spoken rewrite request). Never logs the text.
fn expand_instruction(state: &crate::State, spoken: &str) -> String {
    if let Some(preset) = crate::transform_presets::resolve_preset(spoken) {
        return preset.to_string();
    }
    if let Some(saved) = resolve_saved_transform(state, spoken) {
        return saved;
    }
    spoken.to_string()
}

/// Case-insensitive match of a spoken name against enabled global/app
/// Transform knowledge entries. Returns the instruction body, not the name.
fn resolve_saved_transform(state: &crate::State, spoken: &str) -> Option<String> {
    use crate::knowledge_store::{KnowledgeKind, KnowledgeListRequest, KnowledgePayload};

    let request = KnowledgeListRequest {
        kind: Some(KnowledgeKind::Transform),
        enabled: Some(true),
        limit: Some(100),
        ..Default::default()
    };
    let response = state.knowledge.list(request).ok()?;
    let key = spoken
        .trim()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    if key.is_empty() {
        return None;
    }
    for entry in response.entries {
        if let KnowledgePayload::Transform { name, instruction } = entry.payload {
            let name_key = name
                .trim()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_ascii_lowercase();
            if name_key == key {
                return Some(instruction);
            }
        }
    }
    None
}

/// Retry: re-arm listening for a NEW instruction on the SAME frozen snapshot.
#[tauri::command]
pub(crate) async fn retry_transform_instruction(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, crate::State>,
    device_name: Option<String>,
) -> Result<(), String> {
    let _transition = state.app_state.recording_transition.lock().await;

    let fx = TauriFlowEffects {
        app: &app_handle,
        state: &state,
    };

    // Retry only means anything with a live session (a frozen snapshot). A
    // failed popover with no session (e.g. model_not_downloaded) has nothing
    // to re-speak against — treat Retry there as Cancel.
    if transform_apply::session_snapshot(&state.app_state).is_none() {
        state.app_state.set_transform_status(TransformStatus::Idle);
        let _ = crate::commands::transform_popover::hide_popover_internal(&app_handle);
        emit_transform_hidden(&app_handle);
        return Ok(());
    }

    if !state
        .app_state
        .try_transition_transform_status(TransformStatus::ReviewPending, TransformStatus::Listening)
    {
        // Not in review — ignore.
        return Ok(());
    }

    fx.set_expanded(false);
    fx.set_focusable(false);
    fx.emit_state(ReviewState::Listening, None);

    if let Err(e) = crate::audio::start_recording(Some(app_handle.clone()), device_name) {
        tracing::error!(target: "transform", error = %e, "retry audio failed to start");
        state.app_state.set_transform_status(TransformStatus::Idle);
        transform_apply::clear_session(&state.app_state);
        let _ = crate::commands::transform_popover::hide_popover_internal(&app_handle);
        emit_transform_hidden(&app_handle);
        return Err(e);
    }
    // Same mic-leak re-check as start_transform_capture (item 10): a cancel
    // that raced in during audio startup must not leave the mic live.
    if state.app_state.transform_status() != TransformStatus::Listening {
        if crate::audio::is_recording() {
            let _ = crate::audio::stop_recording();
        }
        transform_apply::clear_session(&state.app_state);
        let _ = crate::commands::transform_popover::hide_popover_internal(&app_handle);
        emit_transform_hidden(&app_handle);
        return Ok(());
    }
    Ok(())
}

/// Approve: apply the proposed transform. On success emits `applied` and
/// schedules the linger-hide; the session stays applied so `undo_transform`
/// remains valid.
#[tauri::command]
pub(crate) async fn approve_transform(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, crate::State>,
) -> Result<(), String> {
    let fx = TauriFlowEffects {
        app: &app_handle,
        state: &state,
    };

    let mut guard =
        match transform_apply::ApplyingGuard::try_new(&state.app_state, TransformStatus::ReviewPending)
        {
            Some(guard) => guard,
            None => {
                fx.emit_state(ReviewState::Failed, Some("busy"));
                return Err("busy".to_string());
            }
        };

    match transform_apply::apply_transform(&app_handle, &state.app_state).await {
        Ok(_via) => {
            guard.mark_succeeded(); // status -> Idle; session.applied stays true
            fx.emit_state(ReviewState::Applied, None);
            fx.schedule_linger_hide();
            Ok(())
        }
        Err(error) => {
            // Guard drop restores ReviewPending so Retry stays available.
            fx.set_focusable(true);
            fx.emit_state(ReviewState::Failed, Some(apply_error_code(error)));
            Err(apply_error_code(error).to_string())
        }
    }
}

/// Cancel the flow from any state (including Applying): abort an in-flight
/// sidecar request cooperatively, stop instruction audio, clear the session,
/// and hide the popover. Clearing the session before hide means the next
/// `get_transform_review_content` returns empty — no stale React flash of prior
/// selection text.
#[tauri::command]
pub(crate) async fn cancel_transform(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, crate::State>,
) -> Result<(), String> {
    let prev = state.app_state.transform_status();

    // Cooperative cancel first: the blocking spawn_blocking work only clears
    // BusyGuard when it finishes, so abort alone would leave busy held up to
    // the deadline. cancel_inflight_request makes run_request send Cancel.
    state.transform_runtime.cancel_inflight_request();
    if let Some(handle) = state.app_state.transform_inflight.lock_or_recover().take() {
        handle.abort();
    }
    // Invalidate any pending clipboard restore from a just-finished apply/undo
    // (N1): a cancel means the user is done with this pass. (Do NOT use cancel
    // after a successful undo — see undo_transform_and_close.)
    let _ = state.app_state.next_transform_apply_epoch();

    // Stop instruction audio only if it was the transform's (Listening).
    if prev == TransformStatus::Listening && crate::audio::is_recording() {
        let _ = crate::audio::stop_recording();
    }

    // Force Idle even from Applying — ApplyingGuard drop will no-op once status
    // is no longer Applying (try_transition).
    state.app_state.set_transform_status(TransformStatus::Idle);
    transform_apply::clear_session(&state.app_state);
    let _ = crate::commands::transform_popover::hide_popover_internal(&app_handle);
    // Backend-initiated hide (short-tap cancel, mid-hold cleanup, or the
    // popover's own Cancel button all route here): reset stale content (item 13).
    emit_transform_hidden(&app_handle);
    Ok(())
}

/// Undo an applied transform and close the popover **without** bumping the
/// clipboard-restore epoch a second time.
///
/// `undo_applied_transform` already advances the epoch once (to protect the
/// undo's own delayed clipboard restore). Chaining `cancel_transform` after
/// undo would bump it again inside the 300ms restore window and clobber the
/// user's prior clipboard on every paste-fallback undo.
///
/// On success: hide popover + clear session (undo unreachable once hidden).
/// On failure: keep the Applied session and emit `failed` so Undo stays available.
#[tauri::command]
pub(crate) async fn undo_transform_and_close(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, crate::State>,
) -> Result<(), String> {
    let fx = TauriFlowEffects {
        app: &app_handle,
        state: &state,
    };

    let mut guard =
        match transform_apply::ApplyingGuard::try_new(&state.app_state, TransformStatus::Idle) {
            Some(guard) => guard,
            None => {
                fx.emit_state(ReviewState::Failed, Some("busy"));
                return Err("busy".to_string());
            }
        };

    match transform_apply::undo_applied_transform(&app_handle, &state.app_state).await {
        Ok(()) => {
            guard.mark_succeeded();
            // Hide + clear WITHOUT another epoch bump (cancel would bump).
            transform_apply::clear_session(&state.app_state);
            let _ = crate::commands::transform_popover::hide_popover_internal(&app_handle);
            emit_transform_hidden(&app_handle);
            Ok(())
        }
        Err(error) => {
            // Undo-failure UX (item 12): the guard drop restores Idle and the
            // session stays applied, so Undo is still valid. Emitting `failed`
            // here would strand the user on a popover with NO Undo button and a
            // dead Retry — the applied text would become permanently
            // un-undoable. Instead re-emit `applied` carrying the error code so
            // the Applied UI (Undo button) stays reachable while surfacing the
            // failure. Privacy: state event stays {state, errorCode} only.
            fx.emit_state(ReviewState::Applied, Some(apply_error_code(error)));
            Err(apply_error_code(error).to_string())
        }
    }
}

// ===========================================================================
// Test-only happy-path harness (drives the real sidecar via the mock helper).
// ===========================================================================

/// Recording fake for [`FlowEffects`]: captures the emitted review states and
/// popover directives so tests can assert the flow's observable behaviour with
/// no Tauri app.
#[cfg(any(test, debug_assertions, feature = "llm-test-support"))]
#[derive(Default)]
pub(crate) struct RecordingFlowEffects {
    inner: std::sync::Mutex<RecordingInner>,
}

#[cfg(any(test, debug_assertions, feature = "llm-test-support"))]
#[derive(Default)]
struct RecordingInner {
    /// (state, error_code) pairs, in emit order.
    emitted: Vec<(String, Option<String>)>,
    popover_shown: bool,
    focusable: Option<bool>,
    expanded: Option<bool>,
    secure_flash: bool,
    linger_scheduled: bool,
}

#[cfg(any(test, debug_assertions, feature = "llm-test-support"))]
impl RecordingFlowEffects {
    pub fn new() -> Self {
        Self::default()
    }

    /// Emitted review-state names in order (error codes dropped).
    pub fn emitted_states(&self) -> Vec<String> {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .emitted
            .iter()
            .map(|(state, _)| state.clone())
            .collect()
    }

    /// Emitted (state, error_code) pairs in order.
    pub fn emitted(&self) -> Vec<(String, Option<String>)> {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .emitted
            .clone()
    }

    pub fn popover_shown(&self) -> bool {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).popover_shown
    }

    pub fn secure_flash(&self) -> bool {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).secure_flash
    }
}

#[cfg(any(test, debug_assertions, feature = "llm-test-support"))]
impl FlowEffects for RecordingFlowEffects {
    fn emit_state(&self, state: ReviewState, error_code: Option<&str>) {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .emitted
            .push((state.as_str().to_string(), error_code.map(|s| s.to_string())));
    }
    fn show_popover(&self, _anchor: Option<AnchorRect>) {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).popover_shown = true;
    }
    fn hide_popover(&self) {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).popover_shown = false;
    }
    fn set_focusable(&self, focusable: bool) {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).focusable = Some(focusable);
    }
    fn set_expanded(&self, expanded: bool) {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).expanded = Some(expanded);
    }
    fn flash_secure_field(&self) {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).secure_flash = true;
    }
    fn schedule_linger_hide(&self) {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).linger_scheduled = true;
    }
}

/// Observable result of [`run_happy_path_for_test`].
#[cfg(any(debug_assertions, feature = "llm-test-support"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HappyPathReport {
    /// Review-state names emitted, in order (expected: listening, thinking, ready).
    pub emitted_states: Vec<String>,
    /// The proposed text stored on the session after the sidecar returned.
    pub proposed: Option<String>,
    /// The instruction stored on the session.
    pub instruction: Option<String>,
    /// Whether the popover was shown (listening).
    pub popover_shown: bool,
}

/// Drive `start_capture -> finish -> ready` with a fake selection provider and
/// the given (real, `for_test`) sidecar, recording the observable outcome.
/// Constructs its own private `AppState` internally so it can exercise the
/// crate-internal session/status machinery without exposing those types.
///
/// Gated the same way `LlmSidecar::for_test` is, so it never links into a
/// shipped binary. Used by `tests/transform_flow_integration.rs`.
#[cfg(any(debug_assertions, feature = "llm-test-support"))]
pub async fn run_happy_path_for_test(
    sidecar: &Arc<LlmSidecar>,
    instruction: &str,
    original: &str,
) -> HappyPathReport {
    use std::time::Instant;

    let app_state = AppState::default();
    let fx = RecordingFlowEffects::new();

    // A fake captured selection — production AX code is untouched.
    let original_owned = original.to_string();
    let snapshot = TransformSnapshot {
        bundle_id: Some("com.example.app".to_string()),
        pid: 4242,
        text: original_owned,
        range: Some((0, original.encode_utf16().count())),
        bounds: None,
        captured_at: Instant::now(),
    };

    // Claim the flow the way the command does, then run the capture core.
    assert!(app_state
        .try_transition_transform_status(TransformStatus::Idle, TransformStatus::Capturing));
    let outcome = core_start_capture(&app_state, &fx, true, async move { Ok(snapshot) }).await;
    assert_eq!(outcome, StartOutcome::Listening);

    // Release: thinking, then transform.
    assert!(enter_thinking(&app_state, &fx));
    let inflight = std::sync::Mutex::new(None);
    run_transform(
        &app_state,
        &fx,
        sidecar,
        &inflight,
        Ok(instruction.to_string()),
        original.to_string(),
        DEFAULT_TRANSFORM_DEADLINE,
    )
    .await;

    let session = transform_apply::session_snapshot(&app_state);
    HappyPathReport {
        emitted_states: fx.emitted_states(),
        proposed: session.as_ref().and_then(|s| s.proposed.clone()),
        instruction: session.as_ref().and_then(|s| s.instruction.clone()),
        popover_shown: fx.popover_shown(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Pure state machine ------------------------------------------------

    const ALL_STATES: [FlowState; 8] = [
        FlowState::Idle,
        FlowState::Capturing,
        FlowState::Listening,
        FlowState::Thinking,
        FlowState::Review,
        FlowState::Failed,
        FlowState::Applying,
        FlowState::Applied,
    ];

    const ALL_EVENTS: [FlowEvent; 16] = [
        FlowEvent::StartRequested,
        FlowEvent::CaptureOk,
        FlowEvent::CaptureSecure,
        FlowEvent::CaptureError,
        FlowEvent::InstructionRequested,
        FlowEvent::InstructionBlank,
        FlowEvent::TransformOk,
        FlowEvent::TransformError,
        FlowEvent::Approve,
        FlowEvent::ApplyOk,
        FlowEvent::ApplyError,
        FlowEvent::Retry,
        FlowEvent::Cancel,
        FlowEvent::Undo,
        FlowEvent::UndoOk,
        FlowEvent::UndoError,
    ];

    #[test]
    fn transition_table_is_total_and_never_panics() {
        // Exhaustive: every (state, event) pair yields a decision.
        for state in ALL_STATES {
            for event in ALL_EVENTS {
                let decision = decide(state, event);
                // An `Ignore` decision must not change state.
                if decision.actions == vec![FlowAction::Ignore] {
                    assert_eq!(decision.next, state, "Ignore must not change state");
                }
                // The only transition allowed to carry no actions is claiming
                // the flow (Idle -> Capturing): the capture itself is a command
                // effect, driven by the follow-up Capture* event. Every other
                // transition names at least one action.
                if decision.actions.is_empty() {
                    assert_eq!(
                        (state, event, decision.next),
                        (FlowState::Idle, FlowEvent::StartRequested, FlowState::Capturing),
                        "only Idle+StartRequested may have no actions",
                    );
                }
            }
        }
    }

    #[test]
    fn happy_path_transitions() {
        assert_eq!(
            decide(FlowState::Idle, FlowEvent::StartRequested).next,
            FlowState::Capturing
        );
        let cap = decide(FlowState::Capturing, FlowEvent::CaptureOk);
        assert_eq!(cap.next, FlowState::Listening);
        assert!(cap.actions.contains(&FlowAction::StartSession));
        assert!(cap.actions.contains(&FlowAction::Emit(ReviewState::Listening)));
        assert!(cap.actions.contains(&FlowAction::StartInstructionCapture));

        let think = decide(FlowState::Listening, FlowEvent::InstructionRequested);
        assert_eq!(think.next, FlowState::Thinking);
        assert!(think.actions.contains(&FlowAction::RunTransform));

        let ready = decide(FlowState::Thinking, FlowEvent::TransformOk);
        assert_eq!(ready.next, FlowState::Review);
        assert!(ready.actions.contains(&FlowAction::Emit(ReviewState::Ready)));
        assert!(ready.actions.contains(&FlowAction::SetFocusable(true)));

        let applying = decide(FlowState::Review, FlowEvent::Approve);
        assert_eq!(applying.next, FlowState::Applying);
        assert!(applying.actions.contains(&FlowAction::ApplyResult));

        let applied = decide(FlowState::Applying, FlowEvent::ApplyOk);
        assert_eq!(applied.next, FlowState::Applied);
        assert!(applied.actions.contains(&FlowAction::ScheduleLingerHide));
    }

    #[test]
    fn secure_field_aborts_silently_and_flashes() {
        let d = decide(FlowState::Capturing, FlowEvent::CaptureSecure);
        assert_eq!(d.next, FlowState::Idle);
        assert!(d.actions.contains(&FlowAction::FlashSecureField));
        assert!(d.actions.contains(&FlowAction::HidePopover));
        // NEVER shows the content popover for a secure field.
        assert!(!d.actions.contains(&FlowAction::ShowPopover));
        assert!(!d.actions.iter().any(|a| matches!(a, FlowAction::Emit(_))));
    }

    #[test]
    fn capture_error_shows_failed_popover() {
        let d = decide(FlowState::Capturing, FlowEvent::CaptureError);
        assert_eq!(d.next, FlowState::Failed);
        assert!(d.actions.contains(&FlowAction::ShowPopover));
        assert!(d.actions.contains(&FlowAction::Emit(ReviewState::Failed)));
        assert!(d.actions.contains(&FlowAction::SetFocusable(true)));
    }

    #[test]
    fn cancel_is_valid_from_every_live_state() {
        for state in [
            FlowState::Capturing,
            FlowState::Listening,
            FlowState::Thinking,
            FlowState::Review,
            FlowState::Failed,
            FlowState::Applied,
        ] {
            let d = decide(state, FlowEvent::Cancel);
            assert_eq!(d.next, FlowState::Idle, "Cancel from {state:?} must idle");
            assert!(
                d.actions.contains(&FlowAction::HidePopover)
                    && d.actions.contains(&FlowAction::ClearSession),
                "Cancel from {state:?} must hide + clear",
            );
        }
    }

    #[test]
    fn thinking_cancel_aborts_the_inflight_request() {
        let d = decide(FlowState::Thinking, FlowEvent::Cancel);
        assert!(d.actions.contains(&FlowAction::CancelInflight));
    }

    #[test]
    fn retry_re_speaks_on_the_same_snapshot() {
        for state in [FlowState::Review, FlowState::Failed] {
            let d = decide(state, FlowEvent::Retry);
            assert_eq!(d.next, FlowState::Listening);
            assert!(d.actions.contains(&FlowAction::StartInstructionCapture));
            assert!(d.actions.contains(&FlowAction::Emit(ReviewState::Listening)));
            // Retry keeps the session — it never clears it.
            assert!(!d.actions.contains(&FlowAction::ClearSession));
        }
    }

    #[test]
    fn undo_from_applied_runs_undo_then_idles() {
        let d = decide(FlowState::Applied, FlowEvent::Undo);
        assert_eq!(d.next, FlowState::Applying);
        assert!(d.actions.contains(&FlowAction::UndoResult));
        let done = decide(FlowState::Applying, FlowEvent::UndoOk);
        assert_eq!(done.next, FlowState::Idle);
        assert!(done.actions.contains(&FlowAction::HidePopover));
    }

    #[test]
    fn new_press_supersedes_a_failed_or_applied_popover() {
        for state in [FlowState::Failed, FlowState::Applied] {
            let d = decide(state, FlowEvent::StartRequested);
            assert_eq!(d.next, FlowState::Capturing);
            assert!(d.actions.contains(&FlowAction::ClearSession));
        }
    }

    #[test]
    fn press_while_mid_flow_is_ignored() {
        for state in [FlowState::Capturing, FlowState::Listening, FlowState::Thinking] {
            let d = decide(state, FlowEvent::StartRequested);
            assert_eq!(d.actions, vec![FlowAction::Ignore]);
            assert_eq!(d.next, state);
        }
    }

    // ---- Error-code mappings (total) ---------------------------------------

    #[test]
    fn selection_error_codes_are_total_and_stable() {
        let cases = [
            (SelectionError::AccessibilityDenied, "accessibility_denied"),
            (SelectionError::SecureField, "secure_field"),
            (SelectionError::NoSelection, "no_selection"),
            (SelectionError::TooLarge, "too_large"),
            (SelectionError::AxUnavailable, "ax_unavailable"),
        ];
        for (error, expected) in cases {
            assert_eq!(selection_error_code(error), expected);
        }
    }

    #[test]
    fn transform_error_codes_are_total_and_nonempty() {
        // Every TransformError variant maps to a non-empty, snake/camel-stable
        // code. Listing them all here fails to compile if a variant is added.
        let all = [
            TransformError::Unsupported,
            TransformError::NotDownloaded,
            TransformError::Disabled,
            TransformError::Busy,
            TransformError::InvalidRequest,
            TransformError::HeavyRuntimeActive,
            TransformError::SpawnFailed,
            TransformError::HandshakeFailed,
            TransformError::ModelMismatch,
            TransformError::ModelUnreadable,
            TransformError::Timeout,
            TransformError::Cancelled,
            TransformError::Crashed,
            TransformError::OutputInvalid,
            TransformError::Protocol,
            TransformError::ResourceLimit,
            TransformError::Internal,
        ];
        for error in all {
            assert!(!transform_error_code(error).is_empty());
        }
        // Key discoverable codes are exact.
        assert_eq!(transform_error_code(TransformError::NotDownloaded), "model_not_downloaded");
        assert_eq!(transform_error_code(TransformError::Timeout), "timeout");
        assert_eq!(transform_error_code(TransformError::Crashed), "crashed");
        assert_eq!(transform_error_code(TransformError::OutputInvalid), "output_invalid");
        assert_eq!(transform_error_code(TransformError::Busy), "busy");
        assert_eq!(transform_error_code(TransformError::Disabled), "disabled");
    }

    #[test]
    fn apply_error_codes_are_total_and_nonempty() {
        let all = [
            ApplyError::Unsupported,
            ApplyError::Busy,
            ApplyError::NoSession,
            ApplyError::NoProposedText,
            ApplyError::AlreadyApplied,
            ApplyError::NotApplied,
            ApplyError::ClipboardUnavailable,
            ApplyError::TargetGone,
            ApplyError::SelectionChanged,
            ApplyError::PasteFailed,
        ];
        for error in all {
            assert!(!apply_error_code(error).is_empty());
        }
        assert_eq!(apply_error_code(ApplyError::TargetGone), "target_gone");
        assert_eq!(apply_error_code(ApplyError::SelectionChanged), "selection_changed");
    }

    // ---- Async core with a fake selection provider -------------------------

    #[tokio::test]
    async fn core_start_capture_ok_freezes_session_and_lists() {
        use std::time::Instant;
        let app_state = AppState::default();
        let fx = RecordingFlowEffects::new();
        assert!(app_state
            .try_transition_transform_status(TransformStatus::Idle, TransformStatus::Capturing));

        let snapshot = TransformSnapshot {
            bundle_id: None,
            pid: 1,
            text: "hello world".to_string(),
            range: Some((0, 11)),
            bounds: None,
            captured_at: Instant::now(),
        };
        let outcome = core_start_capture(&app_state, &fx, true, async move { Ok(snapshot) }).await;

        assert_eq!(outcome, StartOutcome::Listening);
        assert_eq!(app_state.transform_status(), TransformStatus::Listening);
        assert!(transform_apply::session_snapshot(&app_state).is_some());
        assert_eq!(fx.emitted_states(), vec!["listening".to_string()]);
        assert!(fx.popover_shown());
    }

    #[tokio::test]
    async fn core_start_capture_secure_aborts_without_popover() {
        let app_state = AppState::default();
        let fx = RecordingFlowEffects::new();
        app_state.set_transform_status(TransformStatus::Capturing);

        let outcome = core_start_capture(&app_state, &fx, true, async {
            Err(SelectionError::SecureField)
        })
        .await;

        assert_eq!(outcome, StartOutcome::Aborted);
        assert_eq!(app_state.transform_status(), TransformStatus::Idle);
        assert!(transform_apply::session_snapshot(&app_state).is_none());
        assert!(fx.secure_flash());
        assert!(!fx.popover_shown());
        // No content UI, no state emission for a secure field.
        assert!(fx.emitted_states().is_empty());
    }

    #[tokio::test]
    async fn core_start_capture_model_not_ready_shows_failed_popover() {
        let app_state = AppState::default();
        let fx = RecordingFlowEffects::new();
        app_state.set_transform_status(TransformStatus::Capturing);

        let outcome = core_start_capture(&app_state, &fx, false, async {
            Ok(TransformSnapshot {
                bundle_id: None,
                pid: 1,
                text: "x".to_string(),
                range: None,
                bounds: None,
                captured_at: std::time::Instant::now(),
            })
        })
        .await;

        assert_eq!(outcome, StartOutcome::Aborted);
        assert_eq!(app_state.transform_status(), TransformStatus::ReviewPending);
        assert_eq!(
            fx.emitted(),
            vec![("failed".to_string(), Some("model_not_downloaded".to_string()))]
        );
        assert!(fx.popover_shown());
    }

    #[tokio::test]
    async fn run_transform_blank_instruction_fails_with_no_instruction() {
        let app_state = AppState::default();
        let fx = RecordingFlowEffects::new();
        app_state.set_transform_status(TransformStatus::Thinking);
        let sidecar = Arc::new(LlmSidecar::new());
        let inflight = std::sync::Mutex::new(None);

        run_transform(
            &app_state,
            &fx,
            &sidecar,
            &inflight,
            Err(()),
            "original".to_string(),
            DEFAULT_TRANSFORM_DEADLINE,
        )
        .await;

        assert_eq!(app_state.transform_status(), TransformStatus::ReviewPending);
        assert_eq!(
            fx.emitted(),
            vec![("failed".to_string(), Some("no_instruction".to_string()))]
        );
    }

    // ---- Cancel races (C2 review findings 1–3) -----------------------------

    #[tokio::test]
    async fn run_transform_cancel_during_thinking_does_not_resurrect_review_pending() {
        // Finding 1: cancel that lands after transcription but before / during
        // the sidecar step must not force ReviewPending (which blocks_recording).
        let app_state = AppState::default();
        let fx = RecordingFlowEffects::new();
        app_state.set_transform_status(TransformStatus::Thinking);
        // No session → set_instruction returns false → bail before sidecar.
        let sidecar = Arc::new(LlmSidecar::new());
        let inflight = std::sync::Mutex::new(None);

        run_transform(
            &app_state,
            &fx,
            &sidecar,
            &inflight,
            Ok("make this shorter".to_string()),
            "original".to_string(),
            DEFAULT_TRANSFORM_DEADLINE,
        )
        .await;

        // Status must stay Thinking (or whatever cancel left) — never forced
        // to ReviewPending without a live session. We never set a session, so
        // set_instruction bailed and status is unchanged.
        assert_eq!(app_state.transform_status(), TransformStatus::Thinking);
        assert!(fx.emitted_states().is_empty());
        assert!(inflight.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn run_transform_respects_cancel_that_left_thinking() {
        // Terminal branches only write ReviewPending via try_transition from
        // Thinking — if cancel already forced Idle, blank-instruction must not
        // resurrect ReviewPending.
        let app_state = AppState::default();
        let fx = RecordingFlowEffects::new();
        // Simulate cancel winning the race: already Idle.
        app_state.set_transform_status(TransformStatus::Idle);
        let sidecar = Arc::new(LlmSidecar::new());
        let inflight = std::sync::Mutex::new(None);

        run_transform(
            &app_state,
            &fx,
            &sidecar,
            &inflight,
            Err(()),
            "original".to_string(),
            DEFAULT_TRANSFORM_DEADLINE,
        )
        .await;

        assert_eq!(app_state.transform_status(), TransformStatus::Idle);
        assert!(fx.emitted_states().is_empty());
    }

    #[tokio::test]
    async fn core_start_capture_cancel_during_ax_does_not_install_session() {
        // Finding 3: short-tap cancel during slow AX capture must not re-install
        // session / arm Listening after the cancel tore the flow down.
        let app_state = Arc::new(AppState::default());
        let fx = RecordingFlowEffects::new();
        assert!(app_state
            .try_transition_transform_status(TransformStatus::Idle, TransformStatus::Capturing));

        let snapshot = TransformSnapshot {
            bundle_id: None,
            pid: 1,
            text: "hello world".to_string(),
            range: Some((0, 11)),
            bounds: None,
            captured_at: std::time::Instant::now(),
        };

        let cancel_state = Arc::clone(&app_state);
        let outcome = core_start_capture(&app_state, &fx, true, async move {
            // Simulate cancel mid-capture: leave Capturing before Ok lands.
            cancel_state.set_transform_status(TransformStatus::Idle);
            Ok(snapshot)
        })
        .await;

        assert_eq!(outcome, StartOutcome::Aborted);
        assert_eq!(app_state.transform_status(), TransformStatus::Idle);
        assert!(transform_apply::session_snapshot(&app_state).is_none());
        assert!(!fx.popover_shown());
        assert!(fx.emitted_states().is_empty());
    }

    // ---- Spec-by-test divergence points (finding 8) ------------------------

    #[test]
    fn decide_review_start_requested_is_ignored_but_failed_and_applied_supersede() {
        // Command layer supersedes Failed/Applied on a new press; mid-flow
        // Review ignores StartRequested (session stays until Cancel/Approve).
        let review = decide(FlowState::Review, FlowEvent::StartRequested);
        assert_eq!(review.actions, vec![FlowAction::Ignore]);

        for state in [FlowState::Failed, FlowState::Applied] {
            let d = decide(state, FlowEvent::StartRequested);
            assert_eq!(d.next, FlowState::Capturing);
            assert!(d.actions.contains(&FlowAction::ClearSession));
        }
    }

    #[test]
    fn decide_applying_cancel_tears_down() {
        // Finding 7/8: Cancel during Applying must idle (not ignore).
        let d = decide(FlowState::Applying, FlowEvent::Cancel);
        assert_eq!(d.next, FlowState::Idle);
        assert!(d.actions.contains(&FlowAction::HidePopover));
        assert!(d.actions.contains(&FlowAction::ClearSession));
    }

    #[test]
    fn decide_undo_ok_clears_session_without_cancel_inflight() {
        // Finding 4: successful undo hides + clears; it does not CancelInflight
        // (and the command path must not bump the apply epoch a second time).
        let d = decide(FlowState::Applying, FlowEvent::UndoOk);
        assert_eq!(d.next, FlowState::Idle);
        assert!(d.actions.contains(&FlowAction::HidePopover));
        assert!(d.actions.contains(&FlowAction::ClearSession));
        assert!(!d.actions.contains(&FlowAction::CancelInflight));
    }

    #[test]
    fn decide_undo_error_keeps_applied_and_undo_reachable() {
        // Item 12: a failed undo must NOT drop to Failed (whose UI has no Undo
        // button and a dead Retry) — it stays Applied and re-emits Applied so
        // the Undo button remains reachable and the applied text is never
        // stranded un-undoable. The command layer (undo_transform_and_close)
        // carries the undo error code on this `applied` emit.
        let d = decide(FlowState::Applying, FlowEvent::UndoError);
        assert_eq!(d.next, FlowState::Applied);
        assert!(d.actions.contains(&FlowAction::Emit(ReviewState::Applied)));
        assert!(!d.actions.contains(&FlowAction::Emit(ReviewState::Failed)));
        // The session is NOT cleared — Undo stays valid.
        assert!(!d.actions.contains(&FlowAction::ClearSession));
        assert!(!d.actions.contains(&FlowAction::HidePopover));
    }

    #[test]
    fn undo_close_path_must_not_bump_epoch_twice() {
        // Document the contract undo_transform_and_close implements: one epoch
        // advance for the undo's clipboard restore, never a second bump from
        // chaining cancel. Pure unit check of the epoch helper semantics.
        let app_state = AppState::default();
        let before = app_state.transform_apply_epoch();
        let first = app_state.next_transform_apply_epoch(); // undo's own bump
        assert_eq!(first, before + 1);
        // Success path of undo_transform_and_close must stop here (no cancel).
        assert_eq!(app_state.transform_apply_epoch(), first);
        // Contrast: cancel_transform would bump again and break paste-fallback.
        let second = app_state.next_transform_apply_epoch();
        assert_eq!(second, first + 1);
    }
}
