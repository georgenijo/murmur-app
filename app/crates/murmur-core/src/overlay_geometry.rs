//! Shared native overlay geometry, extracted without behavior changes.
#[derive(Clone, Debug, PartialEq)]
pub struct DisplaySnapshot {
    pub notch_info: Option<NotchInfo>,
    pub monitor_position: Option<(i32, i32)>,
    pub monitor_size: Option<(u32, u32)>,
    pub monitor_work_area: Option<(i32, i32, u32, u32)>,
    pub scale_factor: Option<f64>,
    /// An auto-hidden bottom Dock's size preferences. Resizing it leaves the
    /// work area unchanged, so it must invalidate placement on its own.
    pub hidden_dock_tile: Option<HiddenDock>,
}
pub const WING: f64 = 28.0;

pub const DROPDOWN_H: f64 = 64.0;

pub const PILL_W: f64 = 120.0;

pub const PILL_H: f64 = 34.0;

pub const PILL_BOTTOM_GAP: f64 = 14.0;

pub const PILL_EXPANDED_MIN_W: f64 = 241.0;

pub const DOCK_MIN_TILE: f64 = 16.0;

pub const DOCK_MAX_TILE: f64 = 128.0;

pub const DOCK_DEFAULT_TILE: f64 = 64.0;

pub const DOCK_MAGNIFIED_MARGIN: f64 = 52.0;

pub const FLOATING_MIN_H: f64 = 24.0;

pub const POPOVER_GAP: f64 = 6.0;

pub const MEETING_SUGGESTION_MIN_W: f64 = 320.0;

pub const MEETING_SUGGESTION_DROPDOWN_H: f64 = 136.0;

pub const FALLBACK_NOTCH_W: f64 = 185.0;

pub const FALLBACK_NOTCH_H: f64 = 37.0;

pub const MAX_REASONABLE_MENU_BAR_H: f64 = 128.0;

pub const MIN_VERTICAL_OFFSET: f64 = -12.0;

