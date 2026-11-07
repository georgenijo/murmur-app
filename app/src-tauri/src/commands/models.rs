use crate::events::EventsExt;
use crate::host::ServiceAccess;
use crate::model_download::{self, Cancellation};
use crate::model_runtime::{self, InstallKind, InstallState};
use crate::transcriber::{self, TranscriptionBackend};
use crate::vad;
use crate::{MutexExt, State};
use murmur_core::events::{EventKind, EventSink};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::LazyLock;

static VAD_INSTALL_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));
static MODEL_INSTALL_ATTEMPT: AtomicU64 = AtomicU64::new(0);

// Register ownership before waiting on any install lock. Cancellation targets
// a caller-generated identity, never whichever install happens to be current.
struct InstallOwner {
    cancel: std::sync::Arc<Cancellation>,
    finished: std::sync::atomic::AtomicBool,
    termination_confirmed: std::sync::atomic::AtomicBool,
    done: tokio::sync::Notify,
}
impl Default for InstallOwner {
    fn default() -> Self {
        Self {
            cancel: Default::default(),
            finished: false.into(),
            termination_confirmed: true.into(),
            done: Default::default(),
        }
    }
}
fn confirm_installer_cleanup() -> bool {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        crate::coreml_installer::clear_quarantined_installer()
    }
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    {
        true
    }
}
#[derive(Default)]
struct InstallOwners {
    active: std::collections::HashMap<String, (String, std::sync::Arc<InstallOwner>)>,
    early_cancels: std::collections::VecDeque<(String, String)>,
}
static INSTALL_OWNERS: LazyLock<std::sync::Mutex<InstallOwners>> =
    LazyLock::new(|| std::sync::Mutex::new(InstallOwners::default()));

struct InstallGuard {
    model: String,
    request: String,
    owner: std::sync::Arc<InstallOwner>,
}
impl InstallGuard {
    fn register(model: &str, request: String) -> Result<Self, String> {
        if request.is_empty() || request.len() > 128 {
            return Err("Invalid install request ID".to_string());
        }
        let mut owners = INSTALL_OWNERS.lock_or_recover();
        if let Some((_, previous)) = owners.active.get(model) {
            if previous.finished.load(Ordering::SeqCst)
                && !previous.termination_confirmed.load(Ordering::SeqCst)
            {
                if !confirm_installer_cleanup() {
                    return Err(
                        "Wait for installer cleanup, then retry Cancel or Download.".to_string()
                    );
                }
                owners.active.remove(model);
            }
        }
        if owners.active.contains_key(model) {
            return Err(
                "This model is already installing. Cancel it or wait for it to finish.".to_string(),
            );
        }
        let owner = std::sync::Arc::new(InstallOwner::default());
        if let Some(index) = owners
            .early_cancels
            .iter()
            .position(|key| key == &(model.to_string(), request.clone()))
        {
            owners.early_cancels.remove(index);
            owner.cancel.cancel();
        }
        owners
            .active
            .insert(model.to_string(), (request.clone(), owner.clone()));
        Ok(Self {
            model: model.to_string(),
            request,
            owner,
        })
    }
}
impl Drop for InstallGuard {
    fn drop(&mut self) {
        let mut owners = INSTALL_OWNERS.lock_or_recover();
        if self.owner.termination_confirmed.load(Ordering::SeqCst)
            && owners
                .active
                .get(&self.model)
                .is_some_and(|(request, _)| request == &self.request)
        {
            owners.active.remove(&self.model);
        }
        self.owner.finished.store(true, Ordering::SeqCst);
        self.owner.done.notify_waiters();
    }
}

