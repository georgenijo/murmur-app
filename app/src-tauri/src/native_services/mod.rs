//! Production service composition for the GPUI executable. Tauri is an optional
//! host adapter; these are the same recording, meeting and delivery owners.
pub use crate::alloc::RustZoneAllocator;
use crate::host::{self, ServiceAccess};
pub use crate::host::{
    Error, Monitor, NativePaths, NativePorts, WindowCommand, WindowEvent, WindowPort,
    WindowSnapshot, WorkArea,
};
use murmur_core::events::Consumer;
use serde_json::Value;
use std::sync::Arc;
mod commands;
#[path = "../main_thread_queue.rs"]
mod main_thread_queue;
pub use crate::dock_watch::PillDisplay as DockDisplay;
pub use crate::host::window::Color;
pub use dpi::{PhysicalPosition, PhysicalSize, Position, Size};
pub use main_thread_queue::MainThreadQueue;
#[derive(Clone)]
pub struct Services {
    pub(crate) context: host::Context,
}
/// Call before worker dispatch. The explicit identity selects all legacy model,
/// log, corpus, export and pasteboard namespaces before any worker can use them.
pub fn initialize_identity(identifier: &str, home: &std::path::Path) -> Result<(), String> {
    crate::identity::initialize_native(identifier, home)?;
    crate::launch::process_start();
    Ok(())
}
pub fn install_runtime(runtime: tokio::runtime::Handle) -> Result<(), String> {
    host::runtime::install(runtime).map_err(|_| "Host runtime already installed".into())
}
impl Services {
    /// Call on GPUI's main thread after it has registered its windows/ports.
    /// Optional storage initialization stays on the existing startup workers.
    pub fn start(
        context: murmur_core::context::AppContext,
        paths: NativePaths,
        identifier: String,
        ports: Arc<dyn NativePorts>,
    ) -> Result<Self, String> {
        validate_paths(&paths, &identifier)?;
        if crate::identity::initialized_identifier() != Some(identifier.as_str()) {
            return Err("Service identity must be initialized before worker dispatch".into());
        }
        if crate::identity::native_support()
            != Some(paths.home.join("Library/Application Support").as_path())
        {
            return Err("Native legacy path identity mismatch".into());
        }
        let app = host::Context::new(context, paths, identifier, ports);
        crate::events::initialize(&app);
        crate::shutdown::initialize(&app)?;
        crate::dictation_control::initialize(&app);
        crate::dictation_cues::initialize(&app);
        crate::window_state::initialize(&app)?;
        crate::telemetry::init(app.clone());
        crate::audio_inventory::initialize(app.clone());
        crate::smart_auto_probe::initialize(app.clone());
        crate::audio_graph_snapshot::set_app_handle(app.clone());
        let memo_app = app.clone();
        let permit = crate::shutdown::WriterPermit::accepted_producer();
        host::runtime::spawn_blocking(move || {
            let _permit = permit;
            if let Ok(root) = crate::commands::settings_store::data_dir(&memo_app) {
                crate::audio::initialize_capture_memo_persistence(root);
            }
        });
        crate::log_shipper::start(&app);
        crate::completion::initialize(&app)?;
        crate::startup::initialize(&app)?;
        crate::resource_monitor::start_heartbeat(app.clone());
        let snapshot = crate::commands::overlay::capture_display_snapshot(&app);
        *app.services()
            .notch_info
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = snapshot.notch_info;
        *app.services()
            .display_snapshot
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = Some(snapshot);
        // Native presentation owns overlay placement/material observers. The
        // shared recording owner still owns generation/consent and visibility.
        crate::audio_lifecycle::register_sleep_wake_observer();
        crate::frontmost::register_delivery_transition_observers();
        let dock = app.clone();
        let moved = app.clone();
        crate::dock_watch::start(
            move || {
                dock.dock_context()
                    .map(|(display, visible)| crate::dock_watch::WatchContext { display, visible })
            },
            move || moved.dock_changed(),
        );
        crate::smart_destination::start(app.clone());
        crate::meeting_suggestions::initialize(app.clone());
        crate::commands::mode_runtime::spawn_mode_watcher(app.clone());
        crate::launch::prepare(&app);
        Ok(Self { context: app })
    }
    pub fn shutdown(&self, request: murmur_core::shutdown::Request) {
        crate::shutdown::request(&self.context, request)
    }
    pub fn termination_permitted(&self) -> bool {
        crate::shutdown::permitted(&self.context)
    }
    pub fn restart_permitted(&self) -> bool {
        crate::shutdown::restart_permitted(&self.context)
    }
    pub fn terminal_receipt(&self) {
        crate::shutdown::receipt(&self.context)
    }
    pub fn main_hidden(&self) {
        crate::commands::microphone_preview::cancel_for_window_close(self.context.clone());
    }
    pub fn retry_delivery(&self) {
        crate::delivery_recovery::spawn_retry(self.context.clone());
    }
    pub async fn configure_recording(&self, settings: Value) -> Result<(), String> {
        crate::dictation_control::configure(&self.context, settings).await
    }
}

