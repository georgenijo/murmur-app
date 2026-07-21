//! Transform review popover: geometry contract + non-activating window commands.
//!
//! Mirrors the overlay's architecture (`commands/overlay.rs`): Rust is the sole
//! author of every pixel — `popover_geometry_for()` is a pure function asserted
//! by a checked-in fixture from both a cargo test and a vitest test (see
//! `../../src/components/transform-review/transform-popover-geometry.fixture.json`).
//! No TS file may hold a geometry literal for this window.
use crate::{MutexExt, State};
use tauri::Manager;

/// A screen-space rectangle in logical points, top-left origin (y increases
/// downward) — the same convention Tauri's `LogicalPosition`/`LogicalSize`
/// use elsewhere in this codebase (see `overlay::position_overlay_default`,
/// which anchors the overlay at y=0 at the top of the screen).
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// A fully resolved window box for one popover size class.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct PopoverBox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// True when the box was placed *above* the selection because anchoring
    /// below would have clipped the bottom of the visible frame.
    pub flipped: bool,
}

/// The full popover geometry contract: a resolved box for each size class the
/// review UI can be in. `listening`/`thinking` render at `compact`; `ready`/
/// `failed` render at `expanded`. Both are computed independently against the
/// same anchor so each can flip/clamp on its own terms — the compact chip may
/// fit below the selection while the taller expanded diff would clip and need
/// to flip above (or vice versa).
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct TransformPopoverGeometry {
    pub compact: PopoverBox,
    pub expanded: PopoverBox,
}

// Private geometry constants — the ONLY place these magic numbers live, same
// discipline as `overlay::geometry_for`'s WING/DROPDOWN_H constants.
const COMPACT_W: f64 = 320.0;
const COMPACT_H: f64 = 76.0;
const EXPANDED_W: f64 = 420.0;
const EXPANDED_H: f64 = 220.0;
/// Gap between the selection bounds and the popover's nearest edge.
const ANCHOR_GAP: f64 = 8.0;
/// Vertical position of the popover's center as a fraction of the visible
/// frame's height, used only when there is no selection to anchor to.
const CENTERED_HEIGHT_FRACTION: f64 = 0.38;

/// Resolve one size class's box against `selection_bounds` (anchored 8px below,
/// flipping above when the bottom would clip, clamped horizontally to
/// `screen_frame`) or, with no selection, centered horizontally at 38% of the
/// screen's height. `screen_frame` is the *visible* frame — it already
/// excludes the menu bar / notch band, so clamping `y` to `screen_frame.y`
/// keeps the popover from ever overlapping that area.
fn box_for(width: f64, height: f64, selection_bounds: Option<Rect>, screen_frame: Rect) -> PopoverBox {
    let frame_left = screen_frame.x;
    let frame_right = screen_frame.x + screen_frame.width;
    let frame_top = screen_frame.y;
    let frame_bottom = screen_frame.y + screen_frame.height;

    match selection_bounds {
        Some(sel) => {
            let mut flipped = false;
            let mut y = sel.y + sel.height + ANCHOR_GAP;
            if y + height > frame_bottom {
                let flipped_y = sel.y - height - ANCHOR_GAP;
                if flipped_y >= frame_top {
                    y = flipped_y;
                    flipped = true;
                } else {
                    // Neither placement fits fully — clamp to the visible
                    // frame rather than let either edge win outright. The
                    // frame_top clamp is what guarantees we never overlap the
                    // menu bar / notch band even in this degenerate case.
                    y = (frame_bottom - height).max(frame_top);
                }
            }

            let mut x = sel.x + sel.width / 2.0 - width / 2.0;
            let max_x = (frame_right - width).max(frame_left);
            x = x.clamp(frame_left, max_x);

            PopoverBox { x, y, width, height, flipped }
        }
        None => {
            let x = frame_left + (screen_frame.width - width) / 2.0;
            let center_y = frame_top + screen_frame.height * CENTERED_HEIGHT_FRACTION;
            let y = center_y - height / 2.0;
            PopoverBox { x, y, width, height, flipped: false }
        }
    }
}

