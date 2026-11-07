//! Tauri adapter for the window-independent core controller. No frontend listener
//! is on the control path. Only this host module knows commands/managed state.
use crate::events::EventsExt;
use crate::host::ServiceAccess;
use crate::MutexExt;
use murmur_core::dictation::{Controller, Effect, Input, Origin, QueueGuard, Status};
use murmur_core::events::{EventKind, EventSink};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::Instant,
};

struct Queued {
    #[cfg(all(feature = "native-host", not(feature = "tauri-host")))]
    native_active: Option<bool>,
    input: Input,
    generation: u64,
    at: Instant,
}
#[derive(Default)]
pub(crate) struct ConfigurationFence(Mutex<u64>);
impl ConfigurationFence {
    fn run<T>(&self, action: impl FnOnce() -> T) -> T {
        let _guard = self.0.lock_or_recover();
        action()
    }

    // Allocate intent before scheduling work, not when the snapshot is applied.
    pub(crate) fn accept(&self) -> u64 {
        let mut revision = self.0.lock_or_recover();
        *revision += 1;
        *revision
    }

    pub(crate) fn apply<T>(&self, revision: u64, action: impl FnOnce() -> T) -> Option<T> {
        let current = self.0.lock_or_recover();
        (*current == revision).then(action)
    }

    pub(crate) fn override_current<T>(&self, action: impl FnOnce() -> T) -> T {
        let mut revision = self.0.lock_or_recover();
        *revision += 1;
        action()
    }
}
pub(crate) struct Control {
    controller: Mutex<Controller>,
    configuration: ConfigurationFence,
    send: tokio::sync::mpsc::UnboundedSender<Queued>,
    epoch: Instant,
    setup_suspended: AtomicBool,
    pub(crate) settings: Mutex<serde_json::Value>,
}

impl Control {
    fn refresh_model_admission(
        &self,
        installed: impl FnOnce(&str) -> bool,
        listener: impl FnOnce(bool, &str, &str),
    ) {
        self.configuration.run(|| {
            let settings = self.settings.lock_or_recover();
            let Some(model) = settings["model"].as_str() else {
                return;
            };
            let mut controller = self.controller.lock_or_recover();
            controller.gate.model_installed = installed(model);
            listener(
                controller.gate.armed(),
                settings["doubleTapKey"].as_str().unwrap_or_default(),
                settings["recordingMode"].as_str().unwrap_or_default(),
            );
        });
    }
}

// Called synchronously by model installation/removal publication, before the
// command returns. Re-read runtime state under the configuration fence so an
// older event cannot restore admission after a newer removal or selection.
pub(crate) fn model_installation_changed(app: &crate::host::Context) {
    let Some(control) = app.try_control() else {
        return;
    };
    control.refresh_model_admission(
        |model| crate::commands::models::check_specific_model_exists(app.services(), model.into()),
        |armed, key, mode| {
            if armed {
                let _ = crate::commands::keyboard::start_keyboard_listener(
                    app.clone(),
                    key.into(),
                    mode.into(),
                );
            } else {
                crate::commands::keyboard::stop_keyboard_listener();
            }
        },
    );
}

pub(crate) fn initialize_event_subscription<R: crate::host::Runtime>(
    app: &crate::host::Context<R>,
    callback: impl Fn(&str) + Send + Sync + 'static,
) {
    crate::events::subscribe(
        app,
        murmur_core::events::Consumer::DictationControl,
        EventKind::RecordingStatusChanged,
        callback,
    );
}

