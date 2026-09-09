//! Shared NSWindow treatment for non-activating, always-on-top surfaces.
//!
//! Both the notch overlay (`commands/overlay.rs`) and the transform review
//! popover (`commands/transform_popover.rs`) need a window that floats above
//! the menu bar without ever activating the app (which would steal focus from
//! whatever the user was typing into). This module holds that one shared
//! NSWindow-level treatment so the two windows cannot drift apart. It also
//! owns native main-window chrome adjustments that AppKit renders outside the
//! web view.

/// Remove AppKit's automatic line between the overlay title bar and web
/// content. Murmur's header and history toolbar are intentionally one
/// continuous surface.
#[cfg(target_os = "macos")]
pub(crate) fn hide_titlebar_separator(window: &tauri::WebviewWindow) {
    use tauri::Manager;
    let window = window.clone();
    let handle = window.app_handle().clone();
    if let Err(error) = handle.run_on_main_thread(move || {
        apply_hidden_titlebar_separator_on_main(&window);
    }) {
        tracing::warn!(
            target: "system",
            "hide_titlebar_separator: run_on_main_thread failed: {}",
            error
        );
    }
}

#[cfg(target_os = "macos")]
fn apply_hidden_titlebar_separator_on_main(window: &tauri::WebviewWindow) {
    debug_assert!(
        objc2_foundation::MainThreadMarker::new().is_some(),
        "NSWindow mutation must run on the main thread"
    );
    if let Ok(ptr) = window.ns_window() {
        let ns_window: &objc2_app_kit::NSWindow = unsafe { &*(ptr.cast()) };
        ns_window.setTitlebarSeparatorStyle(objc2_app_kit::NSTitlebarSeparatorStyle::None);
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn hide_titlebar_separator(_window: &tauri::WebviewWindow) {}

/// Raise `window` to `level` (an `NSWindow` level, e.g. `NSMainMenuWindowLevel
/// + 1 = 25`) and mark it non-activating via the private `_setPreventsActivation:`
/// API, guarded by `respondsToSelector:` for forward compatibility.
///
/// `prevents_activation = true` means clicking the window will not activate
/// the app / steal key focus (used while the transform popover is listening
/// or thinking, and always for the overlay). `false` allows the window to
/// activate normally (used once the popover reaches ready/failed and needs to
/// accept keyboard shortcuts).
///
/// The raw `NSWindow` mutation is dispatched to the main thread via
/// `run_on_main_thread` — AppKit hard-traps (`EXC_BREAKPOINT`, "Must only be
/// used from the main thread") on off-main window mutation on macOS 26, and the
/// popover show path runs in async Tauri command context (a tokio worker). This
/// mirrors the AX dispatch in `injector`/`selection`/`transform_apply` (issue
/// #325). Fire-and-forget: the raw work has no return value and every caller
/// ignores it, so there is no oneshot to await.
#[cfg(target_os = "macos")]
pub(crate) fn set_window_level_and_activation(
    window: &tauri::WebviewWindow,
    level: isize,
    prevents_activation: bool,
) {
    use tauri::Manager;
    let window = window.clone();
    let handle = window.app_handle().clone();
    if let Err(e) = handle.run_on_main_thread(move || {
        apply_window_level_and_activation_on_main(&window, level, prevents_activation);
    }) {
        tracing::warn!(target: "system", "set_window_level_and_activation: run_on_main_thread failed: {}", e);
    }
}

/// Main-thread-only worker holding the raw `objc2` `NSWindow` mutation. MUST run
/// on the main thread; the `debug_assert!` trips loudly in dev if a future
/// caller invokes it off-main instead of going through the dispatcher above.
#[cfg(target_os = "macos")]
fn apply_window_level_and_activation_on_main(
    window: &tauri::WebviewWindow,
    level: isize,
    prevents_activation: bool,
) {
    debug_assert!(
        objc2_foundation::MainThreadMarker::new().is_some(),
        "NSWindow mutation must run on the main thread"
    );
    let raw = window.ns_window();
    if let Ok(ptr) = raw {
        let ns_window: &objc2_app_kit::NSWindow = unsafe { &*(ptr.cast()) };
        ns_window.setLevel(level);
        // Private API — macOSPrivateApi is already enabled in tauri.conf.json.
        // Tested on macOS 15 (Sequoia). Guard with respondsToSelector in case
        // Apple removes this in a future version.
        let sel = objc2::sel!(_setPreventsActivation:);
        let responds: bool = unsafe { objc2::msg_send![ns_window, respondsToSelector: sel] };
        if responds {
            let _: () =
                unsafe { objc2::msg_send![ns_window, _setPreventsActivation: prevents_activation] };
        } else {
            tracing::warn!(target: "system", "_setPreventsActivation: not available on this macOS version");
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn set_window_level_and_activation(
    _window: &tauri::WebviewWindow,
    _level: isize,
    _prevents_activation: bool,
) {
}

/// NSMainMenuWindowLevel = 24, so +1 = 25 puts a window just above the menu
/// bar. This is what boring.notch, mew-notch, and Murmur's own overlay use.
pub(crate) const ABOVE_MENU_BAR_LEVEL: isize = 25;

/// Shared show/hide/reposition transport for the small, non-activating
/// notch-anchored popovers (`commands/query_popover.rs`,
/// `commands/dictation_preview.rs`). Both windows need the exact same
/// treatment — raised above the menu bar via
/// [`set_window_level_and_activation`], never focusable, shown/hidden by
/// window label — so this holds it once instead of twice. Geometry (the
/// `(x, y, width, height)` frame) stays the caller's responsibility: each
/// popover has its own anchor and sizing rules.
pub(crate) struct PopoverSpec {
    /// The Tauri window label, e.g. `"query-review"` or `"dictation-preview"`.
    pub label: &'static str,
    /// Whether the window should let clicks/text-selection pass through to
    /// whatever is underneath instead of capturing them. `true` for a purely
    /// informational overlay (dictation preview); `false` for a popover the
    /// user interacts with (voice-query answer).
    pub ignore_cursor_events: bool,
}

impl PopoverSpec {
    fn window(&self, app: &tauri::AppHandle) -> Result<tauri::WebviewWindow, String> {
        use tauri::Manager;
        app.get_webview_window(self.label)
            .ok_or_else(|| format!("{} window is unavailable", self.label))
    }

    /// Resize and reposition `window` to `frame` without touching level,
    /// activation, focus, or visibility.
    fn resize_and_position(
        &self,
        window: &tauri::WebviewWindow,
        frame: (f64, f64, f64, f64),
    ) -> Result<(), String> {
        let (x, y, width, height) = frame;
        window
            .set_size(tauri::LogicalSize::new(width, height))
            .map_err(|_| format!("{} window could not be sized", self.label))?;
        window
            .set_position(tauri::LogicalPosition::new(x, y))
            .map_err(|_| format!("{} window could not be positioned", self.label))
    }

    /// Resize and reposition to `frame` without touching level, activation,
    /// focus, or visibility. Used by popovers that reposition while already
    /// visible (voice-query's compact/expanded transition).
    pub(crate) fn reposition(
        &self,
        app: &tauri::AppHandle,
        frame: (f64, f64, f64, f64),
    ) -> Result<(), String> {
        let window = self.window(app)?;
        self.resize_and_position(&window, frame)
    }

    /// Full show sequence: resize/reposition to `frame`, raise above the menu
    /// bar as non-activating (see `set_window_level_and_activation`),
    /// disable focus, apply this spec's cursor-event pass-through, then show.
    pub(crate) fn show(&self, app: &tauri::AppHandle, frame: (f64, f64, f64, f64)) -> Result<(), String> {
        let window = self.window(app)?;
        self.resize_and_position(&window, frame)?;
        set_window_level_and_activation(&window, ABOVE_MENU_BAR_LEVEL, true);
        window
            .set_focusable(false)
            .map_err(|_| format!("{} focus mode could not be set", self.label))?;
        window
            .set_ignore_cursor_events(self.ignore_cursor_events)
            .map_err(|_| format!("{} pointer events could not be set", self.label))?;
        window
            .show()
            .map_err(|_| format!("{} window could not be shown", self.label))
    }

    /// Hide the window if it exists. Missing window is not an error — both
    /// callers treat "already gone" as already hidden.
    pub(crate) fn hide(&self, app: &tauri::AppHandle) -> Result<(), String> {
        use tauri::Manager;
        match app.get_webview_window(self.label) {
            Some(window) => window
                .hide()
                .map_err(|_| format!("{} window could not be hidden", self.label)),
            None => Ok(()),
        }
    }

    /// Set the window's size without positioning, level, or visibility
    /// changes — used once at startup so the window has a sane size before
    /// its first `show`. Errors are ignored, matching prior behavior: an
    /// initial-size failure is not worth surfacing since `show` sizes again.
    pub(crate) fn apply_initial_size(&self, app: &tauri::AppHandle, width: f64, height: f64) {
        use tauri::Manager;
        if let Some(window) = app.get_webview_window(self.label) {
            let _ = window.set_size(tauri::LogicalSize::new(width, height));
        }
    }
}
