use crate::{MutexExt, State};
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
    pub wing_w: f64,
}

/// The window frame `set_overlay_expanded` actually applied. Returned so the
/// frontend can treat the resolved value as an acknowledgment: CSS reveal only
/// starts once the native window is known to have grown to this size.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct AppliedSurface {
    pub window_w: f64,
    pub window_h: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DisplaySnapshot {
    pub(crate) notch_info: Option<(f64, f64)>,
    monitor_position: Option<(i32, i32)>,
    monitor_size: Option<(u32, u32)>,
    scale_factor: Option<f64>,
}

// Private geometry constants — the ONLY place these magic numbers live.
//
// The native window stays at the full active width (`notch_w + 2*WING`) so
// recording/processing indicators can use either wing immediately. The visual
// island is narrower only while truly idle and collapsed: `notch_w + WING`.
// Its left edge stays fixed, which tucks the empty right wing under the notch;
// hover grows the visible right edge back out without moving the mic wing.
//
// WING is the visible strip on each side of the physical notch. The frontend
// treats each wing as a fixed-width slot of this size and *centers* its content
// (icon / waveform) inside it — so content is not flush to the notch edge.
//   - left: red pulse / mic / spinner centered in the wing
//   - right: ~23px 7-bar waveform (7*2px + 6*1.5px) centered in the wing
// WING = 36 fits both with a little slack. Anything wider than a wing
// (recording timer, "Tap missed" label) renders below notch height instead.
const WING: f64 = 36.0;
const DROPDOWN_H: f64 = 44.0;
const FALLBACK_NOTCH_W: f64 = 80.0;
const FALLBACK_NOTCH_H: f64 = 37.0;
const MAX_REASONABLE_MENU_BAR_H: f64 = 128.0;
const MIN_VERTICAL_OFFSET: f64 = -12.0;
const MAX_VERTICAL_OFFSET: f64 = 12.0;
const POSITION_SETTLE_PROBE_MS: u64 = 50;
const POSITION_SETTLE_PROBES: usize = 3;

fn clamp_vertical_offset(offset: f64) -> f64 {
    if offset.is_finite() {
        offset.clamp(MIN_VERTICAL_OFFSET, MAX_VERTICAL_OFFSET)
    } else {
        0.0
    }
}

fn geometry_for(notch: Option<(f64, f64)>) -> OverlayGeometry {
    let (notch_w, notch_h) = notch.unwrap_or((FALLBACK_NOTCH_W, FALLBACK_NOTCH_H));
    let window_w = notch_w + 2.0 * WING;
    OverlayGeometry {
        window_w,
        collapsed_h: notch_h,
        expanded_h: notch_h + DROPDOWN_H,
        // Idle ends at the notch's right edge; active reveals the right wing.
        pill_idle_w: notch_w + WING,
        pill_active_w: window_w,
        pill_margin_idle: 0.0,
        pill_margin_active: 0.0,
        dropdown_h: DROPDOWN_H,
        wing_w: WING,
    }
}

fn applied_surface_for(g: &OverlayGeometry, expanded: bool) -> AppliedSurface {
    AppliedSurface {
        window_w: g.window_w,
        window_h: if expanded {
            g.expanded_h
        } else {
            g.collapsed_h
        },
    }
}

