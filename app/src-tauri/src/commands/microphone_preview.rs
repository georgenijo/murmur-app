use crate::audio_lifecycle::{self, AudioCancelReason, AudioLifecycleEvent};
use crate::microphone_auto::SmartAutoRequest;
use crate::microphone_preview::{MicrophonePreviewStatus, PreviewPhase};
use crate::state::DictationStatus;
use crate::{keyboard, MutexExt, State};
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};

const CONFIRMED_STOP_TIMEOUT: Duration = Duration::from_secs(15);
/// An unconfirmed Fn key-down capture lives at most its backstop TTL, then
/// tears down. A mic test start waits that long for it instead of killing it.
const KEY_DOWN_RESOLUTION_WAIT: Duration = Duration::from_millis(2_500);
const KEY_DOWN_RESOLUTION_POLL: Duration = Duration::from_millis(20);

fn require_main_window(window: &tauri::WebviewWindow) -> Result<(), String> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err("Microphone testing is available only from the main Settings window".to_string())
    }
}

fn emit_status(app_handle: &tauri::AppHandle) {
    let status = app_handle
        .state::<State>()
        .app_state
        .microphone_preview
        .status();
    let _ = app_handle.emit_to("main", "microphone-preview-status", status);
}

fn normalized_device_id(device_id: String) -> Result<Option<String>, String> {
    if device_id.len() > 4_096 {
        return Err("The selected microphone identifier is invalid".to_string());
    }
    if device_id == "system_default" {
        Ok(None)
    } else if device_id.is_empty() {
        Err("Choose a microphone before starting the test".to_string())
    } else {
        Ok(Some(device_id))
    }
}

pub(crate) fn handle_audio_lifecycle(
    app_handle: tauri::AppHandle,
    preview_id: u64,
    event: AudioLifecycleEvent,
) {
    let state = app_handle.state::<State>();
    let preview = &state.app_state.microphone_preview;
    let changed = match event {
        AudioLifecycleEvent::Accepted => false,
        AudioLifecycleEvent::StartupDiagnostic(_) => false,
        AudioLifecycleEvent::Ready => preview.set_phase_if(preview_id, PreviewPhase::Active),
        AudioLifecycleEvent::StillConnecting => preview.set_still_connecting_if(preview_id),
        AudioLifecycleEvent::Recovering { .. } => {
            preview.set_phase_if(preview_id, PreviewPhase::Stopping)
        }
        AudioLifecycleEvent::InitializationFailed { error, kind, .. } => {
            preview.set_error_if(preview_id, kind.as_str(), error)
        }
        AudioLifecycleEvent::CleanupFinished => false,
        AudioLifecycleEvent::RecoveryStalled => preview.set_error_if(
            preview_id,
            "stop_timeout",
            "Microphone cleanup is taking longer than expected. Wait for it to finish before trying again.",
        ),
        AudioLifecycleEvent::Interrupted { .. } => preview.set_error_if(
            preview_id,
            "stream_interrupted",
            "The microphone stopped unexpectedly. Check the selected input and try again.",
        ),
        AudioLifecycleEvent::Idle => preview.clear_if(preview_id),
    };
    if changed {
        emit_status(&app_handle);
        crate::smart_auto_probe::wake();
    }
}

#[tauri::command]
pub fn get_microphone_preview_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, State>,
) -> Result<MicrophonePreviewStatus, String> {
    require_main_window(&window)?;
    Ok(state.app_state.microphone_preview.status())
}

#[tauri::command]
pub fn get_smart_auto_microphone_status(
    smart_auto: SmartAutoRequest,
) -> crate::microphone_auto::SmartAutoStatus {
    crate::audio_inventory::smart_auto_status(&smart_auto)
}

