//! Persistent workspace windows. Popovers retain their own geometry/visibility.
use crate::MutexExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{mpsc, Arc, Mutex},
    time::Duration,
};
use tauri::{LogicalPosition, LogicalSize, Manager};

const LABELS: [&str; 2] = ["main", "diagnostics"];
const DEBOUNCE: Duration = Duration::from_millis(500);
pub(crate) const TRAFFIC_LIGHT_X: f64 = 16.0;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub(crate) struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    fn rounded(self) -> Self {
        Self {
            x: self.x.round(),
            y: self.y.round(),
            width: self.width.round(),
            height: self.height.round(),
        }
    }
    fn to_logical(self, scale: f64) -> Self {
        Self {
            x: self.x / scale,
            y: self.y / scale,
            width: self.width / scale,
            height: self.height / scale,
        }
    }
    fn at_pixel_boundary(self, scale: f64) -> Self {
        Self {
            x: (self.x * scale).round() / scale,
            y: (self.y * scale).round() / scale,
            width: (self.width * scale).round() / scale,
            height: (self.height * scale).round() / scale,
        }
    }
    fn valid(self) -> bool {
        [self.x, self.y, self.width, self.height]
            .iter()
            .all(|v| v.is_finite())
            && self.width > 0.0
            && self.height > 0.0
    }
}

// Port of T3 Code's windowFitsWithinDisplay (MIT; see THIRD_PARTY_NOTICES).
pub(crate) fn window_fits_within_display(rect: Rect, displays: &[Rect]) -> bool {
    rect.valid()
        && displays.iter().any(|display| {
            display.valid()
                && rect.x >= display.x
                && rect.y >= display.y
                && rect.x + rect.width <= display.x + display.width
                && rect.y + rect.height <= display.y + display.height
        })
}

pub(crate) fn traffic_light_y(header_height: f64, zoom: f64) -> f64 {
    (header_height * zoom / 2.0 - 7.0).round().max(0.0)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SavedWindow {
    bounds: Rect,
    inner_width: u32,
    inner_height: u32,
    maximized: bool,
    // Physical coordinates are meaningful only with the scale that produced
    // them. Older state has no scale; interpret it with each candidate display.
    #[serde(default)]
    scale_factor: Option<f64>,
}

struct Display {
    // Tao's macOS monitor origins AND sizes are multiplied by that monitor's
    // scale. Normalize each independently before comparing desktop rectangles.
    bounds: Rect,
    scale_factor: f64,
}
impl Display {
    fn from_physical(bounds: Rect, scale_factor: f64) -> Self {
        Self {
            bounds: bounds.to_logical(scale_factor),
            scale_factor,
        }
    }
}

#[derive(Debug, PartialEq)]
struct RestoredWindow {
    bounds: Rect,
    inner_size: LogicalSize<f64>,
    maximized: bool,
}

fn valid_scale(scale: f64) -> bool {
    scale.is_finite() && scale > 0.0
}

fn restore_geometry(saved: &SavedWindow, displays: &[Display]) -> Option<RestoredWindow> {
    displays.iter().find_map(|display| {
        let target_scale = display.scale_factor;
        let saved_scale = saved.scale_factor.unwrap_or(target_scale);
        if !valid_scale(saved_scale) || !valid_scale(target_scale) {
            return None;
        }
        // Keep the saved logical geometry across a backing-scale change, then
        // round to whole pixels on the TARGET display, never the launch display.
        let bounds = saved
            .bounds
            .to_logical(saved_scale)
            .at_pixel_boundary(target_scale);
        let inner_size = LogicalSize::new(
            (saved.inner_width as f64 / saved_scale * target_scale).round() / target_scale,
            (saved.inner_height as f64 / saved_scale * target_scale).round() / target_scale,
        );
        (inner_size.width >= 840.0
            && inner_size.height >= 620.0
            && inner_size.width <= bounds.width
            && inner_size.height <= bounds.height
            && window_fits_within_display(bounds, &[display.bounds]))
        .then_some(RestoredWindow {
            bounds,
            inner_size,
            maximized: saved.maximized,
        })
    })
}

#[derive(Default, Serialize, Deserialize)]
struct SavedState {
    windows: BTreeMap<String, SavedWindow>,
    #[serde(default)]
    backgrounds: BTreeMap<String, String>,
}

struct Store {
    path: PathBuf,
    saved: SavedState,
}
impl Store {
    fn write(&self) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temp = self.path.with_extension("json.tmp");
        std::fs::write(&temp, serde_json::to_vec_pretty(&self.saved)?)?;
        std::fs::rename(temp, &self.path)
    }
}