/// Pure geometry contract: given optional selection bounds and the active
/// screen's visible frame, resolve both size classes. No I/O, no NSWindow
/// calls — safe to call from any thread and from tests.
pub fn popover_geometry_for(
    selection_bounds: Option<Rect>,
    screen_frame: Rect,
) -> TransformPopoverGeometry {
    TransformPopoverGeometry {
        compact: box_for(COMPACT_W, COMPACT_H, selection_bounds, screen_frame),
        expanded: box_for(EXPANDED_W, EXPANDED_H, selection_bounds, screen_frame),
    }
}

/// Fallback visible frame used when no window/monitor can be queried (e.g.
/// off-macOS, or before any window has a monitor). Matches the overlay's
/// `FALLBACK_NOTCH_*` pattern of always resolving to *something* rather than
/// leaving the caller with no geometry.
const FALLBACK_SCREEN_W: f64 = 1440.0;
const FALLBACK_SCREEN_H: f64 = 900.0;
const FALLBACK_MENU_BAR_H: f64 = 25.0;

/// The active screen's visible frame (excluding the cached menu bar / notch
/// height), in the same top-left-origin logical coordinates as `Rect`.
///
/// Reuses `State.notch_info` (cached on the main thread at setup) for the
/// menu bar height instead of calling NSScreen directly here, since Tauri
/// commands are not guaranteed to run on the main thread. Screen width/height
/// come from the transform-review (or overlay) window's current monitor,
/// following the existing precedent in `overlay::show_overlay` of querying
/// `current_monitor()` from within a command handler.
fn active_screen_visible_frame(app: &tauri::AppHandle, state: &State) -> Rect {
    let menu_bar_h = state
        .notch_info
        .lock_or_recover()
        .map(|(_, h)| h)
        .unwrap_or(FALLBACK_MENU_BAR_H);

    let monitor = app
        .get_webview_window("transform-review")
        .or_else(|| app.get_webview_window("overlay"))
        .and_then(|w| w.current_monitor().ok().flatten());

    match monitor {
        Some(monitor) => {
            let size = monitor.size();
            let sf = monitor.scale_factor();
            let width = size.width as f64 / sf;
            let height = (size.height as f64 / sf - menu_bar_h).max(0.0);
            Rect { x: 0.0, y: menu_bar_h, width, height }
        }
        None => Rect {
            x: 0.0,
            y: menu_bar_h,
            width: FALLBACK_SCREEN_W,
            height: (FALLBACK_SCREEN_H - menu_bar_h).max(0.0),
        },
    }
}

/// Apply the popover's non-activating window treatment: same level + private
/// `_setPreventsActivation:` API as the overlay, shared via `native_window`.
/// `prevents_activation = true` (listening/thinking) means clicking the
/// popover never steals key focus from the source app; `false` (ready/failed)
/// lets it become key so Enter/Esc/Cmd+R keyboard shortcuts reach the webview.
#[cfg(target_os = "macos")]
fn apply_popover_window_treatment(window: &tauri::WebviewWindow, prevents_activation: bool) {
    super::native_window::set_window_level_and_activation(
        window,
        super::native_window::ABOVE_MENU_BAR_LEVEL,
        prevents_activation,
    );
}

#[cfg(not(target_os = "macos"))]
fn apply_popover_window_treatment(_window: &tauri::WebviewWindow, _prevents_activation: bool) {}

/// Return the popover geometry for the given optional selection anchor,
/// resolved against the active screen's visible frame.
#[tauri::command]
pub fn get_transform_popover_geometry(
    app: tauri::AppHandle,
    state: tauri::State<'_, State>,
    anchor: Option<Rect>,
) -> TransformPopoverGeometry {
    popover_geometry_for(anchor, active_screen_visible_frame(&app, &state))
}

/// Show the transform review popover, sized/positioned at the `compact` box
/// (the popover always opens into the listening state) and non-activating so
/// it never steals focus from the source app. The anchor is cached in `State`
/// so `set_transform_popover_expanded` can later resize/reposition for the
/// `expanded` box without the caller having to re-supply it.
#[tauri::command]
pub fn show_transform_popover(
    app: tauri::AppHandle,
    state: tauri::State<'_, State>,
    anchor: Option<Rect>,
) -> Result<(), String> {
    *state.transform_popover_anchor.lock_or_recover() = anchor;
    match app.get_webview_window("transform-review") {
        Some(window) => {
            let screen_frame = active_screen_visible_frame(&app, &state);
            let geometry = popover_geometry_for(anchor, screen_frame);
            let target = geometry.compact;
            window
                .set_size(tauri::LogicalSize::new(target.width, target.height))
                .map_err(|e| e.to_string())?;
            window
                .set_position(tauri::LogicalPosition::new(target.x, target.y))
                .map_err(|e| e.to_string())?;
            apply_popover_window_treatment(&window, true);
            window.show().map_err(|e| e.to_string())?;
            let _ = window.set_ignore_cursor_events(false);
            Ok(())
        }
        None => {
            tracing::warn!(target: "system", "show_transform_popover: transform-review window not found — skipping");
            Ok(())
        }
    }
}

