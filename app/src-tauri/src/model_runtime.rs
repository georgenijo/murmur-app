use crate::transcriber::{
    ParakeetBackend, TranscriptionBackend, WhisperBackend, COREML_MODEL_NAME,
};
use crate::MutexExt;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use tauri::Emitter;

pub const PARAKEET_CPU_MODEL: &str = "parakeet-tdt-0.6b-v2-fp16";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCapabilities {
    pub partial_results: bool,
    pub initial_prompts: bool,
    pub multilingual: bool,
    pub translation: bool,
    pub timestamps: bool,
    pub confidence: bool,
    pub punctuation_control: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendKind {
    Whisper,
    Parakeet,
    Coreml,
}

impl BackendKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Whisper => "whisper",
            Self::Parakeet => "parakeet",
            Self::Coreml => "coreml",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallKind {
    Whisper,
    Parakeet,
    Coreml,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlatformRequirement {
    Mac,
    AppleSiliconMac,
}

#[derive(Clone, Copy, Debug)]
pub struct ModelDefinition {
    pub model_name: &'static str,
    pub label: &'static str,
    pub size: &'static str,
    pub backend: BackendKind,
    pub accelerator: &'static str,
    pub capabilities: ModelCapabilities,
    pub install_kind: InstallKind,
    pub warm_on_startup: bool,
    pub retry_unfiltered_on_empty: bool,
    platform: PlatformRequirement,
}

const WHISPER_EN_CAPABILITIES: ModelCapabilities = ModelCapabilities {
    // Live preview uses a separate, cancellable CPU context for Tiny/Base.
    partial_results: true,
    initial_prompts: true,
    multilingual: false,
    translation: false,
    timestamps: false,
    confidence: false,
    punctuation_control: true,
};

const WHISPER_MULTILINGUAL_CAPABILITIES: ModelCapabilities = ModelCapabilities {
    partial_results: false,
    multilingual: true,
    ..WHISPER_EN_CAPABILITIES
};

const PARAKEET_CPU_CAPABILITIES: ModelCapabilities = ModelCapabilities {
    partial_results: false,
    initial_prompts: false,
    multilingual: false,
    translation: false,
    timestamps: false,
    confidence: false,
    punctuation_control: true,
};

const COREML_CAPABILITIES: ModelCapabilities = ModelCapabilities {
    partial_results: true,
    initial_prompts: false,
    multilingual: true,
    translation: false,
    timestamps: false,
    confidence: true,
    punctuation_control: true,
};

pub const MODEL_DEFINITIONS: &[ModelDefinition] = &[
    ModelDefinition {
        model_name: COREML_MODEL_NAME,
        label: "Parakeet Core ML",
        size: "~470 MB",
        backend: BackendKind::Coreml,
        accelerator: "Apple Neural Engine",
        capabilities: COREML_CAPABILITIES,
        install_kind: InstallKind::Coreml,
        warm_on_startup: true,
        retry_unfiltered_on_empty: true,
        platform: PlatformRequirement::AppleSiliconMac,
    },
    ModelDefinition {
        model_name: PARAKEET_CPU_MODEL,
        label: "Parakeet TDT 0.6B (English, fast)",
        size: "~1.2 GB",
        backend: BackendKind::Parakeet,
        accelerator: "CPU",
        capabilities: PARAKEET_CPU_CAPABILITIES,
        install_kind: InstallKind::Parakeet,
        warm_on_startup: false,
        retry_unfiltered_on_empty: false,
        platform: PlatformRequirement::Mac,
    },
    ModelDefinition {
        model_name: "tiny.en",
        label: "Whisper Tiny (English)",
        size: "~75 MB",
        backend: BackendKind::Whisper,
        accelerator: "Metal GPU",
        capabilities: WHISPER_EN_CAPABILITIES,
        install_kind: InstallKind::Whisper,
        warm_on_startup: false,
        retry_unfiltered_on_empty: false,
        platform: PlatformRequirement::Mac,
    },
    ModelDefinition {
        model_name: "base.en",
        label: "Whisper Base (English)",
        size: "~150 MB",
        backend: BackendKind::Whisper,
        accelerator: "Metal GPU",
        capabilities: WHISPER_EN_CAPABILITIES,
        install_kind: InstallKind::Whisper,
        warm_on_startup: false,
        retry_unfiltered_on_empty: false,
        platform: PlatformRequirement::Mac,
    },
    ModelDefinition {
        model_name: "small.en",
        label: "Whisper Small (English)",
        size: "~500 MB",
        backend: BackendKind::Whisper,
        accelerator: "Metal GPU",
        capabilities: ModelCapabilities {
            partial_results: false,
            ..WHISPER_EN_CAPABILITIES
        },
        install_kind: InstallKind::Whisper,
        warm_on_startup: false,
        retry_unfiltered_on_empty: false,
        platform: PlatformRequirement::Mac,
    },
    ModelDefinition {
        model_name: "medium.en",
        label: "Whisper Medium (English)",
        size: "~1.5 GB",
        backend: BackendKind::Whisper,
        accelerator: "Metal GPU",
        capabilities: ModelCapabilities {
            partial_results: false,
            ..WHISPER_EN_CAPABILITIES
        },
        install_kind: InstallKind::Whisper,
        warm_on_startup: false,
        retry_unfiltered_on_empty: false,
        platform: PlatformRequirement::Mac,
    },
    ModelDefinition {
        model_name: "large-v3-turbo",
        label: "Whisper Large Turbo",
        size: "~1.6 GB",
        backend: BackendKind::Whisper,
        accelerator: "Metal GPU",
        capabilities: WHISPER_MULTILINGUAL_CAPABILITIES,
        install_kind: InstallKind::Whisper,
        warm_on_startup: false,
        retry_unfiltered_on_empty: false,
        platform: PlatformRequirement::Mac,
    },
];

pub fn model_definition(model_name: &str) -> Result<&'static ModelDefinition, String> {
    MODEL_DEFINITIONS
        .iter()
        .find(|definition| definition.model_name == model_name)
        .ok_or_else(|| format!("Unknown transcription model '{model_name}'"))
}

/// Content-free classification of a model preparation failure for logs. The
/// error text itself is never logged; only one of these fixed labels is.
pub(crate) fn prepare_failure_kind(error: &str) -> &'static str {
    if error.starts_with("Failed to load") || error.contains("FluidAudio initialized") {
        "load_failed"
    } else if error.contains("not downloaded")
        || error.contains("not installed")
        || error.contains("not found")
    {
        "not_installed"
    } else if error.starts_with("Unknown ") {
        "unknown_model"
    } else {
        "other"
    }
}

/// Why the selected transcription model cannot serve a dictation right now.
/// The wire name is the camelCase string the overlay switches on; it carries
/// no model name, path or transcript content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ModelUnavailable {
    NotInstalled,
    Incomplete,
    Downloading,
    Unsupported,
    LoadFailed,
}

impl ModelUnavailable {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::NotInstalled => "notInstalled",
            Self::Incomplete => "incomplete",
            Self::Downloading => "downloading",
            Self::Unsupported => "unsupported",
            Self::LoadFailed => "loadFailed",
        }
    }
}