pub const MAX_VERTICAL_OFFSET: f64 = 12.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DockLimit {
    Unknown,
    Clear,
    Above(f64),
}

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
pub struct OverlayLayout {
    pub activity: OverlayActivity,
    pub placement: OverlayPlacement,
    pub expanded: bool,
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
pub struct OverlayFrame {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OverlayCalibrationState {
    pub requested_offset: f64,
    pub applied_offset: f64,
}

/// Size preferences of an auto-hidden bottom Dock: the static fallback used
/// when `dock_watch` has no live reading.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HiddenDock {
    pub tile: f64,
    /// Magnified icon size, when magnification is on.
    pub magnified: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NotchInfo {
    pub width: f64,
    pub height: f64,
    pub physical_notch: bool,
    /// A real menu bar occupies the top of the display (its height was measured
    /// from the screen frame / visible-frame gap). False when the bar is
    /// auto-hidden or the native measurement was unavailable or implausible.
    pub menu_bar_visible: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PopoverWorkArea {
    pub left: f64,
    pub right: f64,
    pub top: f64,
    pub bottom: f64,
}

pub fn clamp_vertical_offset(offset: f64) -> f64 {
    if offset.is_finite() {
        offset.clamp(MIN_VERTICAL_OFFSET, MAX_VERTICAL_OFFSET)
    } else {
        0.0
    }
}

pub fn geometry_for(notch: Option<NotchInfo>) -> OverlayGeometry {
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

pub fn geometry_for_layout(
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
pub fn overlay_click_through(layout: OverlayLayout) -> bool {
    layout.placement == OverlayPlacement::FloatingPill
        && layout.activity == OverlayActivity::Idle
        && !layout.expanded
}

pub fn geometry_for_content(notch: Option<NotchInfo>, content: OverlayContent) -> OverlayGeometry {
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
pub fn animation_surface(
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

pub fn resized_logical_position(
    current_position: (f64, f64),
    current_width: f64,
    target_width: f64,
) -> (f64, f64) {
    (
        current_position.0 + (current_width - target_width) / 2.0,
        current_position.1,
    )
}

pub fn applied_surface_for(g: &OverlayGeometry, expanded: bool) -> AppliedSurface {
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
pub fn notch_info_from_screen_measurements(
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

pub fn centered_logical_position(
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
pub fn usable_top(frame_top: f64, work_top: f64, notch: Option<NotchInfo>) -> f64 {
    work_top.max(frame_top + notch.filter(|n| n.physical_notch).map_or(0.0, |n| n.height))
}

/// Bottom-centre pill frame inside the work area, so it sits above the Dock
/// (including a side Dock). `work_area` is the logical (left, top, right,
/// bottom) of the usable area.
pub fn pill_logical_position(
    work_area: (f64, f64, f64, f64),
    window_size: (f64, f64),
) -> (f64, f64) {
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
pub fn hidden_dock_reserve(dock: HiddenDock) -> f64 {
    let bar = dock.tile.clamp(DOCK_MIN_TILE, DOCK_MAX_TILE) * 1.5 + 8.0;
    dock.magnified.map_or(bar, |size| {
        bar.max(size.clamp(DOCK_MIN_TILE, DOCK_MAX_TILE) + DOCK_MAGNIFIED_MARGIN)
    })
}

/// Bottom edge the pill may rest on. A live Dock reading wins; without one,
/// an auto-hidden bottom Dock (`hidden_dock`) reserves its static height.
pub fn pill_work_bottom(
    display_bottom: f64,
    work_bottom: f64,
    hidden_dock: Option<HiddenDock>,
    live: DockLimit,
) -> f64 {
    use DockLimit;
    match live {
        DockLimit::Clear => work_bottom,
        DockLimit::Above(top) => work_bottom.min(top),
        DockLimit::Unknown => hidden_dock.map_or(work_bottom, |dock| {
            work_bottom.min(display_bottom - hidden_dock_reserve(dock))
        }),
    }
}

/// Vertical calibration only moves the notch island; the pill has no travel.
pub fn vertical_offset_bounds(floating: bool) -> (f64, f64) {
    if floating {
        (0.0, 0.0)
    } else {
        (MIN_VERTICAL_OFFSET, MAX_VERTICAL_OFFSET)
    }
}

pub fn applied_overlay_position(requested_offset: f64, floating: bool) -> AppliedOverlayPosition {
    let requested_offset = clamp_vertical_offset(requested_offset);
    let (min_offset, max_offset) = vertical_offset_bounds(floating);
    AppliedOverlayPosition {
        requested_offset,
        applied_offset: requested_offset.clamp(min_offset, max_offset),
        min_offset,
        max_offset,
    }
}

/// The overlay frame with any hover dropdown removed. The pill grows upward
/// from a fixed bottom edge, the notch island downward from a fixed top.
pub fn collapsed_frame(frame: OverlayFrame, collapsed_h: f64, above: bool) -> OverlayFrame {
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

pub fn dictation_preview_fit_for_frame(
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

pub fn preview_fallback_inset(notch_h: Option<f64>) -> f64 {
    notch_h
        .filter(|h| h.is_finite() && *h > 0.0)
        .unwrap_or(FALLBACK_NOTCH_H)
        + POPOVER_GAP
}

/// True when the authoritative saved frame already matches `target` to within
/// a sub-point rounding error.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn frame_unchanged(saved: Option<OverlayFrame>, target: OverlayFrame) -> bool {
    const EPSILON: f64 = 0.01;
    saved.is_some_and(|f| {
        (f.x - target.x).abs() < EPSILON
            && (f.y - target.y).abs() < EPSILON
            && (f.width - target.width).abs() < EPSILON
            && (f.height - target.height).abs() < EPSILON
    })
}

impl NotchInfo {
    pub fn height(self) -> f64 {
        self.height
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OverlayContent {
    #[default]
    Controls,
    MeetingSuggestion,
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
            include_str!("../../../../tests/fixtures/overlay-geometry.json"),
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
        use super::DockLimit::Unknown;
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
        use super::DockLimit::{Above, Clear};
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
            "../../../../tests/fixtures/overlay-geometry.json"
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