/// Resolve the overlay's obscured center width and collapsed height from one
/// native screen snapshot.
///
/// `safe_top` describes the physical notch on notched Mac displays. It is zero
/// on ordinary/external displays, where the actual menu-bar height is instead
/// the gap between the screen frame and its visible frame. Keeping those two
/// measurements separate avoids extending the synthetic island below a shorter
/// menu bar (the old fixed 37pt fallback did exactly that).
fn notch_info_from_screen_measurements(
    screen_w: f64,
    frame_top: f64,
    visible_frame_top: f64,
    safe_top: f64,
    auxiliary_left_w: f64,
    auxiliary_right_w: f64,
) -> (f64, f64) {
    let visible_top_gap = (frame_top - visible_frame_top).max(0.0);
    let measured_menu_bar_h = safe_top.max(visible_top_gap);
    let menu_bar_h = if measured_menu_bar_h.is_finite()
        && measured_menu_bar_h > 0.0
        && measured_menu_bar_h <= MAX_REASONABLE_MENU_BAR_H
    {
        measured_menu_bar_h
    } else {
        FALLBACK_NOTCH_H
    };

    let measured_notch_w = screen_w - auxiliary_left_w - auxiliary_right_w;
    let notch_w = if safe_top.is_finite()
        && safe_top > 0.0
        && measured_notch_w.is_finite()
        && measured_notch_w > 0.0
        && measured_notch_w < screen_w
    {
        measured_notch_w
    } else {
        FALLBACK_NOTCH_W
    };

    (notch_w, menu_bar_h)
}

fn centered_physical_position(
    monitor_position: (i32, i32),
    monitor_size: (u32, u32),
    scale_factor: f64,
    overlay_w: f64,
) -> (i32, i32) {
    let overlay_physical_w = overlay_w * scale_factor;
    let x = monitor_position.0 as f64 + (monitor_size.0 as f64 - overlay_physical_w) / 2.0;
    (x.round() as i32, monitor_position.1)
}

/// WindowServer applies frame changes asynchronously. Reading `outer_position`
/// immediately after `set_position` can return the previous frame, so position
/// telemetry samples on the main thread until the requested frame is observed
/// or a short bounded settle window expires.
#[cfg(target_os = "macos")]
async fn settled_outer_position(
    app: tauri::AppHandle,
    overlay: tauri::WebviewWindow,
    target: (i32, i32),
) -> Result<(i32, i32), String> {
    let mut last_observed = None;
    for _ in 0..POSITION_SETTLE_PROBES {
        tokio::time::sleep(std::time::Duration::from_millis(POSITION_SETTLE_PROBE_MS)).await;
        let (tx, rx) = tokio::sync::oneshot::channel();
        let window = overlay.clone();
        app.run_on_main_thread(move || {
            let result = window
                .outer_position()
                .map(|position| (position.x, position.y))
                .map_err(|error| error.to_string());
            let _ = tx.send(result);
        })
        .map_err(|error| error.to_string())?;
        let observed = tokio::time::timeout(std::time::Duration::from_secs(1), rx)
            .await
            .map_err(|_| "overlay position read timed out".to_string())?
            .map_err(|_| "overlay position read was dropped".to_string())??;
        last_observed = Some(observed);
        if observed == target {
            return Ok(observed);
        }
    }
    last_observed.ok_or_else(|| "overlay position was not observed".to_string())
}

