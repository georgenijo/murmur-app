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
const PILL_W: f64 = 112.0;
const PILL_H: f64 = 34.0;
/// Distance between the pill and the bottom of the usable work area (Dock).
const PILL_BOTTOM_GAP: f64 = 14.0;
const PILL_EXPANDED_MIN_W: f64 = 241.0;
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

/// Conservative static placement for the interactive Voice Query popover. It
/// reserves the largest calibrated expanded overlay box so hover never moves
/// Copy/Close controls under the pointer. Dictation preview uses the separate
/// actual-frame follower below.
pub(crate) fn overlay_popover_fit(
    app: &tauri::AppHandle,
    monitor: &tauri::Monitor,
    popover_height: f64,
) -> Option<(f64, f64)> {
    let primary = app.primary_monitor().ok().flatten()?;
    if primary.position() != monitor.position() || primary.size() != monitor.size() {
        return None;
    }
    let state = app.try_state::<crate::State>()?;
    let notch = *state.notch_info.lock_or_recover();
    let mut layout = *state.overlay_layout.lock_or_recover();
    layout.expanded = true;
    let geometry = geometry_for_layout(notch, OverlayContent::Controls, layout);
    let scale = monitor.scale_factor();
    let frame_top = monitor.position().y as f64 / scale;
    let work_top = usable_top(
        frame_top,
        monitor.work_area().position.y as f64 / scale,
        notch,
    );
    let work_bottom =
        (monitor.work_area().position.y as f64 + monitor.work_area().size.height as f64) / scale;
    overlay_popover_fit_for_geometry(frame_top, work_top, work_bottom, popover_height, &geometry)
}