#[cfg_attr(feature = "tauri-host", tauri::command)]
pub async fn cancel_model_download(model_name: String, request_id: String) -> Result<(), String> {
    model_runtime::model_definition(&model_name)?;
    if request_id.is_empty() || request_id.len() > 128 {
        return Err("Invalid install request ID".to_string());
    }
    let owner = {
        let mut owners = INSTALL_OWNERS.lock_or_recover();
        match owners.active.get(&model_name) {
            Some((request, owner)) if request == &request_id => Some(owner.clone()),
            _ => {
                // IPC cancellation can arrive before its start command is polled.
                // Bound these tombstones and never cancel a different owner.
                if owners.early_cancels.len() >= 128 {
                    owners.early_cancels.pop_front();
                }
                owners
                    .early_cancels
                    .push_back((model_name.clone(), request_id));
                None
            }
        }
    };
    if let Some(owner) = owner {
        owner.cancel.cancel();
        let done = owner.done.notified();
        tokio::pin!(done);
        done.as_mut().enable();
        if !owner.finished.load(Ordering::SeqCst) {
            done.await;
        }
        if !owner.termination_confirmed.load(Ordering::SeqCst) {
            if !confirm_installer_cleanup() {
                return Err("Installer termination is not yet confirmed. Retry Cancel before leaving setup.".to_string());
            }
            let mut owners = INSTALL_OWNERS.lock_or_recover();
            if owners
                .active
                .get(&model_name)
                .is_some_and(|(_, active)| std::sync::Arc::ptr_eq(active, &owner))
            {
                owner.termination_confirmed.store(true, Ordering::SeqCst);
                owners.active.remove(&model_name);
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct DownloadProgressContext<'a> {
    model_name: &'a str,
    attempt_id: u64,
    cancel: &'a Cancellation,
    request_id: &'a str,
}

#[derive(Clone, Copy)]
struct InstallTerminalEvidence {
    outcome_code: &'static str,
    repaired_cache: bool,
    repeated_repair: bool,
    termination_confirmed: bool,
}

impl Default for InstallTerminalEvidence {
    fn default() -> Self {
        Self {
            outcome_code: "install_failed",
            repaired_cache: false,
            repeated_repair: false,
            termination_confirmed: true,
        }
    }
}

fn install_kind_code(kind: InstallKind) -> &'static str {
    match kind {
        InstallKind::Coreml => "coreml",
        InstallKind::Parakeet => "parakeet",
        InstallKind::Whisper => "whisper",
    }
}

fn publish_download_progress(
    app_handle: &crate::host::Context,
    context: DownloadProgressContext<'_>,
    received: u64,
    total: u64,
    phase: &str,
    repeated_repair: bool,
) {
    if context.cancel.is_cancelled() {
        return;
    }
    let _ = app_handle.events().publish(
        EventKind::DownloadProgress,
        serde_json::json!({
            "modelName": context.model_name,
            "attemptId": context.attempt_id,
            "requestId": context.request_id,
            "received": received,
            "total": total,
            "phase": phase,
            "repeatedRepair": repeated_repair,
        }),
    );
}

fn log_install_terminal(
    attempt_id: u64,
    install_kind: &'static str,
    evidence: InstallTerminalEvidence,
) {
    tracing::info!(
        target: "system",
        event_code = "system.model_install_terminal",
        attempt_id,
        install_kind,
        outcome_code = evidence.outcome_code,
        repaired_cache = evidence.repaired_cache,
        repeated_repair = evidence.repeated_repair,
        termination_confirmed = evidence.termination_confirmed,
        "model install terminal"
    );
}

#[cfg(debug_assertions)]
fn debug_coreml_scenario_reports_success_without_cache() -> bool {
    matches!(
        std::env::var("MURMUR_COREML_INSTALL_SCENARIO").as_deref(),
        Ok("actual" | "success" | "hang_once_then_success")
    )
}

#[cfg(not(debug_assertions))]
fn debug_coreml_scenario_reports_success_without_cache() -> bool {
    false
}

#[cfg_attr(feature = "tauri-host", tauri::command)]
pub fn check_model_exists(state: crate::host::StateRef<'_, State>) -> bool {
    state.app_state.model_runtime.any_model_installed()
}

#[cfg_attr(feature = "tauri-host", tauri::command)]
pub fn get_model_runtime_catalog(
    state: crate::host::StateRef<'_, State>,
) -> Vec<model_runtime::ModelRuntimeSnapshot> {
    state.app_state.model_runtime.catalog()
}

#[cfg_attr(feature = "tauri-host", tauri::command)]
pub fn get_model_runtime_status(
    state: crate::host::StateRef<'_, State>,
    model_name: String,
) -> Result<model_runtime::ModelRuntimeSnapshot, String> {
    state.app_state.model_runtime.snapshot(&model_name)
}

#[cfg_attr(feature = "tauri-host", tauri::command)]
pub fn get_model_hardware_guidance() -> crate::model_hardware_guidance::ModelHardwareGuidance {
    crate::model_hardware_guidance::get()
}

#[cfg_attr(feature = "tauri-host", tauri::command)]
pub fn check_specific_model_exists(
    state: crate::host::StateRef<'_, State>,
    model_name: String,
) -> bool {
    if model_name == crate::diarization_model::MODEL_ID {
        return crate::diarization_model::installed();
    }
    if !is_safe_model_identifier(&model_name) {
        return false;
    }
    state
        .app_state
        .model_runtime
        .snapshot(&model_name)
        .is_ok_and(|snapshot| snapshot.install_state == InstallState::Installed)
}

fn is_safe_model_identifier(model_name: &str) -> bool {
    // Model identifiers are catalog keys, never paths supplied by callers.
    !model_name.contains("..") && !model_name.contains('/') && !model_name.contains('\\')
}

/// Destructive operations use only catalog-derived paths and never follow a
/// symlink in the artifact or its ancestors into another application's data.
fn remove_catalog_artifact(path: &std::path::Path, directory: bool) -> Result<(), String> {
    for ancestor in path.ancestors() {
        let metadata = std::fs::symlink_metadata(ancestor)
            .map_err(|error| format!("Could not inspect model artifact: {error}"))?;
        if metadata.file_type().is_symlink() {
            return Err("Model removal does not follow symbolic links".to_string());
        }
    }
    let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if directory != metadata.is_dir() || (!directory && !metadata.is_file()) {
        return Err("Unexpected model artifact type; nothing was removed".to_string());
    }
    let result = if directory {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
    result.map_err(|error| format!("Could not remove model: {error}"))
}

fn remove_whisper_artifact(
    model_name: &str,
    root: &std::path::Path,
    search_paths: &[std::path::PathBuf],
) -> Result<(), String> {
    if model_runtime::model_definition(model_name)?.install_kind != InstallKind::Whisper {
        return Err("Not a Whisper model".to_string());
    }
    let filename = format!("ggml-{model_name}.bin");
    // Legacy/environment search roots belong to the user or another app.
    // Refuse before deleting anything if another installed copy remains.
    for other in search_paths {
        if other != root
            && crate::model_artifact::binary_model_is_valid(
                &other.join(&filename),
                crate::model_artifact::MIN_WHISPER_MODEL_BYTES,
            )
        {
            return Err("This model has a copy outside Murmur's model storage. Remove that copy manually before using Remove here.".to_string());
        }
    }
    remove_catalog_artifact(&root.join(filename), false)
}

fn remove_installed_artifact(model_name: &str) -> Result<(), String> {
    let definition = model_runtime::model_definition(model_name)?;
    match definition.install_kind {
        InstallKind::Whisper => remove_whisper_artifact(
            model_name,
            &transcriber::WhisperBackend::new().models_dir()?,
            &transcriber::whisper::get_model_search_paths(),
        ),
        InstallKind::Parakeet => {
            remove_catalog_artifact(&transcriber::parakeet::model_directory(model_name)?, true)
        }
        InstallKind::Coreml => {
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            {
                if !crate::coreml_installer::clear_quarantined_installer() {
                    return Err("Wait for the previous Core ML installer to exit before removing this model".to_string());
                }
                remove_catalog_artifact(
                    &transcriber::coreml::model_dir()
                        .ok_or("Could not locate the Core ML model cache")?,
                    true,
                )
            }
            #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
            Err("Core ML transcription requires Apple Silicon macOS".to_string())
        }
    }
}

#[cfg_attr(feature = "tauri-host", tauri::command)]
pub async fn remove_model(
    app_handle: crate::host::Context,
    state: crate::host::StateRef<'_, State>,
    model_name: String,
) -> Result<(), String> {
    if !is_safe_model_identifier(&model_name) {
        return Err("Invalid model identifier".to_string());
    }
    model_runtime::model_definition(&model_name)?;
    // Same admission lock as live capture, files, and meetings. Hold selection
    // and runtime ownership until disk removal and the snapshot are committed.
    let _transition = state
        .app_state
        .recording_transition
        .try_lock()
        .map_err(|_| "Wait for the current recording transition to finish")?;
    let dictation = state
        .app_state
        .dictation
        .try_lock_or_recover()
        .ok_or("Wait for the current dictation operation to finish")?;
    if dictation.status != crate::state::DictationStatus::Idle
        || state.app_state.file_transcribing.load(Ordering::SeqCst)
        || state.app_state.meeting_blocks_asr()
        || crate::audio_lifecycle::is_audio_active()
    {
        return Err(
            "Stop recording and wait for transcription to finish before removing a model"
                .to_string(),
        );
    }
    if !state.benchmark.try_start_shared_backend_change() {
        return Err(
            "Wait for the current model operation to finish before removing a model".to_string(),
        );
    }
    struct ChangeGuard<'a>(&'a crate::benchmark::BenchmarkCoordinator);
    impl Drop for ChangeGuard<'_> {
        fn drop(&mut self) {
            self.0.finish();
        }
    }
    let _change = ChangeGuard(&state.benchmark);
    state.app_state.model_runtime.remove_model_artifact(
        Some(&app_handle),
        &model_name,
        &dictation.model_name,
        || remove_installed_artifact(&model_name),
    )
}

#[cfg_attr(feature = "tauri-host", tauri::command)]
pub async fn download_model(
    app_handle: crate::host::Context,
    state: crate::host::StateRef<'_, State>,
    model_name: String,
    request_id: Option<String>,
) -> Result<(), String> {
    if model_name == crate::diarization_model::MODEL_ID {
        return crate::diarization_model::download(&app_handle).await;
    }
    let definition = model_runtime::model_definition(&model_name)?;
    if !model_runtime::model_supported(definition) {
        return Err("This model is not supported on the current platform".to_string());
    }

    // The entire existence-check/download/install transaction is single-flight
    // per model. Different models may still download concurrently.
    let owner = InstallGuard::register(
        &model_name,
        request_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
    )?;
    let cancel = owner.owner.cancel.clone();
    cancel.check()?;
    let install_lock = state.app_state.model_runtime.install_lock(&model_name)?;
    let _install_guard = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Err(model_download::CANCELLED.to_string()),
        guard = install_lock.lock() => guard,
    };
    cancel.check()?;
    if !state
        .app_state
        .model_runtime
        .begin_install(Some(&app_handle), &model_name)?
    {
        return Ok(());
    }

    let attempt_id = MODEL_INSTALL_ATTEMPT.fetch_add(1, Ordering::SeqCst) + 1;
    let install_kind = install_kind_code(definition.install_kind);
    let progress_context = DownloadProgressContext {
        model_name: &model_name,
        attempt_id,
        cancel: &cancel,
        request_id: &owner.request,
    };
    tracing::info!(
        target: "system",
        event_code = "system.model_install_started",
        attempt_id,
        install_kind,
        "model install started"
    );
    let mut terminal_evidence = InstallTerminalEvidence::default();

    let install_result: Result<(), String> = async {
        // Whisper and sherpa share Murmur's models directory. FluidAudio owns a
        // separate Application Support cache, but VAD must still land here.
        // Keep setup inside this result boundary so every failure after the
        // Installing transition is published as Invalid rather than leaving a
        // permanently in-progress snapshot.
        let models_dir = transcriber::WhisperBackend::new().models_dir()?;
        tokio::fs::create_dir_all(&models_dir)
            .await
            .map_err(|e| format!("Failed to create models directory: {}", e))?;

        let storage = if definition.install_kind == InstallKind::Coreml {
            crate::identity::data_dir().ok_or("Could not locate Core ML storage")?.join("FluidAudio/Models")
        } else { models_dir.clone() };
        let retained = match definition.install_kind {
            InstallKind::Whisper => std::fs::metadata(models_dir.join(format!("ggml-{model_name}.bin.tmp"))).map(|m| m.len()).unwrap_or(0),
            InstallKind::Parakeet => {
                let (_, bundle) = transcriber::parakeet::download_spec(&model_name).ok_or("Unknown Parakeet model")?;
                [format!("{bundle}.tar.bz2"), format!("{bundle}.tar.bz2.download"), format!("{bundle}.tar.bz2.tmp")]
                    .iter().filter_map(|name| std::fs::metadata(models_dir.join(name)).ok().map(|m| m.len())).max().unwrap_or(0).min(1_200_000_000)
            }
            InstallKind::Coreml => 0,
        };
        model_download::preflight(&storage, model_download::install_bytes(&model_name).saturating_sub(retained))?;
        if definition.install_kind == InstallKind::Coreml { model_download::preflight(&models_dir, model_download::RESERVE + 2_000_000)?; }
        cancel.check()?;
        if definition.install_kind == InstallKind::Coreml {
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            {
                let worker_model = model_name.clone();
                let progress_app = app_handle.clone();
                let worker_cancel = cancel.clone();
                let worker_request = owner.request.clone();
                let worker_result = tokio::task::spawn_blocking(move || {
                    crate::coreml_installer::install(
                        &worker_model,
                        crate::coreml_installer::configured_install_timeout(),
                        &worker_cancel,
                        |phase, repeated_repair| {
                            publish_download_progress(
                                &progress_app,
                                DownloadProgressContext {
                                    model_name: &worker_model,
                                    attempt_id,
                                    cancel: &worker_cancel,
                                    request_id: &worker_request,
                                },
                                0,
                                0,
                                phase.code(),
                                repeated_repair,
                            );
                            tracing::info!(
                                target: "system",
                                event_code = "system.model_install_phase",
                                attempt_id,
                                install_kind = "coreml",
                                phase = phase.code(),
                                repeated_repair,
                                "model install phase"
                            );
                        },
                    )
                })
                .await;
                match worker_result {
                    Ok(Ok(receipt)) => {
                        terminal_evidence.repaired_cache = receipt.repaired_cache;
                        terminal_evidence.repeated_repair = receipt.repeated_repair;
                    }
                    Ok(Err(failure)) => {
                        terminal_evidence = InstallTerminalEvidence {
                            outcome_code: failure.code,
                            repaired_cache: failure.repaired_cache,
                            repeated_repair: failure.repeated_repair,
                            termination_confirmed: failure.termination_confirmed,
                        };
                        return Err(failure.user_message());
                    }
                    Err(_) => {
                        terminal_evidence.outcome_code = "worker_task_failed";
                        return Err(
                            "Core ML installer supervision failed. Retry Core ML, or choose Whisper Base or the CPU Parakeet fallback."
                                .to_string(),
                        );
                    }
                }
            }
        } else if definition.install_kind == InstallKind::Parakeet {
            download_parakeet_model(
                &app_handle,
                &model_name,
                &models_dir,
                progress_context,
            )
            .await?;
        } else {
            download_whisper_model(&app_handle, &model_name, &models_dir, progress_context).await?;
        }

        cancel.check()?;
        state.app_state.model_runtime.set_install_state(
            Some(&app_handle),
            &model_name,
            InstallState::Validating,
        )?;
        if !(model_runtime::model_installed(&model_name)
            || definition.install_kind == InstallKind::Coreml
                && debug_coreml_scenario_reports_success_without_cache())
        {
            terminal_evidence.outcome_code = "validation_failed";
            return Err("Model installation completed but validation failed".to_string());
        }

        // Co-download VAD model alongside the transcription model (~1.8MB).
        // Its own lock prevents different model installs from sharing a temp file.
        if let Err(error) = ensure_vad_model_with_progress(&app_handle, Some(progress_context)).await
        {
            tracing::warn!(target: "system", "VAD model co-download failed (non-fatal): {}", error);
        }

        cancel.check()?;
        Ok(())
    }
    .await;

    match install_result {
        Ok(()) => {
            terminal_evidence.outcome_code = "success";
            let result = state.app_state.model_runtime.set_install_state(
                Some(&app_handle),
                &model_name,
                InstallState::Installed,
            );
            if result.is_err() {
                terminal_evidence.outcome_code = "state_publish_failed";
            }
            log_install_terminal(attempt_id, install_kind, terminal_evidence);
            result
        }
        Err(error) => {
            owner
                .owner
                .termination_confirmed
                .store(terminal_evidence.termination_confirmed, Ordering::SeqCst);
            if cancel.is_cancelled() && terminal_evidence.termination_confirmed {
                terminal_evidence.outcome_code = "cancelled";
            }
            let final_state = if cancel.is_cancelled() {
                if model_runtime::model_installed(&model_name) {
                    InstallState::Installed
                } else {
                    InstallState::NotInstalled
                }
            } else {
                InstallState::Invalid
            };
            let _ = state.app_state.model_runtime.set_install_state(
                Some(&app_handle),
                &model_name,
                final_state,
            );
            log_install_terminal(attempt_id, install_kind, terminal_evidence);
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_dir(label: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("murmur-{label}-{}-{nonce}", std::process::id()))
    }

    #[tokio::test]
    async fn owned_cancel_waits_for_cleanup_and_never_cancels_a_different_attempt() {
        let guard = InstallGuard::register("tiny.en", "cancel-owner-test".to_string()).unwrap();
        let token = guard.owner.cancel.clone();
        cancel_model_download("tiny.en".to_string(), "stale-request".to_string())
            .await
            .unwrap();
        assert!(!token.is_cancelled());
        assert!(InstallGuard::register("tiny.en", "duplicate".to_string()).is_err());
        let cancel = tokio::spawn(cancel_model_download(
            "tiny.en".to_string(),
            "cancel-owner-test".to_string(),
        ));
        tokio::time::timeout(std::time::Duration::from_secs(1), token.cancelled())
            .await
            .unwrap();
        assert!(
            !cancel.is_finished(),
            "navigation must wait for the owner to stop"
        );
        drop(guard);
        cancel.await.unwrap().unwrap();
        let replacement = InstallGuard::register("tiny.en", "replacement".to_string()).unwrap();
        assert!(!replacement.owner.cancel.is_cancelled());
    }

    #[tokio::test]
    async fn unconfirmed_termination_retains_exact_owner_until_cleanup_is_confirmed() {
        let model = transcriber::COREML_MODEL_NAME;
        let guard = InstallGuard::register(model, "unconfirmed-owner".to_string()).unwrap();
        guard
            .owner
            .termination_confirmed
            .store(false, Ordering::SeqCst);
        drop(guard);
        cancel_model_download(model.to_string(), "stale-cancel".to_string())
            .await
            .unwrap();
        assert!(INSTALL_OWNERS.lock_or_recover().active.contains_key(model));
        cancel_model_download(model.to_string(), "unconfirmed-owner".to_string())
            .await
            .unwrap();
        assert!(!INSTALL_OWNERS.lock_or_recover().active.contains_key(model));
    }

    #[tokio::test]
    async fn cancel_before_command_admission_is_not_lost() {
        cancel_model_download("base.en".to_string(), "early-request".to_string())
            .await
            .unwrap();
        let guard = InstallGuard::register("base.en", "early-request".to_string()).unwrap();
        assert_eq!(
            guard.owner.cancel.check(),
            Err(model_download::CANCELLED.to_string())
        );
    }

    #[test]
    fn cancelled_extraction_preserves_archive_and_does_not_publish() {
        let root = tempfile::tempdir().unwrap();
        let model = model_runtime::PARAKEET_CPU_MODEL;
        let (_, bundle) = transcriber::parakeet::download_spec(model).unwrap();
        let archive = root.path().join("archive.tar.bz2");
        write_parakeet_archive(&archive, &bundle, true);
        let token = Cancellation::default();
        token.cancel();
        let error =
            extract_parakeet_archive_cancellable(&archive, root.path(), model, &bundle, &token)
                .unwrap_err();
        assert_eq!(error.to_string(), model_download::CANCELLED);
        assert!(!should_discard_archive(&error));
        assert!(archive.exists());
        assert!(!root.path().join(&bundle).exists());
        assert!(!root.path().join(format!(".{bundle}.extracting")).exists());
        let mut reader = CancelReader {
            inner: std::io::Cursor::new(b"bytes"),
            cancel: &token,
        };
        assert!(std::io::Read::read(&mut reader, &mut [0; 3]).is_err());
        assert_eq!(reader.inner.position(), 0);
    }

    #[test]
    fn removal_deletes_only_the_exact_file_or_bundle() {
        let root = test_dir("remove-artifact");
        fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let whisper = root.join("ggml-tiny.en.bin");
        let vad = root.join("ggml-silero-v5.1.2.bin");
        fs::write(&whisper, b"model").unwrap();
        fs::write(&vad, b"preserve").unwrap();
        remove_catalog_artifact(&whisper, false).unwrap();
        assert!(!whisper.exists());
        assert!(vad.exists());
        for name in [
            "sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-fp16",
            "parakeet-tdt-0.6b-v3",
        ] {
            let bundle = root.join(name);
            fs::create_dir_all(bundle.join("nested")).unwrap();
            fs::write(bundle.join("nested/weights"), b"weights").unwrap();
            remove_catalog_artifact(&bundle, true).unwrap();
            assert!(!bundle.exists());
            assert!(vad.exists());
        }
        assert!(remove_catalog_artifact(&vad, true).is_err());
        assert!(vad.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn whisper_removal_preserves_external_copies_and_rejects_non_catalog_names() {
        let root = test_dir("remove-external");
        fs::create_dir_all(root.join("managed")).unwrap();
        fs::create_dir_all(root.join("external")).unwrap();
        let root = root.canonicalize().unwrap();
        let managed = root.join("managed");
        let external = root.join("external");
        let filename = "ggml-tiny.en.bin";
        for directory in [&managed, &external] {
            let file = fs::File::create(directory.join(filename)).unwrap();
            file.set_len(crate::model_artifact::MIN_WHISPER_MODEL_BYTES)
                .unwrap();
        }
        assert!(
            remove_whisper_artifact("tiny.en", &managed, std::slice::from_ref(&external))
                .unwrap_err()
                .contains("outside Murmur")
        );
        assert!(managed.join(filename).exists());
        assert!(external.join(filename).exists());
        for name in [
            "../tiny.en",
            "tiny.en/child",
            "unknown",
            model_runtime::PARAKEET_CPU_MODEL,
        ] {
            assert!(remove_whisper_artifact(name, &managed, &[]).is_err());
        }
        fs::remove_file(external.join(filename)).unwrap();
        remove_whisper_artifact("tiny.en", &managed, std::slice::from_ref(&managed)).unwrap();
        assert!(!managed.join(filename).exists());
        assert!(external.is_dir());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn removal_refuses_symlink_artifacts_and_symlink_ancestors() {
        use std::os::unix::fs::symlink;
        let root = test_dir("remove-symlink");
        fs::create_dir_all(root.join("real")).unwrap();
        let root = root.canonicalize().unwrap();
        let model = root.join("real/model.bin");
        fs::write(&model, b"preserve").unwrap();
        symlink(&model, root.join("model.bin")).unwrap();
        symlink(root.join("real"), root.join("alias")).unwrap();
        assert!(remove_catalog_artifact(&root.join("model.bin"), false).is_err());
        assert!(remove_catalog_artifact(&root.join("alias/model.bin"), false).is_err());
        assert!(remove_catalog_artifact(&root.join("alias"), true).is_err());
        assert_eq!(fs::read(&model).unwrap(), b"preserve");
        fs::remove_dir_all(root).unwrap();
    }

    fn write_parakeet_archive(archive_path: &std::path::Path, dir_name: &str, complete: bool) {
        let source_root = archive_path.with_extension("source");
        let bundle = source_root.join(dir_name);
        fs::create_dir_all(&bundle).unwrap();
        fs::write(bundle.join("encoder.fp16.onnx"), b"encoder").unwrap();
        fs::write(bundle.join("decoder.fp16.onnx"), b"decoder").unwrap();
        if complete {
            fs::write(bundle.join("joiner.fp16.onnx"), b"joiner").unwrap();
        }
        fs::write(bundle.join("tokens.txt"), b"tokens").unwrap();

        let file = fs::File::create(archive_path).unwrap();
        let encoder = bzip2::write::BzEncoder::new(file, bzip2::Compression::best());
        let mut archive = tar::Builder::new(encoder);
        archive.append_dir_all(dir_name, &bundle).unwrap();
        let encoder = archive.into_inner().unwrap();
        encoder.finish().unwrap();
        fs::remove_dir_all(source_root).unwrap();
    }

    #[test]
    fn specific_model_check_rejects_paths() {
        assert!(!is_safe_model_identifier("../base.en"));
        assert!(!is_safe_model_identifier("models/base.en"));
        assert!(!is_safe_model_identifier("models\\base.en"));
        assert!(is_safe_model_identifier("base.en"));
    }

    #[test]
    fn coreml_model_is_not_dispatched_as_sherpa_download() {
        assert!(transcriber::is_coreml_model(transcriber::COREML_MODEL_NAME));
        assert!(!transcriber::parakeet::is_parakeet_model(
            transcriber::COREML_MODEL_NAME
        ));
        assert_eq!(
            model_runtime::model_definition(transcriber::COREML_MODEL_NAME)
                .unwrap()
                .install_kind,
            InstallKind::Coreml
        );
    }

    #[test]
    fn parakeet_extraction_replaces_partial_bundle_only_after_validation() {
        let root = test_dir("parakeet-atomic-install");
        let models_dir = root.join("models");
        fs::create_dir_all(&models_dir).unwrap();
        let model_name = "parakeet-tdt-0.6b-v2-fp16";
        let (_, dir_name) = transcriber::parakeet::download_spec(model_name).unwrap();
        let partial = models_dir.join(&dir_name);
        fs::create_dir_all(&partial).unwrap();
        fs::write(partial.join("encoder.fp16.onnx"), b"partial").unwrap();
        let archive_path = root.join("model.tar.bz2");
        write_parakeet_archive(&archive_path, &dir_name, true);

        extract_parakeet_archive(&archive_path, &models_dir, model_name, &dir_name).unwrap();

        assert!(transcriber::parakeet::specific_model_exists_in(
            model_name,
            &models_dir
        ));
        assert_eq!(
            fs::read(partial.join("encoder.fp16.onnx")).unwrap(),
            b"encoder"
        );
        assert!(!models_dir.join(format!(".{dir_name}.extracting")).exists());
        assert!(
            archive_path.exists(),
            "caller owns archive cleanup after success"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn incomplete_parakeet_archive_never_publishes_a_model() {
        let root = test_dir("parakeet-incomplete-install");
        let models_dir = root.join("models");
        fs::create_dir_all(&models_dir).unwrap();
        let model_name = "parakeet-tdt-0.6b-v2-fp16";
        let (_, dir_name) = transcriber::parakeet::download_spec(model_name).unwrap();
        let archive_path = root.join("model.tar.bz2");
        write_parakeet_archive(&archive_path, &dir_name, false);

        let error = extract_parakeet_archive(&archive_path, &models_dir, model_name, &dir_name)
            .unwrap_err();

        assert!(
            matches!(error, ParakeetInstallError::InvalidArchive(_)),
            "unexpected classification: {error:?}"
        );
        assert!(should_discard_archive(&error));
        assert!(!models_dir.join(&dir_name).exists());
        assert!(!models_dir.join(format!(".{dir_name}.extracting")).exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn corrupt_parakeet_archive_is_marked_for_redownload() {
        let root = test_dir("parakeet-corrupt-archive");
        let models_dir = root.join("models");
        fs::create_dir_all(&models_dir).unwrap();
        let model_name = "parakeet-tdt-0.6b-v2-fp16";
        let (_, dir_name) = transcriber::parakeet::download_spec(model_name).unwrap();
        let archive_path = root.join("model.tar.bz2");
        fs::write(&archive_path, b"not a bzip2 archive").unwrap();

        let error = extract_parakeet_archive(&archive_path, &models_dir, model_name, &dir_name)
            .unwrap_err();

        assert!(
            matches!(error, ParakeetInstallError::InvalidArchive(_)),
            "unexpected classification: {error:?}"
        );
        assert!(should_discard_archive(&error));
        assert!(!models_dir.join(&dir_name).exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn transient_publish_failure_preserves_a_retryable_archive() {
        let root = test_dir("parakeet-transient-install");
        let models_dir = root.join("models");
        fs::create_dir_all(&models_dir).unwrap();
        let model_name = "parakeet-tdt-0.6b-v2-fp16";
        let (_, dir_name) = transcriber::parakeet::download_spec(model_name).unwrap();
        let archive_path = root.join("model.tar.bz2");
        write_parakeet_archive(&archive_path, &dir_name, true);
        fs::write(models_dir.join(&dir_name), b"blocks directory publication").unwrap();

        let error = extract_parakeet_archive(&archive_path, &models_dir, model_name, &dir_name)
            .unwrap_err();

        assert!(matches!(error, ParakeetInstallError::Installation(_)));
        assert!(!should_discard_archive(&error));
        assert!(archive_path.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn model_install_locks_are_keyed_and_reused() {
        let manager = model_runtime::ModelRuntimeManager::default();
        let first = manager.install_lock("base.en").unwrap();
        let same = manager.install_lock("base.en").unwrap();
        let different = manager.install_lock("tiny.en").unwrap();
        assert!(std::sync::Arc::ptr_eq(&first, &same));
        assert!(!std::sync::Arc::ptr_eq(&first, &different));
        assert!(manager.install_lock("unknown-model").is_err());
    }
}

/// Download a single whisper ggml .bin file from Hugging Face.
async fn download_whisper_model(
    app_handle: &crate::host::Context,
    model_name: &str,
    models_dir: &std::path::Path,
    progress_context: DownloadProgressContext<'_>,
) -> Result<(), String> {
    let filename = format!("ggml-{}.bin", model_name);
    let url = format!(
        "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}",
        filename
    );
    let dest_path = models_dir.join(&filename);
    let temp_path = models_dir.join(format!("{}.tmp", filename));

    let received = stream_download(app_handle, &url, &temp_path, Some(progress_context), 0).await?;
    if let Err(error) = crate::model_artifact::validate_binary_model(
        &temp_path,
        crate::model_artifact::MIN_WHISPER_MODEL_BYTES,
        "Downloaded Whisper model",
    ) {
        let _ = tokio::fs::remove_file(&temp_path).await;
        return Err(error);
    }

    progress_context.cancel.check()?;
    tokio::fs::rename(&temp_path, &dest_path)
        .await
        .map_err(|e| {
            let _ = std::fs::remove_file(&temp_path);
            format!("Failed to finalize download: {}", e)
        })?;

    let _ = tokio::fs::remove_file(model_download::resume_path(&temp_path)).await;
    tracing::info!(target: "system", "Model downloaded: {} ({} bytes)", filename, received);
    Ok(())
}

/// --- Parakeet backend (removable): delete this fn to remove. ---
/// Download a Parakeet model bundle (.tar.bz2) and extract it into the models dir.
async fn download_parakeet_model(
    app_handle: &crate::host::Context,
    model_name: &str,
    models_dir: &std::path::Path,
    progress_context: DownloadProgressContext<'_>,
) -> Result<(), String> {
    let cancel = install_cancellation(model_name)?;
    let (url, dir_name) = transcriber::parakeet::download_spec(model_name)
        .ok_or_else(|| format!("Unknown Parakeet model '{}'", model_name))?;
    if transcriber::parakeet::specific_model_exists_in(model_name, models_dir) {
        return Ok(());
    }

    let archive_path = models_dir.join(format!("{}.tar.bz2", dir_name));
    let legacy_temp_path = models_dir.join(format!("{}.tar.bz2.tmp", dir_name));

    // v0.16.0 could leave a complete `.tmp` archive when Murmur exited during
    // extraction. Try that expensive local data before downloading it again.
    if legacy_temp_path.is_file() {
        publish_installing(app_handle, progress_context);
        let archive = legacy_temp_path.clone();
        let root = models_dir.to_path_buf();
        let model = model_name.to_string();
        let bundle = dir_name.clone();
        match extract_parakeet_archive_on_worker(archive, root, model, bundle, cancel.clone()).await
        {
            Ok(()) => {
                let _ = tokio::fs::remove_file(&legacy_temp_path).await;
                let _ = tokio::fs::remove_file(&archive_path).await;
                tracing::info!(target: "system", "Recovered Parakeet installation from retained archive: {}", dir_name);
                return Ok(());
            }
            Err(error) if should_discard_archive(&error) => {
                tracing::warn!(target: "system", "Retained Parakeet archive was unusable; downloading again: {}", error);
                let _ = tokio::fs::remove_file(&legacy_temp_path).await;
            }
            Err(error) => return Err(error.to_string()),
        }
    }

    let mut downloaded_this_attempt = false;
    loop {
        // A finalized archive is retained after transient installation failures
        // so Retry performs only local work. Invalid contents are discarded and
        // downloaded once more in the same attempt.
        if !archive_path.is_file() {
            let download_path = models_dir.join(format!("{}.tar.bz2.download", dir_name));
            let received = stream_download(
                app_handle,
                &url,
                &download_path,
                Some(progress_context),
                model_download::PARAKEET_STAGING,
            )
            .await?;
            cancel.check()?;
            tokio::fs::rename(&download_path, &archive_path)
                .await
                .map_err(|e| {
                    let _ = std::fs::remove_file(&download_path);
                    format!("Failed to finalize Parakeet archive: {}", e)
                })?;
            let _ = tokio::fs::remove_file(model_download::resume_path(&download_path)).await;
            downloaded_this_attempt = true;
            tracing::info!(target: "system", "Parakeet archive downloaded: {} ({} bytes)", dir_name, received);
        }

        publish_installing(app_handle, progress_context);
        let result = extract_parakeet_archive_on_worker(
            archive_path.clone(),
            models_dir.to_path_buf(),
            model_name.to_string(),
            dir_name.clone(),
            cancel.clone(),
        )
        .await;

        match result {
            Ok(()) => {
                let _ = tokio::fs::remove_file(&archive_path).await;
                tracing::info!(target: "system", "Parakeet model installed: {}", dir_name);
                return Ok(());
            }
            Err(error) if should_discard_archive(&error) => {
                let _ = tokio::fs::remove_file(&archive_path).await;
                if downloaded_this_attempt {
                    return Err(error.to_string());
                }
                tracing::warn!(target: "system", "Retained Parakeet archive was invalid; downloading again: {}", error);
            }
            Err(error) => return Err(error.to_string()),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum ParakeetInstallError {
    InvalidArchive(String),
    Installation(String),
}

impl std::fmt::Display for ParakeetInstallError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidArchive(message) | Self::Installation(message) => {
                formatter.write_str(message)
            }
        }
    }
}

fn should_discard_archive(error: &ParakeetInstallError) -> bool {
    matches!(error, ParakeetInstallError::InvalidArchive(_))
}

async fn extract_parakeet_archive_on_worker(
    archive_path: std::path::PathBuf,
    models_dir: std::path::PathBuf,
    model_name: String,
    dir_name: String,
    cancel: std::sync::Arc<Cancellation>,
) -> Result<(), ParakeetInstallError> {
    tokio::task::spawn_blocking(move || {
        extract_parakeet_archive_cancellable(
            &archive_path,
            &models_dir,
            &model_name,
            &dir_name,
            &cancel,
        )
    })
    .await
    .map_err(|error| {
        ParakeetInstallError::Installation(format!("Extraction task failed: {error}"))
    })?
}

fn publish_installing(
    app_handle: &crate::host::Context,
    progress_context: DownloadProgressContext<'_>,
) {
    publish_download_progress(app_handle, progress_context, 0, 0, "installing", false);
}

#[cfg(test)]
fn extract_parakeet_archive(
    archive_path: &std::path::Path,
    models_dir: &std::path::Path,
    model_name: &str,
    dir_name: &str,
) -> Result<(), ParakeetInstallError> {
    extract_parakeet_archive_cancellable(
        archive_path,
        models_dir,
        model_name,
        dir_name,
        &Cancellation::default(),
    )
}

fn extract_parakeet_archive_cancellable(
    archive_path: &std::path::Path,
    models_dir: &std::path::Path,
    model_name: &str,
    dir_name: &str,
    cancel: &Cancellation,
) -> Result<(), ParakeetInstallError> {
    cancel.check().map_err(ParakeetInstallError::Installation)?;
    model_download::preflight(
        models_dir,
        model_download::PARAKEET_STAGING + model_download::RESERVE,
    )
    .map_err(ParakeetInstallError::Installation)?;
    let final_dir = models_dir.join(dir_name);
    let staging_root = models_dir.join(format!(".{}.extracting", dir_name));
    let staged_dir = staging_root.join(dir_name);

    let _ = std::fs::remove_dir_all(&staging_root);
    if final_dir.exists()
        && !transcriber::parakeet::specific_model_exists_in(model_name, models_dir)
    {
        std::fs::remove_dir_all(&final_dir).map_err(|e| {
            ParakeetInstallError::Installation(format!(
                "Failed to remove incomplete model bundle: {}",
                e
            ))
        })?;
    }
    std::fs::create_dir_all(&staging_root).map_err(|e| {
        ParakeetInstallError::Installation(format!(
            "Failed to create extraction staging directory: {}",
            e
        ))
    })?;

    let extraction = (|| {
        let file = std::fs::File::open(archive_path).map_err(|e| {
            ParakeetInstallError::Installation(format!("Failed to open archive: {}", e))
        })?;
        let decompressor = bzip2::read::BzDecoder::new(CancelReader {
            inner: file,
            cancel,
        });
        let mut archive = tar::Archive::new(CancelReader {
            inner: decompressor,
            cancel,
        });
        let mut expanded = 0_u64;
        for entry in archive.entries().map_err(classify_archive_unpack_error)? {
            cancel.check().map_err(ParakeetInstallError::Installation)?;
            let mut entry = entry.map_err(classify_archive_unpack_error)?;
            expanded = expanded.saturating_add(entry.size());
            if expanded > model_download::PARAKEET_STAGING {
                return Err(ParakeetInstallError::InvalidArchive(
                    "Parakeet archive exceeds its staging space budget".to_string(),
                ));
            }
            entry
                .unpack_in(&staging_root)
                .map_err(classify_archive_unpack_error)?;
        }
        cancel.check().map_err(ParakeetInstallError::Installation)?;
        if !transcriber::parakeet::specific_model_exists_in(model_name, &staging_root) {
            return Err(ParakeetInstallError::InvalidArchive(
                "Extracted Parakeet bundle is incomplete".to_string(),
            ));
        }
        cancel.check().map_err(ParakeetInstallError::Installation)?;
        std::fs::rename(&staged_dir, &final_dir).map_err(|e| {
            ParakeetInstallError::Installation(format!(
                "Failed to publish Parakeet model bundle: {}",
                e
            ))
        })?;
        Ok(())
    })();

    let _ = std::fs::remove_dir_all(&staging_root);
    // tar may wrap an interrupted read as an invalid-archive error. Cancellation
    // never makes a previously valid retained archive eligible for deletion.
    if cancel.is_cancelled() {
        return Err(ParakeetInstallError::Installation(
            model_download::CANCELLED.to_string(),
        ));
    }
    extraction
}

struct CancelReader<'a, R> {
    inner: R,
    cancel: &'a Cancellation,
}
impl<R: std::io::Read> std::io::Read for CancelReader<'_, R> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        self.cancel.check().map_err(std::io::Error::other)?;
        self.inner.read(bytes)
    }
}
fn install_cancellation(model: &str) -> Result<std::sync::Arc<Cancellation>, String> {
    INSTALL_OWNERS
        .lock_or_recover()
        .active
        .get(model)
        .map(|(_, owner)| owner.cancel.clone())
        .ok_or("Model install owner is no longer active".to_string())
}

fn classify_archive_unpack_error(error: std::io::Error) -> ParakeetInstallError {
    let message = error.to_string();
    let normalized = message.to_ascii_lowercase();
    if matches!(
        error.kind(),
        std::io::ErrorKind::InvalidData | std::io::ErrorKind::UnexpectedEof
    ) || error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<bzip2::Error>())
        .is_some_and(|error| matches!(error, bzip2::Error::Data | bzip2::Error::DataMagic))
        || normalized.contains("data integrity")
        || normalized.contains("corrupt")
        || normalized.contains("failed to iterate over archive")
    {
        ParakeetInstallError::InvalidArchive(format!("Invalid Parakeet archive: {message}"))
    } else {
        ParakeetInstallError::Installation(format!("Failed to extract archive: {message}"))
    }
}

/// Ensure the VAD model is present, downloading it if necessary.
/// This is the fallback for users who have a transcription model but not the
/// VAD model (e.g. upgrade from a pre-VAD version or manual model install).
pub(crate) async fn ensure_vad_model(app_handle: &crate::host::Context) -> Result<(), String> {
    ensure_vad_model_with_progress(app_handle, None).await
}

async fn ensure_vad_model_with_progress(
    app_handle: &crate::host::Context,
    progress_context: Option<DownloadProgressContext<'_>>,
) -> Result<(), String> {
    let fallback = Cancellation::default();
    let cancel = progress_context
        .map(|context| context.cancel)
        .unwrap_or(&fallback);
    let _install_guard = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Err(model_download::CANCELLED.to_string()),
        guard = VAD_INSTALL_LOCK.lock() => guard,
    };
    cancel.check()?;
    let context = app_handle.context();
    let model_path = crate::model_artifact::vad_model_path(&context, vad::VAD_MODEL_FILENAME)
        .ok_or_else(|| "Could not determine VAD model path".to_string())?;
    if crate::model_artifact::binary_model_is_valid(
        &model_path,
        crate::model_artifact::MIN_VAD_MODEL_BYTES,
    ) {
        return Ok(());
    }

    let models_dir = model_path
        .parent()
        .ok_or_else(|| "Could not determine models directory".to_string())?;

    tokio::fs::create_dir_all(models_dir)
        .await
        .map_err(|e| format!("Failed to create models directory: {}", e))?;

    tracing::info!(target: "system", "VAD model not found, downloading...");

    let temp_path = models_dir.join(format!("{}.tmp", vad::VAD_MODEL_FILENAME));
    let received = stream_download(
        app_handle,
        vad::VAD_MODEL_URL,
        &temp_path,
        progress_context,
        0,
    )
    .await?;
    if let Err(error) = crate::model_artifact::validate_binary_model_on_worker(
        &context,
        temp_path.clone(),
        crate::model_artifact::MIN_VAD_MODEL_BYTES,
        "Downloaded VAD model",
    )
    .await
    {
        let _ = tokio::fs::remove_file(&temp_path).await;
        return Err(error);
    }

    cancel.check()?;
    tokio::fs::rename(&temp_path, &model_path)
        .await
        .map_err(|e| {
            let _ = std::fs::remove_file(&temp_path);
            format!("Failed to finalize VAD model download: {}", e)
        })?;

    let _ = tokio::fs::remove_file(model_download::resume_path(&temp_path)).await;
    tracing::info!(target: "system", "VAD model downloaded: {} ({} bytes)", vad::VAD_MODEL_FILENAME, received);
    Ok(())
}

/// Stream a file download with progress events. Returns total bytes received.
async fn stream_download(
    app_handle: &crate::host::Context,
    url: &str,
    dest: &std::path::Path,
    progress_context: Option<DownloadProgressContext<'_>>,
    staging_bytes: u64,
) -> Result<u64, String> {
    let fallback = Cancellation::default();
    let cancel = progress_context
        .map(|context| context.cancel)
        .unwrap_or(&fallback);
    model_download::transfer(
        url,
        dest,
        cancel,
        model_download::IDLE_TIMEOUT,
        staging_bytes,
        |received, total| {
            if let Some(context) = progress_context {
                if !cancel.is_cancelled() {
                    publish_download_progress(
                        app_handle,
                        context,
                        received,
                        total,
                        "downloading",
                        false,
                    );
                }
            }
        },
    )
    .await
}