pub(crate) fn initialize(app: &crate::host::Context) {
    let (sender, mut receive) = tokio::sync::mpsc::unbounded_channel();
    app.install_control(Control {
        controller: Mutex::new(Controller::default()),
        configuration: ConfigurationFence::default(),
        send: sender,
        epoch: Instant::now(),
        setup_suspended: AtomicBool::new(false),
        settings: Mutex::new(serde_json::Value::Null),
    });
    let sync_handle = app.clone();
    initialize_event_subscription(app, move |_| send(&sync_handle, Input::Sync));
    let handle = app.clone();
    let context = app.context().inner().clone();
    let _ = context.background.spawn(Box::pin(async move {
        let mut input_guard = QueueGuard::default();
        let mut last_locked = false;
        while let Some(queued) = receive.recv().await {
            let input = queued.input;
            let state = handle.services();
            let control = handle.control();
            let (generation, effect, locked) = {
                // Same lock order as accepted native starts: native state then
                // controller. No stale snapshot can roll a newer owner back.
                let dictation = state.app_state.dictation.lock_or_recover();
                let generation = state.app_state.recording_id.load(Ordering::SeqCst);
                #[cfg(all(feature = "native-host", not(feature = "tauri-host")))]
                if let Some(active) = queued.native_active {
                    let phase = status(dictation.status);
                    if generation != queued.generation
                        || if active {
                            !matches!(phase, Status::Starting | Status::Recording)
                        } else {
                            phase != Status::Idle
                        }
                    {
                        continue;
                    }
                }
                let Some(input) =
                    input_guard.route(queued.generation, generation, queued.at, input)
                else {
                    continue;
                };
                let mut controller = control.controller.lock_or_recover();
                controller.observe(generation, status(dictation.status), Origin::Toggle);
                let effect = controller.input(input);
                crate::keyboard::set_recording_state(controller.keyboard_recording());
                (generation, effect, controller.locked)
            };
            if locked != last_locked {
                let _ = handle
                    .events()
                    .publish(EventKind::DictationLockChanged, locked);
                last_locked = locked;
            }
            if let Some(effect) = effect {
                let start = matches!(effect, Effect::Start { .. });
                if let Some(accepted) = execute(&handle, effect).await {
                    input_guard.accepted(generation, accepted, Instant::now());
                } else if start
                    && control
                        .controller
                        .lock_or_recover()
                        .reject_start(generation)
                    && last_locked
                {
                    let _ = handle
                        .events()
                        .publish(EventKind::DictationLockChanged, false);
                    last_locked = false;
                }
            }
        }
    }));
    let permission_handle = app.clone();
    crate::host::runtime::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
        loop {
            interval.tick().await;
            let accessibility = crate::injector::is_accessibility_enabled();
            let changed = permission_handle
                .control()
                .controller
                .lock_or_recover()
                .gate
                .accessibility
                != accessibility;
            if changed {
                refresh(&permission_handle).await;
            }
        }
    });
}
fn status(value: crate::state::DictationStatus) -> Status {
    use crate::state::DictationStatus as S;
    match value {
        S::Idle => Status::Idle,
        S::Starting => Status::Starting,
        S::Recording => Status::Recording,
        S::Processing => Status::Processing,
        S::Recovering => Status::Recovering,
    }
}
pub(crate) fn send(app: &crate::host::Context, input: Input) {
    if let Some(control) = app.try_control() {
        let generation = app.services().app_state.recording_id.load(Ordering::SeqCst);
        let _ = control.send.send(Queued {
            #[cfg(all(feature = "native-host", not(feature = "tauri-host")))]
            native_active: None,
            input,
            generation,
            at: Instant::now(),
        });
    }
}
pub(crate) fn started(app: &crate::host::Context, generation: u64, origin: &str) {
    let control = app.control();
    control.controller.lock_or_recover().observe(
        generation,
        Status::Starting,
        if origin == "hold" {
            Origin::Hold
        } else {
            Origin::Toggle
        },
    );
    crate::keyboard::set_recording_state(origin != "hold");
}
pub(crate) fn level(app: &crate::host::Context, generation: u64, level: f32) {
    if let Some(control) = app.try_control() {
        let _ = control.send.send(Queued {
            #[cfg(all(feature = "native-host", not(feature = "tauri-host")))]
            native_active: None,
            generation,
            at: Instant::now(),
            input: Input::Level {
                generation,
                level: level as f64,
                at_ms: control.epoch.elapsed().as_millis() as u64,
            },
        });
    }
}
async fn execute(app: &crate::host::Context, effect: Effect) -> Option<u64> {
    use crate::commands::recording;
    match effect {
        Effect::Start {
            origin,
            gesture_token,
        } => {
            let settings = app.control().settings.lock_or_recover().clone();
            let smart_auto = if settings["smartAutoMicrophoneEnabled"] == true {
                let request = serde_json::from_value(serde_json::json!({
                    "approvedDeviceIds": settings["smartAutoApprovedDeviceIds"],
                    "preferredDeviceIds": settings["smartAutoPreferredDeviceIds"],
                    "allowContinuity": settings["smartAutoAllowContinuity"],
                    "requireRecentSignal": settings["smartAutoProbeEnabled"],
                }));
                match request {
                    Ok(request) => Some(request),
                    Err(_) => {
                        let _ = app.events().publish(
                            EventKind::DictationControlError,
                            "Check Smart Auto microphone settings.",
                        );
                        return None;
                    }
                }
            } else {
                None
            };
            let device = settings["microphone"]
                .as_str()
                .filter(|id| *id != "system_default" && !id.is_empty())
                .map(str::to_owned);
            let result = recording::start_native_recording(
                app.clone(),
                app.services(),
                if smart_auto.is_some() { None } else { device },
                smart_auto,
                Some(
                    if origin == Origin::Hold {
                        "hold"
                    } else {
                        "toggle"
                    }
                    .into(),
                ),
                gesture_token,
            )
            .await;
            match result {
                Ok(response) if response["type"] == "recording_starting" => {
                    return response["recordingId"].as_u64()
                }
                Ok(response) => {
                    let message = match response["type"].as_str() {
                        Some("busy_benchmarking") => "Wait for the benchmark to finish.",
                        Some("busy_transcribing_file") => "Wait for the file transcription to finish.",
                        Some("busy_meeting") => "Stop the meeting capture before starting dictation.",
                        Some("knowledge_initializing") => "Wait for the local knowledge rules to finish loading, or retry the knowledge store in Settings.",
                        Some("busy_transforming") => "Wait for the transform to finish.",
                        Some("busy_recording_corpus") => "Finish the corpus recording before starting dictation.",
                        Some("audio_recovering") => "Microphone cleanup is still in progress. Try again when Murmur is ready.",
                        _ => return None,
                    };
                    let _ = app
                        .events()
                        .publish(EventKind::DictationControlError, message);
                }
                Err(_) => {
                    let _ = app.events().publish(
                        EventKind::DictationControlError,
                        "Could not start dictation.",
                    );
                }
            }
        }
        Effect::Cancel { generation } => {
            let _ =
                recording::cancel_native_recording(app.clone(), app.services(), Some(generation))
                    .await;
        }
        Effect::Stop {
            generation,
            auto,
            hold,
        } => {
            // Transcription may take seconds. Keep receiving Escape and levels;
            // the existing pipeline rechecks this exact generation at every await.
            let handle = app.clone();
            let context = app.context().inner().clone();
            let _ = context.background.spawn(Box::pin(async move {
                let trigger = if auto {
                    crate::audio_lifecycle::DictationStopTrigger::AutoStop
                } else if hold {
                    crate::audio_lifecycle::DictationStopTrigger::HoldRelease
                } else {
                    crate::audio_lifecycle::DictationStopTrigger::Toggle
                };
                let result = recording::stop_native_recording_for(
                    handle.clone(),
                    handle.services(),
                    Some(generation),
                    trigger,
                )
                .await;
                present_stop_failure(
                    &handle.services().app_state,
                    generation,
                    &result,
                    |failure| {
                        let _ = handle
                            .events()
                            .publish(EventKind::DictationStopFailed, failure);
                    },
                );
            }));
        }
        Effect::Score { generation, pause } => {
            let handle = app.clone();
            let context = app.context().inner().clone();
            let _ = context.background.spawn(Box::pin(async move {
                let answer = crate::commands::smart_turn::check_smart_turn(handle.services()).await;
                let probability = match answer {
                    Ok(crate::commands::smart_turn::SmartTurnCheckResult::Scored(score))
                        if score.recording_id == generation =>
                    {
                        Some(score.probability as f64)
                    }
                    _ => None,
                };
                send(
                    &handle,
                    Input::SmartAnswer {
                        generation,
                        pause,
                        probability,
                    },
                );
            }));
        }
        Effect::PasteLast => crate::delivery_recovery::spawn_retry(app.clone()),
    }
    None
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StopFailure {
    recording_id: u64,
    error: &'static str,
}

fn present_stop_failure(
    state: &crate::state::AppState,
    generation: u64,
    result: &Result<serde_json::Value, String>,
    present: impl FnOnce(StopFailure),
) {
    if !match result {
        Err(_) => true,
        Ok(response) => response["type"] == "error",
    } {
        return;
    }
    // Snapshot under the same fence as cancellation/start. Dispatch after
    // unlocking; client generation/cancellation checks also reject a payload
    // if cancellation or a newer start wins after this snapshot.
    let failure = {
        let _dictation = state.dictation.lock_or_recover();
        (state.recording_id.load(Ordering::SeqCst) == generation && !state.is_cancelled(generation))
            .then_some(StopFailure {
                recording_id: generation,
                // Backend errors may contain private content. Publish fixed wording.
                error: "Transcription failed. Please try again.",
            })
    };
    if let Some(failure) = failure {
        present(failure);
    }
}

/// Settings migration remains 1.5. Accept only a complete v7 native-confirmed
/// snapshot; older/unknown versions wait for the existing frontend migration.
pub(crate) async fn configure(
    app: &crate::host::Context,
    settings: serde_json::Value,
) -> Result<(), String> {
    let admission = crate::shutdown::WriterPermit::acquire()?;
    let app = app.clone();
    crate::shutdown::source_call(move || {
        let _admission = admission;
        crate::commands::settings_store::configure_renderer_control_worker(app, settings)
    })
    .await
    .map_err(|_| "Source writer unavailable".to_owned())?
}

pub(crate) fn configuration_current(app: &crate::host::Context, revision: u64) -> bool {
    app.control().configuration.apply(revision, || ()).is_some()
}

pub(crate) fn accept_configuration(app: &crate::host::Context) -> u64 {
    app.control().configuration.accept()
}

pub(crate) fn configure_accepted(
    app: &crate::host::Context,
    settings: serde_json::Value,
    revision: u64,
) -> Result<bool, String> {
    app.control()
        .configuration
        .apply(revision, || configure_locked(app, settings))
        .transpose()
        .map(|applied| applied.is_some())
}
fn configure_locked(app: &crate::host::Context, settings: serde_json::Value) -> Result<(), String> {
    if settings["settingsVersion"] != 7 || !settings.is_object() {
        return Err("Unsupported control settings".into());
    }
    let key = settings["doubleTapKey"]
        .as_str()
        .ok_or("Missing dictation key")?;
    let mode = settings["recordingMode"]
        .as_str()
        .ok_or("Missing dictation mode")?;
    if !crate::keyboard::is_dictation_key_id(key)
        || !matches!(mode, "hold_down" | "double_tap" | "both")
    {
        return Err("Invalid dictation control configuration".into());
    }
    app.services()
        .app_state
        .dictation
        .lock_or_recover()
        .retain_history = settings["retainHistory"].as_bool().unwrap_or(true);
    let configured = settings["model"]
        .as_str()
        .is_some_and(|model| crate::model_runtime::model_definition(model).is_ok());
    let installed = settings["model"].as_str().is_some_and(|model| {
        crate::commands::models::check_specific_model_exists(app.services(), model.into())
    });
    let accessibility = crate::injector::is_accessibility_enabled();
    let control = app.control();
    let mut gate = murmur_core::dictation::LaunchGate {
        shutdown: crate::shutdown::closing(),
        configured: configured && crate::completion::admitted(app),
        onboarding_complete: settings["launchOnboardingComplete"]
            .as_bool()
            .unwrap_or(false),
        model_installed: installed,
        accessibility,
        disabled: settings["disabled"].as_bool().unwrap_or(true),
    };
    {
        let mut controller = control.controller.lock_or_recover();
        gate.onboarding_complete &= !control.setup_suspended.load(Ordering::SeqCst);
        controller.gate = gate;
        controller.silence_ms = settings["autoStopSilenceMs"]
            .as_u64()
            .filter(|ms| [0, 1500, 2500, 4000].contains(ms))
            .unwrap_or(0);
        controller.smart_enabled = settings["smartAutoStopEnabled"].as_bool().unwrap_or(false);
    }
    let policy = probe_policy(&settings);
    let previous_policy = probe_policy(&control.settings.lock_or_recover());
    if policy != previous_policy {
        crate::smart_auto_probe::configure_host_policy(policy)?;
    }
    *control.settings.lock_or_recover() = settings.clone();
    crate::commands::keyboard::apply_app_disabled(app.clone(), gate.disabled)?;
    if gate.armed() {
        crate::commands::keyboard::start_keyboard_listener(app.clone(), key.into(), mode.into())?;
    } else {
        crate::commands::keyboard::stop_keyboard_listener();
    }
    crate::commands::keyboard::set_paste_last_shortcut(
        app.clone(),
        settings["pasteLastShortcut"].as_str().map(str::to_owned),
    )?;
    Ok(())
}

#[cfg_attr(feature = "tauri-host", tauri::command)]
pub(crate) async fn configure_dictation_control(
    window: crate::host::Window,
    app: crate::host::Context,
    settings: serde_json::Value,
) -> Result<(), String> {
    if window.label() != "main" {
        return Err("Configure dictation from the main window.".into());
    }
    configure(&app, settings).await
}
#[cfg_attr(feature = "tauri-host", tauri::command)]
pub(crate) fn toggle_dictation_lock(app: crate::host::Context) {
    send(&app, Input::Lock);
}

pub(crate) async fn refresh(app: &crate::host::Context) {
    let control = app.control();
    control.configuration.run(|| {
        let settings = control.settings.lock_or_recover().clone();
        if !settings.is_null() {
            let _ = configure_locked(app, settings);
        }
    });
}

pub(crate) fn set_disabled(app: &crate::host::Context, disabled: bool) -> Result<(), String> {
    let Some(control) = app.try_control() else {
        return crate::commands::keyboard::apply_app_disabled(app.clone(), disabled);
    };
    control.configuration.override_current(|| {
        control.controller.lock_or_recover().gate.disabled = disabled;
        let (previous, policy) = {
            let mut settings = control.settings.lock_or_recover();
            let previous = probe_policy(&settings);
            if let Some(object) = settings.as_object_mut() {
                object.insert("disabled".into(), serde_json::json!(disabled));
            }
            (previous, probe_policy(&settings))
        };
        if policy != previous {
            let _ = crate::smart_auto_probe::configure_host_policy(policy);
        }
        let result = crate::commands::keyboard::apply_app_disabled(app.clone(), disabled);
        if disabled {
            if let Some(generation) = active_capture_generation(&app.services().app_state) {
                let handle = app.clone();
                crate::host::runtime::spawn(async move {
                    // A re-enable/new start may win before this continuation.
                    // Cancellation must remain owned by the captured generation.
                    let _ = crate::commands::recording::cancel_native_recording(
                        handle.clone(),
                        handle.services(),
                        Some(generation),
                    )
                    .await;
                });
            }
        }
        result
    })
}

fn active_capture_generation(state: &crate::state::AppState) -> Option<u64> {
    let dictation = state.dictation.lock_or_recover();
    matches!(
        dictation.status,
        crate::state::DictationStatus::Starting | crate::state::DictationStatus::Recording
    )
    .then(|| state.recording_id.load(Ordering::SeqCst))
}
pub(crate) fn admitted(app: &crate::host::Context) -> bool {
    if crate::shutdown::closing() {
        return false;
    }
    app.try_control()
        .is_none_or(|control| control.controller.lock_or_recover().gate.can_start())
}

fn probe_policy(settings: &serde_json::Value) -> serde_json::Value {
    if settings["disabled"] == false
        && settings["microphone"] == "system_default"
        && settings["smartAutoMicrophoneEnabled"] == true
        && settings["smartAutoProbeEnabled"] == true
    {
        serde_json::json!({"enabled":true,"request":{
            "approvedDeviceIds":settings["smartAutoApprovedDeviceIds"],
            "preferredDeviceIds":settings["smartAutoPreferredDeviceIds"],
            "allowContinuity":settings["smartAutoAllowContinuity"],
            "requireRecentSignal":true,
        }})
    } else {
        serde_json::json!({"enabled":false})
    }
}
#[cfg_attr(feature = "tauri-host", tauri::command)]
pub(crate) async fn suspend_dictation_for_setup(
    window: crate::host::Window,
    app: crate::host::Context,
) -> Result<bool, String> {
    if window.label() != "main" {
        return Err("Run setup from the main window.".into());
    }
    let state = app.services();
    let _transition = state.app_state.recording_transition.lock().await;
    let dictation = state.app_state.dictation.lock_or_recover();
    if crate::commands::recording::dictation_start_blocker(&state, dictation.status).is_some()
        || crate::audio_lifecycle::is_audio_active()
    {
        return Ok(false);
    }
    let control = app.control();
    let mut controller = control.controller.lock_or_recover();
    control.setup_suspended.store(true, Ordering::SeqCst);
    controller.gate.onboarding_complete = false;
    Ok(true)
}
#[cfg_attr(feature = "tauri-host", tauri::command)]
pub(crate) async fn resume_dictation_after_setup(
    window: crate::host::Window,
    app: crate::host::Context,
    settings: serde_json::Value,
) -> Result<(), String> {
    if window.label() != "main" {
        return Err("Complete setup from the main window.".into());
    }
    let control = app.control();
    control.setup_suspended.store(false, Ordering::SeqCst);
    let result = configure(&app, settings).await;
    if result.is_err() {
        control.setup_suspended.store(true, Ordering::SeqCst);
        control
            .controller
            .lock_or_recover()
            .gate
            .onboarding_complete = false;
    }
    result
}
#[cfg_attr(feature = "tauri-host", tauri::command)]
pub(crate) fn get_dictation_lock(app: crate::host::Context) -> bool {
    app.control().controller.lock_or_recover().locked
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn delayed_disable_cancellation_cannot_cancel_a_new_recording() {
        use crate::state::DictationStatus;
        let state = crate::state::AppState::default();
        for status in [DictationStatus::Starting, DictationStatus::Recording] {
            state.recording_id.store(12, Ordering::SeqCst);
            state.dictation.lock_or_recover().status = status;
            let generation = active_capture_generation(&state).unwrap();
            // A re-enable/new recording wins before the cancellation runs.
            state.recording_id.store(13, Ordering::SeqCst);
            assert!(crate::commands::recording::cancel_recording_transition(
                &state,
                Some(generation),
                || panic!("old Disable cancelled the new recording"),
            )
            .is_none());
            assert_eq!(state.dictation.lock_or_recover().status, status);
            assert!(!state.is_cancelled(13));
        }
        for status in [
            DictationStatus::Idle,
            DictationStatus::Processing,
            DictationStatus::Recovering,
        ] {
            state.dictation.lock_or_recover().status = status;
            assert!(active_capture_generation(&state).is_none());
        }
    }

    #[test]
    fn in_flight_refresh_cannot_overwrite_a_later_native_disable() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            mpsc, Arc,
        };
        let fence = Arc::new(ConfigurationFence::default());
        let disabled = Arc::new(AtomicBool::new(false));
        let (copied, copied_rx) = mpsc::channel();
        let (apply, apply_rx) = mpsc::channel();
        let refresh_fence = fence.clone();
        let refresh_disabled = disabled.clone();
        let refresh = std::thread::spawn(move || {
            refresh_fence.run(|| {
                let old = refresh_disabled.load(Ordering::SeqCst);
                copied.send(()).unwrap();
                apply_rx.recv().unwrap();
                refresh_disabled.store(old, Ordering::SeqCst);
            })
        });
        copied_rx.recv().unwrap();
        let disable_fence = fence.clone();
        let disable_state = disabled.clone();
        let disable = std::thread::spawn(move || {
            disable_fence.run(|| disable_state.store(true, Ordering::SeqCst))
        });
        apply.send(()).unwrap();
        refresh.join().unwrap();
        disable.join().unwrap();
        assert!(disabled.load(Ordering::SeqCst));
    }
    #[test]
    fn paused_settings_save_cannot_undo_native_disable_or_probe_revocation() {
        let fence = ConfigurationFence::default();
        let settings = Mutex::new(
            serde_json::json!({"disabled":false,"microphone":"system_default",
            "smartAutoMicrophoneEnabled":true,"smartAutoProbeEnabled":true}),
        );
        // The snapshot is captured and accepted before its task starts running.
        let snapshot = settings.lock_or_recover().clone();
        let revision = fence.accept();
        fence.override_current(|| settings.lock_or_recover()["disabled"] = true.into());
        let applied = fence.apply(revision, || *settings.lock_or_recover() = snapshot);
        assert!(applied.is_none());
        let current = settings.lock_or_recover();
        assert_eq!(current["disabled"], true);
        assert_eq!(probe_policy(&current)["enabled"], false);
        drop(current);
        let old = fence.accept();
        let new = fence.accept();
        assert!(fence.apply(new, || ()).is_some());
        assert!(fence
            .apply(old, || panic!("superseded save applied"))
            .is_none());
    }

    #[test]
    fn selected_model_install_arms_immediately_without_a_settings_write() {
        use std::cell::Cell;
        let (send, _receive) = tokio::sync::mpsc::unbounded_channel();
        let mut controller = Controller::default();
        controller.gate = murmur_core::dictation::LaunchGate {
            shutdown: crate::shutdown::closing(),
            configured: true,
            onboarding_complete: true,
            accessibility: true,
            ..Default::default()
        };
        let control = Control {
            controller: Mutex::new(controller),
            configuration: ConfigurationFence::default(),
            send,
            epoch: Instant::now(),
            setup_suspended: AtomicBool::new(false),
            settings: Mutex::new(
                serde_json::json!({"model":"tiny.en", "doubleTapKey":"shift_l", "recordingMode":"both"}),
            ),
        };
        let original_settings = control.settings.lock_or_recover().clone();
        let listener_armed = Cell::new(false);
        for installed in [false, true, false] {
            control.refresh_model_admission(
                |model| {
                    assert_eq!(model, "tiny.en");
                    installed
                },
                |armed, key, mode| {
                    assert_eq!((key, mode), ("shift_l", "both"));
                    listener_armed.set(armed);
                },
            );
            assert_eq!(listener_armed.get(), installed);
            let mut controller = control.controller.lock_or_recover();
            assert_eq!(controller.gate.can_start(), installed);
            assert_eq!(controller.input(Input::Toggle(None)).is_some(), installed);
            assert_eq!(*control.settings.lock_or_recover(), original_settings);
        }
    }

    #[test]
    fn native_stop_failure_is_content_free_and_rejects_cancelled_or_newer_owners() {
        let state = crate::state::AppState::default();
        state.recording_id.store(12, Ordering::SeqCst);
        let failure = Err("private backend content".to_owned());
        let mut presented = Vec::new();
        present_stop_failure(&state, 12, &failure, |value| {
            assert!(
                state.dictation.try_lock().is_ok(),
                "dispatch held dictation"
            );
            presented.push(serde_json::to_value(value).unwrap())
        });
        assert_eq!(
            presented,
            vec![serde_json::json!({
                "recordingId":12, "error":"Transcription failed. Please try again."
            })]
        );
        state.cancel_recording(12);
        present_stop_failure(&state, 12, &failure, |_| panic!("cancelled failure"));
        state.recording_id.store(13, Ordering::SeqCst);
        present_stop_failure(&state, 12, &failure, |_| panic!("stale failure"));
        present_stop_failure(
            &state,
            13,
            &Ok(serde_json::json!({"type":"transcription"})),
            |_| panic!("successful stop"),
        );
        present_stop_failure(
            &state,
            13,
            &Ok(serde_json::json!({"type":"error", "error":"private"})),
            |value| {
                assert_eq!(value.recording_id, 13);
                assert_eq!(value.error, "Transcription failed. Please try again.");
            },
        );
    }

    #[test]
    fn processing_cancel_cannot_publish_failure_after_idle_transition() {
        use crate::state::DictationStatus;
        let state = crate::state::AppState::default();
        state.recording_id.store(12, Ordering::SeqCst);
        state.dictation.lock_or_recover().status = DictationStatus::Processing;
        let mut published = false;
        let cancelled =
            crate::commands::recording::cancel_recording_transition(&state, Some(12), || {
                // Pause the real cancellation path after it releases dictation,
                // before its remaining cancellation work can run. Finish stop
                // in precisely the old Idle-to-cancellation-mark race window.
                assert_eq!(
                    state.dictation.lock_or_recover().status,
                    DictationStatus::Idle
                );
                present_stop_failure(&state, 12, &Err("private".into()), |_| published = true);
            });
        assert_eq!(cancelled, Some((DictationStatus::Processing, 12)));
        assert!(
            !published,
            "cancelled generation published a failure after Idle"
        );
        assert!(state.is_cancelled(12));
    }

    #[test]
    fn native_probe_policy_preserves_consent_and_input_gates() {
        let settings = serde_json::json!({"disabled":false,"microphone":"system_default",
            "smartAutoMicrophoneEnabled":true,"smartAutoProbeEnabled":true,
            "smartAutoApprovedDeviceIds":["synthetic"],"smartAutoPreferredDeviceIds":[],"smartAutoAllowContinuity":false});
        assert_eq!(probe_policy(&settings)["enabled"], true);
        for (key, value) in [
            ("disabled", serde_json::json!(true)),
            ("microphone", serde_json::json!("fixed")),
            ("smartAutoMicrophoneEnabled", serde_json::json!(false)),
            ("smartAutoProbeEnabled", serde_json::json!(false)),
        ] {
            let mut disabled = settings.clone();
            disabled[key] = value;
            assert_eq!(probe_policy(&disabled)["enabled"], false);
        }
    }
}