pub fn validate_paths(paths: &NativePaths, identifier: &str) -> Result<(), String> {
    if !paths.home.is_absolute()
        || identifier.is_empty()
        || identifier.starts_with('.')
        || identifier.ends_with('.')
        || identifier.contains("..")
        || !identifier
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        || paths.app
            != paths
                .home
                .join("Library/Application Support")
                .join(identifier)
    {
        return Err("Native data identity mismatch".into());
    }
    Ok(())
}
impl Services {
    pub fn recording_is_current(&self, generation: u64) -> bool {
        use std::sync::atomic::Ordering;
        let state = self.context.services();
        state.app_state.recording_id.load(Ordering::SeqCst) == generation
            && !state.app_state.is_cancelled(generation)
            && state.app_state.dictation_status_if_uncontended()
                == Some(crate::state::DictationStatus::Recording)
    }
    pub fn settings_revision(&self) -> u64 {
        self.context
            .services()
            .app_state
            .settings_revision
            .load(std::sync::atomic::Ordering::SeqCst)
    }
    pub fn window_event(&self, consumer: Consumer, event: host::WindowEvent) {
        if let Some(window) = consumer
            .window_label()
            .and_then(|label| self.context.get_webview_window(label))
        {
            let label = window.label();
            crate::window_state::on_event(&window, &event);
            if matches!(event, host::WindowEvent::Destroyed) {
                crate::events::clear_window(&self.context, label);
                crate::commands::overlay_material::window_destroyed(&self.context, label);
            }
        }
    }
    pub fn waveform_interest(&self, consumer: murmur_core::events::WaveformConsumer, active: bool) {
        if let Some(window) = consumer
            .consumer()
            .window_label()
            .and_then(|label| self.context.get_webview_window(label))
        {
            let _ = crate::events::set_waveform_subscription(
                self.context.clone(),
                window,
                consumer,
                active,
            );
        }
    }
    pub async fn audio_range(
        &self,
        session_id: String,
        segment_id: i64,
        offset_bytes: u64,
        length_bytes: u32,
    ) -> Result<Vec<u8>, String> {
        let window = self
            .context
            .get_webview_window("main")
            .ok_or("Native view unavailable")?;
        let response = crate::meeting_audio::read_meeting_audio_range(
            self.context.clone(),
            window,
            session_id,
            segment_id,
            offset_bytes,
            length_bytes,
            self.context.services(),
        )
        .await?;
        Ok(response.0)
    }
}

impl Services {
    pub fn clipboard_text(&self) -> Result<String, String> {
        if crate::shutdown::closing() {
            return Err("Application is closing".into());
        }
        crate::injector::read_clipboard_text()
    }
    pub fn copy_text(&self, text: &str) -> Result<(), String> {
        let _permit = crate::shutdown::WriterPermit::acquire()?;
        crate::injector::write_clipboard_text(text)
    }
}
/// Query only; this never opens an input or asks for TCC permission.
pub fn microphone_status() -> murmur_core::context::PermissionStatus {
    use murmur_core::context::PermissionStatus;
    match crate::commands::permissions::check_microphone_permission_status().as_str() {
        "granted" => PermissionStatus::Granted,
        "denied" => PermissionStatus::Denied,
        "notDetermined" => PermissionStatus::NotDetermined,
        _ => PermissionStatus::Unknown,
    }
}