/// Hide the transform review popover.
#[tauri::command]
pub fn hide_transform_popover(app: tauri::AppHandle) -> Result<(), String> {
    match app.get_webview_window("transform-review") {
        Some(window) => window.hide().map_err(|e| e.to_string()),
        None => {
            tracing::warn!(target: "system", "hide_transform_popover: transform-review window not found — skipping");
            Ok(())
        }
    }
}

/// Resize/reposition the popover for the given size class (`true` = the
/// `expanded` box used by ready/failed, `false` = `compact` used by
/// listening/thinking), against the anchor cached by the last
/// `show_transform_popover` call. Not part of the PR-C1 issue's literal
/// command list — added so Rust stays the sole author of every popover pixel
/// across state transitions too, not just on initial show. Mirrors
/// `overlay::set_overlay_expanded`'s shape; PR-C2's real state machine is
/// expected to call this alongside emitting `transform-state-changed`.
#[tauri::command]
pub fn set_transform_popover_expanded(
    app: tauri::AppHandle,
    state: tauri::State<'_, State>,
    expanded: bool,
) -> Result<(), String> {
    let anchor = *state.transform_popover_anchor.lock_or_recover();
    match app.get_webview_window("transform-review") {
        Some(window) => {
            let screen_frame = active_screen_visible_frame(&app, &state);
            let geometry = popover_geometry_for(anchor, screen_frame);
            let target = if expanded { geometry.expanded } else { geometry.compact };
            window
                .set_size(tauri::LogicalSize::new(target.width, target.height))
                .map_err(|e| e.to_string())?;
            window
                .set_position(tauri::LogicalPosition::new(target.x, target.y))
                .map_err(|e| e.to_string())?;
            Ok(())
        }
        None => {
            tracing::warn!(target: "system", "set_transform_popover_expanded: transform-review window not found — skipping");
            Ok(())
        }
    }
}

/// Toggle whether the popover can take key focus. `false` during
/// listening/thinking (never steal focus from the source app mid-instruction);
/// `true` at ready/failed, when Enter/Esc/Cmd+R need to reach the webview.
#[tauri::command]
pub fn set_transform_popover_focusable(app: tauri::AppHandle, focusable: bool) -> Result<(), String> {
    match app.get_webview_window("transform-review") {
        Some(window) => {
            apply_popover_window_treatment(&window, !focusable);
            if focusable {
                let _ = window.set_focus();
            }
            Ok(())
        }
        None => {
            tracing::warn!(target: "system", "set_transform_popover_focusable: transform-review window not found — skipping");
            Ok(())
        }
    }
}

/// Text content for the review popover: instruction, original selection, and
/// proposed transform. Stubbed for PR-C1 — PR-C2 wires this to the real
/// transform pipeline. Content is fetched by the window via this command
/// rather than broadcast in `transform-state-changed`, so instruction/original
/// text (which may be sensitive) is never sent as an event payload.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TransformReviewContent {
    pub instruction: String,
    pub original: String,
    pub proposed: String,
}