#[tauri::command]
pub async fn verify_microphone_preview_signal(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, State>,
    preview_id: u64,
) -> Result<crate::microphone_signal::SignalVerificationResult, String> {
    require_main_window(&window)?;
    let preview = &state.app_state.microphone_preview;
    if preview.is_automatic(preview_id) {
        return Err("Automatic checks own their verification lifecycle.".into());
    }
    let deadline = preview.begin_signal_verification(preview_id, Instant::now())?;
    tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
    Ok(preview.finish_signal_verification(preview_id))
}

fn refuse_revoked_start(
    preview: &crate::microphone_preview::MicrophonePreviewState,
    start_generation: u64,
) -> Result<(), String> {
    if preview.start_revoked(start_generation) {
        Err(PREVIEW_DICTATION_BUSY.to_string())
    } else {
        Ok(())
    }
}

/// The frontend treats exactly this text as a quiet "paused for dictation".
pub(crate) const PREVIEW_DICTATION_BUSY: &str =
    "Finish the current dictation before testing the microphone";

pub(crate) const PREVIEW_AUDIO_BUSY: &str =
    "The microphone is still in use or recovering. Wait for Murmur to become ready.";

/// The microphone-owner check for a preview start, run under the recording
/// transition. An Fn key-down capture admitted after the command's first
/// retire is unconfirmed: it is no dictation, so the preview supersedes it
/// (its own teardown discards its audio) instead of reporting the microphone
/// busy. Any other owner still blocks the preview.
pub(crate) fn preview_audio_blocker(
    unconfirmed_key_down: impl Fn() -> bool,
    retire_key_down: impl FnOnce(),
    audio_active: impl Fn() -> bool,
) -> Option<&'static str> {
    if unconfirmed_key_down() {
        retire_key_down();
    }
    audio_active().then_some(PREVIEW_AUDIO_BUSY)
}

/// Take the recording transition once no unconfirmed Fn key-down capture holds
/// the microphone. That capture may be the first word of a dictation (it may
/// even have just taken over this test's device), so the test waits for its
/// gesture to resolve rather than discarding it. Key-down admission needs this
/// same lock, so none can be admitted while the guard is held. Bounded: past
/// the deadline the old supersession applies.
async fn transition_after_key_down_resolves<'a>(
    transition: &'a tokio::sync::Mutex<()>,
    unconfirmed_key_down: impl Fn() -> bool,
    deadline: Duration,
) -> tokio::sync::MutexGuard<'a, ()> {
    let deadline = tokio::time::Instant::now() + deadline;
    loop {
        let guard = transition.lock().await;
        if !unconfirmed_key_down() || tokio::time::Instant::now() >= deadline {
            return guard;
        }
        drop(guard);
        tokio::time::sleep(KEY_DOWN_RESOLUTION_POLL).await;
    }
}