pub(crate) struct WindowState {
    store: Arc<Mutex<Store>>,
    changed: mpsc::Sender<()>,
    chrome: Mutex<BTreeMap<String, (f64, f64)>>,
    painted: std::sync::atomic::AtomicBool,
}

pub(crate) fn initialize(app: &tauri::AppHandle) -> tauri::Result<()> {
    let path = app.path().app_data_dir()?.join("window-state.json");
    let mut saved: SavedState = std::fs::read(&path)
        .ok()
        .filter(|bytes| bytes.len() < 16 * 1024)
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    for label in LABELS {
        if let Some(window) = app.get_webview_window(label) {
            if let Some(color) = saved
                .backgrounds
                .get(label)
                .and_then(|value| parse_background(value).ok())
            {
                // A renderer failure uses the last resolved background rather
                // than flashing the configuration fallback in a dark theme.
                window.set_background_color(Some(color))?;
            }
            let monitors = window.available_monitors()?;
            let displays: Vec<_> = monitors
                .iter()
                .map(|monitor| {
                    Display::from_physical(
                        Rect {
                            x: monitor.position().x as f64,
                            y: monitor.position().y as f64,
                            width: monitor.size().width as f64,
                            height: monitor.size().height as f64,
                        },
                        monitor.scale_factor(),
                    )
                })
                .collect();
            let restored = saved
                .windows
                .get(label)
                .and_then(|saved| restore_geometry(saved, &displays));
            if let Some(restored) = restored {
                // Tao converts Physical arguments using the window's CURRENT
                // scale, which can still be the launch display's scale. Logical
                // arguments preserve the target geometry through that move.
                window.set_size(restored.inner_size)?;
                window.set_position(LogicalPosition::new(restored.bounds.x, restored.bounds.y))?;
                if restored.maximized {
                    window.maximize()?;
                }
            } else {
                window.set_size(tauri::LogicalSize::new(1100.0, 780.0))?;
                window.center()?;
                let position = window.outer_position()?;
                let size = window.outer_size()?;
                let inner = window.inner_size()?;
                saved.windows.insert(
                    label.into(),
                    SavedWindow {
                        bounds: Rect {
                            x: position.x as f64,
                            y: position.y as f64,
                            width: size.width as f64,
                            height: size.height as f64,
                        }
                        .rounded(),
                        inner_width: inner.width,
                        inner_height: inner.height,
                        maximized: false,
                        scale_factor: Some(window.scale_factor()?),
                    },
                );
            }
        }
    }
    let store = Arc::new(Mutex::new(Store { path, saved }));
    let (changed, receiver) = mpsc::channel();
    let worker_store = store.clone();
    std::thread::Builder::new()
        .name("window-state".into())
        .spawn(move || {
            while receiver.recv().is_ok() {
                while receiver.recv_timeout(DEBOUNCE).is_ok() {}
                if let Err(error) = worker_store.lock_or_recover().write() {
                    tracing::warn!(target: "system", "window state save failed: {error}");
                }
            }
        })?;
    app.manage(WindowState {
        store,
        changed,
        chrome: Mutex::new(BTreeMap::new()),
        painted: false.into(),
    });
    // Independent of JS/hydration/rAF: a broken renderer never strands the app.
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(5)).await;
        if let Some(window) = handle.get_webview_window("main") {
            if let Err(error) = reveal_once(&window, &handle.state::<WindowState>()) {
                tracing::warn!(target: "system", "window safety reveal failed: {error}");
            }
        }
    });
    Ok(())
}

pub(crate) fn on_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    if !LABELS.contains(&window.label()) {
        return;
    }
    if !matches!(
        event,
        tauri::WindowEvent::Moved(_)
            | tauri::WindowEvent::Resized(_)
            | tauri::WindowEvent::CloseRequested { .. }
            | tauri::WindowEvent::ScaleFactorChanged { .. }
    ) {
        return;
    }
    let Some(state) = window.try_state::<WindowState>() else {
        return;
    };
    if let Some((header, zoom)) = state.chrome.lock_or_recover().get(window.label()).copied() {
        if let Some(webview) = window.app_handle().get_webview_window(window.label()) {
            crate::commands::native_window::position_traffic_lights(
                &webview,
                traffic_light_y(header, zoom),
            );
        }
    }
    // Fullscreen is transient; retain the last ordinary bounds/maximized state.
    if window.is_fullscreen().unwrap_or(true) || window.is_minimized().unwrap_or(true) {
        return;
    }
    let Ok(maximized) = window.is_maximized() else {
        return;
    };
    let mut store = state.store.lock_or_recover();
    if maximized {
        if let Some(saved) = store.saved.windows.get_mut(window.label()) {
            saved.maximized = true;
        }
    } else if let (Ok(position), Ok(size), Ok(inner), Ok(scale)) = (
        window.outer_position(),
        window.outer_size(),
        window.inner_size(),
        window.scale_factor(),
    ) {
        store.saved.windows.insert(
            window.label().into(),
            SavedWindow {
                bounds: Rect {
                    x: position.x as f64,
                    y: position.y as f64,
                    width: size.width as f64,
                    height: size.height as f64,
                }
                .rounded(),
                inner_width: inner.width,
                inner_height: inner.height,
                maximized,
                scale_factor: Some(scale),
            },
        );
    }
    let _ = state.changed.send(());
}

