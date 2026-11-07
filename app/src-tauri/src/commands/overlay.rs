use crate::events::EventsExt;
use crate::host::ServiceAccess;
use crate::{MutexExt, State};
use murmur_core::events::{EventKind, EventSink};
use murmur_core::overlay_geometry::{
    animation_surface, applied_overlay_position, applied_surface_for, centered_logical_position,
    clamp_vertical_offset, collapsed_frame, dictation_preview_fit_for_frame, frame_unchanged,
    geometry_for_layout, notch_info_from_screen_measurements, overlay_click_through,
    pill_logical_position, pill_work_bottom, resized_logical_position, usable_top,
    DOCK_DEFAULT_TILE, DOCK_MAX_TILE, DROPDOWN_H,
};
pub(crate) use murmur_core::overlay_geometry::{
    preview_fallback_inset, DisplaySnapshot, HiddenDock, NotchInfo, OverlayCalibrationState,
    OverlayFrame, OverlayLayout, PopoverWorkArea,
};
pub use murmur_core::overlay_geometry::{
    AppliedOverlayPosition, AppliedSurface, OverlayActivity, OverlayContent, OverlayGeometry,
    OverlayPlacement,
};
#[cfg(feature = "tauri-host")]
use tauri::Manager;

// Native geometry is the only surface-size contract. The idle notch island
// matches the notch; an active island reveals a 28pt wing on either side inside
// the menu-bar band (status dot left, level bars right). Flat displays use a
// measured-height 185pt pretend notch. The floating pill is a bottom-centre
// capsule whose window never resizes between idle, recording and processing.
#[cfg(target_os = "macos")]
const OVERLAY_MATERIAL: &str = crate::commands::overlay_material::OVERLAY_LABEL;
const POSITION_SETTLE_PROBE_MS: u64 = 50;
const POSITION_SETTLE_PROBES: usize = 3;

/// The floating pill's geometry for its native material, resolved from the
/// current display for `expanded`/`content`. Placement comes from the caller:
/// the material only exists while the pill is the selected placement.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn floating_pill_geometry(
    app: &crate::host::Context,
    content: OverlayContent,
    expanded: bool,
) -> OverlayGeometry {
    let state = app.services();
    let notch = *state.notch_info.lock_or_recover();
    let mut layout = *state.overlay_layout.lock_or_recover();
    layout.placement = OverlayPlacement::FloatingPill;
    layout.expanded = expanded;
    geometry_for_layout(notch, content, layout)
}

/// True while the stored layout places the overlay as the floating pill.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn floating_pill_placed(app: &crate::host::Context) -> bool {
    app.services().overlay_layout.lock_or_recover().placement == OverlayPlacement::FloatingPill
}

/// Size preferences of the Dock when it auto-hides along the bottom edge. A
/// visible Dock is already excluded from the work area, and a side Dock never
/// covers the bottom-centre pill.
#[cfg(target_os = "macos")]
fn hidden_bottom_dock_tile() -> Option<HiddenDock> {
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::boolean::CFBoolean;
    use core_foundation::number::CFNumber;
    use core_foundation::string::{CFString, CFStringRef};

    extern "C" {
        fn CFPreferencesCopyAppValue(key: CFStringRef, app: CFStringRef)
            -> *const std::ffi::c_void;
        fn CFPreferencesAppSynchronize(app: CFStringRef) -> u8;
    }
    let domain = CFString::from_static_string("com.apple.dock");
    let read = |key: &'static str| -> Option<CFType> {
        let key = CFString::from_static_string(key);
        // SAFETY: both arguments are live CFStrings; the copy rule hands us
        // ownership of a non-null result.
        let value = unsafe {
            CFPreferencesCopyAppValue(key.as_concrete_TypeRef(), domain.as_concrete_TypeRef())
        };
        (!value.is_null()).then(|| unsafe { CFType::wrap_under_create_rule(value) })
    };
    // SAFETY: `domain` is a live CFString. Picks up Dock changes made since launch.
    unsafe { CFPreferencesAppSynchronize(domain.as_concrete_TypeRef()) };
    let autohide = read("autohide")
        .and_then(|v| {
            v.downcast::<CFBoolean>().map(bool::from).or_else(|| {
                v.downcast::<CFNumber>()
                    .and_then(|n| n.to_i64())
                    .map(|n| n != 0)
            })
        })
        .unwrap_or(false);
    let bottom = read("orientation")
        .and_then(|v| v.downcast::<CFString>())
        .is_none_or(|o| o == "bottom");
    let number = |key| {
        read(key)
            .and_then(|v| v.downcast::<CFNumber>())
            .and_then(|n| n.to_f64())
            .filter(|n| n.is_finite())
    };
    let magnification = read("magnification")
        .and_then(|v| {
            v.downcast::<CFBoolean>().map(bool::from).or_else(|| {
                v.downcast::<CFNumber>()
                    .and_then(|n| n.to_i64())
                    .map(|n| n != 0)
            })
        })
        .unwrap_or(false);
    (autohide && bottom).then(|| HiddenDock {
        tile: number("tilesize").unwrap_or(DOCK_DEFAULT_TILE),
        // An absent `largesize` means the Dock's own maximum.
        magnified: magnification.then(|| number("largesize").unwrap_or(DOCK_MAX_TILE)),
    })
}

#[cfg(not(target_os = "macos"))]
fn hidden_bottom_dock_tile() -> Option<HiddenDock> {
    None
}

