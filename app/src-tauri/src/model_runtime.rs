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
    Desktop,
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
    // #279 removed Murmur's current partial-result consumer. Whisper still
    // reports the backend fact so a later product can choose a new contract.
    partial_results: true,
    initial_prompts: true,
    multilingual: false,
    translation: false,
    timestamps: false,
    confidence: false,
    punctuation_control: true,
};

const WHISPER_MULTILINGUAL_CAPABILITIES: ModelCapabilities = ModelCapabilities {
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
    partial_results: false,
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
        platform: PlatformRequirement::Desktop,
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
        platform: PlatformRequirement::Desktop,
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
        platform: PlatformRequirement::Desktop,
    },
    ModelDefinition {
        model_name: "small.en",
        label: "Whisper Small (English)",
        size: "~500 MB",
        backend: BackendKind::Whisper,
        accelerator: "Metal GPU",
        capabilities: WHISPER_EN_CAPABILITIES,
        install_kind: InstallKind::Whisper,
        warm_on_startup: false,
        retry_unfiltered_on_empty: false,
        platform: PlatformRequirement::Desktop,
    },
    ModelDefinition {
        model_name: "medium.en",
        label: "Whisper Medium (English)",
        size: "~1.5 GB",
        backend: BackendKind::Whisper,
        accelerator: "Metal GPU",
        capabilities: WHISPER_EN_CAPABILITIES,
        install_kind: InstallKind::Whisper,
        warm_on_startup: false,
        retry_unfiltered_on_empty: false,
        platform: PlatformRequirement::Desktop,
    },
    ModelDefinition {
        model_name: "large-v3-turbo",
        label: "Whisper Large Turbo",
        size: "~3 GB",
        backend: BackendKind::Whisper,
        accelerator: "Metal GPU",
        capabilities: WHISPER_MULTILINGUAL_CAPABILITIES,
        install_kind: InstallKind::Whisper,
        warm_on_startup: false,
        retry_unfiltered_on_empty: false,
        platform: PlatformRequirement::Desktop,
    },
];

pub fn model_definition(model_name: &str) -> Result<&'static ModelDefinition, String> {
    MODEL_DEFINITIONS
        .iter()
        .find(|definition| definition.model_name == model_name)
        .ok_or_else(|| format!("Unknown transcription model '{model_name}'"))
}

pub fn model_supported(definition: &ModelDefinition) -> bool {
    match definition.platform {
        PlatformRequirement::Desktop => true,
        PlatformRequirement::AppleSiliconMac => {
            cfg!(all(target_os = "macos", target_arch = "aarch64"))
        }
    }
}

pub fn model_accelerator(definition: &ModelDefinition) -> &'static str {
    if definition.backend == BackendKind::Whisper && !cfg!(target_os = "macos") {
        "GPU / CPU"
    } else {
        definition.accelerator
    }
}

fn supported_platforms(definition: &ModelDefinition) -> Vec<String> {
    match definition.platform {
        PlatformRequirement::Desktop => vec!["macos".to_string(), "linux".to_string()],
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
}

impl PreparationReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::Recording => "recording",
            Self::StartupWarm => "startupWarm",
            Self::Pipeline => "pipeline",
            Self::FileTranscription => "fileTranscription",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnloadReason {
    ModelChanged,
    IdleTimeout,
    #[allow(dead_code)]
    // Public lifecycle seam; automatic pressure policy is intentionally out of scope.
    MemoryPressure,
}

impl UnloadReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::ModelChanged => "modelChanged",
            Self::IdleTimeout => "idleTimeout",
            Self::MemoryPressure => "memoryPressure",
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
            install_state: self.current_install_state(model_name),
            lifecycle_state: status.lifecycle,
            failure_present: status.failure_present,
        })
    }

    pub fn catalog(&self) -> Vec<ModelRuntimeSnapshot> {
        self.definitions
            .iter()
            .filter_map(|definition| self.snapshot(definition.model_name).ok())
            .collect()
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
        inner: &'a mut MutexGuard<'_, RuntimeInner>,
        model_name: &str,
    ) -> Result<&'a mut Box<dyn TranscriptionBackend>, String> {
        let definition = self.definition(model_name)?;
        if !model_supported(definition) {
            return Err("This model is not supported on the current platform".to_string());
        }
        if inner.backend.name() != definition.backend.as_str() {
            inner.backend.reset();
            inner.backend = create_backend(model_name)?;
            inner.active_model = None;
        } else if inner
            .active_model
            .as_deref()
            .is_some_and(|active| active != model_name)
        {
            inner.backend.reset();
            inner.active_model = None;
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
        let backend = self.ensure_backend(inner, model_name)?;
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
        let lock_started = std::time::Instant::now();
        let mut inner = self.inner.lock_or_recover();
        let lock_wait_ms = lock_started.elapsed().as_millis() as u64;
        let mut report = self.ensure_loaded(app, &mut inner, model_name, reason)?;
        report.lock_wait_ms = lock_wait_ms;
        Ok(report)
    }

    pub fn with_ready_backend<T>(
        &self,
        app: Option<&tauri::AppHandle>,
        model_name: &str,
        reason: PreparationReason,
        operation: impl FnOnce(&mut dyn TranscriptionBackend) -> Result<T, String>,
    ) -> Result<(T, LoadReport), String> {
        let lock_started = std::time::Instant::now();
        let mut inner = self.inner.lock_or_recover();
        let lock_wait_ms = lock_started.elapsed().as_millis() as u64;
        let mut report = self.ensure_loaded(app, &mut inner, model_name, reason)?;
        report.lock_wait_ms = lock_wait_ms;
        let result = operation(inner.backend.as_mut())?;
        Ok((result, report))
    }

    pub fn select_model(
        &self,
        app: Option<&tauri::AppHandle>,
        model_name: &str,
    ) -> Result<(), String> {
        self.definition(model_name)?;
        let mut inner = self.inner.lock_or_recover();
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

    pub fn token_count(&self, text: &str) -> Option<usize> {
        self.inner.lock_or_recover().backend.token_count(text)
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

    const FAKE_CAPABILITIES: ModelCapabilities = ModelCapabilities {
        partial_results: false,
        initial_prompts: false,
        multilingual: false,
        translation: true,
        timestamps: true,
        confidence: true,
        punctuation_control: false,
    };

    static FAKE_DEFINITIONS: &[ModelDefinition] = &[ModelDefinition {
        model_name: "fake-translation",
        label: "Fake translation backend",
        size: "0 MB",
        backend: BackendKind::Whisper,
        accelerator: "Test",
        capabilities: FAKE_CAPABILITIES,
        install_kind: InstallKind::Whisper,
        warm_on_startup: false,
        retry_unfiltered_on_empty: false,
        platform: PlatformRequirement::Desktop,
    }];

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
            .unload(None, UnloadReason::MemoryPressure)
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
}
