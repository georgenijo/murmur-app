use crate::frontmost::DeliveryTargetSnapshot;
use crate::injector::{self, ClipboardOnlyReason, InjectionOutcome};
use crate::{MutexExt, State};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{Emitter, Manager};

#[derive(Clone)]
pub(crate) struct LastDelivery {
    text: String,
    target: DeliveryTargetSnapshot,
    delay_ms: u64,
}

#[derive(Default)]
pub(crate) struct DeliveryRecoveryState {
    latest: Mutex<Option<LastDelivery>>,
    retry_active: AtomicBool,
}

impl DeliveryRecoveryState {
    pub(crate) fn remember(&self, text: String, target: DeliveryTargetSnapshot, delay_ms: u64) {
        if text.trim().is_empty() {
            return;
        }
        *self.latest.lock_or_recover() = Some(LastDelivery {
            text,
            target,
            delay_ms: delay_ms.min(500),
        });
    }

    fn latest(&self) -> Option<LastDelivery> {
        self.latest.lock_or_recover().clone()
    }
}

struct RetryGuard<'a>(&'a AtomicBool);

impl Drop for RetryGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
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
    let Some(latest) = state.delivery_recovery.latest() else {
        let response = result(RetryResultKind::Empty, "Nothing to paste yet.");
        publish(&app_handle, &response);
        return Ok(response);
    };
    if state
        .delivery_recovery
        .retry_active
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        let response = result(RetryResultKind::Busy, "Paste Last is already running.");
        publish(&app_handle, &response);
        return Ok(response);
    }
    let _guard = RetryGuard(&state.delivery_recovery.retry_active);

    if latest.delay_ms > 0 && injector::is_accessibility_enabled() {
        tokio::time::sleep(std::time::Duration::from_millis(latest.delay_ms)).await;
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    app_handle
        .run_on_main_thread(move || {
            let attempt =
                injector::inject_text(&latest.text, true, latest.delay_ms, &latest.target);
            let _ = tx.send(attempt);
        })
        .map_err(|error| error.to_string())?;

    let response = match tokio::time::timeout(std::time::Duration::from_secs(2), rx).await {
        Ok(Ok(Ok(injection))) => match injection.outcome {
            InjectionOutcome::AutoPasted => result(
                RetryResultKind::AutoPasted,
                "Last delivery pasted securely.",
            ),
            InjectionOutcome::ClipboardOnly(reason) => {
                let message = match reason {
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
        },
        _ => result(
            RetryResultKind::Failed,
            "Paste Last did not finish. Your existing clipboard was left alone.",
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
    fn empty_and_whitespace_never_replace_the_latest_delivery() {
        let state = DeliveryRecoveryState::default();
        assert!(state.latest().is_none());
        state.remember("hello".into(), DeliveryTargetSnapshot::Incomplete, 900);
        state.remember("   ".into(), DeliveryTargetSnapshot::SelfTarget, 0);
        let latest = state.latest().expect("delivery retained");
        assert_eq!(latest.text, "hello");
        assert_eq!(latest.delay_ms, 500);
        assert!(matches!(latest.target, DeliveryTargetSnapshot::Incomplete));
    }
}
