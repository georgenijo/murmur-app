mod host;
mod identity;

pub mod native_services;

#[cfg(feature = "internal-benchmark")]
mod accuracy_corpus;
#[cfg(target_os = "macos")]
mod alloc;

mod audio;
mod audio_decode;
mod audio_graph_snapshot;
mod audio_inventory;
mod audio_lifecycle;
mod events;
// `pub` so the headless benchmark runner (tests/headless_benchmark.rs) can
// call `benchmark::run_headless` without a UI host; not part of any
// stable external API.
pub mod benchmark;
mod browser_site;
mod calendar;
pub mod capture_agent_probe;
mod capture_health;
pub mod capture_helper_probe;
mod cleanup;
mod cli_command;
mod code_signing;
mod commands;
mod completion;
pub mod coreml_installer;
mod correct_and_teach;
mod correction;
mod delivery_recovery;
mod diarization_assignment;
mod diarization_audio;
mod diarization_model;
mod dictation_context;
mod dictation_control;
mod dictation_cues;
mod dictation_diagnostics;
#[cfg(test)]
mod dictation_diagnostics_contract;
mod dictation_telemetry;
mod dock_watch;
pub mod evaluation;
mod file_output;
mod frontmost;
mod hang_diagnostics;
mod homophone_context;
mod hotkey_release;
mod ide_context;
mod injector;
mod keyboard;
mod knowledge_store;
pub mod launch;
mod legacy_query_cleanup;
mod legacy_transform_cleanup;
mod log_shipper;
pub mod managed_child;
mod meeting_artifact;
mod meeting_audio;
mod meeting_bundle;
mod meeting_capture;
pub mod meeting_diarization;
mod meeting_review;
mod meeting_store;
mod meeting_suggestions;
mod microphone_auto;
mod microphone_preview;
mod microphone_signal;

#[cfg(test)]
mod qa_clipboard;
mod shutdown;
mod startup;
use murmur_core::{model_artifact, model_digest};
mod model_download;
mod model_hardware_guidance;
mod model_runtime;
mod performance_metrics;
mod platform;
#[cfg(feature = "preview-decode-reuse")]
mod preview_pause;
#[cfg(feature = "preview-decode-reuse")]
mod preview_reuse;
mod resource_monitor;
mod smart_auto_probe;
mod smart_destination;
mod smart_formatting;
mod smart_turn;
mod spoken_numbers;
mod spoken_structure;
mod sqlite_support;
mod state;
pub mod telemetry;
pub mod transcriber;
mod transcript_transform;
mod vad;
mod vocab;
mod vocabulary_alias;
mod voice_commands;
mod window_state;

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

/// Helper trait to recover from poisoned mutexes
pub(crate) trait MutexExt<T> {
    fn lock_or_recover(&self) -> MutexGuard<'_, T>;

    /// Non-blocking sibling of [`MutexExt::lock_or_recover`]: recovers from
    /// poisoning exactly the same way, but returns `None` instead of waiting
    /// when the lock is held. Diagnostics use this so a probe investigating a
    /// stall can never join the queue behind it — contention is reported as a
    /// fact rather than waited out.
    fn try_lock_or_recover(&self) -> Option<MutexGuard<'_, T>>;
}

impl<T> MutexExt<T> for Mutex<T> {
    fn lock_or_recover(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|poisoned| {
            tracing::warn!(target: "system", "Mutex was poisoned, recovering data");
            poisoned.into_inner()
        })
    }

    fn try_lock_or_recover(&self) -> Option<MutexGuard<'_, T>> {
        match self.try_lock() {
            Ok(guard) => Some(guard),
            Err(std::sync::TryLockError::Poisoned(poisoned)) => Some(poisoned.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) => None,
        }
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
    pub(crate) meeting_suggestions: meeting_suggestions::MeetingSuggestions,
    pub(crate) delivery_recovery: delivery_recovery::DeliveryRecoveryState,
    pub(crate) correct_and_teach: correct_and_teach::CorrectAndTeachState,
    pub(crate) capture_health: capture_health::CaptureHealthDiagnostics,
    pub(crate) performance: performance_metrics::PerformanceMetrics,
    pub(crate) dictation_diagnostics: dictation_diagnostics::DictationDiagnostics,
    /// Cached overlay screen geometry
    /// (physical-or-synthetic-notch width, measured menu-bar height) from the
    /// primary NSScreen. Refreshed on the main thread after display changes.
    pub(crate) notch_info: Mutex<Option<commands::overlay::NotchInfo>>,
    /// Complete primary-display snapshot used to coalesce native screen
    /// notifications and skip geometrically identical updates.
    pub(crate) display_snapshot: Mutex<Option<commands::overlay::DisplaySnapshot>>,
    /// Requested logical-point calibration for this session. Native placement
    /// refreshes re-resolve its effective value against the current display, so
    /// a temporary menu-band clamp does not erase the saved/draft intent.
    pub(crate) overlay_calibration: Mutex<commands::overlay::OverlayCalibrationState>,
    /// Last native overlay frame accepted by WindowServer-facing setters. The
    /// click-through dictation preview follows this actual frame.
    pub(crate) overlay_frame: Mutex<Option<commands::overlay::OverlayFrame>>,
    pub(crate) overlay_layout: Mutex<commands::overlay::OverlayLayout>,
}

// Tauri's pre-runtime generated-context panic path contains exit(101).
// This helper is called only before the application/coordinator is built.

impl Default for State {
    fn default() -> Self {
        Self {
            app_state: AppState::default(),
            benchmark: std::sync::Arc::new(benchmark::BenchmarkCoordinator::new()),
            microphone_startup_benchmark:
                commands::microphone_startup_benchmark::MicrophoneStartupBenchmarkState::default(),
            #[cfg(feature = "internal-benchmark")]
            corpus: commands::corpus::CorpusRecorderState::default(),
            knowledge: knowledge_store::KnowledgeStore::default(),
            meeting_store: meeting_store::MeetingStore::default(),
            meetings: meeting_capture::MeetingCoordinator::default(),
            meeting_suggestions: meeting_suggestions::MeetingSuggestions::default(),
            delivery_recovery: delivery_recovery::DeliveryRecoveryState::default(),
            correct_and_teach: correct_and_teach::CorrectAndTeachState::default(),
            capture_health: capture_health::CaptureHealthDiagnostics::default(),
            performance: performance_metrics::PerformanceMetrics::default(),
            dictation_diagnostics: dictation_diagnostics::DictationDiagnostics::default(),
            notch_info: Mutex::new(None),
            display_snapshot: Mutex::new(None),
            overlay_calibration: Mutex::new(commands::overlay::OverlayCalibrationState::default()),
            overlay_frame: Mutex::new(None),
            overlay_layout: Mutex::new(commands::overlay::OverlayLayout::default()),
        }
    }
}
