//! Native window transport for the live dictation preview popover (#611).
//!
//! The opt-in preview is a small, non-activating, click-through live-words pill
//! attached below the actual overlay frame while recording. It shows the
//! words recognized so far. It is presentation only. It never owns transcript
//! state or takes focus. The hidden webview renders recognized words first,
//! then asks Rust to show the native window for the exact recording. Rust hides
//! the window when that recording stops being current.
//!
//! When a stop hands the recording to delivery, delivery owns the hide: it is
//! queued on the main thread directly behind the paste job, so a hide never
//! sits ahead of the paste. Every path that ends without a paste hides when
//! its owner drops. Cancels and interruptions while recording are still hidden
//! by the partial ticker.
//!
//! Provisional content never enters the island or the final-delivery path.
//! The existing partial API provides no authoritative stable-word boundary.

use crate::commands::native_window::PopoverSpec;
use crate::state::DictationStatus;
use crate::{MutexExt, State};
use std::sync::atomic::{AtomicU64, Ordering};
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
    POPOVER.show(app, frame(app, notch_height(app)))?;
    crate::main_thread_watch::section("preview.refit_material", || refit_material(app));
    Ok(())
}

/// A clamped window height moves the card's bottom edge: re-fit its material
/// in the same main-thread turn as the resize.
fn refit_material(app: &tauri::AppHandle) {
    #[cfg(target_os = "macos")]
    super::overlay_material::refit(app, super::overlay_material::LIVE_WORDS_LABEL);
    #[cfg(not(target_os = "macos"))]
    let _ = app;
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
    crate::main_thread_watch::section("preview.follow_overlay", || {
        if let Err(error) = POPOVER.reposition(app, frame(app, notch_height(app))) {
            tracing::warn!(target: "system", "dictation preview could not follow overlay: {}", error);
        }
        refit_material(app);
    });
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

/// Why a hide was requested. Logged with each hide so the order relative to
/// the paste is visible per dictation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HideTrigger {
    /// Queued directly behind the delivery job (paste or clipboard-only).
    AfterPaste,
    /// Delivery owned the hide but the recording ended without a delivery job:
    /// no speech, empty text, cancel while processing, or an error.
    NoPaste,
    /// The recording stopped being current without a stop handing it to
    /// delivery: cancel or interruption while recording, or a successor.
    RecordingEnded,
}

impl HideTrigger {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::AfterPaste => "after_paste",
            Self::NoPaste => "no_paste",
            Self::RecordingEnded => "recording_ended",
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

fn emit_hide(recording_id: u64, outcome: PresentationOutcome, trigger: HideTrigger) {
    tracing::info!(
        target: "pipeline",
        event_code = "pipeline.dictation_preview_presentation",
        recording_id,
        outcome = outcome.as_str(),
        trigger = trigger.as_str(),
        "dictation preview presentation changed"
    );
}

/// Which recording each side of the hide handoff belongs to.
pub(crate) struct HideOwnership {
    /// Recording whose partial ticker (the only source of a shown preview) runs.
    ticker: AtomicU64,
    /// Recording whose stop handed the hide to delivery.
    delivery: AtomicU64,
}

impl HideOwnership {
    pub(crate) const fn new() -> Self {
        Self {
            ticker: AtomicU64::new(0),
            delivery: AtomicU64::new(0),
        }
    }

    /// Record the ticker's generation. Recording ids only grow, so a stale
    /// generation resuming late is refused atomically instead of replacing
    /// its successor's registration.
    pub(crate) fn ticker_started(&self, recording_id: u64) -> bool {
        recording_id > 0 && self.ticker.fetch_max(recording_id, Ordering::SeqCst) <= recording_id
    }

    /// Called under the dictation lock at the accepted stop transition, before
    /// status leaves `Recording`, so the ticker sees the claim whenever it
    /// sees the stop. Only a recording with a ticker can have a shown preview.
    pub(crate) fn claim_for_delivery(&self, recording_id: u64) -> bool {
        if recording_id == 0 || self.ticker.load(Ordering::SeqCst) != recording_id {
            return false;
        }
        self.delivery.store(recording_id, Ordering::SeqCst);
        true
    }

    /// Whether the ticker for `recording_id` should hide on exit.
    pub(crate) fn ticker_should_hide(&self, recording_id: u64) -> bool {
        self.delivery.load(Ordering::SeqCst) != recording_id
    }
}

static OWNERSHIP: HideOwnership = HideOwnership::new();

/// Register the ticker for `recording_id`; `false` for an older generation.
/// Called under the dictation lock (see `register_partial_ticker_locked`).
pub(crate) fn note_ticker_started(recording_id: u64) -> bool {
    OWNERSHIP.ticker_started(recording_id)
}

/// The partial ticker's exit: hide unless a stop handed the hide to delivery.
pub(crate) fn hide_on_ticker_exit(app: &tauri::AppHandle, recording_id: u64) {
    if OWNERSHIP.ticker_should_hide(recording_id) {
        hide_for_recording(app, recording_id, HideTrigger::RecordingEnded);
    }
}

/// Delivery's ownership of one recording's preview hide. Either
/// [`PreviewHideOwner::after_paste`] queues the hide behind the paste job, or
/// dropping the owner hides at once.
pub(crate) struct PreviewHideOwner {
    hide: Option<Box<dyn FnOnce(HideTrigger) + Send>>,
}

impl PreviewHideOwner {
    fn new(hide: impl FnOnce(HideTrigger) + Send + 'static) -> Self {
        Self {
            hide: Some(Box::new(hide)),
        }
    }