/// The start gate: `Some` when the model on disk cannot be used, so a
/// dictation would record into a dead end. A model that is installed passes
/// even after a failed warm-up; a real load failure surfaces at transcription.
pub(crate) fn start_blocker(
    supported: bool,
    install_state: InstallState,
    cache_incomplete: bool,
) -> Option<ModelUnavailable> {
    if !supported {
        return Some(ModelUnavailable::Unsupported);
    }
    match install_state {
        InstallState::Installed => None,
        InstallState::Installing | InstallState::Validating => Some(ModelUnavailable::Downloading),
        InstallState::Invalid => Some(ModelUnavailable::Incomplete),
        InstallState::NotInstalled if cache_incomplete => Some(ModelUnavailable::Incomplete),
        InstallState::NotInstalled => Some(ModelUnavailable::NotInstalled),
    }
}

/// What to tell the user when preparation failed at transcription time: the
/// on-disk state when it explains the failure, else a plain load failure.
/// `decode_errored` is true when the error came from the decode closure of
/// `with_ready_backend` (the model was prepared and ran): such an error is not a
/// model problem whatever its text says, so it never produces a cue.
pub(crate) fn failure_reason(
    supported: bool,
    install_state: InstallState,
    cache_incomplete: bool,
    error: &str,
    decode_errored: bool,
) -> Option<ModelUnavailable> {
    if decode_errored {
        return None;
    }
    start_blocker(supported, install_state, cache_incomplete).or(
        match prepare_failure_kind(error) {
            "load_failed" => Some(ModelUnavailable::LoadFailed),
            "not_installed" => Some(ModelUnavailable::NotInstalled),
            _ => None,
        },
    )
}

#[cfg(test)]
thread_local! {
    static FORCED_START_BLOCKER: std::cell::Cell<Option<ModelUnavailable>> =
        const { std::cell::Cell::new(None) };
}

/// Test seam: on this thread, `start_gate` answers `blocker` instead of probing
/// the disk, until the guard drops. Without it the gate reads the live runtime,
/// and a test machine has no installed model.
#[cfg(test)]
pub(crate) struct ForcedStartBlocker;

#[cfg(test)]
pub(crate) fn force_start_blocker(blocker: Option<ModelUnavailable>) -> ForcedStartBlocker {
    FORCED_START_BLOCKER.with(|slot| slot.set(blocker));
    ForcedStartBlocker
}

#[cfg(test)]
impl Drop for ForcedStartBlocker {
    fn drop(&mut self) {
        FORCED_START_BLOCKER.with(|slot| slot.set(None));
    }
}

impl ModelRuntimeManager {
    /// The start gate `start_native_recording_inner` consults once a start would
    /// create a generation. Production reads the live runtime; unit tests never
    /// probe the disk (see [`force_start_blocker`]).
    pub(crate) fn start_gate(&self, model_name: &str) -> Option<ModelUnavailable> {
        #[cfg(test)]
        {
            let _ = model_name;
            FORCED_START_BLOCKER.with(|slot| slot.get())
        }
        #[cfg(not(test))]
        {
            self.start_blocker_for(model_name)
        }
    }

    /// The reason the model cannot be used, from the live runtime snapshot.
    #[cfg_attr(test, allow(dead_code))]
    pub(crate) fn start_blocker_for(&self, model_name: &str) -> Option<ModelUnavailable> {
        let snapshot = self.snapshot(model_name).ok()?;
        start_blocker(
            snapshot.supported,
            snapshot.install_state,
            snapshot.cache_incomplete,
        )
    }

    pub(crate) fn failure_reason_for(
        &self,
        model_name: &str,
        error: &str,
        decode_errored: bool,
    ) -> Option<ModelUnavailable> {
        let snapshot = self.snapshot(model_name).ok()?;
        failure_reason(
            snapshot.supported,
            snapshot.install_state,
            snapshot.cache_incomplete,
            error,
            decode_errored,
        )
    }
}

pub fn model_supported(definition: &ModelDefinition) -> bool {
    match definition.platform {
        PlatformRequirement::Mac => true,
        PlatformRequirement::AppleSiliconMac => {
            cfg!(all(target_os = "macos", target_arch = "aarch64"))
        }
    }
}

pub fn model_accelerator(definition: &ModelDefinition) -> &'static str {
    definition.accelerator
}

fn supported_platforms(definition: &ModelDefinition) -> Vec<String> {
    match definition.platform {
        PlatformRequirement::Mac => vec!["macos".to_string()],
        PlatformRequirement::AppleSiliconMac => vec!["macos-arm64".to_string()],
    }
}

pub fn create_backend(model_name: &str) -> Result<Box<dyn TranscriptionBackend>, String> {
    let definition = model_definition(model_name)?;
    if !model_supported(definition) {
        return Err("This model is not supported on the current platform".to_string());
    }
    match definition.backend {
        BackendKind::Whisper => Ok(Box::new(WhisperBackend::new())),
        BackendKind::Parakeet => Ok(Box::new(ParakeetBackend::new())),
        BackendKind::Coreml => {
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            {
                Ok(Box::new(crate::transcriber::CoreMlBackend::new()))
            }
            #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
            {
                Err("Core ML transcription requires Apple Silicon macOS".to_string())
            }
        }
    }
}

fn cache_incomplete_signal(definition: &ModelDefinition, install_state: InstallState) -> bool {
    if install_state == InstallState::Installed || definition.install_kind != InstallKind::Coreml {
        return false;
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        crate::transcriber::coreml::model_cache_incomplete(definition.model_name)
    }
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    {
        false
    }
}

