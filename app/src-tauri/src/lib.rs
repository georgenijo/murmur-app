#[cfg(target_os = "macos")]
mod alloc;
mod audio;
mod audio_decode;
mod audio_inventory;
mod audio_lifecycle;
// `pub` so the headless benchmark runner (tests/headless_benchmark.rs) can
// call `benchmark::run` directly with a mock AppHandle; not part of any
// stable external API.
pub mod benchmark;
pub mod capture_agent_probe;
mod capture_health;
pub mod capture_helper_probe;
mod cleanup;
mod cli_command;
mod code_signing;
mod commands;
mod correct_and_teach;
mod correction;
mod delivery_recovery;
mod dictation_context;
mod dictation_telemetry;
pub mod evaluation;
mod file_output;
mod frontmost;
mod hang_diagnostics;
mod ide_context;
mod injector;
mod keyboard;
mod knowledge_store;
pub mod llm_sidecar;
mod log_shipper;
pub mod managed_child;
mod meeting_capture;
mod meeting_store;
mod microphone_preview;
mod model_artifact;
mod model_runtime;
mod performance_metrics;
mod platform;
mod query_adapter;
mod query_flow;
mod query_history;
mod query_provider;
mod resource_monitor;
mod selection;
mod smart_formatting;
mod spoken_numbers;
mod spoken_structure;
mod state;
pub mod telemetry;
pub mod transcriber;
mod transcript_transform;
mod transform_apply;
mod transform_diagnostics;
pub mod transform_flow;
mod transform_presets;
mod transform_trace;
mod vad;
mod vocab;
mod vocabulary_alias;
mod voice_commands;

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

#[cfg(target_os = "macos")]
pub fn rust_heap_bytes() -> u64 {
    alloc::rust_heap_bytes()
}

#[cfg(target_os = "macos")]
pub fn ffi_heap_bytes() -> u64 {
    alloc::ffi_heap_bytes()
}

#[cfg(not(target_os = "macos"))]
pub fn rust_heap_mb() -> u64 {
    0
}

#[cfg(not(target_os = "macos"))]
pub fn ffi_heap_mb() -> u64 {
    0
}

#[cfg(not(target_os = "macos"))]
pub fn rust_heap_bytes() -> u64 {
    0
}

#[cfg(not(target_os = "macos"))]
pub fn ffi_heap_bytes() -> u64 {
    0
}

use state::AppState;
use std::sync::{Mutex, MutexGuard};
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
#[cfg(target_os = "macos")]
use tauri::RunEvent;
use tauri::{Emitter, Manager};

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
    pub(crate) benchmark: std::sync::Arc<benchmark::BenchmarkCoordinator>,
    pub(crate) microphone_startup_benchmark:
        commands::microphone_startup_benchmark::MicrophoneStartupBenchmarkState,
    #[cfg(feature = "internal-benchmark")]
    pub(crate) corpus: commands::corpus::CorpusRecorderState,
    pub(crate) knowledge: knowledge_store::KnowledgeStore,
    pub(crate) meeting_store: meeting_store::MeetingStore,
    pub(crate) meetings: meeting_capture::MeetingCoordinator,
    pub(crate) delivery_recovery: delivery_recovery::DeliveryRecoveryState,
    pub(crate) correct_and_teach: correct_and_teach::CorrectAndTeachState,
    pub(crate) capture_health: capture_health::CaptureHealthDiagnostics,
    pub(crate) performance: performance_metrics::PerformanceMetrics,
    pub(crate) query_history: query_history::QueryHistoryStore,
    pub(crate) transform_diagnostics: transform_diagnostics::TransformDiagnostics,
    /// Cached overlay screen geometry
    /// (physical-or-synthetic-notch width, measured menu-bar height) from the
    /// primary NSScreen. Refreshed on the main thread after display changes.
    pub(crate) notch_info: Mutex<Option<(f64, f64)>>,
    /// Complete primary-display snapshot used to coalesce native screen
    /// notifications and skip geometrically identical updates.
    pub(crate) display_snapshot: Mutex<Option<commands::overlay::DisplaySnapshot>>,
    /// The selection-bounds anchor from the most recent `show_transform_popover`
    /// call, so `set_transform_popover_expanded` can resize/reposition for a
    /// new size class without the caller re-supplying the anchor.
    pub(crate) transform_popover_anchor: Mutex<Option<commands::transform_popover::Rect>>,
    /// Main-window visibility snapshotted at the FIRST popover show of a
    /// transform pass (issue #329): `Some(was_visible)` while a popover is up,
    /// `None` otherwise. `set_transform_popover_focusable`'s activation guard
    /// reads this sticky value instead of a per-call snapshot — rapid repeated
    /// transform-key presses interleave focus calls, and a per-call snapshot
    /// taken while a previous `set_focus` had transiently surfaced the main
    /// window would record "visible" and disable the re-hide guard, leaking
    /// the main window onto the screen.
    pub(crate) transform_main_was_visible: Mutex<Option<bool>>,
    /// Host-side supervisor for the signed local-LLM transform sidecar (#312).
    pub(crate) transform_runtime: std::sync::Arc<llm_sidecar::LlmSidecar>,
    /// Session-only voice-query state plus exact owned CLI child (#538).
    pub(crate) query: query_flow::QueryCoordinator,
}

