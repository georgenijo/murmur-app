use crate::{MutexExt, State};
#[cfg(target_os = "macos")]
use tauri::Emitter;
use tauri::Manager;

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct OverlayGeometry {
    pub window_w: f64,
    pub collapsed_h: f64,
    pub expanded_h: f64,
    pub pill_idle_w: f64,
    pub pill_active_w: f64,
    pub pill_margin_idle: f64,
    pub pill_margin_active: f64,
    pub dropdown_h: f64,
}

// Private geometry constants — the ONLY place these magic numbers live.
const PILL_IDLE_PAD: f64 = 28.0;
const WINDOW_EXPAND: f64 = 120.0; // active pill pad too (fills window)
const DROPDOWN_H: f64 = 44.0;
const FALLBACK_NOTCH_W: f64 = 80.0;
const FALLBACK_NOTCH_H: f64 = 37.0;

fn geometry_for(notch: Option<(f64, f64)>) -> OverlayGeometry {
    let (notch_w, notch_h) = notch.unwrap_or((FALLBACK_NOTCH_W, FALLBACK_NOTCH_H));
    let window_w = notch_w + WINDOW_EXPAND;
    let pill_idle_w = notch_w + PILL_IDLE_PAD;
    OverlayGeometry {
        window_w,
        collapsed_h: notch_h,
        expanded_h: notch_h + DROPDOWN_H,
        pill_idle_w,
        pill_active_w: window_w,
        pill_margin_idle: (window_w - pill_idle_w) / 2.0,
        pill_margin_active: 0.0,
        dropdown_h: DROPDOWN_H,
    }
}

/// Detect notch width and configure the overlay as a notch-level window.
/// Uses native NSScreen APIs — no subprocess needed.
#[cfg(target_os = "macos")]
pub(crate) fn detect_notch_info() -> Option<(f64, f64)> {
    // Returns (notch_width, menu_bar_height) in logical points
    use objc2_app_kit::NSScreen;
    use objc2_foundation::MainThreadMarker;

    // SAFETY: detect_notch_info() is only called from Tauri's setup() callback,
    // which runs on the main thread. MainThreadMarker::new_unchecked() requires
    // the caller to be on the main thread, which setup() guarantees.
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
    tracing::info!(target: "system", "detect_notch_info: notch_w={}, menu_bar_h={}, screen_w={}", notch_w, insets.top, frame.size.width);
    Some((notch_w, insets.top))
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn detect_notch_info() -> Option<(f64, f64)> {
    None
}

/// Subscribe to macOS display configuration changes (plug/unplug monitor, lid open/close).
/// Re-detects notch info, repositions the overlay, and notifies the frontend.
#[cfg(target_os = "macos")]
pub(crate) fn register_screen_change_observer(app_handle: tauri::AppHandle) {
    use objc2_foundation::{
        NSNotification, NSNotificationCenter, NSNotificationName, NSOperationQueue,
    };

    let notification_name =
        NSNotificationName::from_str("NSApplicationDidChangeScreenParametersNotification");

    let block = block2::RcBlock::new(move |_notification: std::ptr::NonNull<NSNotification>| {
        tracing::info!(target: "system", "screen parameters changed — re-detecting notch info");
        let notch = detect_notch_info();
        let handle = &app_handle;
        // Update cached notch info
        {
            let state = handle.state::<State>();
            *state.notch_info.lock_or_recover() = notch;
        }
        // Reposition overlay window
        if let Some(overlay) = handle.get_webview_window("overlay") {
            position_overlay_default(&overlay, notch);
        }
        // Notify frontend
        let _ = handle.emit("overlay-geometry-changed", geometry_for(notch));
    });

    unsafe {
        let center = NSNotificationCenter::defaultCenter();
        let observer = center.addObserverForName_object_queue_usingBlock(
            Some(&notification_name),
            None,
            Some(&NSOperationQueue::mainQueue()),
            &block,
        );
        // App-lifetime observer — intentionally leak to avoid premature deallocation
        std::mem::forget(observer);
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn register_screen_change_observer(_app_handle: tauri::AppHandle) {}

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
            tracing::warn!(target: "system", "_setPreventsActivation: not available on this macOS version");
        }
    }
}

/// Position and size the overlay to match the notch, anchored at the top of the screen.
/// The window is notch-height tall and wide enough for horizontal expansion.
/// Takes cached notch_info to avoid calling NSScreen APIs off the main thread.
#[cfg(target_os = "macos")]
pub(crate) fn position_overlay_default(
    overlay: &tauri::WebviewWindow,
    notch_info: Option<(f64, f64)>,
) {
    let g = geometry_for(notch_info);
    let overlay_w = g.window_w;
    let overlay_h = g.collapsed_h;
    tracing::info!(target: "system", "position_overlay_default: notch_info={:?}, overlay_w={}, overlay_h={}", notch_info, overlay_w, overlay_h);

    // Resize window to match notch area
    if let Err(e) = overlay.set_size(tauri::LogicalSize::new(overlay_w, overlay_h)) {
        tracing::warn!(target: "system", "position_overlay_default: set_size({}, {}) failed: {}", overlay_w, overlay_h, e);
    }

    // Raise above the menu bar so the window can overlap the notch
    raise_window_above_menubar(overlay);

    if let Some(monitor) = overlay.current_monitor().ok().flatten() {
        let size = monitor.size();
        let sf = monitor.scale_factor();
        let x = (size.width as f64 / sf - overlay_w) / 2.0;
        tracing::info!(target: "system", "position_overlay_default: x={}, y=0, sf={}", x, sf);
        if let Err(e) = overlay.set_position(tauri::LogicalPosition::new(x, 0.0)) {
            tracing::warn!(target: "system", "position_overlay_default: set_position({}, 0) failed: {}", x, e);
        }
    } else {
        tracing::warn!(target: "system", "position_overlay_default: no current monitor, falling back to (100, 100)");
        let _ = overlay.set_position(tauri::LogicalPosition::new(100.0, 100.0));
    }
}

