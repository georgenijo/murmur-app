//! Native window transport for the voice-query answer popover (#538).
//!
//! The popover stays non-activating for its whole lifetime. It used to become
//! focusable (and call `set_focus`) on reaching a terminal state, which
//! activated Murmur the moment an answer arrived — so dismissing the popover
//! left the main window frontmost instead of returning the user to whatever
//! they were working in. Clicks and text selection work fine on a
//! non-activating window, and Escape is delivered by the global rdev listener
//! rather than the webview's own key handler, so nothing needed focus.

use tauri::Manager;

const WIDTH: f64 = 440.0;
const COMPACT_HEIGHT: f64 = 92.0;
const EXPANDED_HEIGHT: f64 = 340.0;
const TOP_INSET: f64 = 72.0;

fn frame(app: &tauri::AppHandle, expanded: bool) -> (f64, f64, f64, f64) {
    let height = if expanded {
        EXPANDED_HEIGHT
    } else {
        COMPACT_HEIGHT
    };
    let monitor = app
        .get_webview_window("main")
        .and_then(|window| window.current_monitor().ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());
    match monitor {
        Some(monitor) => {
            let scale = monitor.scale_factor();
            let x = monitor.position().x as f64 / scale
                + (monitor.size().width as f64 / scale - WIDTH) / 2.0;
            let y = monitor.position().y as f64 / scale + TOP_INSET;
            (x, y, WIDTH, height)
        }
        None => (300.0, TOP_INSET, WIDTH, height),
    }
}

fn apply_treatment(window: &tauri::WebviewWindow, prevents_activation: bool) {
    crate::commands::native_window::set_window_level_and_activation(
        window,
        crate::commands::native_window::ABOVE_MENU_BAR_LEVEL,
        prevents_activation,
    );
}

pub(crate) fn show_internal(app: &tauri::AppHandle, expanded: bool) -> Result<(), String> {
    let Some(window) = app.get_webview_window("query-review") else {
        return Err("query-review window is unavailable".to_string());
    };
    let (x, y, width, height) = frame(app, expanded);
    window
        .set_size(tauri::LogicalSize::new(width, height))
        .map_err(|_| "query-review window could not be sized".to_string())?;
    window
        .set_position(tauri::LogicalPosition::new(x, y))
        .map_err(|_| "query-review window could not be positioned".to_string())?;
    apply_treatment(&window, true);
    window
        .set_focusable(false)
        .map_err(|_| "query-review focus mode could not be set".to_string())?;
    window
        .set_ignore_cursor_events(false)
        .map_err(|_| "query-review pointer events could not be enabled".to_string())?;
    window
        .show()
        .map_err(|_| "query-review window could not be shown".to_string())
}

pub(crate) fn set_expanded_internal(app: &tauri::AppHandle, expanded: bool) -> Result<(), String> {
    let Some(window) = app.get_webview_window("query-review") else {
        return Err("query-review window is unavailable".to_string());
    };
    let (x, y, width, height) = frame(app, expanded);
    window
        .set_size(tauri::LogicalSize::new(width, height))
        .map_err(|_| "query-review window could not be sized".to_string())?;
    window
        .set_position(tauri::LogicalPosition::new(x, y))
        .map_err(|_| "query-review window could not be positioned".to_string())
}

pub(crate) fn hide_internal(app: &tauri::AppHandle) -> Result<(), String> {
    match app.get_webview_window("query-review") {
        Some(window) => window
            .hide()
            .map_err(|_| "query-review window could not be hidden".to_string()),
        None => Ok(()),
    }
}

pub(crate) fn apply_initial_size(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("query-review") {
        let _ = window.set_size(tauri::LogicalSize::new(WIDTH, COMPACT_HEIGHT));
    }
}