    /// Claim the hide for delivery. Call under the dictation lock at the stop
    /// transition. `None` when this recording never ran a preview ticker.
    pub(crate) fn claim(app: &tauri::AppHandle, recording_id: u64) -> Option<Self> {
        if !OWNERSHIP.claim_for_delivery(recording_id) {
            return None;
        }
        let app = app.clone();
        Some(Self::new(move |trigger| {
            hide_for_recording(&app, recording_id, trigger)
        }))
    }

    /// Queue the hide on the main thread. Call only after the delivery job was
    /// dispatched: main-thread jobs run in dispatch order, so the hide runs
    /// after the paste and the paste never waits for it.
    pub(crate) fn after_paste(mut self) {
        if let Some(hide) = self.hide.take() {
            hide(HideTrigger::AfterPaste);
        }
    }
}

impl Drop for PreviewHideOwner {
    fn drop(&mut self) {
        if let Some(hide) = self.hide.take() {
            hide(HideTrigger::NoPaste);
        }
    }
}

/// Dispatch the delivery job, then queue the preview hide behind it. A failed
/// dispatch means no paste, so the owner's drop hides at once.
pub(crate) fn dispatch_then_hide<T, E>(
    dispatch: impl FnOnce() -> Result<T, E>,
    owner: Option<PreviewHideOwner>,
) -> Result<T, E> {
    let dispatched = dispatch()?;
    if let Some(owner) = owner {
        owner.after_paste();
    }
    Ok(dispatched)
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
    app.run_on_main_thread(crate::main_thread_watch::job("preview.show", move || {
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
    }))
    .map_err(|_| "dictation preview presentation could not be scheduled".to_string())?;
    rx.await
        .map_err(|_| "dictation preview presentation was cancelled".to_string())?
}

fn hide_for_recording(app: &tauri::AppHandle, recording_id: u64, trigger: HideTrigger) {
    let dispatch_app = app.clone();
    let _ = app.run_on_main_thread(crate::main_thread_watch::job("preview.hide", move || {
        // A late hide must not hide its successor, including after this
        // operation spent time queued on the native main thread.
        if !can_hide(&dispatch_app.state::<State>().app_state, recording_id) {
            return;
        }
        crate::main_thread_watch::note_preview_hide(recording_id);
        let result = POPOVER.hide(&dispatch_app);
        emit_hide(
            recording_id,
            if result.is_ok() {
                PresentationOutcome::Hidden
            } else {
                PresentationOutcome::HideFailed
            },
            trigger,
        );
    }));
}

pub(crate) fn apply_initial_size(app: &tauri::AppHandle) {
    POPOVER.apply_initial_size(app, WIDTH, HEIGHT);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recording_owner(log: &std::sync::Arc<std::sync::Mutex<Vec<String>>>) -> PreviewHideOwner {
        let log = log.clone();
        PreviewHideOwner::new(move |trigger| log.lock().unwrap().push(trigger.as_str().into()))
    }

    #[test]
    fn delivery_queues_the_hide_behind_the_paste_job() {
        let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let paste_log = log.clone();
        let result: Result<(), ()> = dispatch_then_hide(
            || {
                paste_log.lock().unwrap().push("paste".into());
                Ok(())
            },
            Some(recording_owner(&log)),
        );
        assert!(result.is_ok());
        assert_eq!(*log.lock().unwrap(), ["paste", "after_paste"]);
    }

    #[test]
    fn a_failed_paste_dispatch_still_hides_once() {
        let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let result: Result<(), &str> =
            dispatch_then_hide(|| Err("no main thread"), Some(recording_owner(&log)));
        assert!(result.is_err());
        assert_eq!(*log.lock().unwrap(), ["no_paste"]);
    }

    #[test]
    fn every_path_without_a_paste_hides_when_its_owner_drops() {
        // Empty text, no speech, too short, cancel while processing, missing
        // context, stop failure, and pipeline errors all return without
        // reaching delivery; each drops the owner.
        let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        drop(recording_owner(&log));
        assert_eq!(*log.lock().unwrap(), ["no_paste"]);
    }

    #[test]
    fn after_paste_hides_exactly_once() {
        let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        recording_owner(&log).after_paste();
        assert_eq!(*log.lock().unwrap(), ["after_paste"]);
    }

    #[test]
    fn the_ticker_defers_to_delivery_only_for_a_stopped_recording() {
        let ownership = HideOwnership::new();
        // No ticker ran: there is no preview to hide and delivery claims nothing.
        assert!(!ownership.claim_for_delivery(5));
        assert!(ownership.ticker_started(7));
        // Cancelled or interrupted while recording: no stop claim, ticker hides.
        assert!(ownership.ticker_should_hide(7));
        // Accepted stop: delivery claims, the ticker leaves the hide to it.
        assert!(ownership.claim_for_delivery(7));
        assert!(!ownership.ticker_should_hide(7));
        // A successor's ticker never inherits the old claim.
        assert!(ownership.ticker_started(8));
        assert!(ownership.ticker_should_hide(8));
        assert!(!ownership.claim_for_delivery(7));
        assert!(!ownership.claim_for_delivery(0));
    }

    #[test]
    fn a_stale_ticker_registration_cannot_replace_its_successor() {
        let ownership = HideOwnership::new();
        assert!(ownership.ticker_started(8));
        // Generation 7's ready callback resumes after 8 registered.
        assert!(!ownership.ticker_started(7));
        // 8's stop still claims the hide, so its ticker leaves it to delivery.
        assert!(ownership.claim_for_delivery(8));
        assert!(!ownership.ticker_should_hide(8));
        assert!(!ownership.ticker_started(0));
    }

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