#[cfg(all(feature = "native-host", not(feature = "tauri-host")))]
pub(crate) fn native_start_idle(app: &crate::host::Context) -> Result<(), String> {
    let state = app.services();
    let dictation = state.app_state.dictation.lock_or_recover();
    if dictation.status != crate::state::DictationStatus::Idle {
        return Err("Recording ownership changed".into());
    }
    let generation = state.app_state.recording_id.load(Ordering::SeqCst);
    app.control()
        .send
        .send(Queued {
            #[cfg(all(feature = "native-host", not(feature = "tauri-host")))]
            native_active: Some(false),
            input: Input::Toggle(None),
            generation,
            at: Instant::now(),
        })
        .map_err(|_| "Recording controller unavailable".into())
}

#[cfg(all(feature = "native-host", not(feature = "tauri-host")))]
pub(crate) fn native_setup_suspension(app: &crate::host::Context, value: Option<bool>) -> bool {
    let control = app.control();
    if let Some(value) = value {
        control.setup_suspended.swap(value, Ordering::SeqCst)
    } else {
        control.setup_suspended.load(Ordering::SeqCst)
    }
}

#[cfg(all(feature = "native-host", not(feature = "tauri-host")))]
pub(crate) fn native_control(
    app: &crate::host::Context,
    generation: u64,
    input: Input,
) -> Result<(), String> {
    if !matches!(input, Input::Lock | Input::HoldStop | Input::Escape) {
        return Err("Unsupported recording control".into());
    }
    if app.services().app_state.recording_id.load(Ordering::SeqCst) != generation {
        return Err("Recording ownership changed".into());
    }
    app.control()
        .send
        .send(Queued {
            native_active: Some(true),
            input,
            generation,
            at: Instant::now(),
        })
        .map_err(|_| "Recording controller unavailable".into())
}
