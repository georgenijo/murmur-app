//! Native window transport for the live dictation preview popover (#611).
//!
//! The opt-in preview is a small, non-activating, click-through live-words pill
//! attached below the actual overlay frame while recording. It shows the
//! words recognized so far. It is presentation only. It never owns transcript
//! state or takes focus. The hidden webview renders recognized words first,
//! then asks Rust to show the native window for the exact recording. Rust hides
//! the window when that recording stops being current.
//!
//! Provisional content never enters the island or the final-delivery path.
//! The existing partial API provides no authoritative stable-word boundary.

use crate::commands::native_window::PopoverSpec;
use crate::state::DictationStatus;
use crate::{MutexExt, State};
use std::sync::atomic::Ordering;
use tauri::Manager;

const WIDTH: f64 = 460.0;
const HEIGHT: f64 = 104.0;

const POPOVER: PopoverSpec = PopoverSpec {
    label: "dictation-preview",
    // Purely informational while the user is dictating into another app:
    // never swallow a click meant for whatever is underneath.
    ignore_cursor_events: true,
};

/// Logical frame for the preview window: horizontally centered on the
/// menu-bar display, tucked directly under the notch (or menu bar on displays
/// without one).
///
/// The anchor is deliberately the primary monitor, matching the overlay's
/// selected display. Both placements follow the actual applied overlay frame:
/// the preview sits below the notch island or above the bottom pill.
fn frame(app: &tauri::AppHandle, notch_h: Option<f64>) -> (f64, f64, f64, f64) {
    let top_inset = top_inset_for(notch_h);
    let monitor = app.primary_monitor().ok().flatten();
    match monitor {
        Some(monitor) => {
            let scale = monitor.scale_factor();
            let width = WIDTH.min(monitor.work_area().size.width as f64 / scale);
            let x = monitor.position().x as f64 / scale
                + (monitor.size().width as f64 / scale - width) / 2.0;
            let base_y = monitor.position().y as f64 / scale + top_inset;
            let (x, y, height) =
                super::overlay::dictation_preview_fit(app, &monitor, width, HEIGHT)
                    .unwrap_or((x, base_y, HEIGHT));
            (x, y, width, height)
        }
        None => (300.0, top_inset, WIDTH, HEIGHT),
    }
}

/// Distance from the top of the monitor to the top of the preview card. An
/// unmeasured, zero, or non-finite notch height means the display measurement
/// has not landed yet — fall back rather than render the card over the menu bar.
fn top_inset_for(notch_h: Option<f64>) -> f64 {
    super::overlay::preview_fallback_inset(notch_h)
}

fn notch_height(app: &tauri::AppHandle) -> Option<f64> {
    app.try_state::<crate::State>()
        .and_then(|state| state.notch_info.lock_or_recover().map(|info| info.height()))
}

/// Show the preview under the notch. Idempotent: repositioning a window that is
/// already visible is how a display change is absorbed.
pub(crate) fn show_internal(app: &tauri::AppHandle) -> Result<(), String> {
    POPOVER.show(app, frame(app, notch_height(app)))
}

/// Keep an already-visible preview attached to the overlay as its native frame
/// changes. This never shows a hidden window, so recording generation remains
/// owned by `show_dictation_preview`/the Rust ticker.
pub(crate) fn follow_overlay_if_visible(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window(POPOVER.label) else {
        return;
    };
    if !window.is_visible().unwrap_or(false) {
        return;
    }
    if let Err(error) = POPOVER.reposition(app, frame(app, notch_height(app))) {
        tracing::warn!(target: "system", "dictation preview could not follow overlay: {}", error);
    }
}

#[derive(Clone, Copy)]
enum PresentationOutcome {
    Shown,
    ShowFailed,
    Hidden,
    HideFailed,
}

impl PresentationOutcome {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Shown => "shown",
            Self::ShowFailed => "show_failed",
            Self::Hidden => "hidden",
            Self::HideFailed => "hide_failed",
        }
    }
}