impl Services {
    pub fn dock_limit(&self, display: DockDisplay) -> murmur_core::overlay_geometry::DockLimit {
        crate::dock_watch::current_limit(display)
    }
    pub fn toggle_disabled(&self) -> Result<(), String> {
        crate::commands::settings_store::toggle_tray_disabled(self.context.clone())
    }
    pub fn window_ready(&self, consumer: Consumer) {
        if let Some(window) = consumer
            .window_label()
            .and_then(|label| self.context.get_webview_window(label))
        {
            crate::commands::native_window::hide_titlebar_separator(&window);
        }
    }
}

impl Services {
    /// Admission is synchronous, before returning to the view. The caller may
    /// drop the receiver on hide; the source owner retains the job and permit.
    pub fn accept_settings(
        &self,
        patch: Value,
        expected_revision: u64,
    ) -> Result<tokio::sync::oneshot::Receiver<Result<Value, String>>, String> {
        let permit = crate::shutdown::WriterPermit::acquire()?;
        let app = self.context.clone();
        crate::shutdown::submit_source(move || {
            let _permit = permit;
            crate::commands::settings_store::patch_native_settings_worker(
                app,
                patch,
                expected_revision,
            )
        })
        .map_err(|_| "Source writer unavailable".into())
    }
}

impl Services {
    pub fn accept_source(
        &self,
        key: String,
        expected: Value,
        value: Value,
    ) -> Result<tokio::sync::oneshot::Receiver<Result<(), String>>, String> {
        if ![
            "murmur-appearance",
            "murmur_discovery_v1",
            "murmur.settings-disclosure.v1",
            "usage-dashboard-collapsed",
            "murmur_onboarding_complete",
        ]
        .contains(&key.as_str())
        {
            return Err("Source is owned by another service".into());
        }
        let permit = crate::shutdown::WriterPermit::acquire()?;
        let app = self.context.clone();
        crate::shutdown::submit_source(move || {
            let _permit = permit;
            crate::completion::native_compare_source(&app, key, expected, value)
        })
        .map_err(|_| "Source writer unavailable".into())
    }
    pub async fn history_snapshot(
        &self,
    ) -> Result<(u64, murmur_core::completion::Snapshot), String> {
        crate::completion::native_history_snapshot(self.context.clone()).await
    }
    /// The shared owner serializes the content epoch check with Clear/Delete.
    pub fn copy_history(
        &self,
        revision: u64,
        targets: Vec<Value>,
        text: String,
        live: EffectLease,
    ) -> Result<(), String> {
        crate::completion::native_copy_history(&self.context, revision, targets, text, live)
    }
}

/// Retains a producer's shutdown admission across an asynchronous OS operation.
/// The producer must hand it to the runtime before returning to presentation.
pub struct AcceptedWork {
    _permit: crate::shutdown::WriterPermit,
}
impl Services {
    pub async fn retry_initialization(&self) -> Result<(), String> {
        crate::completion::native_retry_initialization(self.context.clone()).await
    }
    pub fn legacy_store_present(&self) -> bool {
        self.context.path().home_dir().is_ok_and(|home| {
            home.join("Library/WebKit")
                .join(&self.context.config().identifier)
                .exists()
        })
    }
    pub async fn import_legacy(&self, value: Value) -> Result<(Vec<String>, bool), String> {
        crate::completion::native_import(self.context.clone(), value).await
    }
    pub async fn acknowledge_legacy(&self, value: Value) -> Result<(), String> {
        crate::completion::native_acknowledge(self.context.clone(), value).await
    }
    pub fn accept_work(&self) -> Result<AcceptedWork, String> {
        crate::shutdown::WriterPermit::acquire().map(|permit| AcceptedWork { _permit: permit })
    }
    pub async fn settings_snapshot(&self) -> Result<(u64, Value), String> {
        let app = self.context.clone();
        crate::shutdown::source_call(move || {
            crate::commands::settings_store::native_settings_snapshot(&app)
        })
        .await
        .map_err(|_| "Source reader unavailable".to_owned())?
    }
    pub fn recording_generation(&self) -> u64 {
        self.context
            .services()
            .app_state
            .recording_id
            .load(std::sync::atomic::Ordering::SeqCst)
    }
    pub fn start_idle(&self) -> Result<(), String> {
        crate::dictation_control::native_start_idle(&self.context)
    }
    pub fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }
}