/// Logical bottom edge the pill rests on for `monitor`. The live Dock limit
/// comes from `dock_watch` and the fallback preference from the cached display
/// snapshot, so resizes never query the Dock.
#[cfg(target_os = "macos")]
fn pill_bottom_for(app: &crate::host::Context, monitor: &crate::host::Monitor) -> f64 {
    let sf = monitor.scale_factor();
    let area = monitor.work_area();
    let display = pill_display(
        (monitor.position().x, monitor.position().y),
        (monitor.size().width, monitor.size().height),
        (area.position.y, area.size.height),
        sf,
    );
    let hidden_dock_tile = app
        .services()
        .display_snapshot
        .lock_or_recover()
        .as_ref()
        .and_then(|snapshot| snapshot.hidden_dock_tile);
    pill_work_bottom(
        display.bottom,
        display.work_bottom,
        hidden_dock_tile,
        crate::dock_watch::current_limit(display),
    )
}

/// A monitor's bounds and work-area bottom in global logical points, from
/// physical pixels. Shared by placement and `dock_watch` so their displays
/// compare equal.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn pill_display(
    (x, y): (i32, i32),
    (width, height): (u32, u32),
    (work_y, work_h): (i32, u32),
    scale: f64,
) -> crate::dock_watch::PillDisplay {
    crate::dock_watch::PillDisplay {
        left: x as f64 / scale,
        top: y as f64 / scale,
        right: (x as f64 + width as f64) / scale,
        bottom: (y as f64 + height as f64) / scale,
        work_bottom: (work_y as f64 + work_h as f64) / scale,
    }
}

/// Where `dock_watch` should look: the pill's display while the pill placement
/// is active, from the cached display snapshot. Safe off the main thread.
#[cfg(target_os = "macos")]
#[cfg(feature = "tauri-host")]
pub(crate) fn dock_watch_context(
    app: &crate::host::Context,
) -> Option<crate::dock_watch::WatchContext> {
    let state = app.services();
    let layout = *state.overlay_layout.lock_or_recover();
    if layout.placement != OverlayPlacement::FloatingPill {
        return None;
    }
    let snapshot = state.display_snapshot.lock_or_recover().clone()?;
    let scale = snapshot
        .scale_factor
        .filter(|s| s.is_finite() && *s > 0.0)?;
    let (_, work_y, _, work_h) = snapshot.monitor_work_area?;
    Some(crate::dock_watch::WatchContext {
        display: pill_display(
            snapshot.monitor_position?,
            snapshot.monitor_size?,
            (work_y, work_h),
            scale,
        ),
        visible: layout.activity != OverlayActivity::Idle,
    })
}

/// Re-rest the pill after the live Dock limit moved. Dispatched to the main
/// thread; keeps the current size so a hover card stays bottom-anchored.
#[cfg(target_os = "macos")]
#[cfg(feature = "tauri-host")]
pub(crate) fn follow_dock(app: &crate::host::Context) {
    let handle = app.clone();
    let dispatched = app.run_on_main_thread(move || {
        let state = handle.services();
        let layout = *state.overlay_layout.lock_or_recover();
        if layout.placement != OverlayPlacement::FloatingPill {
            return;
        }
        let Some(frame) = *state.overlay_frame.lock_or_recover() else {
            return;
        };
        let (Some(overlay), Ok(Some(monitor))) = (
            handle.get_webview_window("overlay"),
            handle.primary_monitor(),
        ) else {
            return;
        };
        let sf = monitor.scale_factor();
        let area = monitor.work_area();
        let notch = *state.notch_info.lock_or_recover();
        let left = area.position.x as f64 / sf;
        let top = usable_top(
            monitor.position().y as f64 / sf,
            area.position.y as f64 / sf,
            notch,
        );
        let (_, y) = pill_logical_position(
            (
                left,
                top,
                left + area.size.width as f64 / sf,
                pill_bottom_for(&handle, &monitor),
            ),
            (frame.width, frame.height),
        );
        let target = OverlayFrame { y, ..frame };
        if frame_unchanged(Some(frame), target) {
            return;
        }
        *state.overlay_frame.lock_or_recover() = None;
        if let Err(error) = overlay.set_position(crate::host::LogicalPosition::new(frame.x, y)) {
            // The window did not move: keep its frame so the next Dock change
            // can still follow.
            remember_overlay_frame(&handle, frame);
            tracing::warn!(target: "system", "follow_dock: set_position failed: {}", error);
            return;
        }
        remember_overlay_frame(&handle, target);
        follow_dictation_preview(&handle);
    });
    if let Err(error) = dispatched {
        tracing::warn!(target: "system", "follow_dock: main-thread dispatch failed: {}", error);
    }
}

/// Re-read the Dock preference off the main thread and fold it into the cached
/// snapshot. Resizing the Dock changes no display geometry and posts none of
/// the placement triggers, so every pill resize samples it here first.
#[cfg(target_os = "macos")]
fn refresh_cached_dock_tile(app: &crate::host::Context) {
    let tile = hidden_bottom_dock_tile();
    if let Some(snapshot) = app.services().display_snapshot.lock_or_recover().as_mut() {
        snapshot.hidden_dock_tile = tile;
    }
}