/// Detect notch width and configure the overlay as a notch-level window.
/// Uses native NSScreen APIs — no subprocess needed.
#[cfg(target_os = "macos")]
pub(crate) fn detect_notch_info() -> Option<(f64, f64)> {
    // Returns (notch_or_synthetic_center_width, actual_menu_bar_height) in
    // logical points.
    use objc2_app_kit::NSScreen;
    use objc2_foundation::MainThreadMarker;

    // SAFETY: callers are Tauri's setup callback and the screen-change observer
    // registered on NSOperationQueue::mainQueue. MainThreadMarker requires
    // exactly that main-thread confinement.
    let mtm = unsafe { MainThreadMarker::new_unchecked() };
    // AppKit guarantees the first screen is the one containing the menu bar.
    // `mainScreen()` can instead follow the key window, which would make the
    // overlay jump to a secondary display after the main app window moves.
    let screen = NSScreen::screens(mtm).firstObject()?;
    let insets = screen.safeAreaInsets();
    let frame = screen.frame();
    let visible_frame = screen.visibleFrame();
    let left_w = screen.auxiliaryTopLeftArea().size.width;
    let right_w = screen.auxiliaryTopRightArea().size.width;
    let frame_top = frame.origin.y + frame.size.height;
    let visible_frame_top = visible_frame.origin.y + visible_frame.size.height;
    let info = notch_info_from_screen_measurements(
        frame.size.width,
        frame_top,
        visible_frame_top,
        insets.top,
        left_w,
        right_w,
    );
    tracing::info!(
        target: "system",
        "detect_notch_info: notch_w={}, menu_bar_h={}, safe_top={}, visible_top_gap={}, screen_w={}",
        info.0,
        info.1,
        insets.top,
        (frame_top - visible_frame_top).max(0.0),
        frame.size.width
    );
    Some(info)
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn detect_notch_info() -> Option<(f64, f64)> {
    None
}

pub(crate) fn capture_display_snapshot(app_handle: &tauri::AppHandle) -> DisplaySnapshot {
    let notch_info = detect_notch_info();
    let monitor = app_handle.primary_monitor().ok().flatten();
    DisplaySnapshot {
        notch_info,
        monitor_position: monitor.as_ref().map(|monitor| {
            let position = monitor.position();
            (position.x, position.y)
        }),
        monitor_size: monitor.as_ref().map(|monitor| {
            let size = monitor.size();
            (size.width, size.height)
        }),
        scale_factor: monitor.as_ref().map(|monitor| monitor.scale_factor()),
    }
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

    const SCREEN_CHANGE_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(125);
    let (notification_sender, notification_receiver) = std::sync::mpsc::channel::<()>();
    let worker_handle = app_handle.clone();
    std::thread::Builder::new()
        .name("murmur-screen-change".to_string())
        .spawn(move || {
            while notification_receiver.recv().is_ok() {
                while notification_receiver
                    .recv_timeout(SCREEN_CHANGE_DEBOUNCE)
                    .is_ok()
                {}
                let handle = worker_handle.clone();
                let schedule_handle = handle.clone();
                if let Err(error) = schedule_handle.run_on_main_thread(move || {
                    let snapshot = capture_display_snapshot(&handle);
                    let notch = snapshot.notch_info;
                    let changed = {
                        let state = handle.state::<State>();
                        let mut cached = state.display_snapshot.lock_or_recover();
                        if cached.as_ref() == Some(&snapshot) {
                            false
                        } else {
                            *cached = Some(snapshot.clone());
                            *state.notch_info.lock_or_recover() = notch;
                            true
                        }
                    };
                    tracing::info!(
                        target: "system",
                        changed,
                        "screen parameter notifications coalesced"
                    );
                    if !changed {
                        return;
                    }
                    if let Some(overlay) = handle.get_webview_window("overlay") {
                        position_overlay_default(&overlay, notch, "display_change");
                    }
                    let _ = handle.emit("overlay-geometry-changed", geometry_for(notch));
                }) {
                    tracing::warn!(
                        target: "system",
                        "screen change main-thread dispatch failed: {}",
                        error
                    );
                }
            }
        })
        .expect("screen change debounce worker must spawn");

    let block = block2::RcBlock::new(move |_notification: std::ptr::NonNull<NSNotification>| {
        let _ = notification_sender.send(());
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
/// Also prevents clicking the overlay from activating the app (which would
/// unhide the main window) — see `native_window::set_window_level_and_activation`,
/// shared with the transform review popover.
#[cfg(target_os = "macos")]
fn raise_window_above_menubar(overlay: &tauri::WebviewWindow) {
    super::native_window::set_window_level_and_activation(
        overlay,
        super::native_window::ABOVE_MENU_BAR_LEVEL,
        true,
    );
    // Raw NSWindow mutation must run on the main thread (macOS 26 hard-traps
    // off-main), same as `set_window_level_and_activation` (issue #325).
    let overlay = overlay.clone();
    let handle = overlay.app_handle().clone();
    if let Err(e) = handle.run_on_main_thread(move || {
        if let Ok(ptr) = overlay.ns_window() {
            let ns_window: &objc2_app_kit::NSWindow = unsafe { &*(ptr.cast()) };
            ns_window.setHasShadow(false);

            // Tauri's `visibleOnAllWorkspaces` only adds CanJoinAllSpaces.
            // Keep this status surface stationary across Mission Control,
            // allow it into full-screen spaces, and let it join every Stage
            // Manager application set. Without CanJoinAllApplications on
            // macOS 26, WindowServer can leave the overlay attached to
            // Murmur's previous set even though the window still exists.
            let mut behavior = ns_window.collectionBehavior();
            behavior |= objc2_app_kit::NSWindowCollectionBehavior::CanJoinAllSpaces;
            behavior |= objc2_app_kit::NSWindowCollectionBehavior::CanJoinAllApplications;
            behavior |= objc2_app_kit::NSWindowCollectionBehavior::Stationary;
            behavior |= objc2_app_kit::NSWindowCollectionBehavior::IgnoresCycle;
            behavior |= objc2_app_kit::NSWindowCollectionBehavior::FullScreenAuxiliary;
            ns_window.setCollectionBehavior(behavior);
        }
    }) {
        tracing::warn!(target: "system", "raise_window_above_menubar: run_on_main_thread failed: {}", e);
    }
}

/// Position and size the overlay to match the notch, anchored at the top of the screen.
/// The window is notch-height tall and wide enough for horizontal expansion.
/// Takes cached notch_info to avoid calling NSScreen APIs off the main thread.
#[cfg(target_os = "macos")]
pub(crate) fn position_overlay_default(
    overlay: &tauri::WebviewWindow,
    notch_info: Option<(f64, f64)>,
    reason: &'static str,
) {
    let g = geometry_for(notch_info);
    let overlay_w = g.window_w;
    let overlay_h = g.collapsed_h;
    tracing::info!(target: "system", "position_overlay_default: notch_info={:?}, overlay_w={}, overlay_h={}", notch_info, overlay_w, overlay_h);

    // Raise above the menu bar so the window can overlap the notch
    raise_window_above_menubar(overlay);

    // Target the primary/menu-bar display explicitly. `current_monitor()` is
    // determined by the overlay's old frame and can keep it stranded on a
    // disconnected or secondary display. Use physical coordinates so mixed
    // Retina/non-Retina layouts and non-zero monitor origins stay exact.
    let monitor = overlay
        .app_handle()
        .primary_monitor()
        .ok()
        .flatten()
        .or_else(|| overlay.current_monitor().ok().flatten());
    if let Some(monitor) = monitor {
        let size = monitor.size();
        let position = monitor.position();
        let monitor_x = position.x;
        let monitor_y = position.y;
        let sf = monitor.scale_factor();
        let physical_w = (overlay_w * sf).round().max(1.0) as u32;
        let physical_h = (overlay_h * sf).round().max(1.0) as u32;
        let (x, y) = centered_physical_position(
            (position.x, position.y),
            (size.width, size.height),
            sf,
            overlay_w,
        );

        if let Err(e) = overlay.set_size(tauri::PhysicalSize::new(physical_w, physical_h)) {
            tracing::warn!(target: "system", "position_overlay_default: set_size({}, {}) failed: {}", physical_w, physical_h, e);
        }
        tracing::info!(target: "system", "position_overlay_default: x={}, y={}, sf={}", x, y, sf);
        if let Err(e) = overlay.set_position(tauri::PhysicalPosition::new(x, y)) {
            tracing::warn!(target: "system", "position_overlay_default: set_position({}, {}) failed: {}", x, y, e);
        } else {
            let overlay = overlay.clone();
            let app = overlay.app_handle().clone();
            tauri::async_runtime::spawn(async move {
                match settled_outer_position(app, overlay, (x, y)).await {
                    Ok(actual) => tracing::info!(
                        target: "system",
                        event_code = "overlay.position_default",
                        reason,
                        target_x_physical = x,
                        target_y_physical = y,
                        actual_x_physical = actual.0,
                        actual_y_physical = actual.1,
                        matches_target = actual == (x, y),
                        monitor_x_physical = monitor_x,
                        monitor_y_physical = monitor_y,
                        scale_factor = sf,
                        window_w_logical = overlay_w,
                        window_h_logical = overlay_h,
                        "Overlay default position applied"
                    ),
                    Err(error) => tracing::warn!(
                        target: "system",
                        event_code = "overlay.position_read_failed",
                        reason,
                        target_x_physical = x,
                        target_y_physical = y,
                        error = %error,
                        "Overlay default position applied, but its resulting frame could not be read"
                    ),
                }
            });
        }
    } else {
        tracing::warn!(target: "system", "position_overlay_default: no monitor available, retaining configured position");
        if let Err(e) = overlay.set_size(tauri::LogicalSize::new(overlay_w, overlay_h)) {
            tracing::warn!(target: "system", "position_overlay_default: fallback set_size({}, {}) failed: {}", overlay_w, overlay_h, e);
        }
    }
}

/// Return the current overlay geometry so the frontend can size the island.
#[tauri::command]
pub fn get_overlay_geometry(state: tauri::State<'_, State>) -> OverlayGeometry {
    geometry_for(*state.notch_info.lock_or_recover())
}

/// Show the always-on-top macOS notch overlay window.
#[tauri::command]
pub fn show_overlay(app: tauri::AppHandle) -> Result<(), String> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = &app;
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        match app.get_webview_window("overlay") {
            Some(overlay) => {
                overlay.show().map_err(|e| e.to_string())?;
                let _ = overlay.set_ignore_cursor_events(false);
                // Tell the overlay it is visible so it can gate cursor polling.
                let _ = app.emit("overlay-visible-changed", true);
                Ok(())
            }
            None => {
                tracing::warn!(target: "system", "show_overlay: overlay window not found — skipping");
                Ok(())
            }
        }
    }
}

/// Resize the overlay for the hover dropdown and return the applied frame as an
/// acknowledgment. All dimensions come from `geometry_for()`, the same source as
/// `position_overlay_default`, so the collapsed size matches what `show_overlay`
/// set. Only the size changes — the window keeps its current calibrated top
/// edge, so the extra height grows downward.
///
/// Returning the `AppliedSurface` lets the expansion controller await this call
/// and start the CSS reveal only once the native window is known to have grown,
/// so the dropdown can never animate into a window that has not yet resized.
///
/// We resize on hover rather than pre-allocating a tall window because a
/// transparent overlay with cursor events enabled captures the mouse across its
/// whole frame, which would create a click dead-zone below the notch when idle.
#[tauri::command]
pub fn set_overlay_expanded(
    app: tauri::AppHandle,
    state: tauri::State<'_, State>,
    expanded: bool,
) -> Result<AppliedSurface, String> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = &app;
        // Off macOS the window is never resized, but the controller still needs
        // a resolved frame to treat as an ack. Report the geometry it would apply.
        let g = geometry_for(*state.notch_info.lock_or_recover());
        return Ok(applied_surface_for(&g, expanded));
    }

    #[cfg(target_os = "macos")]
    {
        let notch = *state.notch_info.lock_or_recover();
        match app.get_webview_window("overlay") {
            Some(overlay) => {
                let g = geometry_for(notch);
                let applied = applied_surface_for(&g, expanded);
                overlay
                    .set_size(tauri::LogicalSize::new(applied.window_w, applied.window_h))
                    .map_err(|e| e.to_string())?;
                Ok(applied)
            }
            None => {
                tracing::warn!(target: "system", "set_overlay_expanded: overlay window not found — skipping");
                Err("overlay window not found".to_string())
            }
        }
    }
}