pub type EffectLease = Arc<dyn Fn() -> bool + Send + Sync>;
tokio::task_local! { static EFFECT_LEASE: EffectLease; }
static OPEN_PORTS: std::sync::OnceLock<Arc<dyn NativePorts>> = std::sync::OnceLock::new();
pub(crate) fn install_ports(ports: Arc<dyn NativePorts>) {
    let _ = OPEN_PORTS.set(ports);
}
pub(crate) fn open_system_url(value: String) -> Result<(), String> {
    let lease = EFFECT_LEASE
        .try_with(Clone::clone)
        .unwrap_or_else(|_| Arc::new(|| !crate::shutdown::closing()));
    OPEN_PORTS
        .get()
        .ok_or("Native opener unavailable")?
        .open_url(value, lease)
        .map_err(|_| "Native opener unavailable".into())
}
impl Services {
    pub async fn request_guarded(
        &self,
        consumer: Consumer,
        command: &str,
        args: Value,
        lease: EffectLease,
    ) -> Result<Value, String> {
        if !lease() {
            return Err("Presentation changed".into());
        }
        EFFECT_LEASE
            .scope(lease, self.request(consumer, command, args))
            .await
    }
}
impl Services {
    pub fn mutate_history(
        &self,
        epoch: u64,
        command: murmur_core::completion::Command,
    ) -> Result<(), String> {
        crate::completion::native_history_mutate(&self.context, epoch, command)
    }
    /// Prepare export bytes and a sibling staging file before this call. The
    /// owner admits only bounded final publication, never encoding or a panel.
    pub fn publish_history(
        &self,
        epoch: u64,
        targets: Vec<Value>,
        effect: impl FnOnce() -> Result<(), murmur_core::completion::Error> + Send + 'static,
    ) -> Result<(), String> {
        crate::completion::native_history_publish(&self.context, epoch, targets, effect)
    }
}
impl Services {
    pub fn inspect_knowledge(&self, bytes: &[u8]) -> Result<(Value, u64), String> {
        let (summary, revision) = self.context.services().knowledge.inspect_bytes(bytes)?;
        Ok((
            serde_json::to_value(summary).map_err(|_| "Import review unavailable")?,
            revision,
        ))
    }
    pub fn import_knowledge(&self, bytes: &[u8], expected: u64) -> Result<(), String> {
        let _permit = crate::shutdown::WriterPermit::acquire()?;
        self.context
            .services()
            .knowledge
            .import_bytes(bytes, expected)?;
        crate::commands::knowledge::refresh_correction_rules(&self.context.services())
    }
}
impl Services {
    pub fn accept_theme_library(
        &self,
        expected: Value,
        value: Value,
    ) -> Result<tokio::sync::oneshot::Receiver<Result<(), String>>, String> {
        let permit = crate::shutdown::WriterPermit::acquire()?;
        let app = self.context.clone();
        crate::shutdown::submit_source(move || {
            let _permit = permit;
            crate::commands::settings_store::native_theme_library(&app, expected, value)
        })
        .map_err(|_| "Theme owner unavailable".into())
    }
    pub async fn install_speakers(&self) -> Result<(), String> {
        crate::diarization_model::download(&self.context).await
    }
    pub fn preview_cue(&self, cue: &str) -> Result<(), String> {
        if !["start", "stop", "success", "failure"].contains(&cue) {
            return Err("Invalid cue".into());
        }
        let cue = cue.to_owned();
        let control = self.context.control();
        let volume = control.settings.lock().unwrap_or_else(|p| p.into_inner())["soundCueVolume"]
            .as_f64()
            .unwrap_or(45.)
            .clamp(0., 100.);
        crate::host::runtime::spawn_blocking(move || crate::dictation_cues::preview(&cue, volume));
        Ok(())
    }
}
impl Services {
    pub async fn stop_meeting(&self, expected: u64) -> Result<(), String> {
        let state = self.context.services();
        let _transition = state.app_state.recording_transition.lock().await;
        if state.meetings.status().generation != expected {
            return Err("Meeting ownership changed".into());
        }
        state.meetings.stop(&self.context)
    }
}