#[tauri::command]
pub async fn start_microphone_preview(
    window: tauri::WebviewWindow,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, State>,
    device_id: String,
    smart_auto: Option<SmartAutoRequest>,
    vad_sensitivity: u32,
) -> Result<MicrophonePreviewStatus, String> {
    require_main_window(&window)?;
    // Captured before any wait: leaving or hiding Settings revokes this start.
    let start_generation = state.app_state.microphone_preview.start_generation();
    let device_id = normalized_device_id(device_id)?;
    crate::smart_auto_probe::preempt();
    if let Some(id) = state.app_state.microphone_preview.automatic_id() {
        stop_exact_preview(&app_handle, state.inner(), id).await?;
    }
    let transition = transition_after_key_down_resolves(
        &state.app_state.recording_transition,
        audio_lifecycle::is_unconfirmed_key_down_capture,
        KEY_DOWN_RESOLUTION_WAIT,
    )
    .await;
    // Standby is retired only now: no key-down capture is pending and none can
    // be admitted while the transition is held.
    crate::audio_lifecycle::retire_standby();
    {
        // Settings left or hid while this start waited (for example on a plain
        // Fn tap that took the test over): never open the microphone for a
        // test nobody is looking at. The quiet pause reason; reopening
        // Settings starts a fresh test.
        refuse_revoked_start(&state.app_state.microphone_preview, start_generation)?;
        let dictation = state.app_state.dictation.lock_or_recover();
        if dictation.status != DictationStatus::Idle {
            return Err(PREVIEW_DICTATION_BUSY.to_string());
        }
        if state.app_state.meeting_blocks_asr() {
            return Err(
                "Finish the current meeting transcript before testing the microphone".to_string(),
            );
        }

        if state.benchmark.is_running() {
            return Err("Finish the current benchmark before testing the microphone".to_string());
        }
        #[cfg(feature = "internal-benchmark")]
        if state.corpus.is_active() {
            return Err(
                "Finish the personal corpus recording before testing the microphone".to_string(),
            );
        }
        if keyboard::is_app_disabled() {
            return Err("Enable Murmur before testing the microphone".to_string());
        }
        if state.app_state.microphone_preview.is_active() {
            return Err("A microphone test is already active".to_string());
        }
        if let Some(busy) = preview_audio_blocker(
            audio_lifecycle::is_unconfirmed_key_down_capture,
            audio_lifecycle::retire_standby,
            audio_lifecycle::is_audio_active,
        ) {
            return Err(busy.to_string());
        }
    }
    let device_id = match smart_auto.as_ref() {
        Some(request) => {
            if device_id.is_some() {
                return Err("Smart Auto cannot be combined with a fixed microphone.".to_string());
            }
            Some(crate::audio_inventory::resolve_smart_auto_preview(request)?.device_id)
        }
        None => device_id,
    };
    let preview_id = state.app_state.microphone_preview.claim(vad_sensitivity)?;
    crate::audio::prepare::capture_claimed(crate::audio::prepare::CaptureClaim::Preview);
    state.app_state.microphone_preview.bind_signal_evidence(
        preview_id,
        crate::audio_inventory::signal_evidence_key(device_id.as_deref()),
    );
    tracing::info!(
        target: "audio",
        event_code = "audio.preview_started",
        preview_id,
        device_selection = if device_id.is_some() { "explicit" } else { "system_default" },
        "microphone preview requested"
    );
    emit_status(&app_handle);

    // The preview claim blocks competing starts. Do not hold the transition
    // mutex while waiting for the supervisor's acceptance channel.
    drop(transition);
    // Return the claimed generation immediately. Some Core Audio devices can
    // spend a long time inside stream setup (notably unavailable Continuity
    // inputs); keeping the command pending would leave Settings unable to send
    // an exact stop for that Connecting generation.
    let task_app_handle = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        let state = task_app_handle.state::<State>();
        let transition = state.app_state.recording_transition.lock().await;
        if !state.app_state.microphone_preview.is_connecting(preview_id) {
            if state.app_state.microphone_preview.clear_if(preview_id) {
                emit_status(&task_app_handle);
            }
            return;
        }
        let start_handle = task_app_handle.clone();
        let start_result = tokio::task::spawn_blocking(move || {
            audio_lifecycle::start_preview_recording(start_handle, device_id, preview_id)
        })
        .await;
        // Serialize acceptance with a competing stop. Once the supervisor has
        // accepted Preview ownership, exact teardown is authoritative.
        drop(transition);
        let failure = match start_result {
            Ok(Ok(())) => None,
            Ok(Err(error)) => Some(error.to_string()),
            Err(error) => Some(format!("Microphone test task failed: {error}")),
        };
        if let Some(message) = failure {
            if state.app_state.microphone_preview.fail_and_clear(
                preview_id,
                "start_failed",
                message,
            ) {
                emit_status(&task_app_handle);
            }
        }
    });
    Ok(state.app_state.microphone_preview.status())
}

#[tauri::command]
pub fn update_microphone_preview_vad_sensitivity(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, State>,
    preview_id: u64,
    vad_sensitivity: u32,
) -> Result<bool, String> {
    require_main_window(&window)?;
    Ok(state
        .app_state
        .microphone_preview
        .update_vad_sensitivity_if(preview_id, vad_sensitivity))
}

