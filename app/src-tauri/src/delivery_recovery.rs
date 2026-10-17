use crate::frontmost::DeliveryTargetSnapshot;
use crate::injector::authority::{AttemptStop, CancelOnDrop, PasteAttempt, RetrySlot};
use crate::injector::{self, ClipboardOnlyReason, InjectionOutcome};
use crate::{MutexExt, State};
use serde::Serialize;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager};

#[derive(Clone)]
pub(crate) struct LastDelivery {
    pub(crate) recording_id: u64,
    pub(crate) text: String,
    pub(crate) target: DeliveryTargetSnapshot,
    smart_destination: crate::smart_destination::Snapshot,
    delay_ms: u64,
    pub(crate) teaching_context: Option<crate::correct_and_teach::TeachingContext>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecoveryRoute {
    NormalPaste,
    VerifiedSmartDestination,
}

fn recovery_route(snapshot: &crate::smart_destination::Snapshot) -> RecoveryRoute {
    if snapshot.uses_normal_paste() {
        RecoveryRoute::NormalPaste
    } else {
        RecoveryRoute::VerifiedSmartDestination
    }
}

fn retry_target_matches(
    expected: &DeliveryTargetSnapshot,
    current: &DeliveryTargetSnapshot,
) -> bool {
    match (expected, current) {
        (DeliveryTargetSnapshot::Complete(expected), DeliveryTargetSnapshot::Complete(current)) => {
            expected.bundle_id == current.bundle_id
                && expected.process_id == current.process_id
                && expected.process_instance == current.process_instance
        }
        _ => false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreparationFailure {
    TimedOut,
    ReplyLost,
}

impl PreparationFailure {
    fn copied_message(self) -> &'static str {
        match self {
            Self::TimedOut => "Destination verification timed out. Last delivery copied to the clipboard; paste manually.",
            Self::ReplyLost => "Destination verification did not finish. Last delivery copied to the clipboard; paste manually.",
        }
    }
}

fn preparation_fallback(
    snapshot: &crate::smart_destination::Snapshot,
    reply: Option<crate::smart_destination::PrepareOutcome>,
) -> crate::smart_destination::Snapshot {
    if reply == Some(crate::smart_destination::PrepareOutcome::Refused)
        && snapshot.allows_same_app_fallback()
    {
        crate::smart_destination::Snapshot::Unverified
    } else {
        crate::smart_destination::Snapshot::Unavailable
    }
}

fn stopped_retry(reason: AttemptStop) -> RetryResult {
    result(RetryResultKind::Failed, match reason {
        AttemptStop::Expired => "Paste Last expired before completion. Check the destination and clipboard before trying again.",
        AttemptStop::Cancelled => "Paste Last was cancelled. Check the destination and clipboard before trying again.",
        AttemptStop::AlreadyCommitted => "Paste Last already committed a shortcut. Check the destination before trying again.",
        AttemptStop::OperationInFlight => "Paste Last still has a native operation in flight. Check the destination before trying again.",
    })
}

#[derive(Default)]
pub(crate) struct DeliveryRecoveryState {
    latest: Mutex<Option<LastDelivery>>,
    retry: RetrySlot,
}

impl DeliveryRecoveryState {
    pub(crate) fn remember(
        &self,
        recording_id: u64,
        text: String,
        target: DeliveryTargetSnapshot,
        smart_destination: crate::smart_destination::Snapshot,
        delay_ms: u64,
        teaching_context: Option<crate::correct_and_teach::TeachingContext>,
    ) {
        if text.trim().is_empty() {
            return;
        }
        *self.latest.lock_or_recover() = Some(LastDelivery {
            recording_id,
            text,
            target,
            smart_destination,
            delay_ms: delay_ms.min(500),
            teaching_context,
        });
    }

    pub(crate) fn correct(&self, recording_id: u64, original: &str, corrected: &str) {
        if let Some(latest) = self.latest.lock_or_recover().as_mut() {
            if latest.recording_id == recording_id && latest.text == original {
                latest.text = corrected.to_string();
            }
        }
    }

    pub(crate) fn latest(&self) -> Option<LastDelivery> {
        self.latest.lock_or_recover().clone()
    }
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum RetryResultKind {
    AutoPasted,
    ClipboardOnly,
    Empty,
    Busy,
    Failed,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RetryResult {
    kind: RetryResultKind,
    message: &'static str,
}

fn result(kind: RetryResultKind, message: &'static str) -> RetryResult {
    RetryResult { kind, message }
}

fn publish(app: &tauri::AppHandle, response: &RetryResult) {
    let _ = app.emit("delivery-retry-feedback", response);
}

#[tauri::command]
pub(crate) async fn retry_last_delivery(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, State>,
) -> Result<RetryResult, String> {
    let Some(mut latest) = state.delivery_recovery.latest() else {
        let response = result(RetryResultKind::Empty, "Nothing to paste yet.");
        publish(&app_handle, &response);
        return Ok(response);
    };
    let Some(work) = state.delivery_recovery.retry.begin(latest.recording_id) else {
        let response = result(RetryResultKind::Busy,
            "Paste Last is running or a previous paste is unconfirmed. Check the destination before a new delivery.");
        publish(&app_handle, &response);
        return Ok(response);
    };
    let _cancel = CancelOnDrop(work.attempt());

    let retry_generation = {
        let dictation = state.app_state.dictation.lock_or_recover();
        if dictation.status != crate::state::DictationStatus::Idle {
            let response = result(
                RetryResultKind::Busy,
                "Finish the current recording before pasting again.",
            );
            publish(&app_handle, &response);
            return Ok(response);
        }
        state.app_state.recording_id.load(Ordering::SeqCst)
    };
    if recovery_route(&latest.smart_destination) == RecoveryRoute::VerifiedSmartDestination {
        // An explicit retry is a new one-shot operation, never a replay of a
        // consumed native capability. Require the user to focus the original
        // destination app and prove its currently editable field afresh.
        let (sender, receiver) = tokio::sync::oneshot::channel();
        app_handle
            .run_on_main_thread(move || {
                let _ = sender.send(crate::frontmost::capture_delivery_target_snapshot());
            })
            .map_err(|error| error.to_string())?;
        let current =
            match tokio::time::timeout(std::time::Duration::from_millis(250), receiver).await {
                Ok(Ok(current)) => current,
                _ => DeliveryTargetSnapshot::Incomplete,
            };
        latest.smart_destination = if retry_target_matches(&latest.target, &current) {
            match crate::smart_destination::capture(&app_handle, &current, true).await {
                snapshot @ (crate::smart_destination::Snapshot::Target { .. }
                | crate::smart_destination::Snapshot::Unverified) => snapshot,
                _ => crate::smart_destination::Snapshot::Unavailable,
            }
        } else {
            crate::smart_destination::Snapshot::Unavailable
        };
    }
    if latest.delay_ms > 0 && injector::is_accessibility_enabled() {
        tokio::time::sleep(std::time::Duration::from_millis(latest.delay_ms)).await;
    }
    let _smart_cleanup = crate::smart_destination::AbortOnDrop::new(
        app_handle.clone(),
        latest.smart_destination.clone(),
    );
    let mut preparation_failure = None;
    if matches!(
        latest.smart_destination,
        crate::smart_destination::Snapshot::Target { .. }
    ) {
        let preparing = latest.smart_destination.clone();
        let prepare_app = app_handle.clone();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let worker = work.clone();
        // Preparation expires independently. The parent retains clipboard-first
        // authority; prepare_focused never activates or posts a shortcut.
        let preparation_authority = Arc::new(PasteAttempt::new(
            std::time::Duration::from_secs(1),
            latest.recording_id,
        ));
        let _cancel_preparation = CancelOnDrop(&preparation_authority);
        let preparation_worker = preparation_authority.clone();
        app_handle
            .run_on_main_thread(move || {
                let _worker = worker;
                if sender.is_closed() || !preparation_worker.is_current() {
                    return;
                }
                let state = prepare_app.state::<State>();
                let dictation = state.app_state.dictation.lock_or_recover();
                let outcome = if dictation.status == crate::state::DictationStatus::Idle
                    && state.app_state.recording_id.load(Ordering::SeqCst) == retry_generation
                {
                    crate::smart_destination::prepare_focused(&preparing, Some(&preparation_worker))
                } else {
                    crate::smart_destination::PrepareOutcome::Refused
                };
                let _ = sender.send(outcome);
            })
            .map_err(|error| error.to_string())?;
        let reply = match tokio::time::timeout(std::time::Duration::from_secs(1), receiver).await {
            Ok(Ok(reply)) => Some(reply),
            Err(_) => {
                preparation_failure = Some(PreparationFailure::TimedOut);
                None
            }
            Ok(Err(_)) => {
                preparation_failure = Some(PreparationFailure::ReplyLost);
                None
            }
        };
        preparation_authority.cancel();
        if let Some(failure) = preparation_failure {
            tracing::info!(target: "pipeline", event_code = "pipeline.paste_safety", phase_code = "preparation",
                route_code = latest.smart_destination.route_code(), outcome_code = match failure {
                    PreparationFailure::TimedOut => "timed_out", PreparationFailure::ReplyLost => "reply_lost",
                }, "Paste safety decision");
        }
        if reply != Some(crate::smart_destination::PrepareOutcome::Ready) {
            let abandoned = latest.smart_destination.clone();
            let worker = work.clone();
            let _ = app_handle.run_on_main_thread(move || {
                let _worker = worker;
                crate::smart_destination::abort(&abandoned, None);
            });
            latest.smart_destination = preparation_fallback(&latest.smart_destination, reply);
        }
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    let retry_app = app_handle.clone();
    let worker = work.clone();
    app_handle
        .run_on_main_thread(move || {
            let state = retry_app.state::<State>();
            let dictation = state.app_state.dictation.lock_or_recover();
            if dictation.status != crate::state::DictationStatus::Idle
                || state.app_state.recording_id.load(Ordering::SeqCst) != retry_generation
            {
                crate::smart_destination::abort(&latest.smart_destination, None);
                let _ = tx.send(None);
                return;
            }
            // Routes on the snapshot itself: Disabled uses legacy normal paste,
            // Unverified uses normal paste with the secure-field veto, and a
            // fresh smart Target uses its one-shot capability.
            let attempt = injector::inject_with_destination_authorized(
                &latest.text,
                true,
                latest.delay_ms,
                &latest.target,
                &latest.smart_destination,
                Some(worker.attempt()),
            );
            crate::smart_destination::abort(&latest.smart_destination, None);
            let _ = tx.send(Some(attempt));
        })
        .map_err(|error| error.to_string())?;

    let received = tokio::time::timeout(std::time::Duration::from_secs(2), rx).await;
    match &received {
        Ok(Ok(_)) => work.attempt().acknowledge(),
        _ => work.attempt().cancel(),
    }
    let response = match received {
        Ok(Ok(Some(Ok(injection)))) => match injection.outcome {
            InjectionOutcome::AutoPasted => result(
                RetryResultKind::AutoPasted,
                "Paste shortcut sent. Check your destination.",
            ),
            InjectionOutcome::ClipboardOnly(reason) => {
                let message = match reason {
                    ClipboardOnlyReason::SmartTargetUnavailable => {
                        preparation_failure.map(PreparationFailure::copied_message).unwrap_or(
                            "The original text field is no longer verified. Click your destination and press Cmd+V.")
                    }
                    ClipboardOnlyReason::TargetChanged => {
                        "The original app is no longer focused. Text was copied to the clipboard."
                    }
                    ClipboardOnlyReason::ClipboardChanged => {
                        "The clipboard changed before retry. Run Paste Last again."
                    }
                    ClipboardOnlyReason::FocusNotEditable => {
                        "No editable field is focused. Text was copied to the clipboard."
                    }
                    ClipboardOnlyReason::PasteFailed => {
                        "Automatic paste failed. Text was copied to the clipboard."
                    }
                    ClipboardOnlyReason::AccessibilityDenied => {
                        "Accessibility access is unavailable. Text was copied to the clipboard."
                    }
                    ClipboardOnlyReason::AutoPasteDisabled => {
                        "Last delivery copied to the clipboard."
                    }
                };
                result(RetryResultKind::ClipboardOnly, message)
            }
            InjectionOutcome::NoText => result(RetryResultKind::Empty, "Nothing to paste yet."),
            InjectionOutcome::Stopped(reason) => stopped_retry(reason),
        },
        Ok(Ok(None)) => result(
            RetryResultKind::Busy,
            "Recording changed. Run Paste Last after it finishes.",
        ),
        _ => result(
            RetryResultKind::Failed,
            "Paste Last is unconfirmed. Check the destination and clipboard before trying again.",
        ),
    };
    tracing::info!(
        target: "pipeline",
        event_code = "pipeline.delivery_retry_terminal",
        outcome = match response.kind {
            RetryResultKind::AutoPasted => "auto_pasted",
            RetryResultKind::ClipboardOnly => "clipboard_only",
            RetryResultKind::Empty => "empty",
            RetryResultKind::Busy => "busy",
            RetryResultKind::Failed => "failed",
        },
        "delivery retry completed"
    );
    publish(&app_handle, &response);
    Ok(response)
}

pub(crate) fn spawn_retry(app_handle: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let state = app_handle.state::<State>();
        let _ = retry_last_delivery(app_handle.clone(), state).await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preparation_timeout_revokes_only_the_child_and_keeps_safe_clipboard_fallback() {
        let slot = RetrySlot::default();
        let parent = slot.begin(1).unwrap();
        let child = PasteAttempt::new(std::time::Duration::ZERO, 1);
        child.cancel();
        let snapshot = crate::smart_destination::Snapshot::Target {
            token: 1,
            revision: 1,
            identity: DeliveryTargetSnapshot::Incomplete,
            current_app: true,
        };
        let fallback = preparation_fallback(&snapshot, None);
        assert!(matches!(
            fallback,
            crate::smart_destination::Snapshot::Unavailable
        ));
        assert!(
            !fallback.uses_normal_paste(),
            "timeout cannot authorize a shortcut fallback"
        );
        let wrote = std::cell::Cell::new(false);
        assert_eq!(parent.attempt().mutate(|| wrote.set(true)), Ok(()));
        assert!(
            wrote.get(),
            "unexpired parent still owns clipboard-first fallback"
        );
        assert!(PreparationFailure::TimedOut
            .copied_message()
            .contains("timed out"));
        assert!(PreparationFailure::ReplyLost
            .copied_message()
            .contains("did not finish"));
        assert!(!PreparationFailure::TimedOut
            .copied_message()
            .contains("Recording changed"));
    }

    #[test]
    fn stopped_retry_reports_failure_reason_instead_of_recording_changed_busy() {
        for reason in [AttemptStop::Cancelled, AttemptStop::Expired] {
            let response = stopped_retry(reason);
            assert!(matches!(response.kind, RetryResultKind::Failed));
            assert!(!response.message.contains("Recording changed"));
        }
    }

    #[test]
    fn empty_and_whitespace_never_replace_the_latest_delivery() {
        let state = DeliveryRecoveryState::default();
        assert!(state.latest().is_none());
        state.remember(
            1,
            "hello".into(),
            DeliveryTargetSnapshot::Incomplete,
            crate::smart_destination::Snapshot::Disabled,
            900,
            None,
        );
        state.remember(
            2,
            "   ".into(),
            DeliveryTargetSnapshot::SelfTarget,
            crate::smart_destination::Snapshot::Unavailable,
            0,
            None,
        );
        let latest = state.latest().expect("delivery retained");
        assert_eq!(latest.text, "hello");
        assert_eq!(latest.delay_ms, 500);
        assert!(matches!(latest.target, DeliveryTargetSnapshot::Incomplete));
    }
    #[test]
    fn corrections_build_on_approved_text_and_cannot_replace_a_newer_delivery() {
        let state = DeliveryRecoveryState::default();
        state.remember(
            41,
            "Tori Friday".into(),
            DeliveryTargetSnapshot::Incomplete,
            crate::smart_destination::Snapshot::Disabled,
            0,
            None,
        );
        state.correct(41, "Tori Friday", "TAURI Friday");
        assert_eq!(state.latest().unwrap().text, "TAURI Friday");
        state.correct(41, "Tori Friday", "stale");
        assert_eq!(state.latest().unwrap().text, "TAURI Friday");
        state.remember(
            42,
            "Tori Friday".into(),
            DeliveryTargetSnapshot::Incomplete,
            crate::smart_destination::Snapshot::Disabled,
            0,
            None,
        );
        state.correct(41, "Tori Friday", "wrong recording");
        assert_eq!(state.latest().unwrap().text, "Tori Friday");
    }

    #[test]
    fn explicit_retry_requires_the_original_process_instance() {
        use crate::frontmost::{
            AppTransitionSnapshot, DeliveryTargetIdentity, ProcessInstanceToken,
        };
        let target = DeliveryTargetSnapshot::Complete(DeliveryTargetIdentity {
            bundle_id: "com.example.Editor".into(),
            process_id: 42,
            process_instance: ProcessInstanceToken::for_test(7),
            window_token: None,
            transitions: AppTransitionSnapshot {
                activation_generation: 0,
                space_generation: 0,
            },
        });
        assert!(retry_target_matches(&target, &target));
        for current in [
            DeliveryTargetSnapshot::SelfTarget,
            DeliveryTargetSnapshot::Incomplete,
        ] {
            assert!(!retry_target_matches(&target, &current));
        }
        let DeliveryTargetSnapshot::Complete(mut changed) = target.clone() else {
            unreachable!()
        };
        changed.process_instance = ProcessInstanceToken::for_test(8);
        assert!(!retry_target_matches(
            &target,
            &DeliveryTargetSnapshot::Complete(changed.clone())
        ));
        changed.process_instance = ProcessInstanceToken::for_test(7);
        changed.process_id = 43;
        assert!(!retry_target_matches(
            &target,
            &DeliveryTargetSnapshot::Complete(changed.clone())
        ));
        changed.process_id = 42;
        changed.bundle_id = "com.example.Other".into();
        assert!(!retry_target_matches(
            &target,
            &DeliveryTargetSnapshot::Complete(changed)
        ));
    }

    #[test]
    fn smart_recovery_never_downgrades_to_normal_app_paste() {
        use crate::frontmost::{
            AppTransitionSnapshot, DeliveryTargetIdentity, ProcessInstanceToken,
        };

        let target = DeliveryTargetSnapshot::Complete(DeliveryTargetIdentity {
            bundle_id: "com.example.Editor".into(),
            process_id: 42,
            process_instance: ProcessInstanceToken::for_test(7),
            window_token: None,
            transitions: AppTransitionSnapshot {
                activation_generation: 0,
                space_generation: 0,
            },
        });
        let smart = crate::smart_destination::Snapshot::Target {
            token: 9,
            revision: 3,
            identity: target.clone(),
            current_app: true,
        };
        let state = DeliveryRecoveryState::default();
        state.remember(7, "original".into(), target, smart, 0, None);
        state.correct(7, "original", "corrected");

        let latest = state.latest().expect("smart recovery retained");
        assert_eq!(
            recovery_route(&latest.smart_destination),
            RecoveryRoute::VerifiedSmartDestination,
            "a consumed or unconfirmed smart token must fail closed instead of posting Cmd+V",
        );
        assert_eq!(latest.text, "corrected");
        assert_eq!(
            recovery_route(&crate::smart_destination::Snapshot::Unavailable),
            RecoveryRoute::VerifiedSmartDestination,
        );
        assert_eq!(
            recovery_route(&crate::smart_destination::Snapshot::Disabled),
            RecoveryRoute::NormalPaste,
        );
        assert_eq!(
            recovery_route(&crate::smart_destination::Snapshot::Unverified),
            RecoveryRoute::NormalPaste,
            "a recording that never proved a field uses the normal retry policy",
        );
    }
}
