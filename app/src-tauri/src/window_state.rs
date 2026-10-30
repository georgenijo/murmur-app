//! Persistent workspace windows. Popovers retain their own geometry/visibility.
use crate::MutexExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{mpsc, Arc, Mutex},
    time::Duration,
};
use tauri::{Manager, PhysicalPosition, PhysicalSize};

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
                .map(|monitor| Rect {
                    x: monitor.position().x as f64,
                    y: monitor.position().y as f64,
                    width: monitor.size().width as f64,
                    height: monitor.size().height as f64,
                })
                .collect();
            let restored = saved.windows.get(label).filter(|saved| {
                // The restored window may belong to a different-DPI monitor
                // than the one on which Tauri initially created it.
                let scale = monitors
                    .iter()
                    .zip(&displays)
                    .find(|(_, display)| {
                        window_fits_within_display(saved.bounds, std::slice::from_ref(*display))
                    })
                    .map(|(monitor, _)| monitor.scale_factor())
                    .unwrap_or(1.0);
                saved.inner_width as f64 >= 840.0 * scale
                    && saved.inner_height as f64 >= 620.0 * scale
                    && saved.inner_width as f64 <= saved.bounds.width
                    && saved.inner_height as f64 <= saved.bounds.height
                    && window_fits_within_display(saved.bounds, &displays)
            });
            if let Some(saved) = restored {
                window.set_size(PhysicalSize::new(saved.inner_width, saved.inner_height))?;
                window.set_position(PhysicalPosition::new(
                    saved.bounds.x.round() as i32,
                    saved.bounds.y.round() as i32,
                ))?;
                if saved.maximized {
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
    } else if let (Ok(position), Ok(size), Ok(inner)) = (
        window.outer_position(),
        window.outer_size(),
        window.inner_size(),
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
        assert_eq!(restored.backgrounds["main"], "#123456");
        assert!(!path.with_extension("json.tmp").exists());
        std::fs::remove_file(path).unwrap();
    }
}