/// Production mutual-exclusion bridge: lets the sidecar refuse to start over a
/// heavy ASR runtime and release the ASR model (via the existing
/// `MemoryPressure` unload path) before it spawns.
struct AppHostGuard {
    app: tauri::AppHandle,
}

impl llm_sidecar::HostGuard for AppHostGuard {
    fn heavy_runtime_active(&self) -> Option<&'static str> {
        use tauri::Manager;
        let state = self.app.state::<State>();
        if state.benchmark.is_running() {
            return Some("benchmark");
        }
        if state
            .app_state
            .file_transcribing
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Some("fileTranscription");
        }
        if state
            .app_state
            .meeting_inference_active
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Some("meeting");
        }
        if state.app_state.dictation.lock_or_recover().status != state::DictationStatus::Idle {
            return Some("recording");
        }
        None
    }

    fn release_asr(&self) {
        use tauri::Manager;
        let state = self.app.state::<State>();
        let _ = state
            .app_state
            .model_runtime
            .unload(Some(&self.app), model_runtime::UnloadReason::MemoryPressure);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init());

    #[cfg(not(feature = "internal-benchmark"))]
    let builder = builder
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_updater::Builder::new().build());

    #[cfg(feature = "internal-benchmark")]
    let builder = builder.plugin(commands::corpus::plugin());

    let app = builder
        .manage(State {
            app_state: AppState::default(),
            benchmark: std::sync::Arc::new(benchmark::BenchmarkCoordinator::new()),
            microphone_startup_benchmark:
                commands::microphone_startup_benchmark::MicrophoneStartupBenchmarkState::default(),
            #[cfg(feature = "internal-benchmark")]
            corpus: commands::corpus::CorpusRecorderState::default(),
            knowledge: knowledge_store::KnowledgeStore::default(),
            meeting_store: meeting_store::MeetingStore::default(),
            meetings: meeting_capture::MeetingCoordinator::default(),
            delivery_recovery: delivery_recovery::DeliveryRecoveryState::default(),
            correct_and_teach: correct_and_teach::CorrectAndTeachState::default(),
            capture_health: capture_health::CaptureHealthDiagnostics::default(),
            performance: performance_metrics::PerformanceMetrics::default(),
            query_history: query_history::QueryHistoryStore::default(),
            transform_diagnostics: transform_diagnostics::TransformDiagnostics::default(),
            notch_info: Mutex::new(None),
            display_snapshot: Mutex::new(None),
            transform_popover_anchor: Mutex::new(None),
            transform_main_was_visible: Mutex::new(None),
            transform_runtime: std::sync::Arc::new(llm_sidecar::LlmSidecar::new()),
            query: query_flow::QueryCoordinator::default(),
        })
        .invoke_handler(tauri::generate_handler![
            commands::recording::init_dictation,
            delivery_recovery::retry_last_delivery,
            commands::recording::process_audio,
            commands::recording::get_status,
            commands::recording::configure_dictation,
            commands::mode_runtime::get_mode_runtime_status,
            commands::mode_runtime::cycle_mode,
            commands::mode_runtime::clear_temporary_mode_override,
            commands::recording::start_native_recording,
            commands::recording::stop_native_recording,
            commands::recording::cancel_native_recording,
            commands::recording::cancel_audio_initialization,
            commands::recording::count_vocab_tokens,
            commands::recording::preview_vocabulary_aliases,
            commands::recording::reformat_history_text,
            commands::recording::transcribe_file,
            commands::recording::scan_code_vocab,
            commands::recording::cancel_code_vocab_scan,
            commands::recording::get_ide_context_status,
            commands::recording::refresh_ide_context,
            commands::recording::clear_ide_context,
            commands::correct_and_teach::propose_learned_correction,
            commands::correct_and_teach::propose_specific_learned_correction,
            commands::correct_and_teach::confirm_learned_correction,
            commands::correct_and_teach::discard_learned_correction_proposal,
            commands::permissions::open_system_preferences,
            commands::permissions::open_system_audio_preferences,
            commands::permissions::check_accessibility_permission,
            commands::permissions::request_accessibility_permission,
            commands::permissions::reset_accessibility_permission,
            commands::permissions::request_microphone_permission,
            commands::permissions::request_microphone_access,
            commands::permissions::check_microphone_permission,
            commands::permissions::check_microphone_permission_status,
            commands::permissions::reset_microphone_permission,
            commands::permissions::list_audio_devices,
            commands::permissions::get_audio_input_inventory,
            commands::microphone_preview::get_microphone_preview_status,
            commands::microphone_preview::start_microphone_preview,
            commands::microphone_preview::update_microphone_preview_vad_sensitivity,
            commands::microphone_preview::stop_microphone_preview,
            commands::microphone_preview::cancel_microphone_preview,
            commands::microphone_startup_benchmark::run_microphone_startup_benchmark,
            commands::microphone_startup_benchmark::cancel_microphone_startup_benchmark,
            commands::microphone_startup_benchmark::save_microphone_startup_benchmark_report,
            commands::integrations::is_notchpill_installed,
            commands::keyboard::start_keyboard_listener,
            commands::keyboard::stop_keyboard_listener,
            commands::keyboard::update_keyboard_key,
            commands::keyboard::set_keyboard_recording,
            commands::keyboard::set_app_disabled,
            commands::keyboard::get_app_disabled,
            commands::keyboard::set_paste_last_shortcut,
            commands::keyboard::start_transform_listener,
            commands::keyboard::stop_transform_listener,
            commands::keyboard::set_transform_key,
            commands::keyboard::start_query_listener,
            commands::keyboard::stop_query_listener,
            commands::recording::transform_status,
            transform_apply::apply_transform_result,
            transform_apply::undo_transform,
            transform_flow::start_transform_capture,
            transform_flow::finish_transform_instruction,
            transform_flow::retry_transform_instruction,
            transform_flow::approve_transform,
            transform_flow::cancel_transform,
            transform_flow::undo_transform_and_close,
            query_flow::start_query_capture,
            query_flow::finish_query_capture,
            query_flow::cancel_query,
            query_flow::copy_query_answer,
            query_flow::get_query_review_content,
            query_flow::list_query_provider_presets,
            query_flow::load_query_environment,
            query_flow::save_query_environment,
            query_flow::validate_query_command,
            query_flow::test_query_provider,
            query_flow::launch_query_provider_sign_in,
            query_flow::launch_query_sign_in_for_pass,
            query_flow::probe_query_sign_in_for_pass,
            commands::query_history::list_query_history,
            commands::query_history::clear_query_history,
            commands::knowledge::get_knowledge_store_status,
            commands::knowledge::retry_knowledge_store,
            commands::knowledge::list_knowledge,
            commands::knowledge::get_knowledge,
            commands::knowledge::upsert_knowledge,
            commands::knowledge::preview_voice_command,
            commands::knowledge::set_knowledge_enabled,
            commands::knowledge::delete_knowledge,
            commands::knowledge::resolve_knowledge,
            commands::knowledge::export_knowledge_to_file,
            commands::knowledge::inspect_knowledge_import,
            commands::knowledge::import_knowledge_from_file,
            commands::knowledge::delete_all_knowledge,
            commands::meeting::start_meeting,
            commands::meeting::stop_meeting,
            commands::meeting::get_meeting_status,
            commands::meeting::get_system_audio_permission_status,
            commands::meeting::request_system_audio_permission,
            commands::meeting::get_meeting_store_status,
            commands::meeting::list_meetings,
            commands::meeting::get_meeting,
            commands::meeting::get_meeting_export_text,
            commands::meeting::delete_meeting,
            commands::meeting::delete_all_meetings,
            commands::meeting::prune_meetings,
            commands::export::save_text_export,
            commands::settings_store::load_settings_blob,
            commands::settings_store::save_settings_blob,
            commands::settings_store::load_history_blob,
            commands::settings_store::save_history_blob,
            commands::settings_store::clear_history_blob,
            commands::settings_store::load_stats_blob,
            commands::settings_store::save_stats_blob,
            commands::settings_store::clear_stats_blob,
            commands::settings_store::load_theme_library_blob,
            commands::settings_store::save_theme_library_blob,
            commands::settings_store::clear_theme_library_blob,
            commands::theme::read_theme_file,
            commands::theme::write_theme_file,
            commands::logging::get_log_contents,
            commands::logging::clear_logs,
            commands::logging::log_frontend,
            capture_health::get_capture_health_history,
            commands::performance::list_performance_runs,
            commands::performance::get_performance_run,
            commands::performance::get_performance_resource_window,
            commands::performance::get_performance_store_health,
            commands::performance::recover_performance_store,
            commands::performance::clear_performance_diagnostics,
            commands::performance::show_diagnostics_window,
            commands::transform_diagnostics::arm_next_transform_diagnostic_capture,
            commands::transform_diagnostics::get_transform_diagnostic_capture_status,
            commands::transform_diagnostics::list_transform_attempts,
            commands::transform_diagnostics::list_transform_diagnostic_captures,
            commands::transform_diagnostics::get_transform_diagnostic_capture,
            commands::transform_diagnostics::delete_transform_diagnostic_capture,
            commands::models::check_model_exists,
            commands::models::check_specific_model_exists,
            commands::models::get_model_runtime_catalog,
            commands::models::get_model_runtime_status,
            commands::models::download_model,
            commands::transform_model::transform_model_status,
            commands::transform_model::download_transform_model,
            commands::transform_model::remove_transform_model,
            commands::transform_model::reset_transform_runtime,
            frontmost::list_running_applications,
            commands::benchmark::get_benchmark_models,
            commands::benchmark::get_benchmark_activity,
            commands::benchmark::run_benchmark,
            commands::benchmark::cancel_benchmark,
            commands::benchmark::save_benchmark_report,
            commands::benchmark::open_benchmark_output_folder,
            commands::tray::update_tray_icon,
            commands::tray::set_tray_update_available,
            commands::updater::get_update_install_environment,
            commands::updater::updater_canary,
            commands::overlay::show_overlay,
            commands::overlay::hide_overlay,
            commands::overlay::set_overlay_expanded,
            commands::overlay::set_overlay_vertical_offset,
            commands::overlay::show_main_window,
            commands::overlay::get_overlay_geometry,
            commands::transform_popover::get_transform_popover_geometry,
            commands::transform_popover::show_transform_popover,
            commands::transform_popover::hide_transform_popover,
            commands::transform_popover::set_transform_popover_expanded,
            commands::transform_popover::set_transform_popover_focusable,
            commands::transform_popover::get_transform_review_content,
            telemetry::get_event_history,
            telemetry::clear_event_history,
            resource_monitor::get_resource_usage
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // Hide instead of destroy for persistent windows
                if window.label() == "main" || window.label() == "diagnostics" {
                    api.prevent_close();
                    if window.label() == "main" {
                        commands::microphone_preview::cancel_for_window_close(
                            window.app_handle().clone(),
                        );
                    }
                    let _ = window.hide();
                    tracing::info!(target: "system", "{} window hidden on close request", window.label());
                }
            }
        })
        .setup(|app| {
            telemetry::init(app.handle().clone());
            audio_inventory::initialize(app.handle().clone());
            // Restore the durable per-device capture backend memo before any
            // capture can start, so a relaunch keeps the fast-fail tier a
            // known-bad machine already earned. Fail-open: an unavailable
            // directory just means this session stays in memory only.
            match commands::settings_store::data_dir(app.handle()) {
                Ok(dir) => audio::initialize_capture_memo_persistence(dir),
                Err(error) => tracing::warn!(
                    target: "audio",
                    "capture backend memo persistence unavailable: {error}"
                ),
            }
            log_shipper::start(app.handle());

            if let Some(main_window) = app.get_webview_window("main") {
                commands::native_window::hide_titlebar_separator(&main_window);
            }
            if let Some(diagnostics_window) = app.get_webview_window("diagnostics") {
                commands::native_window::hide_titlebar_separator(&diagnostics_window);
            }
            commands::query_popover::apply_initial_size(app.handle());

            let performance_root = app.path().app_data_dir()?.join("diagnostics");
            if let Err(error) = app
                .state::<State>()
                .capture_health
                .initialize(performance_root.clone(), &telemetry::event_jsonl_paths())
            {
                tracing::warn!(
                    target: "system",
                    diagnostics_available = false,
                    "capture-health diagnostics store unavailable: {}",
                    error
                );
            }
            if let Err(error) = app
                .state::<State>()
                .performance
                .initialize(performance_root.clone(), Some(app.handle().clone()))
            {
                tracing::warn!(
                    target: "system",
                    diagnostics_available = false,
                    "performance diagnostics store unavailable: {}",
                    error
                );
            }
            let query_history_root = app.path().app_data_dir()?.join("query-history");
            if let Err(error) = app
                .state::<State>()
                .query_history
                .initialize(query_history_root, Some(app.handle().clone()))
            {
                tracing::warn!(
                    target: "system",
                    query_history_available = false,
                    "Voice Query history store unavailable: {}",
                    error
                );
            }
            if let Err(error) = app
                .state::<State>()
                .transform_diagnostics
                .initialize(performance_root.join("transforms"))
            {
                tracing::warn!(
                    target: "system",
                    diagnostics_available = false,
                    "transform diagnostics store unavailable: {}",
                    error
                );
            }

            let knowledge_root = app.path().app_data_dir()?.join("knowledge");
            let knowledge_status = app.state::<State>().knowledge.initialize(knowledge_root);
            if knowledge_status.availability != knowledge_store::StoreAvailability::Unavailable {
                if let Err(error) = commands::knowledge::refresh_correction_rules(&app.state::<State>()) {
                    tracing::warn!(target: "system", error, "initial knowledge correction matcher refresh failed");
                }
            }
            tracing::info!(
                target: "system",
                availability = ?knowledge_status.availability,
                schema_version = knowledge_status.schema_version,
                record_count = knowledge_status.record_count,
                "personal knowledge store initialized"
            );

            let meeting_root = app.path().app_data_dir()?.join("meetings");
            let meeting_status = app
                .state::<State>()
                .meeting_store
                .initialize(meeting_root);
            tracing::info!(
                target: "meeting",
                availability = ?meeting_status.availability,
                schema_version = meeting_status.schema_version,
                session_count = meeting_status.session_count,
                pending_segment_count = meeting_status.pending_segment_count,
                "meeting transcript store initialized"
            );
            if let Ok(repository) = app.state::<State>().meeting_store.repository() {
                app.state::<State>()
                    .meetings
                    .recover_pending(app.handle().clone(), repository);
            }

            tracing::info!(target: "system", "app setup — Murmur v{}", env!("CARGO_PKG_VERSION"));

            // Emit startup baseline memory snapshot
            {
                let rss = resource_monitor::get_process_rss_mb();
                let heap = rust_heap_mb();
                let ffi = ffi_heap_mb();
                tracing::info!(target: "system", event_code = "system.startup_baseline", rss_mb = rss, rust_heap_mb = heap, ffi_heap_mb = ffi, "startup_baseline");
            }

            // Periodic heartbeat: memory telemetry + idle timeout
            resource_monitor::start_heartbeat(app.handle().clone());

            // Install the local-LLM mutual-exclusion bridge and start its
            // maintenance reaper (RSS ceiling + idle unload).
            {
                let state = app.state::<State>();
                state
                    .transform_runtime
                    .set_host_guard(std::sync::Arc::new(AppHostGuard {
                        app: app.handle().clone(),
                    }));
                let sidecar = std::sync::Arc::clone(&state.transform_runtime);
                tauri::async_runtime::spawn(async move {
                    let mut interval =
                        tokio::time::interval(std::time::Duration::from_secs(30));
                    loop {
                        interval.tick().await;
                        sidecar.maintenance_tick();
                    }
                });
            }

            // Cache notch dimensions on the main thread (safe for NSScreen APIs).
            let display_snapshot =
                commands::overlay::capture_display_snapshot(app.handle());
            let notch = display_snapshot.notch_info;
            {
                let state = app.state::<State>();
                *state.notch_info.lock_or_recover() = notch;
                *state.display_snapshot.lock_or_recover() = Some(display_snapshot);
            }

            // Re-enable mouse events on the overlay window. focusable:false
            // sets ignoresMouseEvents=true; override that while keeping the
            // macOS window non-activating.
            #[cfg(target_os = "macos")]
            if let Some(overlay_win) = app.get_webview_window("overlay") {
                tracing::info!(target: "system", "setup: overlay window found, enabling cursor events");
                commands::overlay::position_overlay_default(&overlay_win, notch, "startup");
                let _ = overlay_win.show();
                if let Err(e) = overlay_win.set_ignore_cursor_events(false) {
                    tracing::warn!(target: "system", "Failed to set overlay cursor events: {}", e);
                }
            } else {
                tracing::warn!(target: "system", "setup: overlay window NOT found");
            }

            // Listen for display config changes (monitor plug/unplug, lid open/close)
            // to re-detect notch info and reposition the overlay.
            commands::overlay::register_screen_change_observer(app.handle().clone());
            audio_lifecycle::register_sleep_wake_observer();
            frontmost::register_delivery_transition_observers();
            #[cfg(not(feature = "internal-benchmark"))]
            commands::tray::register_update_wake_observer(app.handle().clone());

            // Overwrite the transform-review window's initial size from Rust's
            // COMPACT_W/COMPACT_H so tauri.conf.json's matching literal is only
            // ever a startup-flash guard, never the source of truth.
            commands::transform_popover::apply_initial_compact_size(app.handle());

            // Restore tray icon (removed by PR #63 overlay work).
            let idle_icon_data = commands::tray::make_tray_icon_data();
            let show_item = MenuItemBuilder::with_id("show", "Show Murmur").build(app)?;
            let paste_last_item =
                MenuItemBuilder::with_id("paste_last", "Paste Last / Retry Delivery").build(app)?;
            let mode_item = MenuItemBuilder::with_id("cycle_mode", "Mode: Everyday").build(app)?;
            let disabled_item = tauri::menu::CheckMenuItemBuilder::with_id("toggle_disabled", "Disable Murmur")
                .checked(false)
                .build(app)?;
            let quit_item = MenuItemBuilder::with_id("quit", "Quit Murmur").build(app)?;
            let tray_menu = MenuBuilder::new(app).item(&show_item);
            #[cfg(not(feature = "internal-benchmark"))]
            let (tray_menu, update_item) = {
                let update_item =
                    MenuItemBuilder::with_id("check_updates", "Check for Updates…").build(app)?;
                (tray_menu.item(&update_item), update_item)
            };
            let tray_menu = tray_menu
                .item(&paste_last_item)
                .item(&mode_item)
                .separator()
                .item(&disabled_item)
                .separator()
                .item(&quit_item)
                .build()?;
            commands::keyboard::register_tray_disabled_item(disabled_item.clone());
            commands::tray::register_mode_item(mode_item);
            #[cfg(not(feature = "internal-benchmark"))]
            commands::tray::register_tray_update_item(update_item);
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
                            if let Some(win) = app_handle.get_webview_window("main") {
                                let _ = win.show();
                                let _ = win.set_focus();
                            }
                        }
                        "toggle_disabled" => {
                            let new_disabled = !keyboard::is_app_disabled();
                            if let Err(e) = commands::keyboard::set_app_disabled(app_handle.clone(), new_disabled) {
                                tracing::warn!(target: "keyboard", "tray disable toggle failed: {}", e);
                            }
                        }
                        "paste_last" => {
                            delivery_recovery::spawn_retry(app_handle.clone());
                        }
                        "cycle_mode" => {
                            let state = app_handle.state::<State>();
                            let _ = commands::mode_runtime::cycle_mode(app_handle.clone(), state);
                        }
                        "check_updates" => {
                            if let Some(win) = app_handle.get_webview_window("main") {
                                let _ = win.show();
                                let _ = win.set_focus();
                            }
                            let _ = app_handle.emit("check-for-updates-requested", ());
                        }
                        "quit" => {
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
                        if let Some(win) = handle.get_webview_window("main") {
                            let _ = win.show();
                            let _ = win.set_focus();
                        }
                    }
                })
                .build(app)?;

            commands::mode_runtime::spawn_mode_watcher(app.handle().clone());

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|_app_handle, _event| {
        // Suppress Tauri's default RunEvent::Reopen behaviour which shows
        // the main window whenever the macOS app is activated — including
        // when the overlay is clicked.  We only re-show the main window
        // when there are truly no visible windows (e.g. dock-icon click
        // after the user closed everything).
        #[cfg(target_os = "macos")]
        if let RunEvent::Reopen {
            has_visible_windows,
            ..
        } = &_event
        {
            if !has_visible_windows {
                if let Some(win) = _app_handle.get_webview_window("main") {
                    let _ = win.show();
                    let _ = win.set_focus();
                }
            }
        }

        if let RunEvent::Exit = &_event {
            audio_inventory::shutdown();
            // App-exit teardown: stop resident native helpers so none outlive
            // the app (all no-op when no child is running).
            #[cfg(target_os = "macos")]
            if let Some(state) = _app_handle.try_state::<State>() {
                state.meetings.shutdown(_app_handle);
                state.transform_runtime.shutdown();
                state.query.shutdown();
            }
        }
    });
}