pub(crate) fn flush(app: &tauri::AppHandle) {
    if let Some(state) = app.try_state::<WindowState>() {
        if let Err(error) = state.store.lock_or_recover().write() {
            tracing::warn!(target: "system", "window state flush failed: {error}");
        }
    }
}

#[tauri::command]
pub(crate) fn sync_window_chrome(
    window: tauri::WebviewWindow,
    background: String,
    header_height: f64,
    zoom: f64,
    ready: bool,
) -> Result<(), String> {
    if !LABELS.contains(&window.label()) {
        return Err("Window has no workspace chrome".into());
    }
    if !header_height.is_finite()
        || !(14.0..=200.0).contains(&header_height)
        || !zoom.is_finite()
        || !(0.5..=2.0).contains(&zoom)
    {
        return Err("Invalid window metrics".into());
    }
    let color = parse_background(&background)?;
    let state = window.state::<WindowState>();
    let old_zoom = state.chrome.lock_or_recover().get(window.label()).copied();
    if old_zoom.map(|(_, old)| old != zoom).unwrap_or(zoom != 1.0) {
        window.set_zoom(zoom).map_err(|error| error.to_string())?;
    }
    window
        .set_background_color(Some(color))
        .map_err(|error| error.to_string())?;
    state
        .chrome
        .lock_or_recover()
        .insert(window.label().into(), (header_height, zoom));
    {
        let mut store = state.store.lock_or_recover();
        if store.saved.backgrounds.get(window.label()) != Some(&background) {
            store
                .saved
                .backgrounds
                .insert(window.label().into(), background);
            let _ = state.changed.send(());
        }
    }
    crate::commands::native_window::position_traffic_lights(
        &window,
        traffic_light_y(header_height, zoom),
    );
    if ready && window.label() == "main" {
        reveal_once(&window, &state)?;
    }
    Ok(())
}

fn reveal_once(window: &tauri::WebviewWindow, state: &WindowState) -> Result<(), String> {
    use std::sync::atomic::Ordering;
    if state
        .painted
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        if let Err(error) = window.show() {
            // Keep the independent safety timeout available after a failed
            // renderer-triggered show, and allow a later retry to succeed.
            state.painted.store(false, Ordering::SeqCst);
            return Err(error.to_string());
        }
    }
    Ok(())
}