#[tauri::command]
pub async fn stop_microphone_preview(
    window: tauri::WebviewWindow,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, State>,
    preview_id: u64,
) -> Result<MicrophonePreviewStatus, String> {
    require_main_window(&window)?;
    stop_exact_preview(&app_handle, state.inner(), preview_id).await
}

pub(crate) async fn stop_exact_preview(
    app_handle: &tauri::AppHandle,
    state: &State,
    preview_id: u64,
) -> Result<MicrophonePreviewStatus, String> {
    let transition = state.app_state.recording_transition.lock().await;
    if !state.app_state.microphone_preview.is_current(preview_id) {
        return Ok(state.app_state.microphone_preview.status());
    }
    state
        .app_state
        .microphone_preview
        .set_phase_if(preview_id, PreviewPhase::Stopping);
    emit_status(app_handle);

    // Like dictation stop, release the short transition lock before any
    // supervisor wait. The active preview claim keeps racing starts blocked.
    drop(transition);
    let stop_result =
        tokio::task::spawn_blocking(move || audio_lifecycle::stop_preview_recording(preview_id))
            .await;
    if let Err(error) = stop_result {
        state.app_state.microphone_preview.set_error_if(
            preview_id,
            "stop_failed",
            format!("Microphone test stop task failed: {error}"),
        );
        emit_status(app_handle);
        return Err("The microphone test could not be stopped safely".to_string());
    }

    // Stop during Connecting only begins recovery and acknowledges promptly;
    // Stop during Stopping/Recovering may report an expected in-progress error.
    // In every case the Preview Idle lifecycle event is the teardown authority.
    if !state
        .app_state
        .microphone_preview
        .wait_until_inactive(preview_id, CONFIRMED_STOP_TIMEOUT)
        .await
    {
        state.app_state.microphone_preview.set_error_if(
            preview_id,
            "stop_timeout",
            "Microphone cleanup is taking longer than expected. Wait for it to finish before trying again.",
        );
        emit_status(app_handle);
        return Err(
            "Microphone cleanup is still in progress; a new input was not opened".to_string(),
        );
    }
    tracing::info!(
        target: "audio",
        event_code = "audio.preview_stopped",
        preview_id,
        "microphone preview stopped with confirmed worker teardown"
    );
    Ok(state.app_state.microphone_preview.status())
}

/// Acquire the shared pipeline transition after yielding any passive Settings
/// monitor. The loop closes both race directions: a preview that claims first
/// is stopped, while a real pipeline that receives the returned guard can
/// claim its state before another preview is allowed to start.
pub(crate) async fn transition_after_stopping_preview<'a>(
    app_handle: &tauri::AppHandle,
    state: &'a State,
) -> Result<tokio::sync::MutexGuard<'a, ()>, String> {
    transition_after_preview_cleanup(&state.app_state, false, |preview_id| async move {
        stop_exact_preview(app_handle, state, preview_id)
            .await
            .map(|_| ())
    })
    .await
}

/// The dictation start's transition. Like `transition_after_stopping_preview`,
/// except a flowing visible microphone test is left running: the supervisor
/// hands its open device to the dictation, so the start pays no teardown or
/// reopen and loses no audio. Background checks and a test that is still
/// connecting or already stopping are stopped first, as before. Holding the
/// returned guard keeps the test from stopping before the start claims it.
pub(crate) async fn transition_for_dictation_start<'a>(
    app_handle: &tauri::AppHandle,
    state: &'a State,
) -> Result<tokio::sync::MutexGuard<'a, ()>, String> {
    transition_after_preview_cleanup(&state.app_state, true, |preview_id| async move {
        stop_exact_preview(app_handle, state, preview_id)
            .await
            .map(|_| ())
    })
    .await
}