/// Actual-frame placement used only by the click-through live dictation
/// preview. It follows calibrated overlay movement without changing the
/// overlay controls under the pointer.
///
/// The window anchors to the collapsed surface and reserves `DROPDOWN_H` of
/// transparent room beyond the card. Hover expansion never moves it: the
/// webview slides the card into that room in step with the island, so a
/// delayed native shrink can no longer leave a gap and then snap.
pub(crate) fn dictation_preview_fit(
    app: &crate::host::Context,
    monitor: &crate::host::Monitor,
    preview_width: f64,
    preview_height: f64,
) -> Option<(f64, f64, f64)> {
    let primary = app.primary_monitor().ok().flatten()?;
    if primary.position() != monitor.position() || primary.size() != monitor.size() {
        return None;
    }
    let state = app.try_services()?;
    let scale = monitor.scale_factor();
    let area = monitor.work_area();
    let work_area = PopoverWorkArea {
        left: area.position.x as f64 / scale,
        right: (area.position.x as f64 + area.size.width as f64) / scale,
        top: area.position.y as f64 / scale,
        bottom: (area.position.y as f64 + area.size.height as f64) / scale,
    };
    let frame = *state.overlay_frame.lock_or_recover();
    let mut layout = *state.overlay_layout.lock_or_recover();
    let above = layout.placement == OverlayPlacement::FloatingPill;
    layout.expanded = false;
    let notch = *state.notch_info.lock_or_recover();
    let collapsed_h = geometry_for_layout(notch, OverlayContent::Controls, layout).collapsed_h;
    dictation_preview_fit_for_frame(
        work_area,
        (preview_width, preview_height + DROPDOWN_H),
        frame.map(|frame| collapsed_frame(frame, collapsed_h, above)),
        above,
    )
}

#[cfg(target_os = "macos")]
fn remember_overlay_frame(app: &crate::host::Context, frame: OverlayFrame) {
    *app.services().overlay_frame.lock_or_recover() = Some(frame);
}

#[cfg(target_os = "macos")]
fn remember_overlay_position(
    app: &crate::host::Context,
    frame: OverlayFrame,
    applied: AppliedOverlayPosition,
) {
    let state = app.services();
    *state.overlay_calibration.lock_or_recover() = OverlayCalibrationState {
        requested_offset: applied.requested_offset,
        applied_offset: applied.applied_offset,
    };
    *state.overlay_frame.lock_or_recover() = Some(frame);
    let _ = app
        .events()
        .publish(EventKind::OverlayPositionApplied, applied);
}

#[cfg(target_os = "macos")]
fn follow_dictation_preview(app: &crate::host::Context) {
    crate::commands::dictation_preview::follow_overlay_if_visible(app);
}

/// WindowServer applies frame changes asynchronously. Reading `outer_position`
/// immediately after `set_position` can return the previous frame, so position
/// telemetry samples on the main thread until the requested frame is observed
/// or a short bounded settle window expires.
#[cfg(target_os = "macos")]
async fn settled_outer_position(
    app: crate::host::Context,
    overlay: crate::host::Window,
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

/// Detect notch width and the menu-bar band centre, and configure the overlay
/// as a notch-level window. Uses native NSScreen APIs — no subprocess needed.
#[cfg(target_os = "macos")]
pub(crate) fn detect_notch_info() -> Option<NotchInfo> {
    use objc2_app_kit::NSScreen;
    use objc2_foundation::{MainThreadMarker, NSNumber, NSString};

    // SAFETY: callers are Tauri's setup callback and the placement refresh
    // dispatched through `run_on_main_thread`. MainThreadMarker requires
    // exactly that main-thread confinement.
    let mtm = unsafe { MainThreadMarker::new_unchecked() };
    // Match by CoreGraphics display ID, which also backs Tauri's primary
    // monitor. Resolution matching is ambiguous with identical externals and
    // `mainScreen()` can follow the key window instead of the menu-bar display.
    let screens = NSScreen::screens(mtm);
    let main_display_id = core_graphics::display::CGDisplay::main().id;
    let screen_number = NSString::from_str("NSScreenNumber");
    let matched = screens.iter().find(|screen| {
        screen
            .deviceDescription()
            .objectForKey(&screen_number)
            .and_then(|value| {
                value
                    .downcast_ref::<NSNumber>()
                    .map(|number| number.unsignedIntValue())
            })
            == Some(main_display_id)
    });
    if matched.is_none() {
        tracing::warn!(target: "system", "main display ID not found in AppKit screens; using menu-bar screen order");
    }
    let screen = matched.or_else(|| screens.firstObject())?;
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
        "detect_notch_info: notch_w={}, menu_bar_h={}, safe_top={}, visible_top_gap={}, display_left={}, screen_w={}",
        info.width,
        info.height,
        insets.top,
        (frame_top - visible_frame_top).max(0.0),
        frame.origin.x,
        frame.size.width,
    );
    Some(info)
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn detect_notch_info() -> Option<NotchInfo> {
    None
}

/// Capture fresh notch and monitor measurements. Must run on the main thread.
pub(crate) fn capture_display_snapshot(app_handle: &crate::host::Context) -> DisplaySnapshot {
    let monitor = app_handle.primary_monitor().ok().flatten();
    DisplaySnapshot {
        notch_info: detect_notch_info(),
        monitor_position: monitor.as_ref().map(|monitor| {
            let position = monitor.position();
            (position.x, position.y)
        }),
        monitor_size: monitor.as_ref().map(|monitor| {
            let size = monitor.size();
            (size.width, size.height)
        }),
        monitor_work_area: monitor.as_ref().map(|monitor| {
            let area = monitor.work_area();
            (
                area.position.x,
                area.position.y,
                area.size.width,
                area.size.height,
            )
        }),
        scale_factor: monitor.as_ref().map(|monitor| monitor.scale_factor()),
        hidden_dock_tile: hidden_bottom_dock_tile(),
    }
}

/// What prompted a placement refresh. Every trigger re-resolves the same state
/// together: menu-bar visibility and height, placement mode, native window
/// size, and the geometry emitted to the frontend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlacementTrigger {
    #[cfg(feature = "tauri-host")]
    Startup,
    #[cfg(feature = "tauri-host")]
    DisplayChange,
    /// An app activation can hide or restore the menu bar on the current
    /// Space. It only re-measures; the centred x never depends on the app.
    FrontmostApp,
    SpaceChange,
    /// A delayed re-check while the menu bar animates in or out.
    #[cfg(feature = "tauri-host")]
    Settle,
}