#[tauri::command]
pub fn get_transform_review_content() -> TransformReviewContent {
    TransformReviewContent::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> Rect {
        Rect { x: 0.0, y: 25.0, width: 1440.0, height: 875.0 }
    }

    #[test]
    fn compact_never_larger_than_expanded() {
        for anchor in [
            None,
            Some(Rect { x: 560.0, y: 300.0, width: 120.0, height: 20.0 }),
        ] {
            let g = popover_geometry_for(anchor, frame());
            assert!(g.expanded.width >= g.compact.width);
            assert!(g.expanded.height >= g.compact.height);
        }
    }

    #[test]
    fn never_overlaps_menu_bar_or_notch() {
        // Selection right at the top edge of the visible frame: anchoring
        // above would go past frame_top, so the degenerate clamp branch must
        // still respect it.
        let sel = Some(Rect { x: 700.0, y: 26.0, width: 100.0, height: 10.0 });
        let g = popover_geometry_for(sel, frame());
        assert!(g.compact.y >= frame().y);
        assert!(g.expanded.y >= frame().y);
    }

    #[test]
    fn anchored_below_selection_with_room() {
        let sel = Rect { x: 560.0, y: 300.0, width: 120.0, height: 20.0 };
        let g = popover_geometry_for(Some(sel), frame());
        assert!(!g.compact.flipped);
        assert!(!g.expanded.flipped);
        assert_eq!(g.compact.y, sel.y + sel.height + ANCHOR_GAP);
        assert_eq!(g.expanded.y, sel.y + sel.height + ANCHOR_GAP);
    }

    #[test]
    fn flips_above_when_bottom_would_clip() {
        let sel = Rect { x: 560.0, y: 850.0, width: 120.0, height: 20.0 };
        let g = popover_geometry_for(Some(sel), frame());
        assert!(g.compact.flipped);
        assert!(g.expanded.flipped);
        assert_eq!(g.compact.y, sel.y - COMPACT_H - ANCHOR_GAP);
        assert_eq!(g.expanded.y, sel.y - EXPANDED_H - ANCHOR_GAP);
    }

    #[test]
    fn clamps_horizontally_at_left_and_right_edges() {
        let left_sel = Rect { x: 20.0, y: 300.0, width: 40.0, height: 20.0 };
        let g_left = popover_geometry_for(Some(left_sel), frame());
        assert_eq!(g_left.compact.x, 0.0);
        assert_eq!(g_left.expanded.x, 0.0);

        let right_sel = Rect { x: 1400.0, y: 300.0, width: 30.0, height: 20.0 };
        let g_right = popover_geometry_for(Some(right_sel), frame());
        assert_eq!(g_right.compact.x, frame().width - COMPACT_W);
        assert_eq!(g_right.expanded.x, frame().width - EXPANDED_W);
    }

    #[test]
    fn centers_horizontally_at_fixed_height_fraction_with_no_selection() {
        let g = popover_geometry_for(None, frame());
        assert!(!g.compact.flipped);
        assert!(!g.expanded.flipped);
        assert_eq!(g.compact.x, (frame().width - COMPACT_W) / 2.0);
        assert_eq!(g.expanded.x, (frame().width - EXPANDED_W) / 2.0);
        let center_y = frame().y + frame().height * CENTERED_HEIGHT_FRACTION;
        assert_eq!(g.compact.y, center_y - COMPACT_H / 2.0);
        assert_eq!(g.expanded.y, center_y - EXPANDED_H / 2.0);
    }

    #[test]
    fn matches_fixture() {
        #[derive(serde::Deserialize)]
        struct Case {
            #[serde(rename = "selectionBounds")]
            selection_bounds: Option<Rect>,
            #[serde(rename = "screenFrame")]
            screen_frame: Rect,
            output: TransformPopoverGeometry,
        }
        #[derive(serde::Deserialize)]
        struct Fixture {
            anchored: Case,
            #[serde(rename = "flippedAbove")]
            flipped_above: Case,
            #[serde(rename = "clampedLeft")]
            clamped_left: Case,
            #[serde(rename = "clampedRight")]
            clamped_right: Case,
            #[serde(rename = "centeredFallback")]
            centered_fallback: Case,
        }
        let f: Fixture = serde_json::from_str(include_str!(
            "../../../src/components/transform-review/transform-popover-geometry.fixture.json"
        ))
        .unwrap();

        for case in [
            f.anchored,
            f.flipped_above,
            f.clamped_left,
            f.clamped_right,
            f.centered_fallback,
        ] {
            assert_eq!(
                popover_geometry_for(case.selection_bounds, case.screen_frame),
                case.output
            );
        }
    }

    #[test]
    fn rejects_unilateral_shape_drift() {
        let mut value = serde_json::to_value(popover_geometry_for(None, frame())).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("extraField".into(), serde_json::json!(1));
        assert!(serde_json::from_value::<TransformPopoverGeometry>(value).is_err());
    }
}
