use super::model::Screen;
use gpui::Window;
use murmur_core::overlay_geometry::{OverlayFrame, notch_info_from_screen_measurements};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

fn raw(window: &Window) -> Option<*mut std::ffi::c_void> {
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::AppKit(handle) => Some(handle.ns_view.as_ptr()),
        _ => None,
    }
}
/// Must be called from GPUI's main-thread window update, never a worker.
#[derive(Clone, Copy, PartialEq)]
pub struct Surface {
    pub frame: OverlayFrame,
    pub floating: bool,
    pub hidden: bool,
    pub reduce_transparency: bool,
    pub increase_contrast: bool,
    pub dark: bool,
    pub surface_height: f64,
    pub radius: f64,
}
pub fn apply(window: &mut Window, surface: Surface, cx: &mut gpui::App) -> bool {
    let Surface {
        frame,
        floating,
        hidden,
        reduce_transparency,
        increase_contrast,
        dark,
        surface_height,
        radius,
    } = surface;
    #[cfg(target_os = "macos")]
    if let Some(raw) = raw(window) {
        // SAFETY: GPUI retains the view, the main-thread native transaction
        // rejects a missing window and updates frame/material together.
        let applied = unsafe {
            murmur_overlay_apply(
                raw,
                frame.x,
                frame.y,
                frame.width,
                frame.height,
                floating,
                hidden,
                reduce_transparency,
                increase_contrast,
                dark,
                radius,
                surface_height,
            )
        };
        if applied {
            // AppKit's resize callback is queued while this window is borrowed.
            // Refresh before revealing controls, so paint, hits and AX all use
            // the acknowledged size in this same turn, never the old viewport.
            window.bounds_changed(cx);
        }
        return applied;
    }
    // TestWindow / HeadlessWindow have no NSWindow and need their layout size
    // updated through GPUI itself. The native path receives its resize callback.
    window.resize(gpui::size(
        gpui::px(frame.width as f32),
        gpui::px(frame.height as f32),
    ));
    window.bounds_changed(cx);
    true
}

#[cfg(all(test, feature = "a11y-tests"))]
thread_local! {
    // Exercise native-sample decisions in TestWindow; real native query/geometry
    // is additionally exercised by the isolated NSWindow regression probe.
    pub(super) static TEST_POINTER_SAMPLE: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

pub fn pointer_inside(window: &Window) -> Option<bool> {
    #[cfg(all(test, feature = "a11y-tests"))]
    if let Some(inside) = TEST_POINTER_SAMPLE.get() {
        return Some(inside);
    }

    #[cfg(target_os = "macos")]
    if let Some(raw) = raw(window) {
        // SAFETY: main-thread query of the retained view's presentation rect.
        return Some(unsafe { murmur_overlay_pointer_inside(raw) });
    }
    None
}
pub fn announce(window: &Window, label: &'static str) {
    #[cfg(target_os = "macos")]
    if let Some(raw) = raw(window) {
        let message = std::ffi::CString::new(label).expect("static state label");
        // SAFETY: same main-thread/lifetime rule as apply; string is borrowed
        // only for this synchronous call and contains no private content.
        unsafe { murmur_overlay_announce(raw, message.as_ptr()) };
    }
}
pub fn screen() -> Option<Screen> {
    #[cfg(target_os = "macos")]
    {
        let mut v = [0.; 15];
        // SAFETY: caller is GPUI main; buffer has the fifteen documented slots.
        if unsafe { murmur_overlay_screen(v.as_mut_ptr()) } {
            return Some(Screen {
                notch: Some(notch_info_from_screen_measurements(
                    v[2] - v[0],
                    -v[1],
                    -v[5],
                    v[8],
                    v[9],
                    v[10],
                )),
                hidden_dock: (v[12] > 0.).then_some(murmur_core::overlay_geometry::HiddenDock {
                    tile: v[13],
                    magnified: (v[14] > 0.).then_some(v[14]),
                }),
                scale_factor: v[11],
                bounds: (v[0], v[1], v[2], v[3]),
                work: (v[4], v[5], v[6], v[7]),
                ..Screen::default()
            });
        }
    }
    None
}
#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn murmur_overlay_apply(
        view: *mut std::ffi::c_void,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        floating: bool,
        hidden: bool,
        reduced_transparency: bool,
        high_contrast: bool,
        dark: bool,
        radius: f64,
        surface_height: f64,
    ) -> bool;
    fn murmur_overlay_screen(out: *mut f64) -> bool;
    fn murmur_overlay_pointer_inside(view: *mut std::ffi::c_void) -> bool;
    fn murmur_overlay_announce(view: *mut std::ffi::c_void, message: *const std::ffi::c_char);
}

/// Actual loaded-image inventory for this process; never enumerate other apps.
pub fn webkit_images() -> u32 {
    #[cfg(target_os = "macos")]
    // SAFETY: dyld's read-only image list requires no GUI or permission.
    unsafe {
        murmur_overlay_webkit_images()
    }
    #[cfg(not(target_os = "macos"))]
    {
        0
    }
}
#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn murmur_overlay_webkit_images() -> u32;
}

/// Refit the mask from the same main-thread presentation frame as GPUI's
/// value transition; window geometry still follows the acknowledged frame.
pub fn refit_material(
    window: &Window,
    width: gpui::Pixels,
    height: gpui::Pixels,
    radius: f32,
    floating: bool,
) {
    #[cfg(target_os = "macos")]
    if let Some(raw) = raw(window) {
        // SAFETY: view lives for this window update; main-thread-only function.
        unsafe {
            murmur_overlay_refit_hud(
                raw,
                f32::from(width) as f64,
                f32::from(height) as f64,
                radius as f64,
                floating,
            )
        };
    }
}
#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn murmur_overlay_refit_hud(
        view: *mut std::ffi::c_void,
        width: f64,
        height: f64,
        radius: f64,
        floating: bool,
    );
}

/// Install/check the QA-only policy before constructing NSApplication or windows.
#[cfg(feature = "gallery")]
pub fn start_webkit_policy() {
    #[cfg(target_os = "macos")]
    // SAFETY: the native constructor registered the observer before main.
    assert!(
        unsafe { murmur_webkit_start() },
        "nonzero WebKit startup/linkage baseline"
    );
}

/// Print backward-compatible counts and fail closed on app/unknown loads.
/// Additional load records contain image paths and symbols only, never AX data.
#[cfg(feature = "gallery")]
pub fn assert_webkit_policy(prefix: &str) {
    #[cfg(target_os = "macos")]
    {
        let prefix = std::ffi::CString::new(prefix).expect("inventory prefix");
        // SAFETY: borrowed string; native snapshot synchronizes with dyld.
        assert!(
            unsafe { murmur_webkit_inventory(prefix.as_ptr()) },
            "unattributed or app-linked WebKit"
        );
    }
}

#[cfg(all(target_os = "macos", feature = "gallery"))]
unsafe extern "C" {
    fn murmur_webkit_start() -> bool;
    fn murmur_webkit_inventory(prefix: *const std::ffi::c_char) -> bool;
}
