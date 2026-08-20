//! Native window transport for the live dictation preview popover (#611).
//!
//! The preview mirrors the voice-query popover: a small, non-activating card
//! that appears just below the notch while dictation is recording and shows the
//! words recognized so far. It is presentation only — it never owns transcript
//! state, never takes focus, and is torn down the moment the recording that
//! opened it stops being current.
//!
//! It replaces the earlier attempt at rendering partials inside the overlay's
//! 36pt wing, which could only ever show ~4 head-anchored characters ("Oka…")
//! no matter how long the speaker talked.

use crate::MutexExt;
use tauri::Manager;

const WIDTH: f64 = 460.0;
const HEIGHT: f64 = 104.0;
/// Breathing room between the bottom of the notch/menu bar and the card. The
/// window is transparent and taller than the card, so this is the gap to the
/// card's top edge, not to the window frame.
const NOTCH_GAP: f64 = 6.0;
const FALLBACK_NOTCH_H: f64 = 37.0;

/// Logical frame for the preview window: horizontally centered on the
/// menu-bar display, tucked directly under the notch (or menu bar on displays
/// without one).
///
/// The anchor is deliberately the primary monitor, not the main window's
/// monitor. `notch_info` is measured from `NSScreen::screens().firstObject()`
/// — always the built-in/menu-bar display — and the recording overlay lives on
/// that same notch. Anchoring to the main window instead would put the card on
/// an external display while offsetting it by the *built-in* display's notch
/// height, so it would sit neither under a notch nor next to the overlay.
fn frame(app: &tauri::AppHandle, notch_h: Option<f64>) -> (f64, f64, f64, f64) {
    let top_inset = top_inset_for(notch_h);
    let monitor = app.primary_monitor().ok().flatten();
    match monitor {
        Some(monitor) => {
            let scale = monitor.scale_factor();
            let x = monitor.position().x as f64 / scale
                + (monitor.size().width as f64 / scale - WIDTH) / 2.0;
            let y = monitor.position().y as f64 / scale + top_inset;
            (x, y, WIDTH, HEIGHT)
        }
        None => (300.0, top_inset, WIDTH, HEIGHT),
    }
}

/// Distance from the top of the monitor to the top of the preview card. An
/// unmeasured, zero, or non-finite notch height means the display measurement
/// has not landed yet — fall back rather than render the card over the menu bar.
fn top_inset_for(notch_h: Option<f64>) -> f64 {
    notch_h
        .filter(|height| height.is_finite() && *height > 0.0)
        .unwrap_or(FALLBACK_NOTCH_H)
        + NOTCH_GAP
}

fn notch_height(app: &tauri::AppHandle) -> Option<f64> {
    app.try_state::<crate::State>()
        .and_then(|state| state.notch_info.lock_or_recover().map(|(_, h)| h))
}

/// Show the preview under the notch. Idempotent: repositioning a window that is
/// already visible is how a display change is absorbed.
pub(crate) fn show_internal(app: &tauri::AppHandle) -> Result<(), String> {
    let Some(window) = app.get_webview_window("dictation-preview") else {
        return Err("dictation-preview window is unavailable".to_string());
    };
    let (x, y, width, height) = frame(app, notch_height(app));
    window
        .set_size(tauri::LogicalSize::new(width, height))
        .map_err(|_| "dictation-preview window could not be sized".to_string())?;
    window
        .set_position(tauri::LogicalPosition::new(x, y))
        .map_err(|_| "dictation-preview window could not be positioned".to_string())?;
    crate::commands::native_window::set_window_level_and_activation(
        &window,
        crate::commands::native_window::ABOVE_MENU_BAR_LEVEL,
        true,
    );
    window
        .set_focusable(false)
        .map_err(|_| "dictation-preview focus mode could not be set".to_string())?;
    // Purely informational while the user is dictating into another app: never
    // swallow a click meant for whatever is underneath.
    window
        .set_ignore_cursor_events(true)
        .map_err(|_| "dictation-preview pointer events could not be disabled".to_string())?;
    window
        .show()
        .map_err(|_| "dictation-preview window could not be shown".to_string())
}

pub(crate) fn hide_internal(app: &tauri::AppHandle) -> Result<(), String> {
    match app.get_webview_window("dictation-preview") {
        Some(window) => window
            .hide()
            .map_err(|_| "dictation-preview window could not be hidden".to_string()),
        None => Ok(()),
    }
}

pub(crate) fn apply_initial_size(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("dictation-preview") {
        let _ = window.set_size(tauri::LogicalSize::new(WIDTH, HEIGHT));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_sits_below_the_measured_notch() {
        // 37pt notch + 6pt gap: the card clears the physical notch instead of
        // rendering behind it.
        assert_eq!(top_inset_for(Some(37.0)), 43.0);
        assert_eq!(top_inset_for(Some(24.0)), 30.0);
    }

    #[test]
    fn unmeasured_or_absurd_notch_heights_fall_back() {
        let fallback = FALLBACK_NOTCH_H + NOTCH_GAP;
        assert_eq!(top_inset_for(None), fallback);
        assert_eq!(top_inset_for(Some(0.0)), fallback);
        assert_eq!(top_inset_for(Some(-4.0)), fallback);
        assert_eq!(top_inset_for(Some(f64::NAN)), fallback);
        assert_eq!(top_inset_for(Some(f64::INFINITY)), fallback);
    }
}