/// Return the current overlay geometry so the frontend can size the island.
#[tauri::command]
pub fn get_overlay_geometry(state: tauri::State<'_, State>) -> OverlayGeometry {
    geometry_for(*state.notch_info.lock_or_recover())
}

/// Show the always-on-top overlay window (macOS notch overlay; no-op on Linux).
#[tauri::command]
pub fn show_overlay(app: tauri::AppHandle, state: tauri::State<'_, State>) -> Result<(), String> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (&app, &state);
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        let notch = *state.notch_info.lock_or_recover();
        match app.get_webview_window("overlay") {
            Some(overlay) => {
                position_overlay_default(&overlay, notch);
                overlay.show().map_err(|e| e.to_string())?;
                let _ = overlay.set_ignore_cursor_events(false);
                Ok(())
            }
            None => {
                tracing::warn!(target: "system", "show_overlay: overlay window not found — skipping");
                Ok(())
            }
        }
    }
}

/// Resize the overlay window for the hover-expand dropdown.
///
/// Grows the window height to `expanded_h` when expanded so the dropdown row
/// has room, and restores it to `collapsed_h` when collapsed. All dimensions come
/// from `geometry_for()`, the same source as `position_overlay_default`, so the
/// collapsed size matches what `show_overlay` set. Only the size changes — the
/// window stays anchored at y=0, so the extra height grows downward.
///
/// We resize on hover rather than pre-allocating a tall window because a
/// transparent overlay with cursor events enabled captures the mouse across its
/// whole frame, which would create a click dead-zone below the notch when idle.
#[tauri::command]
pub fn set_overlay_expanded(
    app: tauri::AppHandle,
    state: tauri::State<'_, State>,
    expanded: bool,
) -> Result<(), String> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (&app, &state, expanded);
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        let notch = *state.notch_info.lock_or_recover();
        match app.get_webview_window("overlay") {
            Some(overlay) => {
                let g = geometry_for(notch);
                let w = g.window_w;
                let h = if expanded {
                    g.expanded_h
                } else {
                    g.collapsed_h
                };
                overlay
                    .set_size(tauri::LogicalSize::new(w, h))
                    .map_err(|e| e.to_string())
            }
            None => {
                tracing::warn!(target: "system", "set_overlay_expanded: overlay window not found — skipping");
                Ok(())
            }
        }
    }
}

/// Show and focus the main app window.
///
/// The overlay uses this instead of frontend window APIs so it does not need
/// broad `core:window:allow-show` / `allow-set-focus` permissions.
#[tauri::command]
pub fn show_main_window(app: tauri::AppHandle) -> Result<(), String> {
    match app.get_webview_window("main") {
        Some(window) => {
            window.show().map_err(|e| e.to_string())?;
            window.set_focus().map_err(|e| e.to_string())
        }
        None => {
            tracing::warn!(target: "system", "show_main_window: main window not found");
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
            tracing::warn!(target: "system", "hide_overlay: overlay window not found — skipping");
            Ok(())
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invariants() {
        for g in [geometry_for(Some((185.0, 32.0))), geometry_for(None)] {
            assert!(g.window_w >= g.pill_active_w + g.pill_margin_active);
            assert!(g.window_w >= g.pill_idle_w + g.pill_margin_idle);
            assert_eq!(g.expanded_h, g.collapsed_h + g.dropdown_h);
            assert!(g.pill_active_w >= g.pill_idle_w);
        }
    }

    #[test]
    fn characterization() {
        let g = geometry_for(Some((185.0, 32.0)));
        assert_eq!(
            (
                g.window_w,
                g.collapsed_h,
                g.expanded_h,
                g.pill_idle_w,
                g.pill_active_w,
                g.pill_margin_idle,
                g.pill_margin_active,
                g.dropdown_h,
            ),
            (305.0, 32.0, 76.0, 213.0, 305.0, 46.0, 0.0, 44.0)
        );
    }

    #[test]
    fn matches_fixture() {
        #[derive(serde::Deserialize)]
        struct F {
            notched: OverlayGeometry,
            fallback: OverlayGeometry,
        }
        let f: F = serde_json::from_str(include_str!(
            "../../../src/components/overlay/overlay-geometry.fixture.json"
        ))
        .unwrap();
        assert_eq!(geometry_for(Some((185.0, 32.0))), f.notched);
        assert_eq!(geometry_for(None), f.fallback);
    }

    #[test]
    fn rejects_unilateral_shape_drift() {
        let mut value = serde_json::to_value(geometry_for(None)).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("extraField".into(), serde_json::json!(1));
        assert!(serde_json::from_value::<OverlayGeometry>(value).is_err());
    }
}
