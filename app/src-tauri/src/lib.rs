#[cfg(target_os = "macos")]
mod alloc;
mod audio;
mod audio_decode;
mod commands;
mod diagnostics;
mod injector;
mod keyboard;
mod resource_monitor;
mod state;
pub mod telemetry;
pub mod transcriber;
mod vad;

#[cfg(target_os = "macos")]
#[global_allocator]
static ALLOCATOR: alloc::RustZoneAllocator = alloc::RustZoneAllocator;

/// Current Rust heap usage in megabytes (from macOS malloc zone stats).
#[cfg(target_os = "macos")]
pub fn rust_heap_mb() -> u64 {
    alloc::rust_heap_mb()
}

/// Current C/C++ FFI heap usage in megabytes (total zones minus Rust zone).
#[cfg(target_os = "macos")]
pub fn ffi_heap_mb() -> u64 {
    alloc::ffi_heap_mb()
}

#[cfg(not(target_os = "macos"))]
pub fn rust_heap_mb() -> u64 { 0 }

#[cfg(not(target_os = "macos"))]
pub fn ffi_heap_mb() -> u64 { 0 }

use state::AppState;
use std::sync::{Mutex, MutexGuard};
use tauri::Manager;
use tauri::RunEvent;
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};


/// Helper trait to recover from poisoned mutexes
pub(crate) trait MutexExt<T> {
    fn lock_or_recover(&self) -> MutexGuard<'_, T>;
}

impl<T> MutexExt<T> for Mutex<T> {
    fn lock_or_recover(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|poisoned| {
            tracing::warn!(target: "system", "Mutex was poisoned, recovering data");
            poisoned.into_inner()
        })
    }
}

pub(crate) struct State {
    pub(crate) app_state: AppState,
    /// Cached notch dimensions (notch_width, menu_bar_height) from setup (main thread).
    pub(crate) notch_info: Mutex<Option<(f64, f64)>>,
}

/// WebKitGTK environment defaults applied on Linux before GTK/webkit init.
///
/// On Linux/Wayland, webkit2gtk's DMABUF renderer leaves windows invisible
/// on many mesa/NVIDIA stacks (Fedora, Nobara, Ubuntu 23+). Disabling the
/// DMABUF renderer and compositing mode restores rendering. Users can
/// override either default by pre-setting the variable in their environment.
#[cfg(target_os = "linux")]
const LINUX_WEBKIT_ENV_DEFAULTS: &[(&str, &str)] = &[
    ("WEBKIT_DISABLE_DMABUF_RENDERER", "1"),
    ("WEBKIT_DISABLE_COMPOSITING_MODE", "1"),
];