#[cfg(feature = "tauri-host")]
impl PlacementTrigger {
    fn as_str(self) -> &'static str {
        match self {
            Self::Startup => "startup_refresh",
            Self::DisplayChange => "display_change",
            Self::FrontmostApp => "frontmost_app_change",
            Self::SpaceChange => "space_change",
            Self::Settle => "placement_settle",
        }
    }

    /// Shared bounded ladder, each delay measured from the previous refresh.
    fn settle_delays(self) -> &'static [std::time::Duration] {
        const LADDER: &[std::time::Duration] = &[
            PLACEMENT_SETTLE,
            std::time::Duration::from_millis(1500),
            std::time::Duration::from_millis(3000),
        ];
        match self {
            Self::FrontmostApp | Self::SpaceChange | Self::Startup | Self::DisplayChange => LADDER,
            Self::Settle => &[],
        }
    }
}

/// Burst coalescing: a Space animation or display reconfiguration yields many
/// notifications; one refresh runs after they go quiet, or after the cap.
#[cfg(feature = "tauri-host")]
const PLACEMENT_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(125);
#[cfg(feature = "tauri-host")]
const PLACEMENT_MAX_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(1000);
#[cfg(feature = "tauri-host")]
const PLACEMENT_SETTLE: std::time::Duration = std::time::Duration::from_millis(600);

static PLACEMENT_TRIGGERS: std::sync::OnceLock<std::sync::mpsc::Sender<PlacementTrigger>> =
    std::sync::OnceLock::new();

/// Ask the placement worker to re-resolve the overlay placement. Cheap and
/// non-blocking, so notification observers can call it directly. A no-op until
/// the worker has started.
pub(crate) fn request_placement_refresh(trigger: PlacementTrigger) {
    if let Some(sender) = PLACEMENT_TRIGGERS.get() {
        let _ = sender.send(trigger);
    }
}

/// Re-measure and apply the whole placement on the main thread.
#[cfg(target_os = "macos")]
#[cfg(feature = "tauri-host")]
fn refresh_placement(app_handle: &crate::host::Context, trigger: PlacementTrigger) {
    let handle = app_handle.clone();
    if let Err(error) = app_handle.run_on_main_thread(move || {
        apply_placement(&handle, trigger);
    }) {
        tracing::warn!(target: "system", "overlay placement main-thread dispatch failed: {}", error);
    }
}

/// Main-thread half of `refresh_placement`: one snapshot, one resolved mode.
/// Repositions, resizes and notifies the frontend only when it changed.
#[cfg(target_os = "macos")]
#[cfg(feature = "tauri-host")]
fn apply_placement(handle: &crate::host::Context, trigger: PlacementTrigger) {
    let state = handle.services();
    let previous = state.display_snapshot.lock_or_recover().clone();
    let snapshot = capture_display_snapshot(handle);
    let notch = snapshot.notch_info;
    let changed = previous.as_ref() != Some(&snapshot);
    let dock_only = previous.as_ref().is_some_and(|previous| {
        DisplaySnapshot {
            hidden_dock_tile: snapshot.hidden_dock_tile,
            ..previous.clone()
        } == snapshot
    });
    let display_left = snapshot
        .monitor_position
        .zip(snapshot.scale_factor)
        .map(|((x, _), scale)| x as f64 / scale);
    let display_w = snapshot
        .monitor_size
        .zip(snapshot.scale_factor)
        .map(|((width, _), scale)| width as f64 / scale);
    if changed {
        *state.display_snapshot.lock_or_recover() = Some(snapshot);
        *state.notch_info.lock_or_recover() = notch;
    }
    tracing::info!(
        target: "system",
        event_code = "overlay.placement",
        trigger = trigger.as_str(),
        physical_notch = notch.map(|info| info.physical_notch),
        menu_bar_visible = notch.map(|info| info.menu_bar_visible),
        display_left,
        display_w,
        changed,
        "overlay placement refreshed"
    );
    if !changed {
        return;
    }
    let layout = *state.overlay_layout.lock_or_recover();
    // The Dock only moves the bottom pill. A Dock-only change must not
    // collapse an open notch island whose geometry is unchanged.
    if dock_only && !geometry_for_layout(notch, OverlayContent::Controls, layout).floating {
        return;
    }
    if let Some(overlay) = handle.get_webview_window("overlay") {
        position_overlay_default(&overlay, notch, trigger.as_str());
    }
    // The frontend treats this event as authoritative for expansion: it
    // collapses and re-reads geometry. Native placement retained the effective
    // calibration above, including an active unsaved draft.
    let layout = *state.overlay_layout.lock_or_recover();
    let _ = handle.events().publish(
        EventKind::OverlayGeometryChanged,
        geometry_for_layout(notch, OverlayContent::Controls, layout),
    );
}

