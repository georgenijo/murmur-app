use super::overlay_band::{self, resolve_band_center, Band, FrameRect, StatusItems};
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
    /// Global logical x of the centre of the free menu-bar slot the collapsed
    /// floating pill occupies. `Some` only when the bar is visible and its
    /// occupancy was sampled and left a gap wide enough; `None` means the
    /// standalone placement below the menu bar. Every placement decision
    /// (height, anchoring, x) derives from this one resolved value.
    band_center_x: Option<f64>,
}

impl NotchInfo {
    pub(crate) fn height(self) -> f64 {
        self.height
    }

    /// The collapsed floating pill sits inside the menu-bar band.
    fn in_menu_band(self) -> bool {
        !self.physical_notch && self.menu_bar_visible && self.band_center_x.is_some()
    }
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
const DROPDOWN_H: f64 = 64.0;
const FLOATING_W: f64 = 280.0;
const FLOATING_TOP_H: f64 = 36.0;
/// Shortest collapsed floating pill: fits the 14pt waveform and 16pt icons.
const FLOATING_MIN_H: f64 = 24.0;
const FLOATING_GAP: f64 = 8.0;
const POPOVER_GAP: f64 = 6.0;
const MEETING_SUGGESTION_MIN_W: f64 = 320.0;
const MEETING_SUGGESTION_DROPDOWN_H: f64 = 136.0;
const FALLBACK_NOTCH_W: f64 = 80.0;
const FALLBACK_NOTCH_H: f64 = 37.0;
const MAX_REASONABLE_MENU_BAR_H: f64 = 128.0;
const MIN_VERTICAL_OFFSET: f64 = -12.0;
const MAX_VERTICAL_OFFSET: f64 = 12.0;
const POSITION_SETTLE_PROBE_MS: u64 = 50;
const POSITION_SETTLE_PROBES: usize = 3;
/// Clearance kept between the pill and the nearest menu or status item.
const BAND_ITEM_MARGIN: f64 = 8.0;
/// Free menu-bar width the pill needs: the widest window it grows into (the
/// meeting-suggestion surface). The window takes mouse events across its whole
/// width, so every point of it must lie over empty bar.
const BAND_REQUIRED_CLEAR_W: f64 = MEETING_SUGGESTION_MIN_W;

fn clamp_vertical_offset(offset: f64) -> f64 {
    if offset.is_finite() {
        offset.clamp(MIN_VERTICAL_OFFSET, MAX_VERTICAL_OFFSET)
    } else {
        0.0
    }
}

fn geometry_for(notch: Option<NotchInfo>) -> OverlayGeometry {
    let notch_w = notch.map_or(FALLBACK_NOTCH_W, |info| info.width);
    let notch_h = notch.map_or(FALLBACK_NOTCH_H, |info| info.height);
    // When native screen measurement is unavailable, use the spacious
    // floating surface. A synthetic 80pt center cannot fit quick controls.
    let floating = notch.is_none_or(|info| !info.physical_notch);
    // Inside the menu-bar band the collapsed pill takes the bar's own height so
    // it never reaches down over browser tab strips; the standalone placement
    // keeps the 36pt height. Both follow the same resolved band decision.
    let floating_h = notch
        .filter(|info| info.in_menu_band())
        .map_or(FLOATING_TOP_H, |info| {
            info.height.clamp(FLOATING_MIN_H, FLOATING_TOP_H)
        });
    let window_w = if floating {
        FLOATING_W
    } else {
        notch_w + 2.0 * WING
    };
    OverlayGeometry {
        window_w,
        collapsed_h: if floating { floating_h } else { notch_h },
        expanded_h: if floating {
            floating_h + DROPDOWN_H
        } else {
            notch_h + DROPDOWN_H
        },
        // Idle ends at the notch's right edge; active reveals the right wing.
        pill_idle_w: if floating { FLOATING_W } else { notch_w + WING },
        pill_active_w: window_w,
        pill_margin_idle: 0.0,
        pill_margin_active: 0.0,
        dropdown_h: DROPDOWN_H,
        wing_w: WING,
        floating,
    }
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

fn surface_for_content(
    notch: Option<NotchInfo>,
    expanded: bool,
    content: OverlayContent,
) -> AppliedSurface {
    let content = if expanded {
        content
    } else {
        OverlayContent::Controls
    };
    applied_surface_for(&geometry_for_content(notch, content), expanded)
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
        // Occupancy is sampled separately; until then use the standalone layout.
        band_center_x: None,
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

/// Whether the collapsed floating pill sits inside the menu-bar band at the
/// very top of the display (emulating the notch) rather than below the work
/// area. Requires a measured, visible menu bar whose occupancy left a free slot
/// (see `NotchInfo::band_center_x`); an auto-hidden bar, failed measurement,
/// unknown occupancy, or no gap falls back to the work-area placement.
fn floating_in_menu_bar_band(notch: Option<NotchInfo>) -> bool {
    notch.is_some_and(NotchInfo::in_menu_band)
}

/// Left edge of the floating window: centred on the resolved menu-bar slot, or
/// the display-centred default when the pill is not placed in the band.
fn floating_window_x(centered_x: f64, notch: Option<NotchInfo>, window_w: f64) -> f64 {
    notch
        .filter(|info| info.in_menu_band())
        .and_then(|info| info.band_center_x)
        .map_or(centered_x, |center| center - window_w / 2.0)
}

/// Lowest `y` the floating window may take, and the `y` it takes with zero
/// calibration offset.
fn floating_anchor(frame_top: f64, work_top: f64, in_menu_bar_band: bool) -> (f64, f64) {
    if in_menu_bar_band {
        (frame_top, frame_top)
    } else {
        (work_top, work_top + FLOATING_GAP)
    }
}

fn floating_logical_position(
    centered_x: f64,
    frame_top: f64,
    work_area: (i32, i32, u32, u32),
    window_size: (f64, f64),
    scale_factor: f64,
    vertical_offset: f64,
    in_menu_bar_band: bool,
) -> (f64, f64) {
    let (work_x, work_y, work_w, work_h) = work_area;
    let left = work_x as f64 / scale_factor;
    let top = work_y as f64 / scale_factor;
    let (floor_y, base_y) = floating_anchor(frame_top, top, in_menu_bar_band);
    let rightmost_x = (left + work_w as f64 / scale_factor - window_size.0).max(left);
    let bottommost_y = (top + work_h as f64 / scale_factor - window_size.1).max(floor_y);
    let desired_y = base_y + vertical_offset;
    (
        centered_x.clamp(left, rightmost_x),
        desired_y.clamp(floor_y, bottommost_y),
    )
}

/// On a floating display, reserve enough vertical space for expanded controls,
/// even if the pill is currently collapsed or calibration is applied later.
/// Returns `None` under a physical notch, where callers use their own inset.
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
    let geometry = geometry_for(notch);
    let scale = monitor.scale_factor();
    let frame_top = monitor.position().y as f64 / scale;
    let work_top = monitor.work_area().position.y as f64 / scale;
    let work_bottom =
        (monitor.work_area().position.y as f64 + monitor.work_area().size.height as f64) / scale;
    overlay_popover_fit_for_geometry(
        frame_top,
        work_top,
        work_bottom,
        popover_height,
        &geometry,
        floating_in_menu_bar_band(notch),
    )
}

fn overlay_popover_fit_for_geometry(
    frame_top: f64,
    work_top: f64,
    work_bottom: f64,
    popover_height: f64,
    geometry: &OverlayGeometry,
    in_menu_bar_band: bool,
) -> Option<(f64, f64)> {
    // Under a physical notch the collapsed pill fills the notch; callers keep
    // their popover tucked just below it. Reserving the full hover-expanded
    // height there put the live transcript far from the pill.
    if !geometry.floating {
        return None;
    }
    let (_, base_y) = floating_anchor(frame_top, work_top, in_menu_bar_band);
    let below_overlay = base_y + MAX_VERTICAL_OFFSET + geometry.expanded_h + POPOVER_GAP;
    let top = below_overlay.clamp(work_top, work_bottom);
    Some((top, popover_height.min((work_bottom - top).max(0.0))))
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

/// Resolve where the collapsed floating pill may sit in the menu-bar band of
/// the display spanning `[display_left, display_left + display_w]` at the top
/// of global logical coordinates, from the sampled menu and status items.
/// Notched displays and hidden bars are returned unchanged; unknown occupancy
/// or a band with no wide enough gap leaves `band_center_x` unset, which selects
/// the standalone placement.
fn with_menu_bar_slot(
    info: NotchInfo,
    display_left: f64,
    display_w: f64,
    menu_items: Option<&[FrameRect]>,
    status_items: Option<&StatusItems>,
) -> NotchInfo {
    if info.physical_notch || !info.menu_bar_visible {
        return NotchInfo {
            band_center_x: None,
            ..info
        };
    }
    let band = Band {
        left: display_left,
        right: display_left + display_w,
        top: 0.0,
        bottom: info.height,
    };
    NotchInfo {
        band_center_x: resolve_band_center(
            band,
            menu_items,
            status_items.map(|items| items.frames.as_slice()),
            status_items.map_or(0, |items| items.unanswered),
            BAND_ITEM_MARGIN,
            BAND_REQUIRED_CLEAR_W,
        ),
        ..info
    }
}

/// Detect notch width and the menu-bar slot, and configure the overlay as a
/// notch-level window. Uses native NSScreen APIs — no subprocess needed.
/// `menu_items` are the frontmost application's menu-title frames and
/// `status_items` every status item's frame, both sampled off the main thread
/// because Accessibility can block; `None` means unknown.
#[cfg(target_os = "macos")]
pub(crate) fn detect_notch_info(
    menu_items: Option<&[FrameRect]>,
    status_items: Option<&StatusItems>,
) -> Option<NotchInfo> {
    // Returns (notch_or_synthetic_center_width, actual_menu_bar_height) in
    // logical points.
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
    let measured = notch_info_from_screen_measurements(
        frame.size.width,
        frame_top,
        visible_frame_top,
        insets.top,
        left_w,
        right_w,
    );
    // The menu-bar display is the primary one, whose top edge is y = 0 in the
    // top-left global space that Accessibility reports.
    let info = with_menu_bar_slot(
        measured,
        frame.origin.x,
        frame.size.width,
        menu_items,
        status_items,
    );
    tracing::info!(
        target: "system",
        "detect_notch_info: notch_w={}, menu_bar_h={}, safe_top={}, visible_top_gap={}, screen_w={}, band_center_x={:?}, menu_items={:?}, status_items={:?}, status_unanswered={:?}",
        info.width,
        info.height,
        insets.top,
        (frame_top - visible_frame_top).max(0.0),
        frame.size.width,
        info.band_center_x,
        menu_items.map(<[FrameRect]>::len),
        status_items.map(|items| items.frames.len()),
        status_items.map(|items| items.unanswered)
    );
    Some(info)
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn detect_notch_info(
    _menu_items: Option<&[FrameRect]>,
    _status_items: Option<&StatusItems>,
) -> Option<NotchInfo> {
    None
}

/// Capture one display snapshot on the main thread. `menu_items` is the
/// frontmost application's menu-title sample and `status_items` the status-item
/// sample (`None` while unknown, which selects the standalone placement).
pub(crate) fn capture_display_snapshot(
    app_handle: &tauri::AppHandle,
    menu_items: Option<&[FrameRect]>,
    status_items: Option<&StatusItems>,
) -> DisplaySnapshot {
    let monitor = app_handle.primary_monitor().ok().flatten();
    let notch_info = detect_notch_info(menu_items, status_items);
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
/// together: menu-bar visibility and height, menu/status occupancy, placement
/// mode, native window size, and the geometry emitted to the frontend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlacementTrigger {
    Startup,
    DisplayChange,
    FrontmostApp,
    SpaceChange,
    /// One delayed re-check after an app or Space change, once the menu bar has
    /// finished repopulating or animating in/out.
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

    /// Display topology can turn a notched machine into an external one (and
    /// startup has no earlier sample), so these always sample menu occupancy.
    fn forces_sampling(self) -> bool {
        matches!(self, Self::Startup | Self::DisplayChange)
    }

    /// Delayed re-checks after this trigger, each measured from the previous
    /// refresh. App and Space changes need one, once the menu bar finishes
    /// repopulating. Startup and display changes get a short series: Murmur is
    /// usually frontmost while it launches (an update relaunches it), and its
    /// own menu bar cannot answer Accessibility while setup occupies the main
    /// thread, so the first sample is often unknown.
    fn settle_delays(self) -> &'static [std::time::Duration] {
        const APP: &[std::time::Duration] = &[PLACEMENT_SETTLE];
        const LAUNCH: &[std::time::Duration] = &[
            PLACEMENT_SETTLE,
            std::time::Duration::from_millis(1500),
            std::time::Duration::from_millis(3000),
        ];
        match self {
            Self::FrontmostApp | Self::SpaceChange => APP,
            Self::Startup | Self::DisplayChange => LAUNCH,
            Self::Settle => &[],
        }
    }
}

/// Keep the trigger that forces sampling, otherwise the most recent one.
fn merge_triggers(current: PlacementTrigger, next: PlacementTrigger) -> PlacementTrigger {
    if current.forces_sampling() && !next.forces_sampling() {
        current
    } else {
        next
    }
}

/// Burst coalescing: rapid app switching or a Space animation yields many
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

/// Re-resolve the whole placement for `trigger`: sample the frontmost menus and
/// the status items off the main thread, then apply everything together on it.
#[cfg(target_os = "macos")]
fn refresh_placement(app_handle: &tauri::AppHandle, trigger: PlacementTrigger) {
    let cached_physical_notch = app_handle.try_state::<State>().is_some_and(|state| {
        state
            .notch_info
            .lock_or_recover()
            .is_some_and(|info| info.physical_notch)
    });
    // A physical notch never uses the band, so skip the Accessibility round trips.
    let (menu_items, status_items) = if cached_physical_notch && !trigger.forces_sampling() {
        (None, None)
    } else {
        (
            overlay_band::frontmost_menu_item_frames(),
            overlay_band::status_items(),
        )
    };
    let handle = app_handle.clone();
    if let Err(error) = app_handle.run_on_main_thread(move || {
        apply_placement(
            &handle,
            trigger,
            menu_items.as_deref(),
            status_items.as_ref(),
        );
    }) {
        tracing::warn!(
            target: "system",
            "overlay placement main-thread dispatch failed: {}",
            error
        );
    }
}

/// Main-thread half of `refresh_placement`: one snapshot, one resolved mode.
/// Repositions, resizes and notifies the frontend only when it changed.
#[cfg(target_os = "macos")]
fn apply_placement(
    handle: &tauri::AppHandle,
    trigger: PlacementTrigger,
    menu_items: Option<&[FrameRect]>,
    status_items: Option<&StatusItems>,
) {
    let snapshot = capture_display_snapshot(handle, menu_items, status_items);
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
        trigger = trigger.as_str(),
        in_menu_bar_band = floating_in_menu_bar_band(notch),
        "overlay placement refreshed"
    );
    if !changed {
        return;
    }
    if let Some(overlay) = handle.get_webview_window("overlay") {
        position_overlay_default(&overlay, notch, trigger.as_str());
    }
    // The frontend treats this event as authoritative: it collapses and re-reads
    // the geometry (and reapplies the saved vertical calibration).
    let _ = handle.emit("overlay-geometry-changed", geometry_for(notch));
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
                        Ok(next) => trigger = merge_triggers(trigger, next),
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
                            refresh_placement(&app_handle, PlacementTrigger::Settle)
                        }
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
            }
        })
        .expect("overlay placement worker must spawn");
}