/// Apply `LINUX_WEBKIT_ENV_DEFAULTS` via injected get/set closures.
///
/// Separated from `run()` so tests can exercise it against a fake env map
/// without touching process-global state.
#[cfg(target_os = "linux")]
fn apply_linux_webkit_env_defaults<F, G>(mut get: F, mut set: G)
where
    F: FnMut(&str) -> Option<std::ffi::OsString>,
    G: FnMut(&str, &str),
{
    for (key, default) in LINUX_WEBKIT_ENV_DEFAULTS {
        if get(key).is_none() {
            set(key, default);
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "linux")]
    apply_linux_webkit_env_defaults(
        |k| std::env::var_os(k),
        |k, v| std::env::set_var(k, v),
    );

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(State {
            app_state: AppState::default(),
            notch_info: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            commands::recording::init_dictation,
            commands::recording::process_audio,
            commands::recording::get_status,
            commands::recording::configure_dictation,
            commands::recording::start_native_recording,
            commands::recording::stop_native_recording,
            commands::recording::cancel_native_recording,
            commands::recording::count_vocab_tokens,
            commands::recording::transcribe_file,
            commands::permissions::open_system_preferences,
            commands::permissions::check_accessibility_permission,
            commands::permissions::request_accessibility_permission,
            commands::permissions::request_microphone_permission,
            commands::permissions::list_audio_devices,
            commands::permissions::get_default_audio_input_device,
            commands::permissions::log_audio_route_snapshot,
            commands::keyboard::start_keyboard_listener,
            commands::keyboard::stop_keyboard_listener,
            commands::keyboard::update_keyboard_key,
            commands::keyboard::set_keyboard_recording,
            commands::keyboard::set_app_disabled,
            commands::keyboard::get_app_disabled,
            commands::logging::get_log_contents,
            commands::logging::clear_logs,
            commands::logging::log_frontend,
            commands::logging::open_log_viewer,
            commands::logging::log_window_state_snapshot,
            commands::models::check_model_exists,
            commands::models::check_specific_model_exists,
            commands::models::download_model,
            commands::tray::update_tray_icon,
            commands::overlay::show_overlay,
            commands::overlay::hide_overlay,
            commands::overlay::get_notch_info,
            telemetry::get_event_history,
            telemetry::clear_event_history,
            resource_monitor::get_resource_usage
        ])
        .on_window_event(|window, event| {
            diagnostics::log_window_event(window, event);
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // Hide instead of destroy for persistent windows
                if window.label() == "main" || window.label() == "log-viewer" {
                    api.prevent_close();
                    let _ = window.hide();
                    tracing::info!(target: "system", "{} window hidden on close request", window.label());
                    diagnostics::log_native_window_state(window, "after_close_request_hide");
                }
            }
        })
        .setup(|app| {
            telemetry::init(app.handle().clone());

            tracing::info!(target: "system", "app setup — Murmur v{}", env!("CARGO_PKG_VERSION"));
            diagnostics::log_window_state_snapshot(app.handle(), "setup_start");
            let _ = audio::log_audio_route_snapshot("setup_start");

            // Emit startup baseline memory snapshot
            {
                let rss = resource_monitor::get_process_rss_mb();
                let heap = rust_heap_mb();
                let ffi = ffi_heap_mb();
                tracing::info!(target: "system", rss_mb = rss, rust_heap_mb = heap, ffi_heap_mb = ffi, "startup_baseline");
            }

            // Periodic heartbeat: memory telemetry + idle timeout
            resource_monitor::start_heartbeat(app.handle().clone());

            // Cache notch dimensions on the main thread (safe for NSScreen APIs).
            let notch = commands::overlay::detect_notch_info();
            {
                let state = app.state::<State>();
                *state.notch_info.lock_or_recover() = notch;
            }

            // Re-enable mouse events on the overlay window.
            // focusable:false sets ignoresMouseEvents=true on macOS;
            // we override that while keeping the window non-activating.
            // On Linux, skip the overlay — it's designed for the macOS notch.
            #[cfg(target_os = "macos")]
            if let Some(overlay_win) = app.get_webview_window("overlay") {
                tracing::info!(target: "system", "setup: overlay window found, enabling cursor events");
                diagnostics::log_webview_window_state(&overlay_win, "before_setup_overlay_show");
                commands::overlay::position_overlay_default(&overlay_win, notch);
                let _ = overlay_win.show();
                if let Err(e) = overlay_win.set_ignore_cursor_events(false) {
                    tracing::warn!(target: "system", "Failed to set overlay cursor events: {}", e);
                }
                diagnostics::log_webview_window_state(&overlay_win, "after_setup_overlay_show");
            } else {
                tracing::warn!(target: "system", "setup: overlay window NOT found");
            }

            // Listen for display config changes (monitor plug/unplug, lid open/close)
            // to re-detect notch info and reposition the overlay.
            commands::overlay::register_screen_change_observer(app.handle().clone());

            // Restore tray icon (removed by PR #63 overlay work).
            let idle_icon_data = commands::tray::make_tray_icon_data();
            let show_item = MenuItemBuilder::with_id("show", "Show Murmur").build(app)?;
            let quit_item = MenuItemBuilder::with_id("quit", "Quit Murmur").build(app)?;
            let tray_menu = MenuBuilder::new(app)
                .item(&show_item)
                .separator()
                .item(&quit_item)
                .build()?;
            let handle = app.handle().clone();
            TrayIconBuilder::with_id("main-tray")
                .icon(tauri::image::Image::new(&idle_icon_data, 66, 66))
                .icon_as_template(false)
                .tooltip("Murmur")
                .menu(&tray_menu)
                .show_menu_on_left_click(false)
                .on_menu_event(move |app_handle, event| {
                    match event.id().as_ref() {
                        "show" => {
                            tracing::info!(target: "system", source = "tray_menu", action = "show_main", "window action requested");
                            diagnostics::log_window_state_snapshot(app_handle, "before_tray_menu_show_main");
                            if let Some(win) = app_handle.get_webview_window("main") {
                                let _ = win.show();
                                let _ = win.set_focus();
                            }
                            diagnostics::log_window_state_snapshot(app_handle, "after_tray_menu_show_main");
                            let _ = audio::log_audio_route_snapshot("after_tray_menu_show_main");
                        }
                        "quit" => {
                            tracing::info!(target: "system", source = "tray_menu", action = "quit", "window action requested");
                            app_handle.exit(0);
                        }
                        _ => {}
                    }
                })
                .on_tray_icon_event(move |_tray, event| {
                    if matches!(event, TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    }) {
                        tracing::info!(target: "system", source = "tray_icon", event = ?event, action = "show_main", "tray icon clicked");
                        diagnostics::log_window_state_snapshot(&handle, "before_tray_icon_show_main");
                        if let Some(win) = handle.get_webview_window("main") {
                            let _ = win.show();
                            let _ = win.set_focus();
                        }
                        diagnostics::log_window_state_snapshot(&handle, "after_tray_icon_show_main");
                        let _ = audio::log_audio_route_snapshot("after_tray_icon_show_main");
                    }
                })
                .build(app)?;

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|_app_handle, _event| {
        match &_event {
            RunEvent::Ready => {
                tracing::info!(target: "system", "tauri run event ready");
                diagnostics::log_window_state_snapshot(_app_handle, "run_event_ready");
                let _ = audio::log_audio_route_snapshot("run_event_ready");
            }
            RunEvent::Resumed => {
                tracing::info!(target: "system", "tauri run event resumed");
                diagnostics::log_window_state_snapshot(_app_handle, "run_event_resumed");
                let _ = audio::log_audio_route_snapshot("run_event_resumed");
            }
            RunEvent::ExitRequested { code, .. } => {
                tracing::info!(target: "system", code = ?code, "tauri run event exit requested");
                diagnostics::log_window_state_snapshot(_app_handle, "run_event_exit_requested");
            }
            RunEvent::Exit => {
                tracing::info!(target: "system", "tauri run event exit");
            }
            #[cfg(any(target_os = "macos", target_os = "ios", target_os = "android"))]
            RunEvent::Opened { urls } => {
                tracing::info!(target: "system", count = urls.len(), urls = ?urls, "tauri run event opened");
                diagnostics::log_window_state_snapshot(_app_handle, "run_event_opened");
                let _ = audio::log_audio_route_snapshot("run_event_opened");
            }
            _ => {}
        }

        // Suppress Tauri's default RunEvent::Reopen behaviour which shows
        // the main window whenever the macOS app is activated — including
        // when the overlay is clicked.  We only re-show the main window
        // when there are truly no visible windows (e.g. dock-icon click
        // after the user closed everything).
        #[cfg(target_os = "macos")]
        if let RunEvent::Reopen { has_visible_windows, .. } = &_event {
            tracing::info!(
                target: "system",
                has_visible_windows = *has_visible_windows,
                "tauri run event reopen"
            );
            diagnostics::log_window_state_snapshot(_app_handle, "run_event_reopen");
            let _ = audio::log_audio_route_snapshot("run_event_reopen");
            if !has_visible_windows {
                if let Some(win) = _app_handle.get_webview_window("main") {
                    diagnostics::log_window_state_snapshot(_app_handle, "before_reopen_show_main");
                    let _ = win.show();
                    let _ = win.set_focus();
                    diagnostics::log_window_state_snapshot(_app_handle, "after_reopen_show_main");
                    let _ = audio::log_audio_route_snapshot("after_reopen_show_main");
                }
            }
        }
    });
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::ffi::OsString;

    /// Empty env: both defaults must be applied.
    #[test]
    fn applies_all_defaults_when_env_empty() {
        let env: RefCell<HashMap<String, OsString>> = RefCell::new(HashMap::new());
        apply_linux_webkit_env_defaults(
            |k| env.borrow().get(k).cloned(),
            |k, v| {
                env.borrow_mut().insert(k.to_string(), OsString::from(v));
            },
        );
        let map = env.borrow();
        assert_eq!(map.get("WEBKIT_DISABLE_DMABUF_RENDERER"), Some(&OsString::from("1")));
        assert_eq!(map.get("WEBKIT_DISABLE_COMPOSITING_MODE"), Some(&OsString::from("1")));
    }

    /// User-provided values must be preserved (including explicit "0" opt-outs).
    #[test]
    fn preserves_user_overrides() {
        let env: RefCell<HashMap<String, OsString>> = RefCell::new(HashMap::new());
        env.borrow_mut().insert("WEBKIT_DISABLE_DMABUF_RENDERER".into(), OsString::from("0"));
        env.borrow_mut().insert("WEBKIT_DISABLE_COMPOSITING_MODE".into(), OsString::from("custom"));

        apply_linux_webkit_env_defaults(
            |k| env.borrow().get(k).cloned(),
            |k, v| {
                env.borrow_mut().insert(k.to_string(), OsString::from(v));
            },
        );

        let map = env.borrow();
        assert_eq!(map.get("WEBKIT_DISABLE_DMABUF_RENDERER"), Some(&OsString::from("0")));
        assert_eq!(map.get("WEBKIT_DISABLE_COMPOSITING_MODE"), Some(&OsString::from("custom")));
    }

    /// Partial user override: only the unset default should be applied.
    #[test]
    fn applies_only_missing_defaults() {
        let env: RefCell<HashMap<String, OsString>> = RefCell::new(HashMap::new());
        env.borrow_mut().insert("WEBKIT_DISABLE_DMABUF_RENDERER".into(), OsString::from("0"));
        let writes: RefCell<Vec<(String, String)>> = RefCell::new(Vec::new());

        apply_linux_webkit_env_defaults(
            |k| env.borrow().get(k).cloned(),
            |k, v| {
                writes.borrow_mut().push((k.to_string(), v.to_string()));
                env.borrow_mut().insert(k.to_string(), OsString::from(v));
            },
        );

        assert_eq!(
            *writes.borrow(),
            vec![("WEBKIT_DISABLE_COMPOSITING_MODE".to_string(), "1".to_string())],
        );
        assert_eq!(
            env.borrow().get("WEBKIT_DISABLE_DMABUF_RENDERER"),
            Some(&OsString::from("0")),
        );
    }

    /// Empty string is a valid value and must be preserved (matches `var_os` semantics).
    #[test]
    fn treats_empty_string_as_set() {
        let env: RefCell<HashMap<String, OsString>> = RefCell::new(HashMap::new());
        env.borrow_mut().insert("WEBKIT_DISABLE_DMABUF_RENDERER".into(), OsString::from(""));
        let writes: RefCell<Vec<String>> = RefCell::new(Vec::new());

        apply_linux_webkit_env_defaults(
            |k| env.borrow().get(k).cloned(),
            |k, _v| writes.borrow_mut().push(k.to_string()),
        );

        assert_eq!(
            *writes.borrow(),
            vec!["WEBKIT_DISABLE_COMPOSITING_MODE".to_string()],
        );
    }
}