async fn transition_after_preview_cleanup<'a, Stop, Stopped>(
    app_state: &'a crate::state::AppState,
    keep_handoff_candidate: bool,
    mut stop: Stop,
) -> Result<tokio::sync::MutexGuard<'a, ()>, String>
where
    Stop: FnMut(u64) -> Stopped,
    Stopped: std::future::Future<Output = Result<(), String>>,
{
    crate::smart_auto_probe::preempt();
    loop {
        let transition = app_state.recording_transition.lock().await;
        let preview = &app_state.microphone_preview;
        let Some(preview_id) = preview
            .current_id()
            .filter(|&id| !(keep_handoff_candidate && preview.handoff_candidate() == Some(id)))
        else {
            crate::meeting_diarization::preempt()?;
            return Ok(transition);
        };
        drop(transition);
        stop(preview_id).await?;
    }
}

fn resolve_cancel_id(current: Option<u64>, requested: Option<u64>) -> Option<u64> {
    match (current, requested) {
        (Some(current), Some(requested)) if current == requested => Some(current),
        (Some(current), None) => Some(current),
        _ => None,
    }
}

async fn cancel_exact_preview(
    app_handle: tauri::AppHandle,
    requested_id: Option<u64>,
) -> Result<bool, String> {
    let state = app_handle.state::<State>();
    let transition = state.app_state.recording_transition.lock().await;
    let Some(preview_id) = resolve_cancel_id(
        state.app_state.microphone_preview.current_id(),
        requested_id,
    ) else {
        return Ok(false);
    };
    state
        .app_state
        .microphone_preview
        .set_phase_if(preview_id, PreviewPhase::Stopping);
    emit_status(&app_handle);
    drop(transition);
    tokio::task::spawn_blocking(move || {
        audio_lifecycle::cancel_preview_capture(preview_id, AudioCancelReason::User)
    })
    .await
    .map_err(|error| format!("Microphone test cancel task failed: {error}"))?
}

#[tauri::command]
pub async fn cancel_microphone_preview(
    window: tauri::WebviewWindow,
    app_handle: tauri::AppHandle,
    preview_id: Option<u64>,
) -> Result<bool, String> {
    require_main_window(&window)?;
    if preview_id.is_none() {
        // "Whatever is there": also a test start still waiting to claim.
        app_handle
            .state::<State>()
            .app_state
            .microphone_preview
            .revoke_pending_starts();
    }
    cancel_exact_preview(app_handle, preview_id).await
}

