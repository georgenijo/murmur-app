//! Shared NSWindow treatment for non-activating, always-on-top surfaces.
//!
//! Both the notch overlay (`commands/overlay.rs`) and the transform review
//! popover (`commands/transform_popover.rs`) need a window that floats above
//! the menu bar without ever activating the app (which would steal focus from
//! whatever the user was typing into). This module holds that one shared
//! NSWindow-level treatment so the two windows cannot drift apart.

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
            let _: () = unsafe {
                objc2::msg_send![ns_window, _setPreventsActivation: prevents_activation]
            };
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
