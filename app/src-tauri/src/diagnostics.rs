use tauri::{AppHandle, Manager, WebviewWindow, Window, WindowEvent};

const WINDOW_LABELS: &[&str] = &["main", "log-viewer", "overlay"];

fn result_bool(value: tauri::Result<bool>) -> (bool, bool) {
    match value {
        Ok(value) => (value, true),
        Err(_) => (false, false),
    }
}

fn result_i32_pair<T, F>(value: tauri::Result<T>, map: F) -> (i32, i32, bool)
where
    F: FnOnce(T) -> (i32, i32),
{
    match value {
        Ok(value) => {
            let (a, b) = map(value);
            (a, b, true)
        }
        Err(_) => (0, 0, false),
    }
}

fn result_scale_factor(value: tauri::Result<f64>) -> (f64, bool) {
    match value {
        Ok(value) => (value, true),
        Err(_) => (0.0, false),
    }
}

trait DiagnosticWindow {
    fn label(&self) -> &str;
    fn is_visible(&self) -> tauri::Result<bool>;
    fn is_focused(&self) -> tauri::Result<bool>;
    fn is_minimized(&self) -> tauri::Result<bool>;
    fn outer_position(&self) -> tauri::Result<tauri::PhysicalPosition<i32>>;
    fn inner_size(&self) -> tauri::Result<tauri::PhysicalSize<u32>>;
    fn scale_factor(&self) -> tauri::Result<f64>;
}

impl DiagnosticWindow for WebviewWindow {
    fn label(&self) -> &str {
        self.label()
    }

    fn is_visible(&self) -> tauri::Result<bool> {
        self.is_visible()
    }

    fn is_focused(&self) -> tauri::Result<bool> {
        self.is_focused()
    }

    fn is_minimized(&self) -> tauri::Result<bool> {
        self.is_minimized()
    }

    fn outer_position(&self) -> tauri::Result<tauri::PhysicalPosition<i32>> {
        self.outer_position()
    }

    fn inner_size(&self) -> tauri::Result<tauri::PhysicalSize<u32>> {
        self.inner_size()
    }

    fn scale_factor(&self) -> tauri::Result<f64> {
        self.scale_factor()
    }
}

impl DiagnosticWindow for Window {
    fn label(&self) -> &str {
        self.label()
    }

    fn is_visible(&self) -> tauri::Result<bool> {
        self.is_visible()
    }

    fn is_focused(&self) -> tauri::Result<bool> {
        self.is_focused()
    }

    fn is_minimized(&self) -> tauri::Result<bool> {
        self.is_minimized()
    }

    fn outer_position(&self) -> tauri::Result<tauri::PhysicalPosition<i32>> {
        self.outer_position()
    }

    fn inner_size(&self) -> tauri::Result<tauri::PhysicalSize<u32>> {
        self.inner_size()
    }

    fn scale_factor(&self) -> tauri::Result<f64> {
        self.scale_factor()
    }
}

fn log_any_window_state(window: &impl DiagnosticWindow, reason: &str) {
    let label = window.label();
    let (visible, visible_known) = result_bool(window.is_visible());
    let (focused, focused_known) = result_bool(window.is_focused());
    let (minimized, minimized_known) = result_bool(window.is_minimized());
    let (x, y, position_known) = result_i32_pair(window.outer_position(), |p| (p.x, p.y));
    let (width, height, size_known) =
        result_i32_pair(window.inner_size(), |s| (s.width as i32, s.height as i32));
    let (scale_factor, scale_factor_known) = result_scale_factor(window.scale_factor());

    tracing::info!(
        target: "system",
        reason = reason,
        label = label,
        exists = true,
        visible = visible,
        visible_known = visible_known,
        focused = focused,
        focused_known = focused_known,
        minimized = minimized,
        minimized_known = minimized_known,
        x = x,
        y = y,
        position_known = position_known,
        width = width,
        height = height,
        size_known = size_known,
        scale_factor = scale_factor,
        scale_factor_known = scale_factor_known,
        "window state snapshot"
    );
}

pub fn log_webview_window_state(window: &WebviewWindow, reason: &str) {
    log_any_window_state(window, reason);
}

pub fn log_native_window_state(window: &Window, reason: &str) {
    log_any_window_state(window, reason);
}

pub fn log_window_state_snapshot(app: &AppHandle, reason: &str) {
    for label in WINDOW_LABELS {
        if let Some(window) = app.get_webview_window(label) {
            log_webview_window_state(&window, reason);
        } else {
            tracing::info!(
                target: "system",
                reason = reason,
                label = *label,
                exists = false,
                "window state snapshot"
            );
        }
    }
}

pub fn log_window_event(window: &Window, event: &WindowEvent) {
    match event {
        WindowEvent::Focused(focused) => {
            tracing::info!(
                target: "system",
                label = window.label(),
                focused = *focused,
                "native window focus changed"
            );
            log_native_window_state(window, "native_window_focus_changed");
            let _ = crate::audio::log_audio_route_snapshot("native_window_focus_changed");
        }
        WindowEvent::CloseRequested { .. } => {
            tracing::info!(
                target: "system",
                label = window.label(),
                "native window close requested"
            );
            log_native_window_state(window, "native_window_close_requested");
        }
        WindowEvent::Destroyed => {
            tracing::info!(target: "system", label = window.label(), "native window destroyed");
        }
        WindowEvent::Resized(size) => {
            tracing::info!(
                target: "system",
                label = window.label(),
                width = size.width,
                height = size.height,
                "native window resized"
            );
        }
        WindowEvent::Moved(position) => {
            tracing::info!(
                target: "system",
                label = window.label(),
                x = position.x,
                y = position.y,
                "native window moved"
            );
        }
        WindowEvent::ScaleFactorChanged {
            scale_factor,
            new_inner_size,
            ..
        } => {
            tracing::info!(
                target: "system",
                label = window.label(),
                scale_factor = *scale_factor,
                width = new_inner_size.width,
                height = new_inner_size.height,
                "native window scale factor changed"
            );
        }
        WindowEvent::ThemeChanged(theme) => {
            tracing::info!(
                target: "system",
                label = window.label(),
                theme = ?theme,
                "native window theme changed"
            );
        }
        WindowEvent::DragDrop(event) => {
            tracing::info!(
                target: "system",
                label = window.label(),
                event = ?event,
                "native window drag-drop event"
            );
        }
        #[allow(unreachable_patterns)]
        _ => {
            tracing::info!(
                target: "system",
                label = window.label(),
                event = ?event,
                "native window event"
            );
        }
    }
}
