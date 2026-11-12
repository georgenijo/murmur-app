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
    pub floating: bool,
    pub island: IslandMetrics,
}

/// Native-owned island metrics: the notch the idle island hides behind, its
/// corner radii, and the bottom pill's capsule. Hover controls grow the same
/// shape upward from the pill.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct IslandMetrics {
    pub notch_w: f64,
    pub notch_h: f64,
    pub corner_idle: f64,
    pub corner_active: f64,
    pub pill_w: f64,
    pub pill_h: f64,
}

#[derive(serde::Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OverlayActivity {
    #[default]
    Idle,
    Recording,
    Processing,
}

#[derive(serde::Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OverlayPlacement {
    #[default]
    Notch,
    /// Bottom-centre capsule, hidden and click-through while idle. Accepts the
    /// retired below-menu-bar value so older frontends resolve to the pill.
    #[serde(alias = "below_menu_bar")]
    FloatingPill,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct OverlayLayout {
    activity: OverlayActivity,
    placement: OverlayPlacement,
    expanded: bool,
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

/// Effective calibration and the range the current display/window frame can
/// actually honor. The frontend displays these resolved values so clamped
/// movement never masquerades as additional travel.
#[derive(serde::Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AppliedOverlayPosition {
    pub requested_offset: f64,
    pub applied_offset: f64,
    pub min_offset: f64,
    pub max_offset: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct OverlayFrame {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) width: f64,
    pub(crate) height: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct OverlayCalibrationState {
    pub(crate) requested_offset: f64,
    pub(crate) applied_offset: f64,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OverlayContent {
    #[default]
    Controls,
    MeetingSuggestion,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DisplaySnapshot {
    pub(crate) notch_info: Option<NotchInfo>,
    monitor_position: Option<(i32, i32)>,
    monitor_size: Option<(u32, u32)>,
    monitor_work_area: Option<(i32, i32, u32, u32)>,
    scale_factor: Option<f64>,
    /// An auto-hidden bottom Dock's size preferences. Resizing it leaves the
    /// work area unchanged, so it must invalidate placement on its own.
    hidden_dock_tile: Option<HiddenDock>,
}

/// Size preferences of an auto-hidden bottom Dock: the static fallback used
/// when `dock_watch` has no live reading.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct HiddenDock {
    tile: f64,
    /// Magnified icon size, when magnification is on.
    magnified: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct NotchInfo {
    width: f64,
    height: f64,
    physical_notch: bool,
    /// A real menu bar occupies the top of the display (its height was measured
    /// from the screen frame / visible-frame gap). False when the bar is
    /// auto-hidden or the native measurement was unavailable or implausible.
    menu_bar_visible: bool,
}

impl NotchInfo {
    pub(crate) fn height(self) -> f64 {
        self.height
    }
}

// Native geometry is the only surface-size contract. The idle notch island
// matches the notch; an active island reveals a 28pt wing on either side inside
// the menu-bar band (status dot left, level bars right). Flat displays use a
// measured-height 185pt pretend notch. The floating pill is a bottom-centre
// capsule whose window never resizes between idle, recording and processing.
const WING: f64 = 28.0;
const DROPDOWN_H: f64 = 64.0;
const PILL_W: f64 = 120.0;
const PILL_H: f64 = 34.0;
#[cfg(target_os = "macos")]
const OVERLAY_MATERIAL: &str = crate::commands::overlay_material::OVERLAY_LABEL;
/// Distance between the pill and the bottom of the usable work area (Dock).
const PILL_BOTTOM_GAP: f64 = 14.0;
const PILL_EXPANDED_MIN_W: f64 = 241.0;
/// Dock tile-size preference bounds and default, in points.
const DOCK_MIN_TILE: f64 = 16.0;
const DOCK_MAX_TILE: f64 = 128.0;
const DOCK_DEFAULT_TILE: f64 = 64.0;
/// Bottom margin under a magnified icon plus room for its name label.
const DOCK_MAGNIFIED_MARGIN: f64 = 52.0;
/// Minimum plausible menu-bar height for the pretend notch.
const FLOATING_MIN_H: f64 = 24.0;
const POPOVER_GAP: f64 = 6.0;
const MEETING_SUGGESTION_MIN_W: f64 = 320.0;
const MEETING_SUGGESTION_DROPDOWN_H: f64 = 136.0;
const FALLBACK_NOTCH_W: f64 = 185.0;
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

fn geometry_for(notch: Option<NotchInfo>) -> OverlayGeometry {
    // Flat displays deliberately use the same 185pt pretend notch as Hex.
    let notch_w = notch
        .filter(|n| n.physical_notch)
        .map_or(FALLBACK_NOTCH_W, |n| n.width);
    let notch_h = notch.map_or(FALLBACK_NOTCH_H, |n| {
        n.height.clamp(FLOATING_MIN_H, MAX_REASONABLE_MENU_BAR_H)
    });
    let window_w = notch_w + 2.0 * WING;
    OverlayGeometry {
        window_w,
        collapsed_h: notch_h,
        expanded_h: notch_h + DROPDOWN_H,
        pill_idle_w: notch_w,
        pill_active_w: window_w,
        pill_margin_idle: WING,
        pill_margin_active: 0.0,
        dropdown_h: DROPDOWN_H,
        wing_w: WING,
        floating: false,
        island: IslandMetrics {
            notch_w,
            notch_h,
            corner_idle: 8.0,
            corner_active: 12.0,
            pill_w: PILL_W,
            pill_h: PILL_H,
        },
    }
}

fn geometry_for_layout(
    notch: Option<NotchInfo>,
    content: OverlayContent,
    layout: OverlayLayout,
) -> OverlayGeometry {
    let content = if layout.expanded {
        content
    } else {
        OverlayContent::Controls
    };
    let mut g = geometry_for_content(notch, content);
    let active = layout.activity != OverlayActivity::Idle || layout.expanded;
    if layout.placement == OverlayPlacement::FloatingPill {
        // One fixed capsule for every activity: idle only fades it out and makes
        // the window click-through, so recording never waits on a native resize.
        g.floating = true;
        g.island.notch_h = 0.0;
        g.window_w = if layout.expanded {
            g.window_w.max(PILL_EXPANDED_MIN_W)
        } else {
            PILL_W
        };
        g.collapsed_h = PILL_H;
    } else if !active {
        g.window_w = g.island.notch_w;
        g.collapsed_h = g.island.notch_h;
    }
    g.pill_idle_w = g.window_w;
    g.pill_active_w = g.window_w;
    g.pill_margin_idle = 0.0;
    g.pill_margin_active = 0.0;
    g.expanded_h = g.collapsed_h + g.dropdown_h;
    g
}

/// The floating pill's geometry for its native material, resolved from the
/// current display for `expanded`/`content`. Placement comes from the caller:
/// the material only exists while the pill is the selected placement.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn floating_pill_geometry(
    app: &tauri::AppHandle,
    content: OverlayContent,
    expanded: bool,
) -> OverlayGeometry {
    let state = app.state::<State>();
    let notch = *state.notch_info.lock_or_recover();
    let mut layout = *state.overlay_layout.lock_or_recover();
    layout.placement = OverlayPlacement::FloatingPill;
    layout.expanded = expanded;
    geometry_for_layout(notch, content, layout)
}

/// True while the stored layout places the overlay as the floating pill.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn floating_pill_placed(app: &tauri::AppHandle) -> bool {
    app.state::<State>()
        .overlay_layout
        .lock_or_recover()
        .placement
        == OverlayPlacement::FloatingPill
}

/// The floating pill is the only surface that may be idle yet present: it is
/// transparent then, so it must not intercept clicks above the Dock.
pub(crate) fn overlay_click_through(layout: OverlayLayout) -> bool {
    layout.placement == OverlayPlacement::FloatingPill
        && layout.activity == OverlayActivity::Idle
        && !layout.expanded
}

fn geometry_for_content(notch: Option<NotchInfo>, content: OverlayContent) -> OverlayGeometry {
    let mut geometry = geometry_for(notch);
    if content == OverlayContent::MeetingSuggestion {
        let normal_width = geometry.window_w;
        geometry.window_w = normal_width.max(MEETING_SUGGESTION_MIN_W);
        geometry.pill_margin_idle = if geometry.floating {
            (geometry.window_w - geometry.pill_idle_w) / 2.0
        } else {
            (geometry.window_w - normal_width) / 2.0
        };
        geometry.pill_active_w = geometry.window_w;
        geometry.dropdown_h = MEETING_SUGGESTION_DROPDOWN_H;
        geometry.expanded_h = geometry.collapsed_h + geometry.dropdown_h;
    }
    geometry
}

/// During a CSS contraction retain only the bounds the previous visible
/// surface still needs. A second, settled request releases them. Never grow
/// a transparent steady-state hit target merely to simplify animation.
fn animation_surface(
    target: AppliedSurface,
    current: Option<OverlayFrame>,
    defer_shrink: bool,
) -> AppliedSurface {
    match current.filter(|_| defer_shrink) {
        Some(frame) => AppliedSurface {
            window_w: target.window_w.max(frame.width),
            window_h: target.window_h.max(frame.height),
        },
        None => target,
    }
}

fn resized_logical_position(
    current_position: (f64, f64),
    current_width: f64,
    target_width: f64,
) -> (f64, f64) {
    (
        current_position.0 + (current_width - target_width) / 2.0,
        current_position.1,
    )
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
) -> NotchInfo {
    let visible_top_gap = (frame_top - visible_frame_top).max(0.0);
    let measured_menu_bar_h = safe_top.max(visible_top_gap);
    let menu_bar_visible = measured_menu_bar_h.is_finite()
        && measured_menu_bar_h > 0.0
        && measured_menu_bar_h <= MAX_REASONABLE_MENU_BAR_H;
    let menu_bar_h = if menu_bar_visible {
        measured_menu_bar_h
    } else {
        FALLBACK_NOTCH_H
    };

    let measured_notch_w = screen_w - auxiliary_left_w - auxiliary_right_w;
    let physical_notch = safe_top.is_finite()
        && safe_top > 0.0
        && measured_notch_w.is_finite()
        && measured_notch_w > 0.0
        && measured_notch_w < screen_w;
    let notch_w = if physical_notch {
        measured_notch_w
    } else {
        FALLBACK_NOTCH_W
    };

    NotchInfo {
        width: notch_w,
        height: menu_bar_h,
        physical_notch,
        menu_bar_visible,
    }
}

fn centered_logical_position(
    monitor_position: (i32, i32),
    monitor_size: (u32, u32),
    scale_factor: f64,
    overlay_w: f64,
) -> (f64, f64) {
    let x = monitor_position.0 as f64 / scale_factor
        + (monitor_size.0 as f64 / scale_factor - overlay_w) / 2.0;
    (x, monitor_position.1 as f64 / scale_factor)
}

/// Lowest `y` a top-anchored surface may take below a physical notch.
fn usable_top(frame_top: f64, work_top: f64, notch: Option<NotchInfo>) -> f64 {
    work_top.max(frame_top + notch.filter(|n| n.physical_notch).map_or(0.0, |n| n.height))
}

/// Bottom-centre pill frame inside the work area, so it sits above the Dock
/// (including a side Dock). `work_area` is the logical (left, top, right,
/// bottom) of the usable area.
fn pill_logical_position(work_area: (f64, f64, f64, f64), window_size: (f64, f64)) -> (f64, f64) {
    let (left, top, right, bottom) = work_area;
    let (width, height) = window_size;
    (
        ((left + right - width) / 2.0).max(left),
        (bottom - PILL_BOTTOM_GAP - height).max(top),
    )
}

/// Static fallback for when the Dock cannot be read live. An auto-hidden Dock
/// leaves only a few points of work area reserved, so reserve the revealed
/// Dock instead: its bar and bottom margin measure about 1.5× the icon tile
/// size plus a small constant. A magnified icon rises above the bar, with its
/// name label above that.
fn hidden_dock_reserve(dock: HiddenDock) -> f64 {
    let bar = dock.tile.clamp(DOCK_MIN_TILE, DOCK_MAX_TILE) * 1.5 + 8.0;
    dock.magnified.map_or(bar, |size| {
        bar.max(size.clamp(DOCK_MIN_TILE, DOCK_MAX_TILE) + DOCK_MAGNIFIED_MARGIN)
    })
}

/// Bottom edge the pill may rest on. A live Dock reading wins; without one,
/// an auto-hidden bottom Dock (`hidden_dock`) reserves its static height.
fn pill_work_bottom(
    display_bottom: f64,
    work_bottom: f64,
    hidden_dock: Option<HiddenDock>,
    live: crate::dock_watch::DockLimit,
) -> f64 {
    use crate::dock_watch::DockLimit;
    match live {
        DockLimit::Clear => work_bottom,
        DockLimit::Above(top) => work_bottom.min(top),
        DockLimit::Unknown => hidden_dock.map_or(work_bottom, |dock| {
            work_bottom.min(display_bottom - hidden_dock_reserve(dock))
        }),
    }
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
fn pill_bottom_for(app: &tauri::AppHandle, monitor: &tauri::Monitor) -> f64 {
    let sf = monitor.scale_factor();
    let area = monitor.work_area();
    let display = pill_display(
        (monitor.position().x, monitor.position().y),
        (monitor.size().width, monitor.size().height),
        (area.position.y, area.size.height),
        sf,
    );
    let hidden_dock_tile = app
        .state::<State>()
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
pub(crate) fn dock_watch_context(
    app: &tauri::AppHandle,
) -> Option<crate::dock_watch::WatchContext> {
    let state = app.state::<State>();
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
pub(crate) fn follow_dock(app: &tauri::AppHandle) {
    let handle = app.clone();
    let dispatched = app.run_on_main_thread(crate::main_thread_watch::job(
        "overlay.follow_dock",
        move || {
            let state = handle.state::<State>();
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
            if let Err(error) = crate::main_thread_watch::section("overlay.set_position", || {
                overlay.set_position(tauri::LogicalPosition::new(frame.x, y))
            }) {
                // The window did not move: keep its frame so the next Dock change
                // can still follow.
                remember_overlay_frame(&handle, frame);
                tracing::warn!(target: "system", "follow_dock: set_position failed: {}", error);
                return;
            }
            remember_overlay_frame(&handle, target);
            follow_dictation_preview(&handle);
        },
    ));
    if let Err(error) = dispatched {
        tracing::warn!(target: "system", "follow_dock: main-thread dispatch failed: {}", error);
    }
}

/// Re-read the Dock preference off the main thread and fold it into the cached
/// snapshot. Resizing the Dock changes no display geometry and posts none of
/// the placement triggers, so every pill resize samples it here first.
#[cfg(target_os = "macos")]
fn refresh_cached_dock_tile(app: &tauri::AppHandle) {
    let tile = hidden_bottom_dock_tile();
    if let Some(snapshot) = app
        .state::<State>()
        .display_snapshot
        .lock_or_recover()
        .as_mut()
    {
        snapshot.hidden_dock_tile = tile;
    }
}

/// Vertical calibration only moves the notch island; the pill has no travel.
fn vertical_offset_bounds(floating: bool) -> (f64, f64) {
    if floating {
        (0.0, 0.0)
    } else {
        (MIN_VERTICAL_OFFSET, MAX_VERTICAL_OFFSET)
    }
}

fn applied_overlay_position(requested_offset: f64, floating: bool) -> AppliedOverlayPosition {
    let requested_offset = clamp_vertical_offset(requested_offset);
    let (min_offset, max_offset) = vertical_offset_bounds(floating);
    AppliedOverlayPosition {
        requested_offset,
        applied_offset: requested_offset.clamp(min_offset, max_offset),
        min_offset,
        max_offset,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct PopoverWorkArea {
    left: f64,
    right: f64,
    top: f64,
    bottom: f64,
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
    app: &tauri::AppHandle,
    monitor: &tauri::Monitor,
    preview_width: f64,
    preview_height: f64,
) -> Option<(f64, f64, f64)> {
    let primary = app.primary_monitor().ok().flatten()?;
    if primary.position() != monitor.position() || primary.size() != monitor.size() {
        return None;
    }
    let state = app.try_state::<crate::State>()?;
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

/// The overlay frame with any hover dropdown removed. The pill grows upward
/// from a fixed bottom edge, the notch island downward from a fixed top.
fn collapsed_frame(frame: OverlayFrame, collapsed_h: f64, above: bool) -> OverlayFrame {
    let height = collapsed_h.min(frame.height);
    OverlayFrame {
        y: if above {
            frame.y + frame.height - height
        } else {
            frame.y
        },
        height,
        ..frame
    }
}

fn dictation_preview_fit_for_frame(
    work_area: PopoverWorkArea,
    preview_size: (f64, f64),
    frame: Option<OverlayFrame>,
    above: bool,
) -> Option<(f64, f64, f64)> {
    // Both placements follow the acknowledged frame, never a guessed notch
    // inset: below the notch island, above the bottom pill. The caller bounds
    // preview width to this display's work area.
    let frame = frame?;
    let (preview_width, preview_height) = preview_size;
    let rightmost_x = (work_area.right - preview_width).max(work_area.left);
    let x = (frame.x + (frame.width - preview_width) / 2.0).clamp(work_area.left, rightmost_x);
    if above {
        let bottom = frame.y - POPOVER_GAP;
        let top = (bottom - preview_height).max(work_area.top);
        return Some((x, top, preview_height.min((bottom - top).max(0.0))));
    }
    let top = (frame.y + frame.height + POPOVER_GAP).clamp(work_area.top, work_area.bottom);
    Some((
        x,
        top,
        preview_height.min((work_area.bottom - top).max(0.0)),
    ))
}

pub(crate) fn preview_fallback_inset(notch_h: Option<f64>) -> f64 {
    notch_h
        .filter(|h| h.is_finite() && *h > 0.0)
        .unwrap_or(FALLBACK_NOTCH_H)
        + POPOVER_GAP
}

#[cfg(target_os = "macos")]
/// Put back the best-known frame after a failed setter, so the hover poller
/// keeps a frame to sample against instead of going silent.
fn restore_overlay_frame(app: &tauri::AppHandle, frame: Option<OverlayFrame>) {
    if let Some(frame) = frame {
        remember_overlay_frame(app, frame);
    }
}

fn remember_overlay_frame(app: &tauri::AppHandle, frame: OverlayFrame) {
    *app.state::<State>().overlay_frame.lock_or_recover() = Some(frame);
}

/// True when the authoritative saved frame already matches `target` to within
/// a sub-point rounding error.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn frame_unchanged(saved: Option<OverlayFrame>, target: OverlayFrame) -> bool {
    const EPSILON: f64 = 0.01;
    saved.is_some_and(|f| {
        (f.x - target.x).abs() < EPSILON
            && (f.y - target.y).abs() < EPSILON
            && (f.width - target.width).abs() < EPSILON
            && (f.height - target.height).abs() < EPSILON
    })
}

#[cfg(target_os = "macos")]
fn remember_overlay_position(
    app: &tauri::AppHandle,
    frame: OverlayFrame,
    applied: AppliedOverlayPosition,
) {
    let state = app.state::<State>();
    *state.overlay_calibration.lock_or_recover() = OverlayCalibrationState {
        requested_offset: applied.requested_offset,
        applied_offset: applied.applied_offset,
    };
    *state.overlay_frame.lock_or_recover() = Some(frame);
    let _ = app.emit("overlay-position-applied", applied);
}

#[cfg(target_os = "macos")]
fn follow_dictation_preview(app: &tauri::AppHandle) {
    crate::commands::dictation_preview::follow_overlay_if_visible(app);
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
        app.run_on_main_thread(crate::main_thread_watch::job(
            "overlay.settled_outer_position",
            move || {
                let result = window
                    .outer_position()
                    .map(|position| (position.x, position.y))
                    .map_err(|error| error.to_string());
                let _ = tx.send(result);
            },
        ))
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
pub(crate) fn capture_display_snapshot(app_handle: &tauri::AppHandle) -> DisplaySnapshot {
    capture_display_snapshot_with(app_handle, hidden_bottom_dock_tile())
}

/// Like [`capture_display_snapshot`], with the Dock preferences already read.
/// They are a `cfprefsd` round trip and need no main thread, so the placement
/// worker reads them first and the main-thread half only measures screens.
pub(crate) fn capture_display_snapshot_with(
    app_handle: &tauri::AppHandle,
    hidden_dock_tile: Option<HiddenDock>,
) -> DisplaySnapshot {
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
        hidden_dock_tile,
    }
}

/// What prompted a placement refresh. Every trigger re-resolves the same state
/// together: menu-bar visibility and height, placement mode, native window
/// size, and the geometry emitted to the frontend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlacementTrigger {
    Startup,
    DisplayChange,
    /// An app activation can hide or restore the menu bar on the current
    /// Space. It only re-measures; the centred x never depends on the app.
    FrontmostApp,
    SpaceChange,
    /// A delayed re-check while the menu bar animates in or out.
    Settle,
}

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
const PLACEMENT_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(125);
const PLACEMENT_MAX_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(1000);
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
fn refresh_placement(app_handle: &tauri::AppHandle, trigger: PlacementTrigger) {
    let handle = app_handle.clone();
    // Runs on the placement worker, not the main thread: the Dock preference
    // read (`CFPreferencesAppSynchronize` plus copies) is I/O, and an app
    // activation triggers it four times (the refresh and three settle checks).
    let hidden_dock_tile = hidden_bottom_dock_tile();
    if let Err(error) = app_handle.run_on_main_thread(crate::main_thread_watch::job(
        "overlay.refresh_placement",
        move || {
            apply_placement(&handle, trigger, hidden_dock_tile);
        },
    )) {
        tracing::warn!(target: "system", "overlay placement main-thread dispatch failed: {}", error);
    }
}

/// Main-thread half of `refresh_placement`: one snapshot, one resolved mode.
/// Repositions, resizes and notifies the frontend only when it changed.
#[cfg(target_os = "macos")]
fn apply_placement(
    handle: &tauri::AppHandle,
    trigger: PlacementTrigger,
    hidden_dock_tile: Option<HiddenDock>,
) {
    let state = handle.state::<State>();
    let previous = state.display_snapshot.lock_or_recover().clone();
    let snapshot = capture_display_snapshot_with(handle, hidden_dock_tile);
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
    // A concealed popover stays ordered in, so a display change can move it
    // without this code knowing: make its next show re-apply its frame.
    crate::commands::dictation_preview::invalidate_placement();
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
    let _ = handle.emit(
        "overlay-geometry-changed",
        geometry_for_layout(notch, OverlayContent::Controls, layout),
    );
}

/// Start the placement worker: it owns debouncing and sequencing for every
/// trigger so refreshes never overlap or thrash.
#[cfg(target_os = "macos")]
fn start_placement_worker(app_handle: tauri::AppHandle) {
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
pub(crate) fn register_screen_change_observer(app_handle: tauri::AppHandle) {
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
        crate::main_thread_watch::callback("overlay.display_change_notification", || {
            request_placement_refresh(PlacementTrigger::DisplayChange)
        });
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
    if let Err(e) = handle.run_on_main_thread(crate::main_thread_watch::job(
        "overlay.raise_above_menubar",
        move || {
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
        },
    )) {
        tracing::warn!(target: "system", "raise_window_above_menubar: run_on_main_thread failed: {}", e);
    }
}

/// Position and size the overlay to match the notch, anchored at the top of the screen.
/// The window is notch-height tall and wide enough for horizontal expansion.
/// Takes cached notch_info to avoid calling NSScreen APIs off the main thread.
#[cfg(target_os = "macos")]
pub(crate) fn position_overlay_default(
    overlay: &tauri::WebviewWindow,
    notch_info: Option<NotchInfo>,
    reason: &'static str,
) {
    let state = overlay.app_handle().state::<State>();
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
            .state::<State>()
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
            .state::<State>()
            .overlay_frame
            .lock_or_recover() = None;
        let sized = if let Err(e) = crate::main_thread_watch::section("overlay.set_size", || {
            overlay.set_size(tauri::LogicalSize::new(overlay_w, overlay_h))
        }) {
            tracing::warn!(target: "system", "position_overlay_default: set_size({}, {}) failed: {}", overlay_w, overlay_h, e);
            false
        } else {
            true
        };
        tracing::info!(target: "system", "position_overlay_default: x={}, y={}, sf={}", x, y, sf);
        if let Err(e) = crate::main_thread_watch::section("overlay.set_position", || {
            overlay.set_position(tauri::LogicalPosition::new(x, y))
        }) {
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
            tauri::async_runtime::spawn(async move {
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
            .state::<State>()
            .overlay_frame
            .lock_or_recover() = None;
        if let Err(e) = crate::main_thread_watch::section("overlay.set_size", || {
            overlay.set_size(tauri::LogicalSize::new(overlay_w, overlay_h))
        }) {
            tracing::warn!(target: "system", "position_overlay_default: fallback set_size({}, {}) failed: {}", overlay_w, overlay_h, e);
        }
    }
}

/// Return the current overlay geometry so the frontend can size the island.
#[tauri::command]
pub fn get_overlay_geometry(
    state: tauri::State<'_, State>,
    content: Option<OverlayContent>,
) -> OverlayGeometry {
    geometry_for_layout(
        *state.notch_info.lock_or_recover(),
        content.unwrap_or_default(),
        *state.overlay_layout.lock_or_recover(),
    )
}

/// One hover-poller reading, in logical points on the shared top-left desktop
/// space: the overlay window's origin and the cursor. The poller compares them
/// with the island's DOM rectangle (CSS pixels, which are points).
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayPointerSample {
    pub window_x: f64,
    pub window_y: f64,
    pub cursor_x: f64,
    pub cursor_y: f64,
}

/// Combine the overlay's applied frame with a cursor reading. `None` while the
/// frame is unknown (mid-resize) or the cursor could not be read, so the
/// poller skips that tick rather than act on a half-known state.
fn pointer_sample(
    frame: Option<OverlayFrame>,
    cursor: Option<(f64, f64)>,
) -> Option<OverlayPointerSample> {
    let (frame, (cursor_x, cursor_y)) = frame.zip(cursor)?;
    (cursor_x.is_finite() && cursor_y.is_finite()).then_some(OverlayPointerSample {
        window_x: frame.x,
        window_y: frame.y,
        cursor_x,
        cursor_y,
    })
}

/// Current global cursor position in points, origin at the top-left of the
/// main display with y growing down: the space tao reports window frames in.
/// `CGEventCreate` is documented thread-safe and needs no permission, so this
/// never has to hop to the main thread.
#[cfg(target_os = "macos")]
fn global_cursor_position() -> Option<(f64, f64)> {
    #[repr(C)]
    struct CGPoint {
        x: f64,
        y: f64,
    }
    extern "C" {
        fn CGEventCreate(source: *const std::ffi::c_void) -> *mut std::ffi::c_void;
        fn CGEventGetLocation(event: *const std::ffi::c_void) -> CGPoint;
        fn CFRelease(object: *const std::ffi::c_void);
    }
    // SAFETY: a null source asks for an event stamped with the current cursor
    // location; the copy-rule result is released exactly once.
    unsafe {
        let event = CGEventCreate(std::ptr::null());
        if event.is_null() {
            return None;
        }
        let point = CGEventGetLocation(event);
        CFRelease(event);
        Some((point.x, point.y))
    }
}

#[cfg(not(target_os = "macos"))]
fn global_cursor_position() -> Option<(f64, f64)> {
    None
}

/// Read the overlay's frame and the cursor for the hover poller without
/// touching the main thread. This replaces the poller's two blocking
/// main-thread round trips per tick (`outer_position` and `cursor_position`).
/// An `async` command so Tauri runs it on its worker pool.
#[tauri::command]
pub async fn overlay_pointer_sample(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, State>,
) -> Result<Option<OverlayPointerSample>, String> {
    if window.label() != "overlay" {
        return Err("overlay pointer sampling is restricted to the overlay window".to_string());
    }
    let frame = *state.overlay_frame.lock_or_recover();
    let sample = pointer_sample(frame, global_cursor_position());
    note_pointer_sample(sample.is_some(), frame.is_some());
    Ok(sample)
}

/// Consecutive empty hover samples before one warning is logged: 20 ticks is
/// three seconds at the poller's 150 ms cadence.
const EMPTY_POINTER_SAMPLES_BEFORE_WARNING: u32 = 20;
static EMPTY_POINTER_SAMPLES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// A frame that stays unknown (or a cursor that stays unreadable) silently
/// disables hover expansion, so say so once per stretch.
fn note_pointer_sample(had_sample: bool, frame_known: bool) {
    use std::sync::atomic::Ordering;
    if had_sample {
        EMPTY_POINTER_SAMPLES.store(0, Ordering::Relaxed);
        return;
    }
    let empty = EMPTY_POINTER_SAMPLES.fetch_add(1, Ordering::Relaxed) + 1;
    if empty == EMPTY_POINTER_SAMPLES_BEFORE_WARNING {
        tracing::warn!(
            target: "system",
            event_code = "overlay.pointer_sample_empty",
            frame_known,
            "overlay hover samples have been empty for several seconds"
        );
    }
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
                crate::main_thread_watch::section("overlay.show", || overlay.show())
                    .map_err(|e| e.to_string())?;
                let layout = *app.state::<State>().overlay_layout.lock_or_recover();
                let _ = overlay.set_ignore_cursor_events(overlay_click_through(layout));
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

/// Resize the overlay through the expansion controller's single writer. A
/// suggestion grows around the current center; every resize retains the
/// calibrated top edge. Collapsing restores the ordinary controls dimensions.
/// The acknowledgment resolves after the main-thread frame update, before the
/// frontend reveals content or starts accepting clicks in the expanded area.
#[tauri::command]
pub async fn set_overlay_expanded(
    app: tauri::AppHandle,
    _state: tauri::State<'_, State>,
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
        let _ = app.emit("overlay-surface-changed", &g);
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
        app.run_on_main_thread(crate::main_thread_watch::job(
            "overlay.apply_geometry",
            move || {
                let result = (|| {
                    // Resolve geometry only once this mutation reaches the native
                    // main thread. A queued display refresh may have replaced the
                    // notch/scale contract since the command was invoked.
                    let state = dispatch_app.state::<State>();
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
                            let scale =
                                overlay.scale_factor().map_err(|error| error.to_string())?;
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
                    if let Some(monitor) =
                        dispatch_app.primary_monitor().map_err(|e| e.to_string())?
                    {
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
                        if let Err(error) =
                            crate::main_thread_watch::section("overlay.set_size", || {
                                overlay.set_size(tauri::LogicalSize::new(
                                    applied.window_w,
                                    applied.window_h,
                                ))
                            })
                        {
                            // Nothing moved: the window still has its old frame.
                            // Leaving the cache empty would make the hover poller
                            // return no sample for as long as the overlay lives.
                            restore_overlay_frame(&dispatch_app, saved_frame);
                            return Err(error.to_string());
                        }
                        if let Err(error) =
                            crate::main_thread_watch::section("overlay.set_position", || {
                                overlay.set_position(tauri::LogicalPosition::new(x, y))
                            })
                        {
                            // The size took and the position did not: the window
                            // has the new size at its old origin.
                            restore_overlay_frame(
                                &dispatch_app,
                                saved_frame.map(|old| OverlayFrame {
                                    width: applied.window_w,
                                    height: applied.window_h,
                                    ..old
                                }),
                            );
                            return Err(error.to_string());
                        }
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
                    let _ = dispatch_app.emit("overlay-surface-changed", geometry);
                    Ok(applied)
                })();
                let _ = tx.send(result);
            },
        ))
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
#[tauri::command]
pub async fn set_overlay_vertical_offset(
    app: tauri::AppHandle,
    _state: tauri::State<'_, State>,
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
        app.run_on_main_thread(crate::main_thread_watch::job(
            "overlay.set_vertical_offset",
            move || {
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
                    let notch = *dispatch_app.state::<State>().notch_info.lock_or_recover();
                    let floating = dispatch_app
                        .state::<State>()
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
                    let (centered_x, top_y) = centered_logical_position(
                        monitor_position,
                        monitor_size,
                        scale_factor,
                        width,
                    );
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
                    crate::main_thread_watch::section("overlay.set_position", || {
                        overlay_to_move.set_position(tauri::LogicalPosition::new(x, y))
                    })
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
            },
        ))
        .map_err(|error| error.to_string())?;
        let ((x, y), applied, monitor_x, monitor_y, scale_factor) =
            tokio::time::timeout(std::time::Duration::from_secs(2), rx)
                .await
                .map_err(|_| "overlay position update timed out".to_string())?
                .map_err(|_| "overlay position update was dropped".to_string())??;

        let probe_app = app.clone();
        tauri::async_runtime::spawn(async move {
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
#[tauri::command]
pub fn show_main_window(app: tauri::AppHandle) -> Result<(), String> {
    match app.get_webview_window("main") {
        Some(window) => {
            crate::main_thread_watch::section("main_window.show", || window.show())
                .map_err(|e| e.to_string())?;
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
            crate::main_thread_watch::section("overlay.hide", || overlay.hide())
                .map_err(|e| e.to_string())?;
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

    fn notched(width: f64, height: f64) -> NotchInfo {
        NotchInfo {
            width,
            height,
            physical_notch: true,
            menu_bar_visible: true,
        }
    }

    fn external(height: f64) -> NotchInfo {
        NotchInfo {
            width: FALLBACK_NOTCH_W,
            height,
            physical_notch: false,
            menu_bar_visible: true,
        }
    }

    /// Auto-hidden menu bar or failed native measurement on a non-notched display.
    fn external_without_menu_bar() -> NotchInfo {
        NotchInfo {
            menu_bar_visible: false,
            ..external(FALLBACK_NOTCH_H)
        }
    }

    fn pill(activity: OverlayActivity, expanded: bool) -> OverlayLayout {
        OverlayLayout {
            activity,
            placement: OverlayPlacement::FloatingPill,
            expanded,
        }
    }

    #[test]
    fn layout_fixtures_cover_each_activity_placement_and_suggestion_surface() {
        let fixtures: std::collections::HashMap<String, OverlayGeometry> = serde_json::from_str(
            include_str!("../../../src/components/overlay/overlay-geometry.fixture.json"),
        )
        .unwrap();
        for (screen, notch) in [
            ("Notched", Some(notched(185.0, 32.0))),
            ("External", Some(external(30.0))),
        ] {
            for (name, activity) in [
                ("idle", OverlayActivity::Idle),
                ("recording", OverlayActivity::Recording),
                ("processing", OverlayActivity::Processing),
            ] {
                let g = geometry_for_layout(
                    notch,
                    OverlayContent::Controls,
                    OverlayLayout {
                        activity,
                        ..OverlayLayout::default()
                    },
                );
                assert_eq!(&g, &fixtures[&format!("{name}{screen}")]);
                assert_eq!(g.pill_active_w, g.window_w);
                assert_eq!(g.pill_margin_active, 0.0);
            }
        }
        for (name, activity) in [
            ("pillIdle", OverlayActivity::Idle),
            ("pillRecording", OverlayActivity::Recording),
            ("pillProcessing", OverlayActivity::Processing),
        ] {
            assert_eq!(
                geometry_for_layout(
                    Some(notched(185.0, 32.0)),
                    OverlayContent::Controls,
                    pill(activity, false)
                ),
                fixtures[name]
            );
        }
        for (name, placement, content) in [
            (
                "pillExpanded",
                OverlayPlacement::FloatingPill,
                OverlayContent::Controls,
            ),
            (
                "expandedNotched",
                OverlayPlacement::Notch,
                OverlayContent::Controls,
            ),
            (
                "expandedMeetingNotched",
                OverlayPlacement::Notch,
                OverlayContent::MeetingSuggestion,
            ),
            (
                "pillMeetingSuggestion",
                OverlayPlacement::FloatingPill,
                OverlayContent::MeetingSuggestion,
            ),
        ] {
            let layout = OverlayLayout {
                expanded: true,
                placement,
                ..OverlayLayout::default()
            };
            assert_eq!(
                geometry_for_layout(Some(notched(185.0, 32.0)), content, layout),
                fixtures[name]
            );
        }
    }

    #[test]
    fn pill_window_never_resizes_between_activities() {
        let sizes: Vec<_> = [
            OverlayActivity::Idle,
            OverlayActivity::Recording,
            OverlayActivity::Processing,
        ]
        .into_iter()
        .map(|activity| {
            let g = geometry_for_layout(None, OverlayContent::Controls, pill(activity, false));
            applied_surface_for(&g, false)
        })
        .collect();
        assert!(sizes.windows(2).all(|pair| pair[0] == pair[1]));
        assert_eq!(
            sizes[0],
            AppliedSurface {
                window_w: PILL_W,
                window_h: PILL_H
            }
        );
    }

    #[test]
    fn notch_wings_stay_inside_the_menu_bar() {
        for notch in [Some(notched(185.0, 32.0)), Some(external(24.0)), None] {
            let idle =
                geometry_for_layout(notch, OverlayContent::Controls, OverlayLayout::default());
            let recording = geometry_for_layout(
                notch,
                OverlayContent::Controls,
                OverlayLayout {
                    activity: OverlayActivity::Recording,
                    ..OverlayLayout::default()
                },
            );
            assert_eq!(idle.window_w, idle.island.notch_w);
            assert_eq!(recording.window_w, idle.island.notch_w + 2.0 * WING);
            assert_eq!(idle.collapsed_h, idle.island.notch_h);
            assert_eq!(recording.collapsed_h, idle.island.notch_h);
            assert!(!recording.floating);
        }
    }

    #[test]
    fn notch_centres_on_the_display_while_the_pill_centres_on_the_work_area() {
        // 1512pt display with an 80pt left Dock: the pill shifts right, but the
        // notch (expanded, deferred-shrink, or idle) stays on the display centre.
        let pill = pill_logical_position((80.0, 32.0, 1512.0, 982.0), (120.0, 34.0));
        assert_eq!(pill.0, 736.0);
        for width in [185.0, 241.0] {
            let (x, _) = centered_logical_position((0, 0), (3024, 1964), 2.0, width);
            assert_eq!(x + width / 2.0, 756.0);
        }
    }

    #[test]
    fn pointer_samples_need_both_a_known_frame_and_a_readable_cursor() {
        let frame = OverlayFrame {
            x: 867.0,
            y: 0.0,
            width: 185.0,
            height: 32.0,
        };
        assert_eq!(
            pointer_sample(Some(frame), Some((900.5, 12.0))),
            Some(OverlayPointerSample {
                window_x: 867.0,
                window_y: 0.0,
                cursor_x: 900.5,
                cursor_y: 12.0,
            })
        );
        // Mid-resize the frame is unknown: skip the tick.
        assert_eq!(pointer_sample(None, Some((900.5, 12.0))), None);
        assert_eq!(pointer_sample(Some(frame), None), None);
        assert_eq!(pointer_sample(Some(frame), Some((f64::NAN, 1.0))), None);
    }

    #[test]
    fn unchanged_frames_skip_native_setters() {
        let frame = OverlayFrame {
            x: 640.0,
            y: 0.0,
            width: 241.0,
            height: 32.0,
        };
        assert!(!frame_unchanged(None, frame));
        assert!(frame_unchanged(Some(frame), frame));
        assert!(frame_unchanged(
            Some(OverlayFrame {
                x: 640.004,
                ..frame
            }),
            frame
        ));
        assert!(!frame_unchanged(
            Some(OverlayFrame {
                width: 185.0,
                ..frame
            }),
            frame
        ));
        assert!(!frame_unchanged(
            Some(OverlayFrame { y: 4.0, ..frame }),
            frame
        ));
    }

    #[test]
    fn only_the_idle_collapsed_pill_is_click_through() {
        assert!(overlay_click_through(pill(OverlayActivity::Idle, false)));
        assert!(!overlay_click_through(pill(OverlayActivity::Idle, true)));
        assert!(!overlay_click_through(pill(
            OverlayActivity::Recording,
            false
        )));
        assert!(!overlay_click_through(pill(
            OverlayActivity::Processing,
            false
        )));
        assert!(!overlay_click_through(OverlayLayout::default()));
    }

    #[test]
    fn legacy_below_menu_bar_placement_deserializes_as_the_pill() {
        assert_eq!(
            serde_json::from_str::<OverlayPlacement>("\"below_menu_bar\"").unwrap(),
            OverlayPlacement::FloatingPill
        );
        assert_eq!(
            serde_json::from_str::<OverlayPlacement>("\"floating_pill\"").unwrap(),
            OverlayPlacement::FloatingPill
        );
        assert_eq!(
            serde_json::from_str::<OverlayPlacement>("\"notch\"").unwrap(),
            OverlayPlacement::Notch
        );
    }

    #[test]
    fn native_bounds_retain_only_the_animation_then_release_the_hit_area() {
        let prior = Some(OverlayFrame {
            x: 100.0,
            y: 32.0,
            width: 126.0,
            height: 104.0,
        });
        let target = AppliedSurface {
            window_w: 90.0,
            window_h: 40.0,
        };
        assert_eq!(
            animation_surface(target.clone(), prior, true),
            AppliedSurface {
                window_w: 126.0,
                window_h: 104.0
            }
        );
        assert_eq!(animation_surface(target.clone(), prior, false), target);
        assert_eq!(animation_surface(target.clone(), None, true), target);
        let bigger = AppliedSurface {
            window_w: 320.0,
            window_h: 208.0,
        };
        assert_eq!(animation_surface(bigger.clone(), prior, true), bigger);
    }

    #[test]
    fn usable_top_clears_a_physical_notch_only() {
        assert_eq!(usable_top(128.0, 128.0, Some(notched(185.0, 32.0))), 160.0);
        assert_eq!(usable_top(128.0, 158.0, Some(external(30.0))), 158.0);
    }

    #[test]
    fn invariants() {
        for g in [
            geometry_for(Some(notched(185.0, 32.0))),
            geometry_for(Some(external(30.0))),
            geometry_for(None),
            geometry_for_content(
                Some(notched(185.0, 32.0)),
                OverlayContent::MeetingSuggestion,
            ),
            geometry_for_content(None, OverlayContent::MeetingSuggestion),
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
        let g = geometry_for(Some(notched(185.0, 32.0)));
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
            (241.0, 32.0, 96.0, 185.0, 241.0, 28.0, 0.0, 64.0, 28.0)
        );
    }

    #[test]
    fn external_display_uses_its_measured_menu_bar_height() {
        let info = notch_info_from_screen_measurements(2560.0, 1440.0, 1410.0, 0.0, 0.0, 0.0);
        assert_eq!(info, external(30.0));
        let g = geometry_for(Some(info));
        assert_eq!(g.window_w, 241.0);
        assert_eq!(g.pill_idle_w, 185.0);
        assert_eq!(g.collapsed_h, 30.0);
        assert_eq!(g.expanded_h, 94.0);
        assert!(!g.floating);
    }

    #[test]
    fn pretend_notch_height_tracks_measurements() {
        let height = |info| geometry_for(Some(info)).collapsed_h;
        assert_eq!(height(external(24.0)), 24.0);
        assert_eq!(height(external(10.0)), FLOATING_MIN_H);
        assert_eq!(height(external(190.0)), MAX_REASONABLE_MENU_BAR_H);
        assert_eq!(height(external_without_menu_bar()), FALLBACK_NOTCH_H);
        assert_eq!(geometry_for(None).collapsed_h, FALLBACK_NOTCH_H);
    }

    #[test]
    fn menu_bar_measurement_flags_hidden_or_invalid_bars() {
        assert!(
            notch_info_from_screen_measurements(2560.0, 1440.0, 1416.0, 0.0, 0.0, 0.0)
                .menu_bar_visible
        );
        assert_eq!(
            notch_info_from_screen_measurements(2560.0, 1440.0, 1440.0, 0.0, 0.0, 0.0),
            external_without_menu_bar()
        );
        assert!(
            notch_info_from_screen_measurements(1512.0, 982.0, 950.0, 32.0, 663.5, 663.5)
                .menu_bar_visible
        );
    }

    #[test]
    fn notched_display_prefers_safe_area_and_auxiliary_widths() {
        let info = notch_info_from_screen_measurements(1512.0, 982.0, 950.0, 32.0, 663.5, 663.5);
        assert_eq!(info, notched(185.0, 32.0));
    }

    #[test]
    fn invalid_native_measurements_fail_closed_to_geometry_defaults() {
        assert_eq!(
            notch_info_from_screen_measurements(2560.0, 1440.0, 1440.0, f64::NAN, 0.0, 0.0,),
            external_without_menu_bar()
        );
    }

    #[test]
    fn logical_centering_includes_monitor_origin_and_scale() {
        assert_eq!(
            centered_logical_position((5120, 128), (2560, 1440), 1.0, 152.0),
            (6324.0, 128.0)
        );
        assert_eq!(
            centered_logical_position((0, 0), (5120, 2880), 2.0, 152.0),
            (1204.0, 0.0)
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
            notch_info: Some(notched(185.0, 32.0)),
            monitor_position: Some((0, 0)),
            monitor_size: Some((3024, 1964)),
            monitor_work_area: Some((0, 48, 3024, 1916)),
            scale_factor: Some(2.0),
            hidden_dock_tile: None,
        };
        assert_eq!(baseline, baseline.clone());
        for changed in [
            DisplaySnapshot {
                notch_info: Some(notched(184.0, 32.0)),
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
                monitor_work_area: Some((0, 52, 3024, 1912)),
                ..baseline.clone()
            },
            DisplaySnapshot {
                scale_factor: Some(1.0),
                ..baseline.clone()
            },
            DisplaySnapshot {
                hidden_dock_tile: Some(HiddenDock {
                    tile: 64.0,
                    magnified: None,
                }),
                ..baseline.clone()
            },
        ] {
            assert_ne!(baseline, changed);
        }
    }

    #[test]
    fn pill_clears_an_auto_hidden_bottom_dock_without_a_live_reading() {
        use crate::dock_watch::DockLimit::Unknown;
        let dock = |tile, magnified| Some(HiddenDock { tile, magnified });
        // A visible Dock already shrinks the work area: nothing to add.
        assert_eq!(pill_work_bottom(1080.0, 990.0, None, Unknown), 990.0);
        // Auto-hidden: macOS keeps ~4pt, so reserve the revealed Dock instead.
        assert_eq!(
            pill_work_bottom(1080.0, 1076.0, dock(64.0, None), Unknown),
            1080.0 - 104.0
        );
        assert_eq!(
            pill_work_bottom(1080.0, 1076.0, dock(83.0, None), Unknown),
            1080.0 - 132.5
        );
        // Magnified icons and their label rise above the bar.
        assert_eq!(
            pill_work_bottom(1080.0, 1076.0, dock(54.0, Some(113.0)), Unknown),
            1080.0 - 165.0
        );
        // Never lower than the work area, and the size preferences are bounded.
        assert_eq!(
            pill_work_bottom(1080.0, 900.0, dock(48.0, None), Unknown),
            900.0
        );
        assert_eq!(
            pill_work_bottom(1080.0, 1076.0, dock(500.0, Some(500.0)), Unknown),
            1080.0 - 200.0
        );
    }

    #[test]
    fn a_live_dock_reading_replaces_the_static_reserve() {
        use crate::dock_watch::DockLimit::{Above, Clear};
        let dock = Some(HiddenDock {
            tile: 54.0,
            magnified: Some(113.0),
        });
        // Hidden: the pill rests low, just above the work area.
        assert_eq!(pill_work_bottom(1080.0, 1076.0, dock, Clear), 1076.0);
        // Revealed: above the magnified icons and label.
        assert_eq!(pill_work_bottom(1080.0, 1076.0, dock, Above(887.0)), 887.0);
        // A pinned Dock's work area already clears an unmagnified bar.
        assert_eq!(pill_work_bottom(1080.0, 990.0, None, Above(1000.0)), 990.0);
    }

    #[test]
    fn pill_sits_bottom_centre_above_the_dock_in_the_work_area() {
        // Bottom Dock: work area ends at 1340 on a 1440pt display.
        assert_eq!(
            pill_logical_position((0.0, 24.0, 2560.0, 1340.0), (PILL_W, PILL_H)),
            (1220.0, 1340.0 - PILL_BOTTOM_GAP - PILL_H),
        );
        // Secondary display at a negative origin, side Dock narrowing the area.
        assert_eq!(
            pill_logical_position((-2560.0, 30.0, -80.0, 1440.0), (241.0, 98.0)),
            (-1440.5, 1328.0),
        );
        // Never above or left of the work area.
        assert_eq!(
            pill_logical_position((100.0, 24.0, 150.0, 60.0), (241.0, 98.0)),
            (100.0, 24.0),
        );
    }

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

    #[test]
    fn notch_preview_sits_below_the_island_and_pill_preview_above_it() {
        let work_area = PopoverWorkArea {
            left: 0.0,
            right: 2560.0,
            top: 30.0,
            bottom: 1400.0,
        };
        let island = OverlayFrame {
            x: 1159.5,
            y: 0.0,
            width: 241.0,
            height: 30.0,
        };
        assert_eq!(
            dictation_preview_fit_for_frame(work_area, (560.0, 44.0), Some(island), false),
            Some((1000.0, 36.0, 44.0))
        );
        let pill_frame = OverlayFrame {
            x: 1220.0,
            y: 1352.0,
            width: PILL_W,
            height: PILL_H,
        };
        assert_eq!(
            dictation_preview_fit_for_frame(work_area, (560.0, 44.0), Some(pill_frame), true),
            Some((1000.0, 1302.0, 44.0))
        );
        // A short work area clips the preview and still keeps the gap.
        assert_eq!(
            dictation_preview_fit_for_frame(
                PopoverWorkArea {
                    top: 1330.0,
                    ..work_area
                },
                (560.0, 44.0),
                Some(pill_frame),
                true
            ),
            Some((1000.0, 1330.0, 16.0))
        );
        // The full request, card plus hover room, clamps the same way.
        assert_eq!(
            dictation_preview_fit_for_frame(
                PopoverWorkArea {
                    top: 24.0,
                    ..work_area
                },
                (560.0, 104.0 + DROPDOWN_H),
                Some(OverlayFrame {
                    y: 180.0,
                    ..pill_frame
                }),
                true
            ),
            Some((1000.0, 24.0, 180.0 - POPOVER_GAP - 24.0))
        );
    }

    #[test]
    fn preview_anchors_to_the_collapsed_surface_while_hover_expanded() {
        let expanded_pill = OverlayFrame {
            x: 1159.5,
            y: 1288.0,
            width: PILL_EXPANDED_MIN_W,
            height: PILL_H + DROPDOWN_H,
        };
        assert_eq!(
            collapsed_frame(expanded_pill, PILL_H, true),
            OverlayFrame {
                y: 1352.0,
                height: PILL_H,
                ..expanded_pill
            }
        );
        let expanded_island = OverlayFrame {
            x: 1159.5,
            y: 0.0,
            width: 241.0,
            height: 32.0 + DROPDOWN_H,
        };
        assert_eq!(
            collapsed_frame(expanded_island, 32.0, false),
            OverlayFrame {
                height: 32.0,
                ..expanded_island
            }
        );
        // A collapsed frame is already the anchor.
        let collapsed = OverlayFrame {
            height: PILL_H,
            y: 1352.0,
            ..expanded_pill
        };
        assert_eq!(collapsed_frame(collapsed, PILL_H, true), collapsed);
    }

    #[test]
    fn preview_requires_an_applied_frame_and_clamps_to_work_area() {
        let work_area = PopoverWorkArea {
            left: 100.0,
            right: 900.0,
            top: 24.0,
            bottom: 1440.0,
        };
        assert_eq!(
            dictation_preview_fit_for_frame(work_area, (460.0, 104.0), None, false),
            None
        );
        assert_eq!(
            dictation_preview_fit_for_frame(
                work_area,
                (460.0, 104.0),
                Some(OverlayFrame {
                    x: -40.0,
                    y: 0.0,
                    width: 241.0,
                    height: 24.0,
                }),
                false
            ),
            Some((100.0, 30.0, 104.0))
        );
    }

    #[test]
    fn calibration_applies_to_the_notch_only() {
        assert_eq!(
            applied_overlay_position(-12.0, false),
            AppliedOverlayPosition {
                requested_offset: -12.0,
                applied_offset: -12.0,
                min_offset: MIN_VERTICAL_OFFSET,
                max_offset: MAX_VERTICAL_OFFSET,
            }
        );
        // The pill keeps the saved intent but applies no travel.
        assert_eq!(
            applied_overlay_position(7.0, true),
            AppliedOverlayPosition {
                requested_offset: 7.0,
                applied_offset: 0.0,
                min_offset: 0.0,
                max_offset: 0.0,
            }
        );
        assert_eq!(
            applied_overlay_position(f64::NAN, false).requested_offset,
            0.0
        );
    }

    #[test]
    fn matches_fixture() {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct F {
            notched: OverlayGeometry,
            external: OverlayGeometry,
            fallback: OverlayGeometry,
            meeting_suggestion_notched: OverlayGeometry,
            meeting_suggestion_external: OverlayGeometry,
            meeting_suggestion_fallback: OverlayGeometry,
        }
        let f: F = serde_json::from_str(include_str!(
            "../../../src/components/overlay/overlay-geometry.fixture.json"
        ))
        .unwrap();
        assert_eq!(geometry_for(Some(notched(185.0, 32.0))), f.notched);
        assert_eq!(geometry_for(Some(external(30.0))), f.external);
        assert_eq!(geometry_for(None), f.fallback);
        assert_eq!(
            geometry_for_content(
                Some(notched(185.0, 32.0)),
                OverlayContent::MeetingSuggestion
            ),
            f.meeting_suggestion_notched,
        );
        assert_eq!(
            geometry_for_content(Some(external(30.0)), OverlayContent::MeetingSuggestion),
            f.meeting_suggestion_external,
        );
        assert_eq!(
            geometry_for_content(None, OverlayContent::MeetingSuggestion),
            f.meeting_suggestion_fallback,
        );
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
    fn content_names_are_explicit_and_unknown_profiles_are_rejected() {
        assert_eq!(OverlayContent::default(), OverlayContent::Controls);
        assert_eq!(
            serde_json::from_str::<OverlayContent>("\"meeting_suggestion\"").unwrap(),
            OverlayContent::MeetingSuggestion,
        );
        assert!(serde_json::from_str::<OverlayContent>("\"meeting\"").is_err());
    }

    #[test]
    fn suggestion_profile_preserves_notch_and_restores_controls_on_collapse() {
        for notch in [
            None,
            Some(notched(185.0, 32.0)),
            Some(external(30.0)),
            Some(notched(300.0, 37.0)),
        ] {
            let controls = geometry_for(notch);
            let suggestion = geometry_for_content(notch, OverlayContent::MeetingSuggestion);
            assert_eq!(
                geometry_for_content(notch, OverlayContent::Controls),
                controls
            );
            assert_eq!(suggestion.window_w, controls.window_w.max(320.0));
            assert_eq!(suggestion.collapsed_h, controls.collapsed_h);
            assert_eq!(suggestion.expanded_h, controls.collapsed_h + 136.0);
            assert_eq!(suggestion.pill_idle_w, controls.pill_idle_w);
            assert_eq!(
                suggestion.pill_margin_idle + controls.window_w / 2.0,
                suggestion.window_w / 2.0
            );
            assert_eq!(
                applied_surface_for(
                    &geometry_for_layout(
                        notch,
                        OverlayContent::MeetingSuggestion,
                        OverlayLayout {
                            activity: OverlayActivity::Recording,
                            expanded: true,
                            ..OverlayLayout::default()
                        }
                    ),
                    true
                ),
                applied_surface_for(&suggestion, true),
            );
            assert_eq!(
                applied_surface_for(
                    &geometry_for_layout(
                        notch,
                        OverlayContent::MeetingSuggestion,
                        OverlayLayout {
                            activity: OverlayActivity::Recording,
                            ..OverlayLayout::default()
                        }
                    ),
                    false
                ),
                applied_surface_for(&controls, false),
            );
        }
    }

    #[test]
    fn width_transitions_preserve_calibrated_top_and_do_not_drift() {
        for scale in [1.0, 2.0] {
            for notch in [None, Some(notched(185.0, 32.0)), Some(external(30.0))] {
                let controls = geometry_for(notch);
                let suggestion = geometry_for_content(notch, OverlayContent::MeetingSuggestion);
                let normal_width = controls.window_w;
                let suggestion_width = suggestion.window_w;
                for original in [
                    (5120.0 / scale, -24.0 / scale),
                    (-2560.0 / scale, 140.0 / scale),
                ] {
                    let mut current = original;
                    for _ in 0..20 {
                        let expanded =
                            resized_logical_position(current, normal_width, suggestion_width);
                        assert_eq!(expanded.1, original.1);
                        let old_center = current.0 + normal_width / 2.0;
                        let new_center = expanded.0 + suggestion_width / 2.0;
                        assert_eq!(old_center, new_center);
                        assert_eq!(
                            resized_logical_position(expanded, suggestion_width, suggestion_width),
                            expanded,
                        );
                        current =
                            resized_logical_position(expanded, suggestion_width, normal_width);
                        assert_eq!(current, original);
                    }
                }
            }
        }
    }

    #[test]
    fn applied_surface_tracks_notched_and_fallback_geometry_states() {
        let notched = geometry_for(Some(notched(185.0, 32.0)));
        assert_eq!(
            applied_surface_for(&notched, false),
            AppliedSurface {
                window_w: 241.0,
                window_h: 32.0,
            }
        );
        assert_eq!(
            applied_surface_for(&notched, true),
            AppliedSurface {
                window_w: 241.0,
                window_h: 96.0,
            }
        );
        let fallback = geometry_for(None);
        assert_eq!(
            applied_surface_for(&fallback, false),
            AppliedSurface {
                window_w: 241.0,
                window_h: 37.0,
            }
        );
        assert_eq!(
            applied_surface_for(&fallback, true),
            AppliedSurface {
                window_w: 241.0,
                window_h: 101.0,
            }
        );
    }
}