fn emit_presentation(recording_id: u64, outcome: PresentationOutcome) {
    tracing::info!(
        target: "pipeline",
        event_code = "pipeline.dictation_preview_presentation",
        recording_id,
        outcome = outcome.as_str(),
        "dictation preview presentation changed"
    );
}

/// Show only after the preview webview has committed non-empty text for the
/// exact recording. Rust remains the native visibility and ownership boundary.
fn can_show(app_state: &crate::state::AppState, recording_id: u64) -> bool {
    recording_id > 0
        && app_state.recording_id.load(Ordering::SeqCst) == recording_id
        && app_state.dictation.lock_or_recover().status == DictationStatus::Recording
        && !app_state.is_cancelled(recording_id)
}

fn can_hide(app_state: &crate::state::AppState, recording_id: u64) -> bool {
    app_state.recording_id.load(Ordering::SeqCst) == recording_id
}

#[tauri::command]
pub async fn show_dictation_preview(
    window: tauri::WebviewWindow,
    recording_id: u64,
) -> Result<(), String> {
    if window.label() != "dictation-preview" {
        return Err("dictation preview show is restricted to its own window".to_string());
    }
    let app = window.app_handle().clone();
    let dispatch_app = app.clone();
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        // Validate at presentation time, not before a queued native mutation.
        let state = dispatch_app.state::<State>();
        let result = if can_show(&state.app_state, recording_id) {
            show_internal(&dispatch_app)
        } else {
            Err("dictation preview recording is no longer current".to_string())
        };
        emit_presentation(
            recording_id,
            if result.is_ok() {
                PresentationOutcome::Shown
            } else {
                PresentationOutcome::ShowFailed
            },
        );
        let _ = tx.send(result);
    })
    .map_err(|_| "dictation preview presentation could not be scheduled".to_string())?;
    rx.await
        .map_err(|_| "dictation preview presentation was cancelled".to_string())?
}

pub(crate) fn hide_for_recording(app: &tauri::AppHandle, recording_id: u64) {
    let dispatch_app = app.clone();
    let _ = app.run_on_main_thread(move || {
        // A late ticker must not hide its successor, including after this
        // operation spent time queued on the native main thread.
        if !can_hide(&dispatch_app.state::<State>().app_state, recording_id) {
            return;
        }
        let result = POPOVER.hide(&dispatch_app);
        emit_presentation(
            recording_id,
            if result.is_ok() {
                PresentationOutcome::Hidden
            } else {
                PresentationOutcome::HideFailed
            },
        );
    });
}

pub(crate) fn apply_initial_size(app: &tauri::AppHandle) {
    POPOVER.apply_initial_size(app, WIDTH, HEIGHT);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queued_presentation_rechecks_stop_and_restart_ownership() {
        let state = crate::state::AppState::default();
        state.recording_id.store(7, Ordering::SeqCst);
        state.dictation.lock_or_recover().status = DictationStatus::Recording;
        assert!(can_show(&state, 7));
        state.dictation.lock_or_recover().status = DictationStatus::Processing;
        assert!(!can_show(&state, 7));
        assert!(can_hide(&state, 7));
        state.recording_id.store(8, Ordering::SeqCst);
        state.dictation.lock_or_recover().status = DictationStatus::Recording;
        assert!(!can_show(&state, 7));
        assert!(!can_hide(&state, 7));
        assert!(can_show(&state, 8));
        state.cancel_recording(8);
        assert!(!can_show(&state, 8));
    }

    #[test]
    fn preview_sits_below_the_measured_notch() {
        // Without an applied frame, clear the in-menu-bar island plus a 6pt gap.
        assert_eq!(top_inset_for(Some(37.0)), 43.0);
        assert_eq!(top_inset_for(Some(24.0)), 30.0);
    }

    #[test]
    fn unmeasured_or_absurd_notch_heights_fall_back() {
        let fallback = 43.0;
        assert_eq!(top_inset_for(None), fallback);
        assert_eq!(top_inset_for(Some(0.0)), fallback);
        assert_eq!(top_inset_for(Some(-4.0)), fallback);
        assert_eq!(top_inset_for(Some(f64::NAN)), fallback);
        assert_eq!(top_inset_for(Some(f64::INFINITY)), fallback);
    }
}