fn parse_background(value: &str) -> Result<tauri::window::Color, String> {
    // Tauri's hex parser indexes bytes; reject non-ASCII before calling it.
    if !value.is_ascii()
        || !value
            .trim_start_matches('#')
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("Invalid background color".into());
    }
    let color = value
        .parse::<tauri::window::Color>()
        .map_err(|_| "Invalid background color")?;
    if color.3 != 255 {
        return Err("Workspace background must be opaque".into());
    }
    Ok(color)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }
    #[test]
    fn traffic_lights_follow_css_height_and_page_zoom() {
        assert_eq!(TRAFFIC_LIGHT_X, 16.0);
        assert_eq!(traffic_light_y(52.0, 1.0), 19.0);
        assert_eq!(traffic_light_y(52.0, 1.2), 24.0);
        assert_eq!(traffic_light_y(42.0, 1.0), 14.0);
    }
    #[test]
    fn fit_requires_one_whole_connected_display() {
        let displays = [
            rect(0.0, 0.0, 1920.0, 1080.0),
            rect(-1600.0, 0.0, 1600.0, 900.0),
        ];
        assert!(window_fits_within_display(displays[0], &displays));
        assert!(window_fits_within_display(
            rect(-1500.0, 30.0, 1100.0, 780.0),
            &displays
        ));
        assert!(!window_fits_within_display(
            rect(-1500.0, 30.0, 1100.0, 780.0),
            &displays[..1]
        ));
        assert!(!window_fits_within_display(
            rect(1800.0, 0.0, 1100.0, 780.0),
            &displays
        ));
        assert!(!window_fits_within_display(
            rect(-500.0, 0.0, 1100.0, 780.0),
            &displays
        ));
        assert!(!window_fits_within_display(
            rect(0.0, 301.0, 1100.0, 780.0),
            &displays
        ));
        assert!(!window_fits_within_display(
            rect(0.0, 0.0, 1100.0, 780.0),
            &[]
        ));
    }
    #[test]
    fn invalid_rects_refuse_restore() {
        let displays = [rect(0.0, 0.0, 1920.0, 1080.0)];
        for bad in [
            rect(f64::NAN, 0.0, 100.0, 100.0),
            rect(0.0, 0.0, 0.0, 100.0),
            rect(0.0, 0.0, 100.0, f64::INFINITY),
        ] {
            assert!(!window_fits_within_display(bad, &displays));
        }
    }
    #[test]
    fn bounds_are_rounded_to_whole_pixels() {
        assert_eq!(
            rect(-123.7, 44.2, 1100.5, 779.9).rounded(),
            rect(-124.0, 44.0, 1101.0, 780.0)
        );
    }

    fn saved_window(bounds: Rect, scale: f64, maximized: bool) -> SavedWindow {
        SavedWindow {
            bounds: Rect {
                x: bounds.x * scale,
                y: bounds.y * scale,
                width: bounds.width * scale,
                height: bounds.height * scale,
            }
            .rounded(),
            inner_width: (bounds.width * scale).round() as u32,
            inner_height: (bounds.height * scale).round() as u32,
            maximized,
            scale_factor: Some(scale),
        }
    }

    fn display(bounds: Rect, scale: f64) -> Display {
        Display::from_physical(
            Rect {
                x: bounds.x * scale,
                y: bounds.y * scale,
                width: bounds.width * scale,
                height: bounds.height * scale,
            },
            scale,
        )
    }

    #[test]
    fn restore_from_2x_launch_to_1x_target_keeps_size_position_and_maximized_state() {
        let bounds = rect(2000.0, 100.0, 1100.0, 780.0);
        let displays = [
            display(rect(0.0, 0.0, 1920.0, 1080.0), 2.0),
            display(rect(1920.0, 0.0, 1920.0, 1080.0), 1.0),
        ];
        for maximized in [false, true] {
            let restored = restore_geometry(&saved_window(bounds, 1.0, maximized), &displays)
                .expect("the ordinary bounds fit the 1x target display");
            assert_eq!(restored.bounds, bounds);
            assert_eq!(restored.maximized, maximized);
            // Simulate Tao's conversion while the window is still on the 2x
            // launch monitor. Physical inputs used to halve both values here.
            assert_eq!(
                tauri::Size::from(restored.inner_size).to_logical::<f64>(2.0),
                LogicalSize::new(1100.0, 780.0)
            );
            assert_eq!(
                tauri::Position::from(LogicalPosition::new(bounds.x, bounds.y))
                    .to_logical::<f64>(2.0),
                LogicalPosition::new(2000.0, 100.0)
            );
        }
    }

    #[test]
    fn restore_from_1x_launch_to_2x_target_normalizes_negative_monitor_origins() {
        let bounds = rect(-1800.0, -100.0, 1100.0, 780.0);
        let saved = saved_window(bounds, 2.0, false);
        let displays = [
            display(rect(0.0, 0.0, 1920.0, 1080.0), 1.0),
            display(rect(-1920.0, -200.0, 1920.0, 1080.0), 2.0),
        ];
        let restored = restore_geometry(&saved, &displays).unwrap();
        assert_eq!(restored.bounds, bounds);
        // On the 1x launch display, Physical inputs used to double the size and
        // send x=-3600 rather than x=-1800 to AppKit.
        assert_eq!(
            tauri::Size::from(restored.inner_size).to_logical::<f64>(1.0),
            LogicalSize::new(1100.0, 780.0)
        );
        assert_eq!(
            tauri::Position::from(LogicalPosition::new(restored.bounds.x, restored.bounds.y))
                .to_logical::<f64>(1.0),
            LogicalPosition::new(-1800.0, -100.0)
        );
    }

    #[test]
    fn restore_preserves_logical_geometry_when_the_target_display_changes_scale() {
        let bounds = rect(50.0, 60.0, 1100.0, 780.0);
        for (saved_scale, target_scale) in [(1.0, 2.0), (2.0, 1.0)] {
            let saved = saved_window(bounds, saved_scale, true);
            let restored = restore_geometry(
                &saved,
                &[display(rect(0.0, 0.0, 1920.0, 1080.0), target_scale)],
            )
            .unwrap();
            assert_eq!(restored.bounds, bounds);
            assert_eq!(restored.inner_size, LogicalSize::new(1100.0, 780.0));
            assert!(restored.maximized);
        }
    }

    #[test]
    fn disconnected_or_spanning_normal_bounds_require_default_centered_fallback() {
        let main = display(rect(0.0, 0.0, 1920.0, 1080.0), 1.0);
        let external = display(rect(-1920.0, -200.0, 1920.0, 1080.0), 2.0);
        let saved = saved_window(rect(-1800.0, -100.0, 1100.0, 780.0), 2.0, true);
        assert!(restore_geometry(&saved, &[main]).is_none());
        let spanning = saved_window(rect(-500.0, 50.0, 1100.0, 780.0), 2.0, false);
        assert!(restore_geometry(&spanning, &[external]).is_none());
        assert!(restore_geometry(&saved, &[]).is_none());
    }

    #[test]
    fn restored_geometry_rounds_at_target_pixel_boundaries_and_checks_inner_minimum() {
        let saved = saved_window(rect(-1700.5, -99.5, 1100.5, 780.5), 2.0, false);
        let displays = [display(rect(-1920.0, -200.0, 1920.0, 1080.0), 1.0)];
        let restored = restore_geometry(&saved, &displays).unwrap();
        assert_eq!(restored.bounds, rect(-1701.0, -100.0, 1101.0, 781.0));
        assert_eq!(restored.inner_size, LogicalSize::new(1101.0, 781.0));
        let undersized = saved_window(rect(-1700.0, 0.0, 839.0, 780.0), 2.0, false);
        assert!(restore_geometry(&undersized, &displays).is_none());
        let mut oversized_inner = saved;
        oversized_inner.inner_width = 4000;
        assert!(restore_geometry(&oversized_inner, &displays).is_none());
    }

    #[test]
    fn legacy_physical_state_uses_the_target_scale_and_invalid_scales_refuse_restore() {
        let mut saved = saved_window(rect(-1800.0, -100.0, 1100.0, 780.0), 2.0, false);
        let displays = [display(rect(-1920.0, -200.0, 1920.0, 1080.0), 2.0)];
        saved.scale_factor = None;
        let legacy_json = serde_json::to_value(&saved).unwrap();
        let mut legacy_json = legacy_json.as_object().unwrap().clone();
        legacy_json.remove("scale_factor");
        let legacy: SavedWindow = serde_json::from_value(legacy_json.into()).unwrap();
        assert_eq!(
            restore_geometry(&legacy, &displays).unwrap().bounds.x,
            -1800.0
        );
        for scale in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            saved.scale_factor = Some(scale);
            assert!(restore_geometry(&saved, &displays).is_none());
            assert!(restore_geometry(&legacy, &[display(displays[0].bounds, scale)]).is_none());
        }
    }

    #[test]
    fn native_background_refuses_transparency_and_invalid_input() {
        assert_eq!(
            parse_background("#123456").unwrap(),
            tauri::window::Color(18, 52, 86, 255)
        );
        for invalid in ["#12345600", "transparent", "rgb(0,0,0)", "é0", ""] {
            assert!(parse_background(invalid).is_err());
        }
    }

    #[test]
    fn persistence_roundtrip_keeps_normal_bounds_maximized_and_theme() {
        let path = std::env::temp_dir().join(format!(
            "murmur-window-state-test-{}.json",
            std::process::id()
        ));
        let mut saved = SavedState::default();
        saved.windows.insert(
            "main".into(),
            SavedWindow {
                bounds: rect(50.0, 60.0, 1100.0, 780.0),
                inner_width: 1100,
                inner_height: 780,
                maximized: true,
                scale_factor: Some(1.0),
            },
        );
        saved.backgrounds.insert("main".into(), "#123456".into());
        let store = Store {
            path: path.clone(),
            saved,
        };
        store.write().unwrap();
        let restored: SavedState = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(restored.windows["main"].maximized);
        assert_eq!(
            restored.windows["main"].bounds,
            rect(50.0, 60.0, 1100.0, 780.0)
        );
        assert_eq!(restored.windows["main"].inner_width, 1100);
        assert_eq!(restored.windows["main"].scale_factor, Some(1.0));
        assert_eq!(restored.backgrounds["main"], "#123456");
        assert!(!path.with_extension("json.tmp").exists());
        std::fs::remove_file(path).unwrap();
    }
}