pub(crate) fn cancel_for_window_close(app_handle: tauri::AppHandle) {
    // Synchronously, before the window hides: a test start still waiting
    // (it has no preview to cancel yet) must not open the mic afterwards.
    app_handle
        .state::<State>()
        .app_state
        .microphone_preview
        .revoke_pending_starts();
    tauri::async_runtime::spawn(async move {
        if app_handle
            .state::<State>()
            .app_state
            .microphone_preview
            .automatic_id()
            .is_some()
        {
            return;
        }
        let _ = cancel_exact_preview(app_handle, None).await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn file_and_base64_processing_claims_wait_for_exact_preview_teardown() {
        use std::sync::{atomic::Ordering, Arc};
        for file in [true, false] {
            let app_state = Arc::new(crate::state::AppState::default());
            let id = app_state.microphone_preview.claim_automatic().unwrap();
            let stop_seen = Arc::new(tokio::sync::Notify::new());
            let teardown = Arc::new(tokio::sync::Notify::new());
            let owned = app_state.clone();
            let task_seen = stop_seen.clone();
            let task_teardown = teardown.clone();
            let task = tokio::spawn(async move {
                let _transition = transition_after_preview_cleanup(&owned, false, |actual| {
                    let owned = owned.clone();
                    let seen = task_seen.clone();
                    let teardown = task_teardown.clone();
                    async move {
                        assert_eq!(actual, id);
                        seen.notify_one();
                        teardown.notified().await;
                        owned.microphone_preview.clear_if(actual);
                        Ok(())
                    }
                })
                .await
                .unwrap();
                if file {
                    owned.file_transcribing.store(true, Ordering::SeqCst);
                } else {
                    owned.dictation.lock_or_recover().status = DictationStatus::Processing;
                }
            });
            stop_seen.notified().await;
            assert!(app_state.microphone_preview.is_current(id));
            assert!(!app_state.file_transcribing.load(Ordering::SeqCst));
            assert_eq!(
                app_state.dictation.lock_or_recover().status,
                DictationStatus::Idle
            );
            assert!(!task.is_finished());
            teardown.notify_one();
            task.await.unwrap();
            assert!(!app_state.microphone_preview.is_active());
            assert_eq!(app_state.file_transcribing.load(Ordering::SeqCst), file);
            assert_eq!(
                app_state.dictation.lock_or_recover().status,
                if file {
                    DictationStatus::Idle
                } else {
                    DictationStatus::Processing
                }
            );
        }
    }

    #[tokio::test]
    async fn failed_preview_teardown_does_not_release_a_processing_transition() {
        let app_state = crate::state::AppState::default();
        let id = app_state.microphone_preview.claim_automatic().unwrap();
        let result = transition_after_preview_cleanup(&app_state, false, |_| async {
            Err("still recovering".into())
        })
        .await;
        assert!(result.is_err());
        assert!(app_state.microphone_preview.is_current(id));
        assert_eq!(
            app_state.dictation.lock_or_recover().status,
            DictationStatus::Idle
        );
    }

    #[tokio::test]
    async fn dictation_start_keeps_only_a_flowing_visible_mic_test_for_handoff() {
        let app_state = crate::state::AppState::default();
        let preview = &app_state.microphone_preview;
        let id = preview.claim(50).unwrap();
        preview.set_phase_if(id, PreviewPhase::Active);
        // A dictation start leaves it running for the supervisor to adopt.
        let guard = transition_after_preview_cleanup(&app_state, true, |_| async {
            panic!("a flowing mic test is handed over, not stopped")
        })
        .await
        .unwrap();
        assert!(preview.is_current(id));
        drop(guard);

        // Every other real pipeline still stops it first.
        let stops = std::sync::atomic::AtomicUsize::new(0);
        let _guard = transition_after_preview_cleanup(&app_state, false, |actual| {
            stops.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            assert_eq!(actual, id);
            preview.clear_if(actual);
            async { Ok(()) }
        })
        .await
        .unwrap();
        assert_eq!(stops.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn dictation_start_still_stops_a_mic_test_it_cannot_take_over() {
        for (automatic, phase) in [
            (false, None),
            (false, Some(PreviewPhase::Stopping)),
            (true, Some(PreviewPhase::Active)),
        ] {
            let app_state = crate::state::AppState::default();
            let preview = &app_state.microphone_preview;
            let id = if automatic {
                preview.claim_automatic().unwrap()
            } else {
                preview.claim(50).unwrap()
            };
            if let Some(phase) = phase {
                preview.set_phase_if(id, phase);
            }
            let stops = std::sync::atomic::AtomicUsize::new(0);
            let _guard = transition_after_preview_cleanup(&app_state, true, |actual| {
                stops.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                preview.clear_if(actual);
                async { Ok(()) }
            })
            .await
            .unwrap();
            assert_eq!(
                stops.load(std::sync::atomic::Ordering::SeqCst),
                1,
                "{automatic} {phase:?}"
            );
            assert!(!preview.is_active());
        }
    }

    #[tokio::test]
    async fn mic_test_restart_waits_for_a_pending_key_down_gesture_instead_of_killing_it() {
        let transition = tokio::sync::Mutex::new(());
        let pending = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let observed = pending.clone();
        let started = tokio::time::Instant::now();
        let resolve = pending.clone();
        let resolver = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(60)).await;
            resolve.store(false, std::sync::atomic::Ordering::SeqCst);
        });
        let _guard = transition_after_key_down_resolves(
            &transition,
            || observed.load(std::sync::atomic::Ordering::SeqCst),
            Duration::from_secs(2),
        )
        .await;
        assert!(
            started.elapsed() >= Duration::from_millis(60),
            "waited for the gesture"
        );
        assert!(!pending.load(std::sync::atomic::Ordering::SeqCst));
        resolver.await.unwrap();
        drop(_guard);

        // The transition is free while waiting, so the gesture's own confirmed
        // start (or its discard) can take it.
        let waiter = {
            let transition = std::sync::Arc::new(tokio::sync::Mutex::new(()));
            let held = transition.clone();
            let task = tokio::spawn(async move {
                let _guard =
                    transition_after_key_down_resolves(&held, || true, Duration::from_millis(80))
                        .await;
            });
            tokio::time::sleep(Duration::from_millis(20)).await;
            assert!(transition.try_lock().is_ok(), "not held across the wait");
            task
        };
        // Bounded: past the backstop it proceeds and the old supersession applies.
        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn settings_closed_during_a_key_down_wait_never_opens_the_mic_test() {
        // Fn tap with Settings open: the takeover clears the test, Settings
        // restarts it, and that start waits for the tap to resolve. Settings
        // closes during the wait; there is no preview yet for the close to
        // cancel, so the waiting start itself must refuse afterwards.
        let preview =
            std::sync::Arc::new(crate::microphone_preview::MicrophonePreviewState::default());
        let transition = tokio::sync::Mutex::new(());
        let pending = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let start_generation = preview.start_generation();
        let closer = {
            let preview = preview.clone();
            let pending = pending.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(30)).await;
                preview.revoke_pending_starts(); // window close / Settings hidden
                tokio::time::sleep(Duration::from_millis(30)).await;
                pending.store(false, std::sync::atomic::Ordering::SeqCst); // tap discarded
            })
        };
        let observed = pending.clone();
        let _guard = transition_after_key_down_resolves(
            &transition,
            || observed.load(std::sync::atomic::Ordering::SeqCst),
            Duration::from_secs(2),
        )
        .await;
        closer.await.unwrap();
        assert_eq!(
            refuse_revoked_start(&preview, start_generation),
            Err(PREVIEW_DICTATION_BUSY.to_string()),
            "the quiet dictation pause, not an alert"
        );
        assert_eq!(preview.current_id(), None, "nothing claimed the microphone");

        // Reopening Settings starts afresh and is not affected.
        let reopened = preview.start_generation();
        assert_eq!(refuse_revoked_start(&preview, reopened), Ok(()));
    }

    #[test]
    fn a_start_that_kept_the_mic_test_waits_as_long_as_the_test_may_take_to_close() {
        // When the kept test cannot be taken over, the start queues behind the
        // test's confirmed exit. It must not give up sooner than the stop it
        // replaced (the ordinary 2 s start response would fail a slow close).
        assert!(audio_lifecycle::START_AFTER_PREVIEW_TIMEOUT > CONFIRMED_STOP_TIMEOUT);
    }

    #[test]
    fn optional_cleanup_never_becomes_an_ownerless_audio_cancel() {
        assert_eq!(resolve_cancel_id(None, None), None);
        assert_eq!(resolve_cancel_id(None, Some(1)), None);
        assert_eq!(resolve_cancel_id(Some(7), Some(6)), None);
        assert_eq!(resolve_cancel_id(Some(7), Some(7)), Some(7));
        assert_eq!(resolve_cancel_id(Some(7), None), Some(7));
    }

    #[test]
    fn missing_explicit_ids_are_not_rewritten_to_system_default() {
        assert_eq!(normalized_device_id("system_default".into()).unwrap(), None);
        assert_eq!(
            normalized_device_id("missing-stable-id".into()).unwrap(),
            Some("missing-stable-id".into())
        );
    }
}