/// Start the placement worker and subscribe to macOS display configuration
/// changes (plug/unplug monitor, lid open/close). App and Space changes arrive
/// through `frontmost::register_delivery_transition_observers`.
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
    let g = geometry_for(notch_info);
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
        let (x, y) = if g.floating {
            let area = monitor.work_area();
            let in_band = floating_in_menu_bar_band(notch_info);
            floating_logical_position(
                floating_window_x(centered_x, notch_info, overlay_w),
                centered_y,
                (
                    area.position.x,
                    area.position.y,
                    area.size.width,
                    area.size.height,
                ),
                (overlay_w, overlay_h),
                sf,
                0.0,
                in_band,
            )
        } else {
            (centered_x, centered_y)
        };

        if let Err(e) = overlay.set_size(tauri::LogicalSize::new(overlay_w, overlay_h)) {
            tracing::warn!(target: "system", "position_overlay_default: set_size({}, {}) failed: {}", overlay_w, overlay_h, e);
        }
        tracing::info!(target: "system", "position_overlay_default: x={}, y={}, sf={}", x, y, sf);
        if let Err(e) = overlay.set_position(tauri::LogicalPosition::new(x, y)) {
            tracing::warn!(target: "system", "position_overlay_default: set_position({}, {}) failed: {}", x, y, e);
        } else {
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
    geometry_for_content(
        *state.notch_info.lock_or_recover(),
        content.unwrap_or_default(),
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

/// Resize the overlay through the expansion controller's single writer. A
/// suggestion grows around the current center; every resize retains the
/// calibrated top edge. Collapsing restores the ordinary controls dimensions.
/// The acknowledgment resolves after the main-thread frame update, before the
/// frontend reveals content or starts accepting clicks in the expanded area.
#[tauri::command]
pub async fn set_overlay_expanded(
    app: tauri::AppHandle,
    state: tauri::State<'_, State>,
    expanded: bool,
    content: Option<OverlayContent>,
) -> Result<AppliedSurface, String> {
    let applied = surface_for_content(
        *state.notch_info.lock_or_recover(),
        expanded,
        content.unwrap_or_default(),
    );
    #[cfg(not(target_os = "macos"))]
    {
        let _ = &app;
        return Ok(applied);
    }

    #[cfg(target_os = "macos")]
    {
        let overlay = app
            .get_webview_window("overlay")
            .ok_or_else(|| "overlay window not found".to_string())?;
        let (tx, rx) = tokio::sync::oneshot::channel();
        app.run_on_main_thread(move || {
            let result = (|| {
                let position = overlay
                    .outer_position()
                    .map_err(|error| error.to_string())?;
                let size = overlay.outer_size().map_err(|error| error.to_string())?;
                let scale = overlay.scale_factor().map_err(|error| error.to_string())?;
                let logical_position = (position.x as f64 / scale, position.y as f64 / scale);
                let current_width = size.width as f64 / scale;
                let (x, y) =
                    resized_logical_position(logical_position, current_width, applied.window_w);
                overlay
                    .set_size(tauri::LogicalSize::new(applied.window_w, applied.window_h))
                    .map_err(|error| error.to_string())?;
                overlay
                    .set_position(tauri::LogicalPosition::new(x, y))
                    .map_err(|error| error.to_string())?;
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
        let _ = &state;
        let monitor = app
            .primary_monitor()
            .map_err(|error| error.to_string())?
            .or_else(|| overlay.current_monitor().ok().flatten())
            .ok_or_else(|| "no monitor available".to_string())?;
        let position = monitor.position();
        let size = monitor.size();
        let scale_factor = monitor.scale_factor();
        let monitor_position = (position.x, position.y);
        let monitor_size = (size.width, size.height);
        let monitor_x = position.x;
        let monitor_y = position.y;
        let overlay_to_move = overlay.clone();
        let notch = *state.notch_info.lock_or_recover();
        let floating = geometry_for(notch).floating;
        let area = monitor.work_area();
        let work_area = (
            area.position.x,
            area.position.y,
            area.size.width,
            area.size.height,
        );
        let (tx, rx) = tokio::sync::oneshot::channel();
        app.run_on_main_thread(move || {
            let result = (|| {
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
                let (x, y) = if floating {
                    let in_band = floating_in_menu_bar_band(notch);
                    floating_logical_position(
                        floating_window_x(centered_x, notch, width),
                        top_y,
                        work_area,
                        (width, height),
                        scale_factor,
                        offset,
                        in_band,
                    )
                } else {
                    (centered_x, top_y + offset)
                };
                overlay_to_move
                    .set_position(tauri::LogicalPosition::new(x, y))
                    .map_err(|error| error.to_string())?;
                Ok::<_, String>((
                    (x * scale_factor).round() as i32,
                    (y * scale_factor).round() as i32,
                ))
            })();
            let _ = tx.send(result);
        })
        .map_err(|error| error.to_string())?;
        let (x, y) = tokio::time::timeout(std::time::Duration::from_secs(2), rx)
            .await
            .map_err(|_| "overlay position update timed out".to_string())?
            .map_err(|_| "overlay position update was dropped".to_string())??;

        let probe_app = app.clone();
        tauri::async_runtime::spawn(async move {
            match settled_outer_position(probe_app, overlay, (x, y)).await {
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
        });
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

    fn notched(width: f64, height: f64) -> NotchInfo {
        NotchInfo {
            width,
            height,
            physical_notch: true,
            menu_bar_visible: true,
            band_center_x: None,
        }
    }

    fn external(height: f64) -> NotchInfo {
        NotchInfo {
            width: FALLBACK_NOTCH_W,
            height,
            physical_notch: false,
            menu_bar_visible: true,
            band_center_x: Some(1280.0),
        }
    }

    /// Visible menu bar, but its occupancy is unknown or leaves no gap.
    fn external_without_slot(height: f64) -> NotchInfo {
        NotchInfo {
            band_center_x: None,
            ..external(height)
        }
    }

    /// Auto-hidden menu bar or failed native measurement on a non-notched display.
    fn external_without_menu_bar() -> NotchInfo {
        NotchInfo {
            menu_bar_visible: false,
            band_center_x: None,
            ..external(FALLBACK_NOTCH_H)
        }
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
            (257.0, 32.0, 96.0, 221.0, 257.0, 0.0, 0.0, 64.0, 36.0)
        );
    }

    #[test]
    fn external_display_uses_its_measured_menu_bar_height() {
        let measured = notch_info_from_screen_measurements(2560.0, 1440.0, 1410.0, 0.0, 0.0, 0.0);
        // Measurement alone proves nothing about menu occupancy: no slot yet.
        assert_eq!(measured, external_without_slot(30.0));
        let info = NotchInfo {
            band_center_x: Some(1280.0),
            ..measured
        };
        assert_eq!(info, external(30.0));

        let g = geometry_for(Some(info));
        assert_eq!(g.window_w, 280.0);
        assert_eq!(g.pill_idle_w, 280.0);
        assert_eq!(g.pill_margin_idle, 0.0);
        assert_eq!(g.pill_active_w, 280.0);
        // The collapsed pill takes the menu bar's own height so it fits the band.
        assert_eq!(g.collapsed_h, 30.0);
        assert_eq!(g.expanded_h, 94.0);
        assert!(g.floating);
    }

    #[test]
    fn collapsed_floating_pill_height_follows_the_menu_bar_within_bounds() {
        let collapsed = |info| geometry_for(Some(info)).collapsed_h;
        assert_eq!(collapsed(external(24.0)), 24.0);
        assert_eq!(collapsed(external(30.0)), 30.0);
        // Implausibly short or tall bars (e.g. a Dock at the top) are clamped.
        assert_eq!(collapsed(external(10.0)), FLOATING_MIN_H);
        assert_eq!(collapsed(external(90.0)), FLOATING_TOP_H);
        // No visible bar, or no safe slot in it: the standalone 36pt pill.
        assert_eq!(collapsed(external_without_menu_bar()), FLOATING_TOP_H);
        assert_eq!(collapsed(external_without_slot(30.0)), FLOATING_TOP_H);
        assert_eq!(geometry_for(None).collapsed_h, FLOATING_TOP_H);
    }

    #[test]
    fn menu_bar_measurement_flags_hidden_or_invalid_bars() {
        assert!(
            notch_info_from_screen_measurements(2560.0, 1440.0, 1416.0, 0.0, 0.0, 0.0)
                .menu_bar_visible
        );
        // Auto-hide: visible frame top == frame top.
        assert_eq!(
            notch_info_from_screen_measurements(2560.0, 1440.0, 1440.0, 0.0, 0.0, 0.0),
            external_without_menu_bar()
        );
        // Notched display: the safe-area inset is the bar.
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
    fn floating_position_uses_selected_display_work_area_at_both_scales() {
        // Work-area fallback: 8pt below the work-area top.
        assert_eq!(
            floating_logical_position(
                1204.0,
                0.0,
                (0, 60, 5120, 2820),
                (280.0, 100.0),
                2.0,
                0.0,
                false
            ),
            (1204.0, 38.0),
        );
        assert_eq!(
            floating_logical_position(
                -1960.0,
                0.0,
                (-2560, 30, 2560, 1410),
                (280.0, 100.0),
                1.0,
                0.0,
                false
            ),
            (-1960.0, 38.0),
        );
        assert_eq!(
            floating_logical_position(
                5000.0,
                0.0,
                (0, 60, 5120, 2820),
                (280.0, 100.0),
                2.0,
                -12.0,
                false
            ),
            (2280.0, 30.0),
        );
    }

    #[test]
    fn collapsed_floating_pill_sits_inside_the_menu_bar_band() {
        let info = external(24.0);
        let g = geometry_for(Some(info));
        // 1x external display, menu bar 24pt tall.
        let (x, y) = floating_logical_position(
            1140.0,
            0.0,
            (0, 24, 2560, 1416),
            (g.window_w, g.collapsed_h),
            1.0,
            0.0,
            floating_in_menu_bar_band(Some(info)),
        );
        assert_eq!((x, y), (1140.0, 0.0));
        assert!(y + g.collapsed_h <= 24.0, "pill must not pass the menu bar");

        // 2x display on a secondary-origin monitor: logical frame top 128.
        let (_, y) = floating_logical_position(
            1204.0,
            128.0,
            (0, 304, 5120, 2576),
            (g.window_w, g.collapsed_h),
            2.0,
            0.0,
            floating_in_menu_bar_band(Some(info)),
        );
        assert_eq!(y, 128.0);
        assert!(y + g.collapsed_h <= 152.0);
    }

    #[test]
    fn menu_bar_band_calibration_never_rises_above_the_display() {
        let args = |offset| {
            floating_logical_position(
                0.0,
                0.0,
                (0, 24, 2560, 1416),
                (280.0, 24.0),
                1.0,
                offset,
                true,
            )
            .1
        };
        assert_eq!(args(-12.0), 0.0);
        assert_eq!(args(7.0), 7.0);
        assert_eq!(args(12.0), 12.0);
    }

    #[test]
    fn menu_bar_band_requires_a_visible_bar_a_free_slot_and_a_non_notched_display() {
        assert!(floating_in_menu_bar_band(Some(external(24.0))));
        // Visible bar whose occupancy is unknown or leaves no gap.
        assert!(!floating_in_menu_bar_band(Some(external_without_slot(
            24.0
        ))));
        // Auto-hidden bar or failed measurement.
        assert!(!floating_in_menu_bar_band(
            Some(external_without_menu_bar())
        ));
        assert!(!floating_in_menu_bar_band(None));
        // Notched displays keep their own layout, even if a slot were recorded.
        assert!(!floating_in_menu_bar_band(Some(notched(185.0, 32.0))));
        assert!(!floating_in_menu_bar_band(Some(NotchInfo {
            band_center_x: Some(756.0),
            ..notched(185.0, 32.0)
        })));
    }

    #[test]
    fn height_and_anchor_follow_the_same_resolved_mode() {
        // A visible bar with no safe slot must not keep the shorter band height
        // while anchoring below the work area (or vice versa).
        for info in [
            external(30.0),
            external_without_slot(30.0),
            external_without_menu_bar(),
        ] {
            let g = geometry_for(Some(info));
            let in_band = floating_in_menu_bar_band(Some(info));
            assert_eq!(g.collapsed_h, if in_band { 30.0 } else { FLOATING_TOP_H });
            let (floor, base) = floating_anchor(0.0, 30.0, in_band);
            assert_eq!(
                (floor, base),
                if in_band { (0.0, 0.0) } else { (30.0, 38.0) }
            );
        }
    }

    #[test]
    fn floating_window_x_centres_on_the_resolved_slot() {
        // Display-centred default when not in the band.
        assert_eq!(floating_window_x(1140.0, None, 280.0), 1140.0);
        assert_eq!(
            floating_window_x(1140.0, Some(external_without_slot(24.0)), 280.0),
            1140.0
        );
        assert_eq!(
            floating_window_x(1140.0, Some(notched(185.0, 32.0)), 257.0),
            1140.0
        );
        // Slot centre 1800 -> a 280pt window starts at 1660; stays centred as the
        // window widens for a meeting suggestion.
        let shifted = Some(NotchInfo {
            band_center_x: Some(1800.0),
            ..external(24.0)
        });
        assert_eq!(floating_window_x(1140.0, shifted, 280.0), 1660.0);
        assert_eq!(floating_window_x(1140.0, shifted, 320.0), 1640.0);
    }

    #[test]
    fn menu_bar_slot_resolution_falls_back_without_proof_of_free_space() {
        let measured = notch_info_from_screen_measurements(2560.0, 1440.0, 1416.0, 0.0, 0.0, 0.0);
        let item = |x, width| FrameRect {
            x,
            y: 0.0,
            width,
            height: 24.0,
        };
        let menus = [item(0.0, 500.0)];
        let status = StatusItems {
            frames: vec![item(2300.0, 260.0)],
            unanswered: 0,
        };
        let resolve = |menus: Option<&[FrameRect]>, status: Option<&StatusItems>| {
            with_menu_bar_slot(measured, 0.0, 2560.0, menus, status).band_center_x
        };
        assert_eq!(resolve(Some(&menus), Some(&status)), Some(1280.0));
        // Unknown occupancy (Accessibility failure or timeout).
        assert_eq!(resolve(None, Some(&status)), None);
        assert_eq!(resolve(Some(&menus), None), None);
        // A long menu reaching the centre moves the slot; no room left means none.
        let wide = [item(0.0, 1400.0)];
        assert_eq!(resolve(Some(&wide), Some(&status)), Some(1568.0));
        let full = [item(0.0, 2000.0)];
        assert_eq!(resolve(Some(&full), Some(&status)), None);
        // Status items that did not answer reserve room left of the run.
        let unanswered = StatusItems {
            unanswered: 8,
            ..status.clone()
        };
        assert_eq!(resolve(Some(&menus), Some(&unanswered)), Some(1108.0));
        // Hidden bar and physical notch never get a slot.
        let hidden = notch_info_from_screen_measurements(2560.0, 1440.0, 1440.0, 0.0, 0.0, 0.0);
        assert_eq!(
            with_menu_bar_slot(hidden, 0.0, 2560.0, Some(&menus), Some(&status)).band_center_x,
            None
        );
        let notch = notch_info_from_screen_measurements(1512.0, 982.0, 950.0, 32.0, 663.5, 663.5);
        assert_eq!(
            with_menu_bar_slot(notch, 0.0, 1512.0, Some(&menus), Some(&status)).band_center_x,
            None
        );
    }

    #[test]
    fn placement_triggers_coalesce_toward_the_sampling_trigger() {
        use PlacementTrigger::*;
        assert_eq!(merge_triggers(FrontmostApp, SpaceChange), SpaceChange);
        assert_eq!(merge_triggers(DisplayChange, FrontmostApp), DisplayChange);
        assert_eq!(merge_triggers(Startup, SpaceChange), Startup);
        assert_eq!(merge_triggers(SpaceChange, DisplayChange), DisplayChange);
        assert!(Startup.forces_sampling() && DisplayChange.forces_sampling());
        assert!(!FrontmostApp.forces_sampling() && !SpaceChange.forces_sampling());
    }

    #[test]
    fn launch_triggers_recheck_until_the_menu_bar_can_answer() {
        use PlacementTrigger::*;
        let total = |trigger: PlacementTrigger| -> std::time::Duration {
            trigger.settle_delays().iter().sum()
        };
        assert_eq!(FrontmostApp.settle_delays(), &[PLACEMENT_SETTLE]);
        assert_eq!(SpaceChange.settle_delays(), &[PLACEMENT_SETTLE]);
        assert!(Settle.settle_delays().is_empty());
        // Startup and display changes keep re-checking for a few seconds, so
        // an unknown first sample cannot strand the pill below the bar.
        for trigger in [Startup, DisplayChange] {
            assert!(trigger.settle_delays().len() > 1);
            assert!(total(trigger) >= std::time::Duration::from_secs(4));
            assert!(total(trigger) <= std::time::Duration::from_secs(10));
        }
    }

    #[test]
    fn hidden_menu_bar_falls_back_to_work_area_placement() {
        let info = external_without_menu_bar();
        let g = geometry_for(Some(info));
        let in_band = floating_in_menu_bar_band(Some(info));
        assert!(!in_band);
        assert_eq!(
            floating_logical_position(
                1140.0,
                0.0,
                (0, 0, 2560, 1440),
                (g.window_w, g.collapsed_h),
                1.0,
                0.0,
                in_band
            ),
            (1140.0, 8.0),
        );
    }

    #[test]
    fn floating_popovers_clear_expanded_overlay_and_stay_in_work_area() {
        // Standalone placement (36pt pill): 30 + 8 + 12 + 100 + 6.
        let geometry = geometry_for(Some(external_without_slot(30.0)));
        assert_eq!(
            overlay_popover_fit_for_geometry(0.0, 30.0, 1440.0, 104.0, &geometry, false),
            Some((156.0, 104.0))
        );
        assert_eq!(
            overlay_popover_fit_for_geometry(0.0, 30.0, 1440.0, 340.0, &geometry, false),
            Some((156.0, 340.0))
        );
        assert_eq!(
            overlay_popover_fit_for_geometry(0.0, 30.0, 400.0, 340.0, &geometry, false),
            Some((156.0, 244.0))
        );
    }

    #[test]
    fn menu_bar_band_popovers_clear_the_hover_expanded_overlay() {
        let info = external(24.0);
        let geometry = geometry_for(Some(info));
        // Expanded overlay spans y 0..88 (+12 calibration); popover starts 6pt below.
        let (top, height) =
            overlay_popover_fit_for_geometry(0.0, 24.0, 1440.0, 104.0, &geometry, true).unwrap();
        assert_eq!((top, height), (106.0, 104.0));
        assert!(top >= 12.0 + geometry.expanded_h);
        // Still capped by the work area.
        assert_eq!(
            overlay_popover_fit_for_geometry(0.0, 24.0, 200.0, 340.0, &geometry, true),
            Some((106.0, 94.0))
        );
    }

    #[test]
    fn notched_popovers_stay_tucked_under_the_notch() {
        // Callers fall back to their own under-notch inset instead of clearing
        // the hover-expanded overlay, which left a large gap below the pill.
        let notched = geometry_for(Some(notched(185.0, 32.0)));
        assert_eq!(
            overlay_popover_fit_for_geometry(0.0, 32.0, 1440.0, 104.0, &notched, false),
            None
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
            if controls.floating {
                assert_eq!(
                    suggestion.pill_margin_idle + suggestion.pill_idle_w / 2.0,
                    suggestion.window_w / 2.0
                );
            } else {
                assert_eq!(
                    suggestion.pill_margin_idle + controls.window_w / 2.0,
                    suggestion.window_w / 2.0
                );
            }
            assert_eq!(
                surface_for_content(notch, true, OverlayContent::MeetingSuggestion),
                applied_surface_for(&suggestion, true),
            );
            assert_eq!(
                surface_for_content(notch, false, OverlayContent::MeetingSuggestion),
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
                window_w: 257.0,
                window_h: 32.0,
            }
        );
        assert_eq!(
            applied_surface_for(&notched, true),
            AppliedSurface {
                window_w: 257.0,
                window_h: 96.0,
            }
        );

        let fallback = geometry_for(None);
        assert_eq!(
            applied_surface_for(&fallback, false),
            AppliedSurface {
                window_w: 280.0,
                window_h: 36.0,
            }
        );
        assert_eq!(
            applied_surface_for(&fallback, true),
            AppliedSurface {
                window_w: 280.0,
                window_h: 100.0,
            }
        );
    }
}
