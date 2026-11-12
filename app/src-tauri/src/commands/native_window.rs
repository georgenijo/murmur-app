//! Shared NSWindow treatment for non-activating, always-on-top surfaces.
//!
//! Both the notch overlay (`commands/overlay.rs`) and the transform review
//! popover (`commands/transform_popover.rs`) need a window that floats above
//! the menu bar without ever activating the app (which would steal focus from
//! whatever the user was typing into). This module holds that one shared
//! NSWindow-level treatment so the two windows cannot drift apart. It also
//! owns native main-window chrome adjustments that AppKit renders outside the
//! web view.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// Place AppKit's real buttons in window points, independent of webview zoom
/// and display backing scale. The CSS header height arrives from the renderer.
#[cfg(target_os = "macos")]
pub(crate) fn position_traffic_lights(window: &tauri::WebviewWindow, top: f64) {
    use tauri::Manager;
    let window = window.clone();
    let handle = window.app_handle().clone();
    let _ = handle.run_on_main_thread(crate::main_thread_watch::job(
        "native_window.position_traffic_lights",
        move || {
            use objc2_app_kit::{NSWindow, NSWindowButton, NSWindowStyleMask};
            use objc2_foundation::NSPoint;
            debug_assert!(objc2_foundation::MainThreadMarker::new().is_some());
            let Ok(ptr) = window.ns_window() else {
                return;
            };
            let native: &NSWindow = unsafe { &*ptr.cast() };
            if native.styleMask().contains(NSWindowStyleMask::FullScreen) {
                return;
            }
            let Some(close) = native.standardWindowButton(NSWindowButton::CloseButton) else {
                return;
            };
            // SAFETY: AppKit view hierarchy access stays on the main thread.
            let Some(parent) = (unsafe { close.superview() }) else {
                return;
            };
            // Enlarge the titlebar's hit area as well as moving its buttons. A
            // position below AppKit's default 28pt titlebar must remain clickable.
            if let Some(container) = unsafe { parent.superview() } {
                let mut frame = container.frame();
                frame.size.height = (top + 7.0) * 2.0;
                frame.origin.y = native.frame().size.height - frame.size.height;
                container.setFrame(frame);
            }
            let close_x = close.frame().origin.x;
            let buttons = [
                NSWindowButton::CloseButton,
                NSWindowButton::MiniaturizeButton,
                NSWindowButton::ZoomButton,
            ];
            let positions: Vec<_> = buttons
                .into_iter()
                .filter_map(|kind| {
                    native.standardWindowButton(kind).map(|button| {
                        let offset = button.frame().origin.x - close_x;
                        (button, offset)
                    })
                })
                .collect();
            for (button, offset) in positions {
                let target = NSPoint::new(
                    crate::window_state::TRAFFIC_LIGHT_X + offset,
                    native.frame().size.height - top - button.frame().size.height,
                );
                button.setFrameOrigin(parent.convertPoint_fromView(target, None));
            }
        },
    ));
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn position_traffic_lights(_window: &tauri::WebviewWindow, _top: f64) {}

/// Remove AppKit's automatic line between the overlay title bar and web
/// content. Murmur's header and history toolbar are intentionally one
/// continuous surface.
#[cfg(target_os = "macos")]
pub(crate) fn hide_titlebar_separator(window: &tauri::WebviewWindow) {
    use tauri::Manager;
    let window = window.clone();
    let handle = window.app_handle().clone();
    if let Err(error) = handle.run_on_main_thread(crate::main_thread_watch::job(
        "native_window.hide_titlebar_separator",
        move || {
            apply_hidden_titlebar_separator_on_main(&window);
        },
    )) {
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
    if let Err(e) = handle.run_on_main_thread(crate::main_thread_watch::job(
        "native_window.set_level_and_activation",
        move || {
            apply_window_level_and_activation_on_main(&window, level, prevents_activation);
        },
    )) {
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

/// A popover frame in logical points: `(x, y, width, height)`.
pub(crate) type PopoverFrame = (f64, f64, f64, f64);

/// True when `applied` already matches `target` to within a sub-point rounding
/// error, so the native setters would only add a no-op WindowServer
/// transaction (the overlay applies the same guard).
pub(crate) fn popover_frame_unchanged(applied: Option<PopoverFrame>, target: PopoverFrame) -> bool {
    const EPSILON: f64 = 0.01;
    applied.is_some_and(|f| {
        (f.0 - target.0).abs() < EPSILON
            && (f.1 - target.1).abs() < EPSILON
            && (f.2 - target.2).abs() < EPSILON
            && (f.3 - target.3).abs() < EPSILON
    })
}

/// What one `show` has to do to the native window, decided from what is
/// already applied. The first show orders the window in and configures it
/// (level, activation, focus, pointer pass-through). The window then stays
/// ordered in between dictations, fully transparent and click-through, like
/// the overlay, so later shows only reveal it: no `orderFront`, no level or
/// focus churn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ShowPlan {
    pub set_frame: bool,
    pub order_in: bool,
    pub reveal: bool,
}

pub(crate) fn plan_show(
    ordered_in: bool,
    presented: bool,
    applied: Option<PopoverFrame>,
    target: PopoverFrame,
) -> ShowPlan {
    ShowPlan {
        set_frame: !popover_frame_unchanged(applied, target),
        order_in: !ordered_in,
        reveal: !presented,
    }
}

/// Queue `f` on the main dispatch queue. Tao applies `set_size`,
/// `set_position`, `set_ignore_cursor_events` and `show` as blocks queued on
/// that queue, so work queued here runs behind them in order: a reveal never
/// lands before the frame it belongs to, and a hide never overtakes a show.
#[cfg(target_os = "macos")]
fn queue_on_main_queue(f: impl FnOnce() + Send + 'static) {
    use std::ffi::c_void;
    extern "C" {
        // `dispatch_get_main_queue()` is this symbol's address.
        static _dispatch_main_q: c_void;
        fn dispatch_async_f(
            queue: *const c_void,
            context: *mut c_void,
            work: extern "C" fn(*mut c_void),
        );
    }
    extern "C" fn run(context: *mut c_void) {
        // SAFETY: `context` is the box leaked below and GCD runs each block
        // exactly once.
        let work = unsafe { Box::from_raw(context.cast::<Box<dyn FnOnce() + Send>>()) };
        work();
    }
    let work: Box<Box<dyn FnOnce() + Send>> = Box::new(Box::new(f));
    // SAFETY: `run` takes ownership of the leaked box; the main queue is
    // process-lifetime.
    unsafe {
        dispatch_async_f(
            std::ptr::addr_of!(_dispatch_main_q),
            Box::into_raw(work).cast(),
            run,
        )
    };
}

/// Set the window's alpha on the main queue, behind any queued tao setters.
#[cfg(target_os = "macos")]
fn queue_window_alpha(window: &tauri::WebviewWindow, alpha: f64) {
    let window = window.clone();
    queue_on_main_queue(crate::main_thread_watch::job(
        "native_window.set_alpha",
        move || {
            debug_assert!(objc2_foundation::MainThreadMarker::new().is_some());
            if let Ok(ptr) = window.ns_window() {
                let native: &objc2_app_kit::NSWindow = unsafe { &*ptr.cast() };
                native.setAlphaValue(alpha);
            }
        },
    ));
}

#[cfg(not(target_os = "macos"))]
fn queue_window_alpha(_window: &tauri::WebviewWindow, _alpha: f64) {}

/// Show/hide/reposition transport for the non-activating dictation preview.
/// Geometry stays in `commands/dictation_preview.rs`; native window mutations
/// are dispatched onto the main thread.
///
/// "Hidden" does not order the window out. A WindowServer order-out per
/// dictation was the suspected cost, and the overlay already avoids it by
/// staying ordered in: this window is made transparent and click-through
/// instead (`NSWindow` alpha 0, pointer events ignored).
pub(crate) struct PopoverSpec {
    /// The Tauri window label, e.g. `"dictation-preview"`.
    pub label: &'static str,
    /// Whether the window should let clicks/text-selection pass through to
    /// whatever is underneath instead of capturing them. `true` for a purely
    /// informational overlay (dictation preview); `false` for a popover the
    /// user interacts with. A concealed window is always click-through.
    pub ignore_cursor_events: bool,
    /// The last `show` has not been followed by a `hide`.
    presented: AtomicBool,
    /// `show` has ordered the window in and configured it.
    ordered_in: AtomicBool,
    /// The last frame the native setters accepted.
    applied: Mutex<Option<PopoverFrame>>,
}

impl PopoverSpec {
    pub(crate) const fn new(label: &'static str, ignore_cursor_events: bool) -> Self {
        Self {
            label,
            ignore_cursor_events,
            presented: AtomicBool::new(false),
            ordered_in: AtomicBool::new(false),
            applied: Mutex::new(None),
        }
    }

    fn window(&self, app: &tauri::AppHandle) -> Result<tauri::WebviewWindow, String> {
        use tauri::Manager;
        app.get_webview_window(self.label)
            .ok_or_else(|| format!("{} window is unavailable", self.label))
    }

    /// Whether the window is currently meant to be on screen. Unlike
    /// `WebviewWindow::is_visible` this stays accurate while the window is
    /// ordered in but transparent.
    pub(crate) fn is_presented(&self) -> bool {
        self.presented.load(Ordering::SeqCst)
    }

    fn applied_frame(&self) -> Option<PopoverFrame> {
        *self.applied.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn remember_frame(&self, frame: Option<PopoverFrame>) {
        *self.applied.lock().unwrap_or_else(|e| e.into_inner()) = frame;
    }

    /// Resize and reposition `window` to `frame` without touching level,
    /// activation, focus, or visibility. Returns whether the setters ran:
    /// an unchanged frame is skipped.
    fn resize_and_position(
        &self,
        window: &tauri::WebviewWindow,
        frame: PopoverFrame,
    ) -> Result<bool, String> {
        if popover_frame_unchanged(self.applied_frame(), frame) {
            return Ok(false);
        }
        let (x, y, width, height) = frame;
        // Never keep an authoritative old frame after a half-applied pair.
        self.remember_frame(None);
        crate::main_thread_watch::section("window.set_size", || {
            window.set_size(tauri::LogicalSize::new(width, height))
        })
        .map_err(|_| format!("{} window could not be sized", self.label))?;
        crate::main_thread_watch::section("window.set_position", || {
            window.set_position(tauri::LogicalPosition::new(x, y))
        })
        .map_err(|_| format!("{} window could not be positioned", self.label))?;
        self.remember_frame(Some(frame));
        Ok(true)
    }

    /// Resize and reposition to `frame` without touching level, activation,
    /// focus, or visibility. Used by popovers that reposition while already
    /// visible. `Ok(false)` means the frame was already applied.
    pub(crate) fn reposition(
        &self,
        app: &tauri::AppHandle,
        frame: PopoverFrame,
    ) -> Result<bool, String> {
        let window = self.window(app)?;
        self.resize_and_position(&window, frame)
    }

    /// Full show sequence: apply `frame` (skipped when unchanged), then make the
    /// window visible. The first show raises it above the menu bar as
    /// non-activating (see `set_window_level_and_activation`), disables focus,
    /// applies this spec's cursor-event pass-through, and orders it in. Later
    /// shows find it still ordered in and only reveal it.
    pub(crate) fn show(&self, app: &tauri::AppHandle, frame: PopoverFrame) -> Result<(), String> {
        let window = self.window(app)?;
        // Something else ordering the window out (app hide, a display change)
        // sends it back through the full sequence.
        let ordered_in =
            self.ordered_in.load(Ordering::SeqCst) && window.is_visible().unwrap_or(false);
        let plan = plan_show(
            ordered_in,
            self.is_presented(),
            self.applied_frame(),
            frame,
        );
        if plan.set_frame {
            self.resize_and_position(&window, frame)?;
        }
        if plan.order_in {
            crate::main_thread_watch::section("window.set_level_and_activation", || {
                set_window_level_and_activation(&window, ABOVE_MENU_BAR_LEVEL, true)
            });
            crate::main_thread_watch::section("window.set_focusable", || {
                window.set_focusable(false)
            })
            .map_err(|_| format!("{} focus mode could not be set", self.label))?;
            crate::main_thread_watch::section("window.set_ignore_cursor_events", || {
                window.set_ignore_cursor_events(self.ignore_cursor_events)
            })
            .map_err(|_| format!("{} pointer events could not be set", self.label))?;
            crate::main_thread_watch::section("window.show", || window.show())
                .map_err(|_| format!("{} window could not be shown", self.label))?;
            self.ordered_in.store(true, Ordering::SeqCst);
        } else if plan.reveal && !self.ignore_cursor_events {
            // A concealed window is always click-through; restore this spec's
            // pointer behaviour with the reveal.
            crate::main_thread_watch::section("window.set_ignore_cursor_events", || {
                window.set_ignore_cursor_events(false)
            })
            .map_err(|_| format!("{} pointer events could not be set", self.label))?;
        }
        if plan.reveal {
            queue_window_alpha(&window, 1.0);
        }
        self.presented.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// Conceal the window if it was shown: fully transparent and click-through,
    /// still ordered in. Missing or never-shown windows are not an error: both
    /// callers treat "already gone" as already hidden.
    pub(crate) fn hide(&self, app: &tauri::AppHandle) -> Result<(), String> {
        use tauri::Manager;
        self.presented.store(false, Ordering::SeqCst);
        if !self.ordered_in.load(Ordering::SeqCst) {
            return Ok(());
        }
        let Some(window) = app.get_webview_window(self.label) else {
            return Ok(());
        };
        crate::main_thread_watch::section("window.conceal", || {
            if !self.ignore_cursor_events {
                window
                    .set_ignore_cursor_events(true)
                    .map_err(|_| format!("{} pointer events could not be set", self.label))?;
            }
            queue_window_alpha(&window, 0.0);
            Ok(())
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: PopoverFrame = (300.0, 43.0, 460.0, 104.0);

    #[test]
    fn the_first_show_orders_the_window_in_and_applies_its_frame() {
        let plan = plan_show(false, false, None, FRAME);
        assert_eq!(
            plan,
            ShowPlan {
                set_frame: true,
                order_in: true,
                reveal: true
            }
        );
    }

    #[test]
    fn later_shows_only_reveal_the_window_that_is_still_ordered_in() {
        // Hidden between dictations (ordered in, transparent), same frame.
        let plan = plan_show(true, false, Some(FRAME), FRAME);
        assert_eq!(
            plan,
            ShowPlan {
                set_frame: false,
                order_in: false,
                reveal: true
            }
        );
    }

    #[test]
    fn a_show_with_a_new_frame_applies_it_before_the_reveal() {
        let moved = (300.0, 61.0, 460.0, 104.0);
        let plan = plan_show(true, false, Some(FRAME), moved);
        assert!(plan.set_frame && !plan.order_in && plan.reveal);
    }

    #[test]
    fn showing_an_already_presented_window_is_a_pure_reposition() {
        let plan = plan_show(true, true, Some(FRAME), FRAME);
        assert_eq!(
            plan,
            ShowPlan {
                set_frame: false,
                order_in: false,
                reveal: false
            }
        );
    }

    #[test]
    fn a_window_ordered_out_behind_our_back_is_ordered_in_again() {
        let plan = plan_show(false, true, Some(FRAME), FRAME);
        assert!(plan.order_in && !plan.reveal && !plan.set_frame);
    }

    #[test]
    fn frames_within_a_sub_point_rounding_error_are_unchanged() {
        assert!(!popover_frame_unchanged(None, FRAME));
        assert!(popover_frame_unchanged(Some(FRAME), FRAME));
        assert!(popover_frame_unchanged(
            Some(FRAME),
            (300.004, 43.0, 460.0, 103.996)
        ));
        assert!(!popover_frame_unchanged(
            Some(FRAME),
            (300.0, 43.0, 460.0, 100.0)
        ));
        assert!(!popover_frame_unchanged(
            Some(FRAME),
            (301.0, 43.0, 460.0, 104.0)
        ));
    }

    #[test]
    fn presentation_state_starts_hidden_and_not_ordered_in() {
        let spec = PopoverSpec::new("test", true);
        assert!(!spec.is_presented());
        assert_eq!(spec.applied_frame(), None);
        spec.remember_frame(Some(FRAME));
        assert_eq!(spec.applied_frame(), Some(FRAME));
    }
}
