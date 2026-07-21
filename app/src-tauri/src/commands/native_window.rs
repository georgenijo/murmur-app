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
#[cfg(target_os = "macos")]
pub(crate) fn set_window_level_and_activation(
    window: &tauri::WebviewWindow,
    level: isize,
    prevents_activation: bool,
) {
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