/// Start the placement worker: it owns debouncing and sequencing for every
/// trigger so refreshes never overlap or thrash.
#[cfg(target_os = "macos")]
#[cfg(feature = "tauri-host")]
fn start_placement_worker(app_handle: crate::host::Context) {
    use std::sync::mpsc::RecvTimeoutError;
    use std::time::Instant;

    let (sender, receiver) = std::sync::mpsc::channel::<PlacementTrigger>();
    if PLACEMENT_TRIGGERS.set(sender).is_err() {
        return;
    }
    std::thread::Builder::new()
        .name("murmur-overlay-placement".to_string())
        .spawn(move || {
            let mut carried: Option<PlacementTrigger> = None;
            loop {
                let mut trigger = match carried.take() {
                    Some(trigger) => trigger,
                    None => match receiver.recv() {
                        Ok(trigger) => trigger,
                        Err(_) => return,
                    },
                };
                let burst_started = Instant::now();
                while let Some(remaining) =
                    PLACEMENT_MAX_DEBOUNCE.checked_sub(burst_started.elapsed())
                {
                    match receiver.recv_timeout(remaining.min(PLACEMENT_DEBOUNCE)) {
                        Ok(next) => trigger = next,
                        Err(RecvTimeoutError::Timeout) => break,
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
                refresh_placement(&app_handle, trigger);
                // A new trigger supersedes the remaining re-checks; it brings
                // its own.
                for &delay in trigger.settle_delays() {
                    match receiver.recv_timeout(delay) {
                        Ok(next) => {
                            carried = Some(next);
                            break;
                        }
                        Err(RecvTimeoutError::Timeout) => {
                            refresh_placement(&app_handle, PlacementTrigger::Settle);
                        }
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
            }
        })
        .expect("overlay placement worker must spawn");
}

/// Start the placement worker and subscribe to macOS display configuration
/// changes (plug/unplug monitor, lid open/close). Space changes arrive through
/// `frontmost::register_delivery_transition_observers`.
#[cfg(target_os = "macos")]
#[cfg(feature = "tauri-host")]
pub(crate) fn register_screen_change_observer(app_handle: crate::host::Context) {
    use objc2_foundation::{
        NSNotification, NSNotificationCenter, NSNotificationName, NSOperationQueue,
    };

    let notification_name =
        NSNotificationName::from_str("NSApplicationDidChangeScreenParametersNotification");

    start_placement_worker(app_handle.clone());
    let context_app = app_handle.clone();
    let follow_app = app_handle.clone();
    crate::dock_watch::start(
        move || dock_watch_context(&context_app),
        move || follow_dock(&follow_app),
    );

    let block = block2::RcBlock::new(move |_notification: std::ptr::NonNull<NSNotification>| {
        request_placement_refresh(PlacementTrigger::DisplayChange);
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
#[cfg(feature = "tauri-host")]
pub(crate) fn register_screen_change_observer(_app_handle: crate::host::Context) {}

/// Raise the overlay window above the menu bar so it overlaps the notch.
/// Also prevents clicking the overlay from activating the app (which would
/// unhide the main window) — see `native_window::set_window_level_and_activation`,
/// shared with the transform review popover.
#[cfg(target_os = "macos")]
#[cfg(feature = "tauri-host")]
fn raise_window_above_menubar(overlay: &crate::host::Window) {
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
#[cfg(feature = "tauri-host")]
pub(crate) fn position_overlay_default(
    overlay: &crate::host::Window,
    notch_info: Option<NotchInfo>,
    reason: &'static str,
) {
    let state = overlay.app_handle().services();
    let layout = {
        let mut layout = state.overlay_layout.lock_or_recover();
        layout.expanded = false;
        *layout
    };
    let g = geometry_for_layout(notch_info, OverlayContent::Controls, layout);
    let overlay_w = g.window_w;
    let overlay_h = g.collapsed_h;
    tracing::info!(target: "system", "position_overlay_default: notch_info={:?}, overlay_w={}, overlay_h={}", notch_info, overlay_w, overlay_h);

    // Raise above the menu bar so the window can overlap the notch
    raise_window_above_menubar(overlay);

    // Target the primary/menu-bar display explicitly. `current_monitor()` is
    // determined by the overlay's old frame and can keep it stranded on a
    // disconnected or secondary display. Pass logical coordinates to Tao:
    // physical inputs would be divided by the window's stale scale during a
    // 1x/2x display move.
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
        let (centered_x, centered_y) = centered_logical_position(
            (position.x, position.y),
            (size.width, size.height),
            sf,
            overlay_w,
        );
        let area = monitor.work_area();
        let work_left = area.position.x as f64 / sf;
        let work_top = usable_top(centered_y, area.position.y as f64 / sf, notch_info);
        let work_right = work_left + area.size.width as f64 / sf;
        let calibration = *overlay
            .app_handle()
            .services()
            .overlay_calibration
            .lock_or_recover();
        let requested_offset = calibration.requested_offset;
        tracing::debug!(
            target: "system",
            requested_offset,
            previous_applied_offset = calibration.applied_offset,
            "re-resolving overlay calibration for native placement"
        );
        let applied_position = applied_overlay_position(requested_offset, g.floating);
        let (x, y) = if g.floating {
            pill_logical_position(
                (
                    work_left,
                    work_top,
                    work_right,
                    pill_bottom_for(overlay.app_handle(), &monitor),
                ),
                (overlay_w, overlay_h),
            )
        } else {
            (centered_x, centered_y + applied_position.applied_offset)
        };
        let _ = overlay.set_ignore_cursor_events(overlay_click_through(layout));

        // A partial native update invalidates the old frame. Publish the new
        // frame only after both setters succeed; otherwise the next resize
        // must recover from the actual window frame.
        *overlay
            .app_handle()
            .services()
            .overlay_frame
            .lock_or_recover() = None;
        let sized = if let Err(e) =
            overlay.set_size(crate::host::LogicalSize::new(overlay_w, overlay_h))
        {
            tracing::warn!(target: "system", "position_overlay_default: set_size({}, {}) failed: {}", overlay_w, overlay_h, e);
            false
        } else {
            true
        };
        tracing::info!(target: "system", "position_overlay_default: x={}, y={}, sf={}", x, y, sf);
        if let Err(e) = overlay.set_position(crate::host::LogicalPosition::new(x, y)) {
            tracing::warn!(target: "system", "position_overlay_default: set_position({}, {}) failed: {}", x, y, e);
        } else {
            if sized {
                let app = overlay.app_handle();
                remember_overlay_position(
                    app,
                    OverlayFrame {
                        x,
                        y,
                        width: overlay_w,
                        height: overlay_h,
                    },
                    applied_position,
                );
                crate::commands::overlay_material::refit(app, OVERLAY_MATERIAL);
                follow_dictation_preview(app);
            }
            let overlay = overlay.clone();
            let app = overlay.app_handle().clone();
            let target = ((x * sf).round() as i32, (y * sf).round() as i32);
            crate::host::runtime::spawn(async move {
                match settled_outer_position(app, overlay, target).await {
                    Ok(actual) => tracing::info!(
                        target: "system",
                        event_code = "overlay.position_default",
                        reason,
                        target_x_physical = target.0,
                        target_y_physical = target.1,
                        actual_x_physical = actual.0,
                        actual_y_physical = actual.1,
                        matches_target = actual == target,
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
                        target_x_physical = target.0,
                        target_y_physical = target.1,
                        error = %error,
                        "Overlay default position applied, but its resulting frame could not be read"
                    ),
                }
            });
        }
    } else {
        tracing::warn!(target: "system", "position_overlay_default: no monitor available, retaining configured position");
        // Even without a monitor, resizing invalidates the cached dimensions.
        *overlay
            .app_handle()
            .services()
            .overlay_frame
            .lock_or_recover() = None;
        if let Err(e) = overlay.set_size(crate::host::LogicalSize::new(overlay_w, overlay_h)) {
            tracing::warn!(target: "system", "position_overlay_default: fallback set_size({}, {}) failed: {}", overlay_w, overlay_h, e);
        }
    }
}

/// Return the current overlay geometry so the frontend can size the island.
#[cfg_attr(feature = "tauri-host", tauri::command)]
pub fn get_overlay_geometry(
    state: crate::host::StateRef<'_, State>,
    content: Option<OverlayContent>,
) -> OverlayGeometry {
    geometry_for_layout(
        *state.notch_info.lock_or_recover(),
        content.unwrap_or_default(),
        *state.overlay_layout.lock_or_recover(),
    )
}

/// Show the always-on-top macOS notch overlay window.
#[cfg_attr(feature = "tauri-host", tauri::command)]
pub fn show_overlay(app: crate::host::Context) -> Result<(), String> {
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
                let layout = *app.services().overlay_layout.lock_or_recover();
                let _ = overlay.set_ignore_cursor_events(overlay_click_through(layout));
                // Tell the overlay it is visible so it can gate cursor polling.
                let _ = app.events().publish(EventKind::OverlayVisibleChanged, true);
                Ok(())
            }
            None => {
                tracing::warn!(target: "system", "show_overlay: overlay window not found — skipping");
                Ok(())
            }
        }
    }
}

/// Resize the overlay through the expansion controller's single writer. A
/// suggestion grows around the current center; every resize retains the
/// calibrated top edge. Collapsing restores the ordinary controls dimensions.
/// The acknowledgment resolves after the main-thread frame update, before the
/// frontend reveals content or starts accepting clicks in the expanded area.
#[cfg_attr(feature = "tauri-host", tauri::command)]
pub async fn set_overlay_expanded(
    app: crate::host::Context,
    _state: crate::host::StateRef<'_, State>,
    expanded: bool,
    content: Option<OverlayContent>,
    activity: Option<OverlayActivity>,
    placement: Option<OverlayPlacement>,
    defer_shrink: Option<bool>,
) -> Result<AppliedSurface, String> {
    let content = content.unwrap_or_default();
    #[cfg(not(target_os = "macos"))]
    {
        let mut layout = _state.overlay_layout.lock_or_recover();
        layout.expanded = expanded;
        if let Some(activity) = activity {
            layout.activity = activity;
        }
        if let Some(placement) = placement {
            layout.placement = placement;
        }
        let g = geometry_for_layout(None, content, *layout);
        let _ = app.events().publish(EventKind::OverlaySurfaceChanged, &g);
        return Ok(animation_surface(
            applied_surface_for(&g, expanded),
            *_state.overlay_frame.lock_or_recover(),
            defer_shrink.unwrap_or(false),
        ));
    }

    #[cfg(target_os = "macos")]
    {
        let overlay = app
            .get_webview_window("overlay")
            .ok_or_else(|| "overlay window not found".to_string())?;
        refresh_cached_dock_tile(&app);
        let (tx, rx) = tokio::sync::oneshot::channel();
        let dispatch_app = app.clone();
        app.run_on_main_thread(move || {
            let result = (|| {
                // Resolve geometry only once this mutation reaches the native
                // main thread. A queued display refresh may have replaced the
                // notch/scale contract since the command was invoked.
                let state = dispatch_app.services();
                let notch = *state.notch_info.lock_or_recover();
                let mut layout = *state.overlay_layout.lock_or_recover();
                layout.expanded = expanded;
                if let Some(activity) = activity {
                    layout.activity = activity;
                }
                if let Some(placement) = placement {
                    layout.placement = placement;
                }
                let geometry = geometry_for_layout(notch, content, layout);
                if tx.is_closed() {
                    return Err("overlay resize expired before native dispatch".to_string());
                }
                let applied = animation_surface(
                    applied_surface_for(&geometry, expanded),
                    *state.overlay_frame.lock_or_recover(),
                    defer_shrink.unwrap_or(false),
                );
                // Accepted native setters update `overlay_frame` synchronously
                // on this same main-thread queue. Prefer that authoritative
                // target over an immediate WindowServer read, which can still
                // report the frame preceding a queued placement/calibration.
                let saved_frame = *state.overlay_frame.lock_or_recover();
                let (logical_position, current_width) = match saved_frame {
                    Some(frame) => ((frame.x, frame.y), frame.width),
                    None => {
                        let position = overlay
                            .outer_position()
                            .map_err(|error| error.to_string())?;
                        let size = overlay.outer_size().map_err(|error| error.to_string())?;
                        let scale = overlay.scale_factor().map_err(|error| error.to_string())?;
                        (
                            (position.x as f64 / scale, position.y as f64 / scale),
                            size.width as f64 / scale,
                        )
                    }
                };
                let (mut x, mut y) =
                    resized_logical_position(logical_position, current_width, applied.window_w);
                // Placement and size are one main-thread transaction. Re-clamp
                // on every state/hover change, including a short work area.
                let mut calibration = None;
                if let Some(monitor) = dispatch_app.primary_monitor().map_err(|e| e.to_string())? {
                    let sf = monitor.scale_factor();
                    let area = monitor.work_area();
                    let left = area.position.x as f64 / sf;
                    let right = left + area.size.width as f64 / sf;
                    let frame_top = monitor.position().y as f64 / sf;
                    let work_top = usable_top(frame_top, area.position.y as f64 / sf, notch);
                    let bottom = (area.position.y as f64 + area.size.height as f64) / sf;
                    let offset = state.overlay_calibration.lock_or_recover().requested_offset;
                    let position = applied_overlay_position(offset, geometry.floating);
                    if geometry.floating {
                        // Bottom-anchored: width changes keep the centre, height
                        // changes grow the hover card upward from the pill.
                        (x, y) = pill_logical_position(
                            (
                                left,
                                work_top,
                                right,
                                pill_bottom_for(&dispatch_app, &monitor),
                            ),
                            (applied.window_w, applied.window_h),
                        );
                    } else {
                        // The notch is centred on the display frame, never the
                        // work area. Re-derive x so a side Dock left behind by
                        // the bottom pill cannot shift the island off the notch.
                        (x, _) = centered_logical_position(
                            (monitor.position().x, monitor.position().y),
                            (monitor.size().width, monitor.size().height),
                            sf,
                            applied.window_w,
                        );
                        y = frame_top + position.applied_offset;
                        y = y.min((bottom - applied.window_h).max(frame_top));
                    }
                    calibration = Some(position);
                }
                let frame = OverlayFrame {
                    x,
                    y,
                    width: applied.window_w,
                    height: applied.window_h,
                };
                // Most activity changes keep the same bounds (the pill never
                // resizes). Skipping the setters avoids a WindowServer
                // transaction, and its dropped frames, for a no-op.
                if !frame_unchanged(saved_frame, frame) {
                    // Never retain an authoritative old frame after a partially
                    // successful size/position pair.
                    *state.overlay_frame.lock_or_recover() = None;
                    overlay
                        .set_size(crate::host::LogicalSize::new(
                            applied.window_w,
                            applied.window_h,
                        ))
                        .map_err(|error| error.to_string())?;
                    overlay
                        .set_position(crate::host::LogicalPosition::new(x, y))
                        .map_err(|error| error.to_string())?;
                }
                if let Some(position) = calibration {
                    // Publish effective bounds only after both setters succeed,
                    // so an open calibration sheet follows placement changes.
                    remember_overlay_position(&dispatch_app, frame, position);
                } else {
                    remember_overlay_frame(&dispatch_app, frame);
                }
                *state.overlay_layout.lock_or_recover() = layout;
                let _ = overlay.set_ignore_cursor_events(overlay_click_through(layout));
                // The material re-fits in this same main-thread turn, so the
                // grown or shrunk window never shows it off-centre for a frame.
                if layout.placement == OverlayPlacement::FloatingPill {
                    crate::commands::overlay_material::refit(&dispatch_app, OVERLAY_MATERIAL);
                } else {
                    crate::commands::overlay_material::remove_all();
                }
                follow_dictation_preview(&dispatch_app);
                // Emitted only after native setters succeed; the webview can
                // never reveal a larger surface before its native grow ack.
                let _ = dispatch_app
                    .events()
                    .publish(EventKind::OverlaySurfaceChanged, geometry);
                Ok(applied)
            })();
            let _ = tx.send(result);
        })
        .map_err(|error| error.to_string())?;
        tokio::time::timeout(std::time::Duration::from_secs(2), rx)
            .await
            .map_err(|_| "overlay resize timed out".to_string())?
            .map_err(|_| "overlay resize was dropped".to_string())?
    }
}

/// Apply a user-calibrated vertical offset to the notch-anchored overlay.
///
/// The frontend persists the saved logical-point offset. Native state retains
/// both the requested session offset (including an unsaved calibration draft)
/// and its effective clamped value, so placement refreshes can re-resolve the
/// same intent without resetting or competing with it.
#[cfg_attr(feature = "tauri-host", tauri::command)]
pub async fn set_overlay_vertical_offset(
    app: crate::host::Context,
    _state: crate::host::StateRef<'_, State>,
    offset: f64,
) -> Result<AppliedOverlayPosition, String> {
    let offset = clamp_vertical_offset(offset);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = &app;
        return Ok(AppliedOverlayPosition {
            requested_offset: offset,
            applied_offset: offset,
            min_offset: MIN_VERTICAL_OFFSET,
            max_offset: MAX_VERTICAL_OFFSET,
        });
    }

    #[cfg(target_os = "macos")]
    {
        let overlay = app
            .get_webview_window("overlay")
            .ok_or_else(|| "overlay window not found".to_string())?;
        let overlay_to_move = overlay.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let dispatch_app = app.clone();
        app.run_on_main_thread(move || {
            let result = (|| {
                let monitor = dispatch_app
                    .primary_monitor()
                    .map_err(|error| error.to_string())?
                    .or_else(|| overlay_to_move.current_monitor().ok().flatten())
                    .ok_or_else(|| "no monitor available".to_string())?;
                let position = monitor.position();
                let size = monitor.size();
                let scale_factor = monitor.scale_factor();
                let monitor_position = (position.x, position.y);
                let monitor_size = (size.width, size.height);
                let area = monitor.work_area();
                let notch = *dispatch_app.services().notch_info.lock_or_recover();
                let floating = dispatch_app
                    .services()
                    .overlay_layout
                    .lock_or_recover()
                    .placement
                    == OverlayPlacement::FloatingPill;
                let current_scale = overlay_to_move
                    .scale_factor()
                    .map_err(|error| error.to_string())?;
                let current_size = overlay_to_move
                    .outer_size()
                    .map_err(|error| error.to_string())?;
                let width = current_size.width as f64 / current_scale;
                let height = current_size.height as f64 / current_scale;
                let (centered_x, top_y) =
                    centered_logical_position(monitor_position, monitor_size, scale_factor, width);
                let work_left = area.position.x as f64 / scale_factor;
                let work_top = usable_top(top_y, area.position.y as f64 / scale_factor, notch);
                let work_right = work_left + area.size.width as f64 / scale_factor;
                let applied = applied_overlay_position(offset, floating);
                let (x, y) = if floating {
                    pill_logical_position(
                        (
                            work_left,
                            work_top,
                            work_right,
                            pill_bottom_for(&dispatch_app, &monitor),
                        ),
                        (width, height),
                    )
                } else {
                    (centered_x, top_y + applied.applied_offset)
                };
                overlay_to_move
                    .set_position(crate::host::LogicalPosition::new(x, y))
                    .map_err(|error| error.to_string())?;
                remember_overlay_position(
                    &dispatch_app,
                    OverlayFrame {
                        x,
                        y,
                        width,
                        height,
                    },
                    applied,
                );
                follow_dictation_preview(&dispatch_app);
                Ok::<_, String>((
                    (
                        (x * scale_factor).round() as i32,
                        (y * scale_factor).round() as i32,
                    ),
                    applied,
                    position.x,
                    position.y,
                    scale_factor,
                ))
            })();
            let _ = tx.send(result);
        })
        .map_err(|error| error.to_string())?;
        let ((x, y), applied, monitor_x, monitor_y, scale_factor) =
            tokio::time::timeout(std::time::Duration::from_secs(2), rx)
                .await
                .map_err(|_| "overlay position update timed out".to_string())?
                .map_err(|_| "overlay position update was dropped".to_string())??;

        let probe_app = app.clone();
        crate::host::runtime::spawn(async move {
            match settled_outer_position(probe_app, overlay, (x, y)).await {
                Ok(actual) => tracing::info!(
                    target: "system",
                    event_code = "overlay.position_offset_applied",
                    requested_offset_logical = applied.requested_offset,
                    applied_offset_logical = applied.applied_offset,
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
                    requested_offset_logical = applied.requested_offset,
                    applied_offset_logical = applied.applied_offset,
                    target_x_physical = x,
                    target_y_physical = y,
                    error = %error,
                    "Overlay calibrated position applied, but its resulting frame could not be read"
                ),
            }
        });
        Ok(applied)
    }
}

/// Show and focus the main app window.
///
/// The overlay uses this instead of frontend window APIs so it does not need
/// broad `core:window:allow-show` / `allow-set-focus` permissions.
#[cfg_attr(feature = "tauri-host", tauri::command)]
pub fn show_main_window(app: crate::host::Context) -> Result<(), String> {
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
#[cfg_attr(feature = "tauri-host", tauri::command)]
pub fn hide_overlay(app: crate::host::Context) -> Result<(), String> {
    match app.get_webview_window("overlay") {
        Some(overlay) => {
            overlay.hide().map_err(|e| e.to_string())?;
            // Tell the overlay it is hidden so it can stop cursor polling.
            let _ = app
                .events()
                .publish(EventKind::OverlayVisibleChanged, false);
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
    fn placement_triggers_recheck_while_the_menu_bar_settles() {
        use PlacementTrigger::*;
        let total = |trigger: PlacementTrigger| -> std::time::Duration {
            trigger.settle_delays().iter().sum()
        };
        assert!(Settle.settle_delays().is_empty());
        for trigger in [Startup, DisplayChange, FrontmostApp, SpaceChange] {
            assert!(trigger.settle_delays().len() > 1);
            assert!(total(trigger) >= std::time::Duration::from_secs(4));
            assert!(total(trigger) <= std::time::Duration::from_secs(10));
        }
    }
}
