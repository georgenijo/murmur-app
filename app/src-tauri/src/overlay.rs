use crate::{MutexExt, State};
use crate::{log_info, log_warn};
use tauri::Manager;

const NOTCH_EXPAND: f64 = 120.0; // 60px expansion room on each side
const FALLBACK_OVERLAY_W: f64 = 200.0;

#[derive(serde::Serialize, Clone)]
pub(crate) struct NotchInfo {
    notch_width: f64,
    notch_height: f64,
}

/// Detect notch width and configure the overlay as a notch-level window.
/// Uses native NSScreen APIs — no subprocess needed.
#[cfg(target_os = "macos")]
pub(crate) fn detect_notch_info() -> Option<(f64, f64)> {
    // Returns (notch_width, menu_bar_height) in logical points
    use objc2_app_kit::NSScreen;
    use objc2_foundation::MainThreadMarker;

    let mtm = unsafe { MainThreadMarker::new_unchecked() };
    let screen = NSScreen::mainScreen(mtm)?;
    let insets = screen.safeAreaInsets();
    if insets.top <= 0.0 {
        return None; // No notch
    }
    let frame = screen.frame();
    let left_w = screen.auxiliaryTopLeftArea().size.width;
    let right_w = screen.auxiliaryTopRightArea().size.width;
    let notch_w = frame.size.width - left_w - right_w;
    log_info!("detect_notch_info: notch_w={}, menu_bar_h={}, screen_w={}", notch_w, insets.top, frame.size.width);
    Some((notch_w, insets.top))
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn detect_notch_info() -> Option<(f64, f64)> { None }

/// Raise the overlay window above the menu bar so it overlaps the notch.
#[cfg(target_os = "macos")]
fn raise_window_above_menubar(overlay: &tauri::WebviewWindow) {
    // NSMainMenuWindowLevel = 24, so +1 = 25 puts us just above the menu bar.
    // This is what boring.notch and mew-notch use.
    let raw = overlay.ns_window();
    if let Ok(ptr) = raw {
        let ns_window: &objc2_app_kit::NSWindow = unsafe { &*(ptr.cast()) };
        ns_window.setLevel(25);
        ns_window.setHasShadow(false);
        // Prevent clicking the overlay from activating the app (which unhides the main window).
        // Private API — macOSPrivateApi is already enabled in tauri.conf.json.
        // Tested on macOS 15 (Sequoia). Guard with respondsToSelector in case Apple
        // removes this in a future version.
        let sel = objc2::sel!(_setPreventsActivation:);
        let responds: bool = unsafe { objc2::msg_send![ns_window, respondsToSelector: sel] };
        if responds {
            let _: () = unsafe { objc2::msg_send![ns_window, _setPreventsActivation: true] };
        } else {
            log_warn!("_setPreventsActivation: not available on this macOS version");
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn raise_window_above_menubar(_overlay: &tauri::WebviewWindow) {}

/// Position and size the overlay to match the notch, anchored at the top of the screen.
/// The window is notch-height tall and wide enough for horizontal expansion.
/// Takes cached notch_info to avoid calling NSScreen APIs off the main thread.
pub(crate) fn position_overlay_default(overlay: &tauri::WebviewWindow, notch_info: Option<(f64, f64)>) {
    let overlay_w = notch_info.map(|(w, _)| w + NOTCH_EXPAND).unwrap_or(FALLBACK_OVERLAY_W);
    let overlay_h = notch_info.map(|(_, h)| h).unwrap_or(37.0);
    log_info!("position_overlay_default: notch_info={:?}, overlay_w={}, overlay_h={}", notch_info, overlay_w, overlay_h);

    // Resize window to match notch area
    if let Err(e) = overlay.set_size(tauri::LogicalSize::new(overlay_w, overlay_h)) {
        log_warn!("position_overlay_default: set_size({}, {}) failed: {}", overlay_w, overlay_h, e);
    }

    // Raise above the menu bar so the window can overlap the notch
    raise_window_above_menubar(overlay);

    if let Some(monitor) = overlay.current_monitor().ok().flatten() {
        let size = monitor.size();
        let sf = monitor.scale_factor();
        let x = (size.width as f64 / sf - overlay_w) / 2.0;
        log_info!("position_overlay_default: x={}, y=0, sf={}", x, sf);
        if let Err(e) = overlay.set_position(tauri::LogicalPosition::new(x, 0.0)) {
            log_warn!("position_overlay_default: set_position({}, 0) failed: {}", x, e);
        }
    } else {
        log_warn!("position_overlay_default: no current monitor, falling back to (100, 100)");
        let _ = overlay.set_position(tauri::LogicalPosition::new(100.0, 100.0));
    }
}

/// Return cached notch dimensions so the frontend can position content precisely.
#[tauri::command]
pub fn get_notch_info(state: tauri::State<'_, State>) -> Option<NotchInfo> {
    state.notch_info.lock_or_recover().map(|(w, h)| NotchInfo { notch_width: w, notch_height: h })
}

/// Show the always-on-top overlay window.
#[tauri::command]
pub fn show_overlay(app: tauri::AppHandle, state: tauri::State<'_, State>) -> Result<(), String> {
    let notch = *state.notch_info.lock_or_recover();
    match app.get_webview_window("overlay") {
        Some(overlay) => {
            position_overlay_default(&overlay, notch);
            overlay.show().map_err(|e| e.to_string())?;
            // Re-enable mouse events (focusable:false disables them on macOS)
            let _ = overlay.set_ignore_cursor_events(false);
            Ok(())
        }
        None => {
            log_warn!("show_overlay: overlay window not found — skipping");
            Ok(())
        }
    }
}

/// Hide the always-on-top overlay window.
#[tauri::command]
pub fn hide_overlay(app: tauri::AppHandle) -> Result<(), String> {
    match app.get_webview_window("overlay") {
        Some(overlay) => overlay.hide().map_err(|e| e.to_string()),
        None => {
            log_warn!("hide_overlay: overlay window not found — skipping");
            Ok(())
        }
    }
}