fn overlay_popover_fit_for_geometry(
    frame_top: f64,
    work_top: f64,
    work_bottom: f64,
    popover_height: f64,
    geometry: &OverlayGeometry,
) -> Option<(f64, f64)> {
    // The bottom pill leaves the top of the display free.
    let desired = if geometry.floating {
        work_top + POPOVER_GAP
    } else {
        frame_top + MAX_VERTICAL_OFFSET + geometry.expanded_h + POPOVER_GAP
    };
    let top = desired.clamp(work_top, work_bottom);
    Some((top, popover_height.min((work_bottom - top).max(0.0))))
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct PopoverWorkArea {
    left: f64,
    right: f64,
    top: f64,
    bottom: f64,
}

/// Actual-frame placement used only by the click-through live dictation
/// preview. It follows collapsed/expanded/calibrated overlay movement without
/// changing the interactive Voice Query surface under the pointer.
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
    let above = state.overlay_layout.lock_or_recover().placement == OverlayPlacement::FloatingPill;
    dictation_preview_fit_for_frame(work_area, (preview_width, preview_height), frame, above)
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
        let top = (frame.y - POPOVER_GAP - preview_height).max(work_area.top);
        return Some((x, top, preview_height.min((frame.y - top).max(0.0))));
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
pub(crate) fn capture_display_snapshot(app_handle: &tauri::AppHandle) -> DisplaySnapshot {
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
    if let Err(error) = app_handle.run_on_main_thread(move || {
        apply_placement(&handle, trigger);
    }) {
        tracing::warn!(target: "system", "overlay placement main-thread dispatch failed: {}", error);
    }
}

/// Main-thread half of `refresh_placement`: one snapshot, one resolved mode.
/// Repositions, resizes and notifies the frontend only when it changed.
#[cfg(target_os = "macos")]
fn apply_placement(handle: &tauri::AppHandle, trigger: PlacementTrigger) {
    let state = handle.state::<State>();
    let previous = state.display_snapshot.lock_or_recover().clone();
    let snapshot = capture_display_snapshot(handle);
    let notch = snapshot.notch_info;
    let changed = previous.as_ref() != Some(&snapshot);
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

    start_placement_worker(app_handle);

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
        let work_bottom = (area.position.y as f64 + area.size.height as f64) / sf;
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
                (work_left, work_top, work_right, work_bottom),
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
        let sized = if let Err(e) = overlay.set_size(tauri::LogicalSize::new(overlay_w, overlay_h))
        {
            tracing::warn!(target: "system", "position_overlay_default: set_size({}, {}) failed: {}", overlay_w, overlay_h, e);
            false
        } else {
            true
        };
        tracing::info!(target: "system", "position_overlay_default: x={}, y={}, sf={}", x, y, sf);
        if let Err(e) = overlay.set_position(tauri::LogicalPosition::new(x, y)) {
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
        if let Err(e) = overlay.set_size(tauri::LogicalSize::new(overlay_w, overlay_h)) {
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
        let (tx, rx) = tokio::sync::oneshot::channel();
        let dispatch_app = app.clone();
        app.run_on_main_thread(move || {
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
                            (left, work_top, right, bottom),
                            (applied.window_w, applied.window_h),
                        );
                    } else {
                        y = frame_top + position.applied_offset;
                        y = y.min((bottom - applied.window_h).max(frame_top));
                        x = x.clamp(left, (right - applied.window_w).max(left));
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
                        .set_size(tauri::LogicalSize::new(applied.window_w, applied.window_h))
                        .map_err(|error| error.to_string())?;
                    overlay
                        .set_position(tauri::LogicalPosition::new(x, y))
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
                follow_dictation_preview(&dispatch_app);
                // Emitted only after native setters succeed; the webview can
                // never reveal a larger surface before its native grow ack.
                let _ = dispatch_app.emit("overlay-surface-changed", geometry);
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
                let (centered_x, top_y) =
                    centered_logical_position(monitor_position, monitor_size, scale_factor, width);
                let work_left = area.position.x as f64 / scale_factor;
                let work_top = usable_top(top_y, area.position.y as f64 / scale_factor, notch);
                let work_right = work_left + area.size.width as f64 / scale_factor;
                let work_bottom = (area.position.y as f64 + area.size.height as f64) / scale_factor;
                let applied = applied_overlay_position(offset, floating);
                let (x, y) = if floating {
                    pill_logical_position(
                        (work_left, work_top, work_right, work_bottom),
                        (width, height),
                    )
                } else {
                    (centered_x, top_y + applied.applied_offset)
                };
                overlay_to_move
                    .set_position(tauri::LogicalPosition::new(x, y))
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
        ] {
            assert_ne!(baseline, changed);
        }
    }

    #[test]
    fn pill_sits_bottom_centre_above_the_dock_in_the_work_area() {
        // Bottom Dock: work area ends at 1340 on a 1440pt display.
        assert_eq!(
            pill_logical_position((0.0, 24.0, 2560.0, 1340.0), (PILL_W, PILL_H)),
            (1224.0, 1340.0 - PILL_BOTTOM_GAP - PILL_H),
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
            x: 1224.0,
            y: 1352.0,
            width: PILL_W,
            height: PILL_H,
        };
        assert_eq!(
            dictation_preview_fit_for_frame(work_area, (560.0, 44.0), Some(pill_frame), true),
            Some((1000.0, 1302.0, 44.0))
        );
        // A short work area clips the preview rather than overlapping the pill.
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
            Some((1000.0, 1330.0, 22.0))
        );
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
    fn query_popovers_keep_conservative_static_clearance() {
        let notched_g = geometry_for(Some(notched(185.0, 32.0)));
        assert_eq!(
            overlay_popover_fit_for_geometry(0.0, 32.0, 1440.0, 104.0, &notched_g),
            Some((114.0, 104.0))
        );
        let pill_g = geometry_for_layout(
            Some(notched(185.0, 32.0)),
            OverlayContent::Controls,
            pill(OverlayActivity::Idle, true),
        );
        assert_eq!(
            overlay_popover_fit_for_geometry(0.0, 32.0, 1440.0, 104.0, &pill_g),
            Some((38.0, 104.0))
        );
        assert_eq!(
            overlay_popover_fit_for_geometry(0.0, 32.0, 100.0, 104.0, &notched_g),
            Some((100.0, 0.0))
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
