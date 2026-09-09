//! Native window transport for the voice-query answer popover (#538).
//!
//! The popover stays non-activating for its whole lifetime. It used to become
//! focusable (and call `set_focus`) on reaching a terminal state, which
//! activated Murmur the moment an answer arrived — so dismissing the popover
//! left the main window frontmost instead of returning the user to whatever
//! they were working in. Clicks and text selection work fine on a
//! non-activating window, and Escape is delivered by the global rdev listener
//! rather than the webview's own key handler, so nothing needed focus.

use crate::commands::native_window::PopoverSpec;
use tauri::Manager;

const WIDTH: f64 = 440.0;
const COMPACT_HEIGHT: f64 = 92.0;
const EXPANDED_HEIGHT: f64 = 340.0;
const TOP_INSET: f64 = 72.0;

const POPOVER: PopoverSpec = PopoverSpec {
    label: "query-review",
    // The user interacts with this popover (clicks, selects text), so
    // cursor events must not pass through to whatever's underneath.
    ignore_cursor_events: false,
};

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

pub(crate) fn show_internal(app: &tauri::AppHandle, expanded: bool) -> Result<(), String> {
    POPOVER.show(app, frame(app, expanded))
}

pub(crate) fn set_expanded_internal(app: &tauri::AppHandle, expanded: bool) -> Result<(), String> {
    POPOVER.reposition(app, frame(app, expanded))
}

pub(crate) fn hide_internal(app: &tauri::AppHandle) -> Result<(), String> {
    POPOVER.hide(app)
}

pub(crate) fn apply_initial_size(app: &tauri::AppHandle) {
    POPOVER.apply_initial_size(app, WIDTH, COMPACT_HEIGHT);
}