impl Services {
    pub fn next_mode(&self, revision: u64, id: Option<&str>) -> Result<(), String> {
        crate::commands::mode_runtime::select_next_recording_mode_at(
            &self.context,
            &self.context.services(),
            id,
            Some(revision),
        )
        .map(|_| ())
    }
}
impl Services {
    /// The source FIFO owns OS registration, runtime admission and publication
    /// as one accepted preference. An OS failure never publishes its checkbox.
    pub fn accept_settings_effect(
        &self,
        patch: Value,
        revision: u64,
        setup: bool,
        before: impl std::future::Future<Output = Result<(), String>> + Send + 'static,
        rollback: impl std::future::Future<Output = ()> + Send + 'static,
    ) -> Result<tokio::sync::oneshot::Receiver<Result<Value, String>>, String> {
        let permit = crate::shutdown::WriterPermit::acquire()?;
        let app = self.context.clone();
        crate::shutdown::submit_source(move || {
            let _permit = permit;
            crate::commands::settings_store::patch_native_settings_effect_worker(
                app,
                patch,
                revision,
                setup,
                || crate::host::runtime::block_on(before),
                || crate::host::runtime::block_on(rollback),
            )
        })
        .map_err(|_| "Settings owner unavailable".into())
    }
}
impl Services {
    pub fn recording_clock(&self) -> Option<(u64, u64)> {
        let state = self.context.services();
        let dictation = state.app_state.dictation.try_lock().ok()?;
        if dictation.status != crate::state::DictationStatus::Recording {
            return None;
        }
        let started = dictation.recording_started_at_ms?;
        let elapsed = chrono::Utc::now()
            .timestamp_millis()
            .saturating_sub(started)
            .max(0) as u64
            / 1000;
        Some((
            state
                .app_state
                .recording_id
                .load(std::sync::atomic::Ordering::SeqCst),
            elapsed,
        ))
    }
}

impl Services {
    pub fn recording_control(
        &self,
        generation: u64,
        input: murmur_core::dictation::Input,
    ) -> Result<(), String> {
        crate::dictation_control::native_control(&self.context, generation, input)
    }
}

impl Services {
    /// Freeze the reviewed proposal and scope before returning to presentation.
    /// Hide cannot revoke it or discard the confirmation queued for this ID.
    pub fn accept_correction(
        &self,
        proposal: u64,
        scope: Value,
    ) -> Result<tokio::task::JoinHandle<Result<Value, String>>, String> {
        let permit = crate::shutdown::WriterPermit::acquire()?;
        let scope = serde_json::from_value(scope).map_err(|_| "Invalid correction scope")?;
        let pending = self
            .context
            .services()
            .correct_and_teach
            .take_confirmed(proposal, &scope)?;
        let app = self.context.clone();
        Ok(crate::host::runtime::spawn_blocking(move || {
            let _permit = permit;
            let entry = crate::commands::correct_and_teach::commit_confirmation(
                app.clone(),
                proposal,
                pending,
                scope,
                app.services(),
            )?;
            serde_json::to_value(entry).map_err(|_| "Correction result unavailable".into())
        }))
    }
}