pub fn model_installed(model_name: &str) -> bool {
    let Ok(definition) = model_definition(model_name) else {
        return false;
    };
    if !model_supported(definition) {
        return false;
    }
    match definition.install_kind {
        InstallKind::Whisper => crate::transcriber::whisper::specific_model_exists(model_name),
        InstallKind::Parakeet => crate::transcriber::parakeet::specific_model_exists(model_name),
        InstallKind::Coreml => {
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            {
                crate::transcriber::coreml::specific_model_exists(model_name)
            }
            #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
            {
                false
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum InstallState {
    NotInstalled,
    Installing,
    Validating,
    Installed,
    Invalid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LifecycleState {
    Unloaded,
    Loading,
    Warming,
    Ready,
    Unloading,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreparationReason {
    Recording,
    StartupWarm,
    Pipeline,
    FileTranscription,
    Meeting,
}

impl PreparationReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::Recording => "recording",
            Self::StartupWarm => "startupWarm",
            Self::Pipeline => "pipeline",
            Self::FileTranscription => "fileTranscription",
            Self::Meeting => "meeting",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnloadReason {
    ModelChanged,
    IdleTimeout,
}

impl UnloadReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::ModelChanged => "modelChanged",
            Self::IdleTimeout => "idleTimeout",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRuntimeSnapshot {
    pub generation: u64,
    pub model_name: String,
    pub label: String,
    pub size: String,
    pub backend: BackendKind,
    pub accelerator: String,
    pub capabilities: ModelCapabilities,
    pub supported_platforms: Vec<String>,
    pub supported: bool,
    pub unavailable_reason: Option<&'static str>,
    pub install_state: InstallState,
    pub lifecycle_state: LifecycleState,
    pub failure_present: bool,
    /// The model is not installed but left an incomplete cache behind. Only a
    /// real on-disk signal sets this; a failed warm-up alone never does.
    pub cache_incomplete: bool,
}

#[derive(Clone, Copy)]
struct RuntimeStatus {
    lifecycle: LifecycleState,
    failure_present: bool,
}

impl Default for RuntimeStatus {
    fn default() -> Self {
        Self {
            lifecycle: LifecycleState::Unloaded,
            failure_present: false,
        }
    }
}

struct RuntimeInner {
    backend: Box<dyn TranscriptionBackend>,
    active_model: Option<String>,
}

#[derive(Clone, Copy, Debug)]
pub struct LoadReport {
    pub cache_hit: bool,
    pub lock_wait_ms: u64,
    pub load_ms: u64,
}

pub struct ModelRuntimeManager {
    definitions: &'static [ModelDefinition],
    inner: Mutex<RuntimeInner>,
    statuses: Mutex<HashMap<String, RuntimeStatus>>,
    install_states: Mutex<HashMap<String, InstallState>>,
    install_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    generation: AtomicU64,
}

impl Default for ModelRuntimeManager {
    fn default() -> Self {
        Self {
            definitions: MODEL_DEFINITIONS,
            inner: Mutex::new(RuntimeInner {
                backend: Box::new(WhisperBackend::new()),
                active_model: None,
            }),
            statuses: Mutex::new(HashMap::new()),
            install_states: Mutex::new(HashMap::new()),
            install_locks: Mutex::new(HashMap::new()),
            generation: AtomicU64::new(0),
        }
    }
}

impl ModelRuntimeManager {
    fn definition(&self, model_name: &str) -> Result<&'static ModelDefinition, String> {
        self.definitions
            .iter()
            .find(|definition| definition.model_name == model_name)
            .ok_or_else(|| format!("Unknown transcription model '{model_name}'"))
    }

    fn current_install_state(&self, model_name: &str) -> InstallState {
        if let Some(state) = self
            .install_states
            .lock_or_recover()
            .get(model_name)
            .copied()
        {
            return state;
        }
        if model_installed(model_name) {
            InstallState::Installed
        } else {
            InstallState::NotInstalled
        }
    }

    pub fn snapshot(&self, model_name: &str) -> Result<ModelRuntimeSnapshot, String> {
        let definition = self.definition(model_name)?;
        let status = self
            .statuses
            .lock_or_recover()
            .get(model_name)
            .copied()
            .unwrap_or_default();
        let supported = model_supported(definition);
        let install_state = self.current_install_state(model_name);
        Ok(ModelRuntimeSnapshot {
            generation: self.generation.load(Ordering::SeqCst),
            model_name: definition.model_name.to_string(),
            label: definition.label.to_string(),
            size: definition.size.to_string(),
            backend: definition.backend,
            accelerator: model_accelerator(definition).to_string(),
            capabilities: definition.capabilities,
            supported_platforms: supported_platforms(definition),
            supported,
            unavailable_reason: (!supported).then_some("unsupportedPlatform"),
            install_state,
            lifecycle_state: status.lifecycle,
            failure_present: status.failure_present,
            cache_incomplete: cache_incomplete_signal(definition, install_state),
        })
    }

    pub fn catalog(&self) -> Vec<ModelRuntimeSnapshot> {
        self.definitions
            .iter()
            .filter_map(|definition| self.snapshot(definition.model_name).ok())
            .collect()
    }

    /// Lifecycle-only view for hang diagnostics: no filesystem probes (unlike
    /// `snapshot`/`catalog`) and never touches `inner`, which is held across a
    /// whole model load. `None` means the status map was contended.
    pub fn lifecycle_states(&self) -> Option<Vec<(String, LifecycleState, bool)>> {
        let statuses = self.statuses.try_lock_or_recover()?;
        Some(
            statuses
                .iter()
                .map(|(model, status)| (model.clone(), status.lifecycle, status.failure_present))
                .collect(),
        )
    }

    /// The loaded model, or `None` when `inner` is contended — an in-flight
    /// load holds that lock for its whole duration, so a diagnostic must not
    /// block on it.
    pub fn active_model_if_uncontended(&self) -> Option<Option<String>> {
        self.inner
            .try_lock_or_recover()
            .map(|inner| inner.active_model.clone())
    }

    pub fn any_model_installed(&self) -> bool {
        self.definitions
            .iter()
            .any(|definition| model_installed(definition.model_name))
    }

    pub fn install_lock(&self, model_name: &str) -> Result<Arc<tokio::sync::Mutex<()>>, String> {
        self.definition(model_name)?;
        let mut locks = self.install_locks.lock_or_recover();
        Ok(locks
            .entry(model_name.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone())
    }

    /// Hold runtime ownership through deletion and publication. Never wait behind
    /// inference/preparation and then delete a model after the request became stale.
    pub fn remove_model_artifact(
        &self,
        app: Option<&tauri::AppHandle>,
        model_name: &str,
        selected_model: &str,
        remove: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        self.remove_model_artifact_with_presence(app, model_name, selected_model, remove, || {
            model_installed(model_name)
        })
    }

    fn remove_model_artifact_with_presence(
        &self,
        app: Option<&tauri::AppHandle>,
        model_name: &str,
        selected_model: &str,
        remove: impl FnOnce() -> Result<(), String>,
        still_installed: impl FnOnce() -> bool,
    ) -> Result<(), String> {
        self.definition(model_name)?;
        let _install_lock = self.install_lock(model_name)?;
        let _install_guard = _install_lock
            .try_lock()
            .map_err(|_| "Wait for this model's download or removal to finish".to_string())?;
        let inner = self
            .inner
            .try_lock_or_recover()
            .ok_or("Wait for model preparation or transcription to finish")?;
        if selected_model == model_name || inner.active_model.as_deref() == Some(model_name) {
            return Err(
                "Choose another model before removing the selected or active model".to_string(),
            );
        }
        match self.current_install_state(model_name) {
            InstallState::Installing | InstallState::Validating => {
                return Err("Wait for this model's download to finish".to_string());
            }
            InstallState::Installed => {}
            _ => return Err("This model is not installed".to_string()),
        }
        if let Err(error) = remove() {
            // Recursive deletion can remove required files before another entry
            // fails. Never retain a cached Installed snapshot for that bundle.
            if !still_installed() {
                self.set_install_state(app, model_name, InstallState::Invalid)?;
            }
            return Err(error);
        }
        self.statuses.lock_or_recover().remove(model_name);
        self.set_install_state(app, model_name, InstallState::NotInstalled)
    }

    pub fn begin_install(
        &self,
        app: Option<&tauri::AppHandle>,
        model_name: &str,
    ) -> Result<bool, String> {
        self.begin_install_with_presence(app, model_name, model_installed(model_name))
    }

    fn begin_install_with_presence(
        &self,
        app: Option<&tauri::AppHandle>,
        model_name: &str,
        already_installed: bool,
    ) -> Result<bool, String> {
        self.definition(model_name)?;
        let state = if already_installed {
            InstallState::Installed
        } else {
            InstallState::Installing
        };
        self.set_install_state(app, model_name, state)?;
        Ok(!already_installed)
    }

    /// A download or repair finished. A warm-up failure recorded before it
    /// described files that no longer exist, so drop it. Other Installed
    /// transitions (files already present, a cancelled attempt) must not clear a
    /// genuine load failure.
    pub fn complete_install(
        &self,
        app: Option<&tauri::AppHandle>,
        model_name: &str,
    ) -> Result<(), String> {
        self.definition(model_name)?;
        let was_installing = matches!(
            self.install_states.lock_or_recover().get(model_name),
            Some(InstallState::Installing | InstallState::Validating)
        );
        if was_installing {
            let mut statuses = self.statuses.lock_or_recover();
            if statuses
                .get(model_name)
                .is_some_and(|status| status.failure_present)
            {
                statuses.remove(model_name);
            }
        }
        self.set_install_state(app, model_name, InstallState::Installed)
    }

    pub fn set_install_state(
        &self,
        app: Option<&tauri::AppHandle>,
        model_name: &str,
        state: InstallState,
    ) -> Result<(), String> {
        self.definition(model_name)?;
        self.install_states
            .lock_or_recover()
            .insert(model_name.to_string(), state);
        self.publish(app, model_name, "installStateChanged")
    }

    fn publish(
        &self,
        app: Option<&tauri::AppHandle>,
        model_name: &str,
        reason: &'static str,
    ) -> Result<(), String> {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let mut snapshot = self.snapshot(model_name)?;
        snapshot.generation = generation;
        tracing::info!(
            target: "pipeline",
            model = model_name,
            backend = snapshot.backend.as_str(),
            lifecycle = ?snapshot.lifecycle_state,
            install = ?snapshot.install_state,
            reason,
            failure_present = snapshot.failure_present,
            generation,
            "model_runtime_transition"
        );
        if let Some(app) = app {
            let _ = app.emit("model-runtime-status-changed", &snapshot);
        }
        Ok(())
    }

    fn set_lifecycle(
        &self,
        app: Option<&tauri::AppHandle>,
        model_name: &str,
        lifecycle: LifecycleState,
        failure_present: bool,
        reason: &'static str,
    ) -> Result<(), String> {
        self.statuses.lock_or_recover().insert(
            model_name.to_string(),
            RuntimeStatus {
                lifecycle,
                failure_present,
            },
        );
        self.publish(app, model_name, reason)
    }

    fn ensure_backend<'a>(
        &self,
        app: Option<&tauri::AppHandle>,
        inner: &'a mut MutexGuard<'_, RuntimeInner>,
        model_name: &str,
    ) -> Result<&'a mut Box<dyn TranscriptionBackend>, String> {
        let definition = self.definition(model_name)?;
        if !model_supported(definition) {
            return Err("This model is not supported on the current platform".to_string());
        }

        if let Some(active) = inner
            .active_model
            .as_deref()
            .filter(|active| *active != model_name)
            .map(str::to_string)
        {
            self.set_lifecycle(
                app,
                &active,
                LifecycleState::Unloading,
                false,
                UnloadReason::ModelChanged.as_str(),
            )?;
            inner.backend.reset();
            inner.active_model = None;
            self.set_lifecycle(
                app,
                &active,
                LifecycleState::Unloaded,
                false,
                UnloadReason::ModelChanged.as_str(),
            )?;
        }

        if inner.backend.name() != definition.backend.as_str() {
            inner.backend.reset();
            inner.backend = create_backend(model_name)?;
        }
        Ok(&mut inner.backend)
    }

    fn ensure_loaded(
        &self,
        app: Option<&tauri::AppHandle>,
        inner: &mut MutexGuard<'_, RuntimeInner>,
        model_name: &str,
        reason: PreparationReason,
    ) -> Result<LoadReport, String> {
        let lifecycle = if reason == PreparationReason::StartupWarm {
            LifecycleState::Warming
        } else {
            LifecycleState::Loading
        };
        let backend = self.ensure_backend(app, inner, model_name)?;
        let cache_hit = backend.is_model_loaded(model_name);
        if cache_hit {
            inner.active_model = Some(model_name.to_string());
            self.set_lifecycle(
                app,
                model_name,
                LifecycleState::Ready,
                false,
                reason.as_str(),
            )?;
            crate::launch::model_ready();
            return Ok(LoadReport {
                cache_hit: true,
                lock_wait_ms: 0,
                load_ms: 0,
            });
        }
        self.set_lifecycle(app, model_name, lifecycle, false, reason.as_str())?;
        let started = std::time::Instant::now();
        let result = backend.load_model(model_name);
        let load_ms = started.elapsed().as_millis() as u64;
        match result {
            Ok(()) => {
                inner.active_model = Some(model_name.to_string());
                self.set_lifecycle(
                    app,
                    model_name,
                    LifecycleState::Ready,
                    false,
                    reason.as_str(),
                )?;
                crate::launch::model_ready();
                Ok(LoadReport {
                    cache_hit: false,
                    lock_wait_ms: 0,
                    load_ms,
                })
            }
            Err(error) => {
                inner.active_model = None;
                self.set_lifecycle(
                    app,
                    model_name,
                    LifecycleState::Failed,
                    true,
                    reason.as_str(),
                )?;
                Err(error)
            }
        }
    }

    pub fn prepare(
        &self,
        app: Option<&tauri::AppHandle>,
        model_name: &str,
        reason: PreparationReason,
    ) -> Result<LoadReport, String> {
        let _diarization_reservation =
            crate::meeting_diarization::ForegroundReservation::acquire()?;
        let lock_started = std::time::Instant::now();
        let mut inner = self.inner.lock_or_recover();
        let lock_wait_ms = lock_started.elapsed().as_millis() as u64;
        let mut report = self.ensure_loaded(app, &mut inner, model_name, reason)?;
        report.lock_wait_ms = lock_wait_ms;
        Ok(report)
    }

    /// Native launch preparation must not replace a newer frontend selection.
    /// Recheck the launch generation under the same lock that selects/loads.
    pub(crate) fn prepare_startup_if(
        &self,
        app: Option<&tauri::AppHandle>,
        model_name: &str,
        is_current: impl FnOnce() -> bool,
    ) -> Result<Option<LoadReport>, String> {
        let _reservation = crate::meeting_diarization::ForegroundReservation::acquire()?;
        let mut inner = self.inner.lock_or_recover();
        if !is_current() {
            return Ok(None);
        }
        self.ensure_loaded(app, &mut inner, model_name, PreparationReason::StartupWarm)
            .map(Some)
    }

    pub fn with_ready_backend<T>(
        &self,
        app: Option<&tauri::AppHandle>,
        model_name: &str,
        reason: PreparationReason,
        operation: impl FnOnce(&mut dyn TranscriptionBackend) -> Result<T, String>,
    ) -> Result<(T, LoadReport), String> {
        let _diarization_reservation =
            crate::meeting_diarization::ForegroundReservation::acquire()?;
        let lock_started = std::time::Instant::now();
        let mut inner = self.inner.lock_or_recover();
        let lock_wait_ms = lock_started.elapsed().as_millis() as u64;
        let mut report = self.ensure_loaded(app, &mut inner, model_name, reason)?;
        report.lock_wait_ms = lock_wait_ms;
        let result = operation(inner.backend.as_mut())?;
        Ok((result, report))
    }

    /// Opportunistic display-only inference: never queue or load/switch models.
    /// Revalidate ownership under the backend lock immediately before decode.
    /// That callback must not block on any other ownership/status lock.
    pub(crate) fn try_with_ready_backend<T>(
        &self,
        model_name: &str,
        is_current: impl FnOnce() -> bool,
        operation: impl FnOnce(&mut dyn TranscriptionBackend) -> Result<T, String>,
    ) -> Result<Option<T>, String> {
        let Some(_reservation) = crate::meeting_diarization::ForegroundReservation::try_acquire()
        else {
            return Ok(None);
        };
        // Recover poison consistently with final passes; skip only contention.
        let Some(mut inner) = self.inner.try_lock_or_recover() else {
            return Ok(None);
        };
        if inner.active_model.as_deref() != Some(model_name)
            || !inner.backend.is_model_loaded(model_name)
            || !is_current()
        {
            return Ok(None);
        }
        operation(inner.backend.as_mut()).map(Some)
    }

    pub fn select_model(
        &self,
        app: Option<&tauri::AppHandle>,
        model_name: &str,
    ) -> Result<(), String> {
        self.definition(model_name)?;
        let mut inner = self.inner.lock_or_recover();
        // Frontend configuration may confirm the model already warmed by Rust.
        // Preserve that warm backend instead of unloading it.
        if inner.active_model.as_deref() == Some(model_name)
            && inner.backend.is_model_loaded(model_name)
        {
            return Ok(());
        }
        if let Some(active) = inner.active_model.take() {
            self.set_lifecycle(
                app,
                &active,
                LifecycleState::Unloading,
                false,
                UnloadReason::ModelChanged.as_str(),
            )?;
            inner.backend.reset();
            self.set_lifecycle(
                app,
                &active,
                LifecycleState::Unloaded,
                false,
                UnloadReason::ModelChanged.as_str(),
            )?;
        }
        if inner.backend.name() != self.definition(model_name)?.backend.as_str() {
            inner.backend = create_backend(model_name)?;
        }
        self.set_lifecycle(
            app,
            model_name,
            LifecycleState::Unloaded,
            false,
            "modelSelected",
        )
    }

    pub fn unload(
        &self,
        app: Option<&tauri::AppHandle>,
        reason: UnloadReason,
    ) -> Result<Option<String>, String> {
        let mut inner = self.inner.lock_or_recover();
        let Some(model_name) = inner.active_model.take() else {
            return Ok(None);
        };
        self.set_lifecycle(
            app,
            &model_name,
            LifecycleState::Unloading,
            false,
            reason.as_str(),
        )?;
        let backend_name = inner.backend.name().to_string();
        inner.backend.reset();
        self.set_lifecycle(
            app,
            &model_name,
            LifecycleState::Unloaded,
            false,
            reason.as_str(),
        )?;
        Ok(Some(backend_name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;
    use std::time::Duration;

    #[test]
    fn removal_refuses_unknown_selected_active_busy_and_installing_models() {
        let manager = ModelRuntimeManager::default();
        manager
            .set_install_state(None, "tiny.en", InstallState::Installed)
            .unwrap();
        let forbidden = || -> Result<(), String> { panic!("removal must not run") };
        for unknown in ["../tiny.en", "tiny.en/other", "unknown"] {
            assert!(manager
                .remove_model_artifact(None, unknown, "base.en", forbidden)
                .is_err());
        }
        assert!(manager
            .remove_model_artifact(None, "tiny.en", "tiny.en", forbidden)
            .unwrap_err()
            .contains("selected"));
        manager.inner.lock_or_recover().active_model = Some("tiny.en".into());
        assert!(manager
            .remove_model_artifact(None, "tiny.en", "base.en", forbidden)
            .unwrap_err()
            .contains("active"));
        manager.inner.lock_or_recover().active_model = None;
        {
            let _runtime = manager.inner.lock_or_recover();
            assert!(manager
                .remove_model_artifact(None, "tiny.en", "base.en", forbidden)
                .unwrap_err()
                .contains("preparation"));
        }
        {
            let install = manager.install_lock("tiny.en").unwrap();
            let _download = install.try_lock().unwrap();
            assert!(manager
                .remove_model_artifact(None, "tiny.en", "base.en", forbidden)
                .unwrap_err()
                .contains("download"));
        }
        for state in [
            InstallState::Installing,
            InstallState::Validating,
            InstallState::NotInstalled,
        ] {
            manager.set_install_state(None, "tiny.en", state).unwrap();
            assert!(manager
                .remove_model_artifact(None, "tiny.en", "base.en", forbidden)
                .is_err());
        }
    }

    #[test]
    fn removal_owns_runtime_and_install_until_not_installed_is_published() {
        let manager = ModelRuntimeManager::default();
        manager
            .set_install_state(None, "tiny.en", InstallState::Installed)
            .unwrap();
        let generation = manager.snapshot("tiny.en").unwrap().generation;
        manager
            .remove_model_artifact(None, "tiny.en", "base.en", || {
                assert!(manager.inner.try_lock_or_recover().is_none());
                assert!(manager.install_lock("tiny.en").unwrap().try_lock().is_err());
                Ok(())
            })
            .unwrap();
        let snapshot = manager.snapshot("tiny.en").unwrap();
        assert_eq!(snapshot.install_state, InstallState::NotInstalled);
        assert_eq!(snapshot.lifecycle_state, LifecycleState::Unloaded);
        assert!(snapshot.generation > generation);
        assert_eq!(
            manager
                .catalog()
                .into_iter()
                .find(|m| m.model_name == "tiny.en")
                .unwrap()
                .install_state,
            InstallState::NotInstalled
        );
        manager
            .set_install_state(None, "tiny.en", InstallState::Installed)
            .unwrap();
        assert!(manager
            .remove_model_artifact_with_presence(
                None,
                "tiny.en",
                "base.en",
                || Err("preflight refused".into()),
                || true,
            )
            .is_err());
        assert_eq!(
            manager.snapshot("tiny.en").unwrap().install_state,
            InstallState::Installed
        );
        let present = std::cell::Cell::new(true);
        assert!(manager
            .remove_model_artifact_with_presence(
                None,
                "tiny.en",
                "base.en",
                || {
                    present.set(false);
                    Err("partial deletion failed".into())
                },
                || present.get(),
            )
            .is_err());
        assert_eq!(
            manager.snapshot("tiny.en").unwrap().install_state,
            InstallState::Invalid
        );
        assert!(manager.install_lock("tiny.en").unwrap().try_lock().is_ok());
    }

    #[test]
    fn shipped_catalog_is_unique_and_fail_closed() {
        let names = MODEL_DEFINITIONS
            .iter()
            .map(|model| model.model_name)
            .collect::<HashSet<_>>();
        assert_eq!(names.len(), 7);
        assert!(model_definition("base.en").is_ok());
        assert!(model_definition("future-unknown-model").is_err());
        assert!(create_backend("future-unknown-model").is_err());
    }

    #[test]
    fn shipped_catalog_reports_only_macos_platforms() {
        for definition in MODEL_DEFINITIONS {
            let platforms = supported_platforms(definition);
            assert!(!platforms.is_empty());
            assert!(platforms
                .iter()
                .all(|platform| platform.starts_with("macos")));
        }
    }

    #[test]
    fn shipped_capabilities_match_current_backend_facts() {
        assert!(
            model_definition("base.en")
                .unwrap()
                .capabilities
                .partial_results
        );
        assert!(
            model_definition("base.en")
                .unwrap()
                .capabilities
                .initial_prompts
        );
        assert!(
            !model_definition("base.en")
                .unwrap()
                .capabilities
                .multilingual
        );
        assert!(
            model_definition("large-v3-turbo")
                .unwrap()
                .capabilities
                .multilingual
        );
        assert!(
            !model_definition(PARAKEET_CPU_MODEL)
                .unwrap()
                .capabilities
                .partial_results
        );
        assert!(
            model_definition(COREML_MODEL_NAME)
                .unwrap()
                .capabilities
                .confidence
        );
    }

    #[test]
    fn partial_capabilities_match_enabled_producers() {
        for model in MODEL_DEFINITIONS {
            assert_eq!(
                model.capabilities.partial_results,
                model.backend == BackendKind::Coreml
                    || crate::transcriber::whisper::supports_live_preview(model.model_name),
                "{}",
                model.model_name
            );
        }
    }

    const FAKE_CAPABILITIES: ModelCapabilities = ModelCapabilities {
        partial_results: false,
        initial_prompts: false,
        multilingual: false,
        translation: true,
        timestamps: true,
        confidence: true,
        punctuation_control: false,
    };

    static FAKE_DEFINITIONS: &[ModelDefinition] = &[
        ModelDefinition {
            model_name: "fake-translation",
            label: "Fake translation backend",
            size: "0 MB",
            backend: BackendKind::Whisper,
            accelerator: "Test",
            capabilities: FAKE_CAPABILITIES,
            install_kind: InstallKind::Whisper,
            warm_on_startup: false,
            retry_unfiltered_on_empty: false,
            platform: PlatformRequirement::Mac,
        },
        ModelDefinition {
            model_name: "fake-second",
            label: "Second fake model",
            size: "0 MB",
            backend: BackendKind::Whisper,
            accelerator: "Test",
            capabilities: FAKE_CAPABILITIES,
            install_kind: InstallKind::Whisper,
            warm_on_startup: false,
            retry_unfiltered_on_empty: false,
            platform: PlatformRequirement::Mac,
        },
    ];

    struct FakeBackend {
        active: Arc<AtomicUsize>,
        maximum: Arc<AtomicUsize>,
        loaded: bool,
        load_error: Option<&'static str>,
    }

    impl TranscriptionBackend for FakeBackend {
        fn name(&self) -> &str {
            "whisper"
        }
        fn load_model(&mut self, _model_name: &str) -> Result<(), String> {
            let now = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.maximum.fetch_max(now, Ordering::SeqCst);
            thread::sleep(Duration::from_millis(20));
            if let Some(error) = self.load_error {
                self.active.fetch_sub(1, Ordering::SeqCst);
                return Err(error.to_string());
            }
            self.loaded = true;
            self.active.fetch_sub(1, Ordering::SeqCst);
            Ok(())
        }
        fn is_model_loaded(&self, _model_name: &str) -> bool {
            self.loaded
        }
        fn transcribe(
            &mut self,
            _samples: &[f32],
            _language: &str,
            _initial_prompt: Option<&str>,
            _smart_punctuation: bool,
        ) -> Result<String, String> {
            Ok(String::new())
        }
        fn token_count(&self, _text: &str) -> Option<usize> {
            None
        }
        fn model_exists(&self) -> bool {
            true
        }
        fn models_dir(&self) -> Result<PathBuf, String> {
            Ok(std::env::temp_dir())
        }
        fn reset(&mut self) {
            self.loaded = false;
        }
    }

    fn fake_manager(
        active: Arc<AtomicUsize>,
        maximum: Arc<AtomicUsize>,
        load_error: Option<&'static str>,
    ) -> ModelRuntimeManager {
        ModelRuntimeManager {
            definitions: FAKE_DEFINITIONS,
            inner: Mutex::new(RuntimeInner {
                backend: Box::new(FakeBackend {
                    active,
                    maximum,
                    loaded: false,
                    load_error,
                }),
                active_model: None,
            }),
            ..ModelRuntimeManager::default()
        }
    }

    #[test]
    fn native_startup_preparation_is_reused_by_frontend_selection() {
        let manager = fake_manager(
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicUsize::new(0)),
            None,
        );
        assert!(manager
            .prepare_startup_if(None, "fake-translation", || {
                assert!(
                    manager.inner.try_lock_or_recover().is_none(),
                    "ownership must be rechecked under the backend lock"
                );
                false
            })
            .unwrap()
            .is_none());
        assert!(manager.inner.lock_or_recover().active_model.is_none());
        let report = manager
            .prepare_startup_if(None, "fake-translation", || true)
            .unwrap()
            .unwrap();
        assert!(!report.cache_hit);
        manager.select_model(None, "fake-translation").unwrap();
        let confirmed = manager
            .prepare(None, "fake-translation", PreparationReason::StartupWarm)
            .unwrap();
        assert!(
            confirmed.cache_hit,
            "frontend confirmation must preserve the native warm model"
        );
        manager.select_model(None, "fake-second").unwrap();
        assert!(!manager
            .inner
            .lock_or_recover()
            .backend
            .is_model_loaded("fake-second"));
    }

    #[test]
    fn preview_never_loads_or_switches_models() {
        let manager = fake_manager(
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicUsize::new(0)),
            Some("must not load"),
        );
        assert_eq!(
            manager
                .try_with_ready_backend("fake-translation", || true, |_| panic!("must not decode"))
                .unwrap(),
            None::<()>
        );
        assert!(manager.inner.lock_or_recover().active_model.is_none());
        manager.inner.lock_or_recover().active_model = Some("other-model".into());
        assert_eq!(
            manager
                .try_with_ready_backend("fake-translation", || true, |_| panic!("must not decode"))
                .unwrap(),
            None::<()>
        );
    }

    #[test]
    fn preview_skips_busy_backend_and_rechecks_owner_under_lock() {
        let manager = Arc::new(fake_manager(
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicUsize::new(0)),
            None,
        ));
        manager
            .prepare(None, "fake-translation", PreparationReason::Recording)
            .unwrap();
        let lock = manager.inner.lock_or_recover();
        let worker = manager.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let join = thread::spawn(move || {
            tx.send(worker.try_with_ready_backend(
                "fake-translation",
                || panic!("busy must skip ownership/decode"),
                |_| Ok(()),
            ))
            .unwrap();
        });
        let result = rx.recv_timeout(Duration::from_secs(1));
        drop(lock); // Release before assertion, even on failure.
        assert_eq!(result.unwrap().unwrap(), None);
        join.join().unwrap();
        let current = std::sync::atomic::AtomicBool::new(true);
        assert!(current.load(Ordering::SeqCst)); // Worker was scheduled while recording.
        current.store(false, Ordering::SeqCst); // Stop before backend acquisition.
        let mut checked = false;
        let result = manager
            .try_with_ready_backend(
                "fake-translation",
                || {
                    assert!(
                        manager.inner.try_lock().is_err(),
                        "ownership must be checked with backend held"
                    );
                    checked = true;
                    current.load(Ordering::SeqCst)
                },
                |_| panic!("stopped recording must never start decode"),
            )
            .unwrap();
        assert!(checked);
        assert_eq!(result, None::<()>);
        assert_eq!(
            manager
                .try_with_ready_backend("fake-translation", || true, |_| Ok(7))
                .unwrap(),
            Some(7)
        );
    }

    #[test]
    fn preview_skips_status_contention_under_backend_and_allows_idle_unload() {
        let manager = Arc::new(fake_manager(
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicUsize::new(0)),
            None,
        ));
        manager
            .prepare(None, "fake-translation", PreparationReason::Recording)
            .unwrap();
        let state = Arc::new(crate::state::AppState::default());
        state.recording_id.store(41, Ordering::SeqCst);
        state.dictation.lock_or_recover().status = crate::state::DictationStatus::Recording;
        let mut status = state.dictation.lock_or_recover();
        // Model the idle unload side of the inversion: status is already held
        // while a previously scheduled preview acquires the backend.
        status.status = crate::state::DictationStatus::Idle;
        state.cancel_recording(41);
        let worker_manager = manager.clone();
        let worker_state = state.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = thread::spawn(move || {
            tx.send(worker_manager.try_with_ready_backend(
                "fake-translation",
                || {
                    assert!(
                        worker_manager.inner.try_lock_or_recover().is_none(),
                        "owner check must execute with backend held"
                    );
                    crate::commands::recording::dictation_partial_is_current_if_uncontended(
                        &worker_state,
                        41,
                    )
                },
                |_| panic!("contended ownership must never decode"),
            ))
            .unwrap();
        });
        let completed_while_status_held = rx.recv_timeout(Duration::from_secs(1));
        if completed_while_status_held.is_ok() {
            // Idle unload can now take the backend without releasing status.
            assert!(manager
                .unload(None, UnloadReason::IdleTimeout)
                .unwrap()
                .is_some());
        }
        drop(status); // Unstick a regressed blocking check before asserting.
        worker.join().unwrap();
        assert_eq!(completed_while_status_held.unwrap().unwrap(), None::<()>);
    }

    #[test]
    fn preview_recovers_poisoned_backend_repeatedly_without_clearing_poison() {
        let manager = fake_manager(
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicUsize::new(0)),
            None,
        );
        manager
            .prepare(None, "fake-translation", PreparationReason::Recording)
            .unwrap();
        let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _backend = manager.inner.lock_or_recover();
            panic!("intentional backend poison");
        }));
        assert!(poisoned.is_err() && manager.inner.is_poisoned());
        for _ in 0..2 {
            assert_eq!(
                manager
                    .try_with_ready_backend("fake-translation", || true, |_| Ok(7))
                    .unwrap(),
                Some(7)
            );
            assert!(manager.inner.is_poisoned());
        }
    }

    #[test]
    fn fake_backend_exposes_new_capabilities_without_pipeline_changes() {
        let manager = fake_manager(
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicUsize::new(0)),
            None,
        );
        manager
            .prepare(None, "fake-translation", PreparationReason::Recording)
            .unwrap();

        let snapshot = manager.snapshot("fake-translation").unwrap();
        assert!(snapshot.capabilities.translation);
        assert!(snapshot.capabilities.timestamps);
        assert!(snapshot.capabilities.confidence);
        assert_eq!(snapshot.lifecycle_state, LifecycleState::Ready);
    }

    #[test]
    fn preparation_is_serialized() {
        let active = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        let manager = Arc::new(fake_manager(
            Arc::clone(&active),
            Arc::clone(&maximum),
            None,
        ));
        let threads = (0..4)
            .map(|_| {
                let manager = Arc::clone(&manager);
                thread::spawn(move || {
                    manager
                        .prepare(None, "fake-translation", PreparationReason::Recording)
                        .unwrap();
                })
            })
            .collect::<Vec<_>>();
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(maximum.load(Ordering::SeqCst), 1);
        assert_eq!(
            manager
                .snapshot("fake-translation")
                .unwrap()
                .lifecycle_state,
            LifecycleState::Ready
        );
    }

    #[test]
    fn preparation_switch_marks_the_previous_model_unloaded() {
        let manager = fake_manager(
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicUsize::new(0)),
            None,
        );
        manager
            .prepare(None, "fake-translation", PreparationReason::Recording)
            .unwrap();
        manager
            .prepare(None, "fake-second", PreparationReason::Recording)
            .unwrap();

        assert_eq!(
            manager
                .snapshot("fake-translation")
                .unwrap()
                .lifecycle_state,
            LifecycleState::Unloaded
        );
        assert_eq!(
            manager.snapshot("fake-second").unwrap().lifecycle_state,
            LifecycleState::Ready
        );
    }

    #[test]
    fn existing_install_is_reused_without_reentering_installing() {
        let manager = fake_manager(
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicUsize::new(0)),
            None,
        );

        assert!(!manager
            .begin_install_with_presence(None, "fake-translation", true)
            .unwrap());
        assert_eq!(
            manager.snapshot("fake-translation").unwrap().install_state,
            InstallState::Installed
        );
    }

    #[test]
    fn missing_install_enters_installing_state() {
        let manager = fake_manager(
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicUsize::new(0)),
            None,
        );

        assert!(manager
            .begin_install_with_presence(None, "fake-translation", false)
            .unwrap());
        assert_eq!(
            manager.snapshot("fake-translation").unwrap().install_state,
            InstallState::Installing
        );
    }

    #[test]
    fn memory_pressure_unload_is_explicit_and_does_not_fallback() {
        let manager = fake_manager(
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicUsize::new(0)),
            None,
        );
        manager
            .prepare(None, "fake-translation", PreparationReason::Recording)
            .unwrap();
        assert!(manager
            .unload(None, UnloadReason::ModelChanged)
            .unwrap()
            .is_some());
        assert!(manager
            .prepare(None, "unknown", PreparationReason::Recording)
            .is_err());
        assert_eq!(
            manager
                .snapshot("fake-translation")
                .unwrap()
                .lifecycle_state,
            LifecycleState::Unloaded
        );
    }

    #[test]
    fn load_failure_is_observable_without_exposing_raw_error_text() {
        const PRIVATE_ERROR: &str = "failed at /Users/private/project/transcript.wav";
        let manager = fake_manager(
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicUsize::new(0)),
            Some(PRIVATE_ERROR),
        );

        assert_eq!(
            manager
                .prepare(None, "fake-translation", PreparationReason::Recording)
                .unwrap_err(),
            PRIVATE_ERROR
        );
        let snapshot = manager.snapshot("fake-translation").unwrap();
        assert_eq!(snapshot.lifecycle_state, LifecycleState::Failed);
        assert!(snapshot.failure_present);
        assert!(!serde_json::to_string(&snapshot)
            .unwrap()
            .contains(PRIVATE_ERROR));
    }

    #[test]
    fn runtime_event_shape_contains_only_bounded_metadata() {
        let snapshot = ModelRuntimeManager::default().snapshot("base.en").unwrap();
        let json = serde_json::to_string(&snapshot).unwrap();
        assert!(!json.contains("transcript"));
        assert!(!json.contains("path"));
        assert!(!json.contains("error"));
        assert!(json.contains("failurePresent"));
    }

    #[test]
    fn start_blocker_refuses_only_a_model_that_cannot_run() {
        use InstallState::*;
        assert_eq!(start_blocker(true, Installed, false), None);
        // A stale partial-cache signal never blocks an installed model.
        assert_eq!(start_blocker(true, Installed, true), None);
        assert_eq!(
            start_blocker(true, NotInstalled, false),
            Some(ModelUnavailable::NotInstalled)
        );
        assert_eq!(
            start_blocker(true, NotInstalled, true),
            Some(ModelUnavailable::Incomplete)
        );
        assert_eq!(
            start_blocker(true, Invalid, false),
            Some(ModelUnavailable::Incomplete)
        );
        assert_eq!(
            start_blocker(true, Installing, false),
            Some(ModelUnavailable::Downloading)
        );
        assert_eq!(
            start_blocker(true, Validating, false),
            Some(ModelUnavailable::Downloading)
        );
        assert_eq!(
            start_blocker(false, Installed, false),
            Some(ModelUnavailable::Unsupported)
        );
    }

    #[test]
    fn failure_reason_prefers_disk_state_then_the_error_kind() {
        use InstallState::*;
        assert_eq!(
            failure_reason(true, NotInstalled, true, "anything", false),
            Some(ModelUnavailable::Incomplete)
        );
        assert_eq!(
            failure_reason(
                true,
                Installed,
                false,
                "Failed to load Core ML model: boom",
                false
            ),
            Some(ModelUnavailable::LoadFailed)
        );
        assert_eq!(
            failure_reason(true, Installed, false, "model file not found", false),
            Some(ModelUnavailable::NotInstalled)
        );
        // An error of some other kind on a healthy model is not a model problem.
        assert_eq!(
            failure_reason(true, Installed, false, "decode exploded", false),
            None
        );
        // An error out of the decode closure never is, whatever its text says,
        // even when the disk state also looks wrong.
        for error in ["audio file not found", "Failed to load samples"] {
            assert_eq!(failure_reason(true, Installed, false, error, true), None);
            assert_eq!(failure_reason(true, NotInstalled, true, error, true), None);
        }
    }

    #[test]
    fn model_unavailable_wire_names_are_stable() {
        let all = [
            (ModelUnavailable::NotInstalled, "notInstalled"),
            (ModelUnavailable::Incomplete, "incomplete"),
            (ModelUnavailable::Downloading, "downloading"),
            (ModelUnavailable::Unsupported, "unsupported"),
            (ModelUnavailable::LoadFailed, "loadFailed"),
        ];
        for (reason, name) in all {
            assert_eq!(reason.as_str(), name);
        }
    }

    #[test]
    fn prepare_failure_kind_covers_the_real_missing_model_errors() {
        let whisper = crate::transcriber::whisper::get_model_path("no-such-model").unwrap_err();
        assert_eq!(prepare_failure_kind(&whisper), "not_installed");
        let parakeet = crate::transcriber::parakeet::bundle_missing_error(
            "bundle",
            std::path::Path::new("/tmp/x"),
        );
        assert_eq!(prepare_failure_kind(&parakeet), "not_installed");
        assert_eq!(
            prepare_failure_kind(crate::transcriber::coreml::NOT_DOWNLOADED_MESSAGE),
            "not_installed"
        );
        assert_eq!(
            prepare_failure_kind("Failed to load whisper model: file not found"),
            "load_failed"
        );
        assert_eq!(
            prepare_failure_kind("Unknown Core ML model 'x'"),
            "unknown_model"
        );
        assert_eq!(
            prepare_failure_kind("Failed to load Core ML model: boom"),
            "load_failed"
        );
        assert_eq!(prepare_failure_kind("something else"), "other");
    }

    fn fail_warm_up(manager: &ModelRuntimeManager) {
        manager
            .set_lifecycle(None, "base.en", LifecycleState::Failed, true, "startupWarm")
            .unwrap();
        assert!(manager.snapshot("base.en").unwrap().failure_present);
    }

    #[test]
    fn completing_a_download_clears_a_failure_left_by_an_earlier_warm_up() {
        let manager = ModelRuntimeManager::default();
        fail_warm_up(&manager);
        manager
            .set_install_state(None, "base.en", InstallState::Validating)
            .unwrap();
        manager.complete_install(None, "base.en").unwrap();
        let snapshot = manager.snapshot("base.en").unwrap();
        assert!(!snapshot.failure_present);
        assert_eq!(snapshot.lifecycle_state, LifecycleState::Unloaded);
        assert_eq!(snapshot.install_state, InstallState::Installed);
    }

    #[test]
    fn already_installed_and_cancelled_transitions_keep_a_genuine_failure() {
        let manager = ModelRuntimeManager::default();
        fail_warm_up(&manager);
        // download_model retriggered while the files are already present.
        manager
            .begin_install_with_presence(None, "base.en", true)
            .unwrap();
        assert!(manager.snapshot("base.en").unwrap().failure_present);
        // The cancel path republishes Installed without completing a download.
        manager
            .set_install_state(None, "base.en", InstallState::Installed)
            .unwrap();
        assert!(manager.snapshot("base.en").unwrap().failure_present);
        // Completing with no download in flight is not a repair either.
        manager.complete_install(None, "base.en").unwrap();
        assert!(manager.snapshot("base.en").unwrap().failure_present);
    }

    #[test]
    fn installing_and_not_installed_do_not_clear_a_failure() {
        let manager = ModelRuntimeManager::default();
        fail_warm_up(&manager);
        for state in [InstallState::Installing, InstallState::NotInstalled] {
            manager.set_install_state(None, "base.en", state).unwrap();
            assert!(manager.snapshot("base.en").unwrap().failure_present);
        }
    }

    #[test]
    fn incomplete_cache_signal_is_never_set_for_installed_or_non_coreml_models() {
        let whisper = model_definition("base.en").unwrap();
        assert!(!cache_incomplete_signal(
            whisper,
            InstallState::NotInstalled
        ));
        assert!(!cache_incomplete_signal(whisper, InstallState::Invalid));
        let coreml = model_definition("parakeet-tdt-0.6b-v3-coreml").unwrap();
        assert!(!cache_incomplete_signal(coreml, InstallState::Installed));
    }
}