/// Apply a user-calibrated vertical offset to the notch-anchored overlay.
///
/// The frontend persists the logical-point offset in its local settings and
/// reapplies it after launch/display changes. Keeping the native command
/// stateless avoids a second settings store while still ensuring the actual
/// NSWindow moves, rather than merely translating web content inside it.
#[tauri::command]
pub async fn set_overlay_vertical_offset(
    app: tauri::AppHandle,
    state: tauri::State<'_, State>,
    offset: f64,
) -> Result<(), String> {
    let offset = clamp_vertical_offset(offset);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (&app, &state, offset);
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        let overlay = app
            .get_webview_window("overlay")
            .ok_or_else(|| "overlay window not found".to_string())?;
        let g = geometry_for(*state.notch_info.lock_or_recover());
        let monitor = app
            .primary_monitor()
            .map_err(|error| error.to_string())?
            .or_else(|| overlay.current_monitor().ok().flatten())
            .ok_or_else(|| "no monitor available".to_string())?;
        let position = monitor.position();
        let size = monitor.size();
        let scale_factor = monitor.scale_factor();
        let (x, base_y) = centered_physical_position(
            (position.x, position.y),
            (size.width, size.height),
            scale_factor,
            g.window_w,
        );
        let monitor_x = position.x;
        let monitor_y = position.y;
        let y = base_y + (offset * scale_factor).round() as i32;
        let overlay_to_move = overlay.clone();
        let (tx, rx) = tokio::sync::oneshot::channel::<Result<(), String>>();
        app.run_on_main_thread(move || {
            let result = overlay_to_move
                .set_position(tauri::PhysicalPosition::new(x, y))
                .map_err(|error| error.to_string());
            let _ = tx.send(result);
        })
        .map_err(|error| error.to_string())?;
        tokio::time::timeout(std::time::Duration::from_secs(2), rx)
            .await
            .map_err(|_| "overlay position update timed out".to_string())?
            .map_err(|_| "overlay position update was dropped".to_string())??;

        match settled_outer_position(app, overlay, (x, y)).await {
            Ok(actual) => tracing::info!(
                target: "system",
                event_code = "overlay.position_offset_applied",
                offset_logical = offset,
                target_x_physical = x,
                target_y_physical = y,
                actual_x_physical = actual.0,
                actual_y_physical = actual.1,
                matches_target = actual == (x, y),
                monitor_x_physical = monitor_x,
                monitor_y_physical = monitor_y,
                scale_factor,
                "Overlay calibrated position applied"
            ),
            Err(error) => tracing::warn!(
                target: "system",
                event_code = "overlay.position_read_failed",
                offset_logical = offset,
                target_x_physical = x,
                target_y_physical = y,
                error = %error,
                "Overlay calibrated position applied, but its resulting frame could not be read"
            ),
        }
        Ok(())
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
        Some(overlay) => {
            overlay.hide().map_err(|e| e.to_string())?;
            // Tell the overlay it is hidden so it can stop cursor polling.
            let _ = app.emit("overlay-visible-changed", false);
            Ok(())
        }
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
        for g in [
            geometry_for(Some((185.0, 32.0))),
            geometry_for(Some((FALLBACK_NOTCH_W, 30.0))),
            geometry_for(None),
        ] {
            assert!(g.window_w >= g.pill_active_w + g.pill_margin_active);
            assert!(g.window_w >= g.pill_idle_w + g.pill_margin_idle);
            assert_eq!(g.expanded_h, g.collapsed_h + g.dropdown_h);
            assert!(g.pill_active_w >= g.pill_idle_w);
            assert!(g.wing_w > 0.0);
            assert!(g.window_w >= 2.0 * g.wing_w);
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
                g.wing_w,
            ),
            (257.0, 32.0, 76.0, 221.0, 257.0, 0.0, 0.0, 44.0, 36.0)
        );
    }

    #[test]
    fn external_display_uses_its_measured_menu_bar_height() {
        let info = notch_info_from_screen_measurements(2560.0, 1440.0, 1410.0, 0.0, 0.0, 0.0);
        assert_eq!(info, (FALLBACK_NOTCH_W, 30.0));

        let g = geometry_for(Some(info));
        assert_eq!(g.window_w, 152.0);
        assert_eq!(g.pill_idle_w, 116.0);
        assert_eq!(g.pill_active_w, 152.0);
        assert_eq!(g.collapsed_h, 30.0);
        assert_eq!(g.expanded_h, 74.0);
    }

    #[test]
    fn notched_display_prefers_safe_area_and_auxiliary_widths() {
        let info = notch_info_from_screen_measurements(1512.0, 982.0, 950.0, 32.0, 663.5, 663.5);
        assert_eq!(info, (185.0, 32.0));
    }

    #[test]
    fn invalid_native_measurements_fail_closed_to_geometry_defaults() {
        assert_eq!(
            notch_info_from_screen_measurements(2560.0, 1440.0, 1440.0, f64::NAN, 0.0, 0.0,),
            (FALLBACK_NOTCH_W, FALLBACK_NOTCH_H)
        );
    }

    #[test]
    fn physical_centering_includes_monitor_origin_and_scale() {
        assert_eq!(
            centered_physical_position((5120, 128), (2560, 1440), 1.0, 152.0),
            (6324, 128)
        );
        assert_eq!(
            centered_physical_position((0, 0), (5120, 2880), 2.0, 152.0),
            (2408, 0)
        );
    }

    #[test]
    fn vertical_calibration_offset_is_finite_and_bounded() {
        assert_eq!(clamp_vertical_offset(-13.0), MIN_VERTICAL_OFFSET);
        assert_eq!(clamp_vertical_offset(49.0), MAX_VERTICAL_OFFSET);
        assert_eq!(clamp_vertical_offset(7.5), 7.5);
        assert_eq!(clamp_vertical_offset(f64::NAN), 0.0);
        assert_eq!(clamp_vertical_offset(f64::INFINITY), 0.0);
    }

    #[test]
    fn complete_display_snapshot_detects_every_geometry_dimension() {
        let baseline = DisplaySnapshot {
            notch_info: Some((185.0, 32.0)),
            monitor_position: Some((0, 0)),
            monitor_size: Some((3024, 1964)),
            scale_factor: Some(2.0),
        };
        assert_eq!(baseline, baseline.clone());
        for changed in [
            DisplaySnapshot {
                notch_info: Some((184.0, 32.0)),
                ..baseline.clone()
            },
            DisplaySnapshot {
                monitor_position: Some((3024, 0)),
                ..baseline.clone()
            },
            DisplaySnapshot {
                monitor_size: Some((2560, 1440)),
                ..baseline.clone()
            },
            DisplaySnapshot {
                scale_factor: Some(1.0),
                ..baseline.clone()
            },
        ] {
            assert_ne!(baseline, changed);
        }
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

    #[test]
    fn applied_surface_tracks_notched_and_fallback_geometry_states() {
        let notched = geometry_for(Some((185.0, 32.0)));
        assert_eq!(
            applied_surface_for(&notched, false),
            AppliedSurface {
                window_w: 257.0,
                window_h: 32.0,
            }
        );
        assert_eq!(
            applied_surface_for(&notched, true),
            AppliedSurface {
                window_w: 257.0,
                window_h: 76.0,
            }
        );

        let fallback = geometry_for(None);
        assert_eq!(
            applied_surface_for(&fallback, false),
            AppliedSurface {
                window_w: 152.0,
                window_h: 37.0,
            }
        );
        assert_eq!(
            applied_surface_for(&fallback, true),
            AppliedSurface {
                window_w: 152.0,
                window_h: 81.0,
            }
        );
    }
}
