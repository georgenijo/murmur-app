//! Tauri adapter for the window-independent core controller. No frontend listener
//! is on the control path. Only this host module knows commands/managed state.
use crate::{MutexExt, State};
use murmur_core::{
    context::AppContext,
    dictation::{Controller, Effect, Input, Origin, Status},
};
use std::{
    sync::{atomic::Ordering, Mutex},
    time::Instant,
};
use tauri::{Emitter, Manager};

pub(crate) struct Control {
    controller: Mutex<Controller>,
    send: tokio::sync::mpsc::UnboundedSender<Input>,
    epoch: Instant,
    pub(crate) settings: Mutex<serde_json::Value>,
}

pub(crate) fn initialize(app: &tauri::AppHandle) {
    let (send, mut receive) = tokio::sync::mpsc::unbounded_channel();
    app.manage(Control {
        controller: Mutex::new(Controller::default()),
        send,
        epoch: Instant::now(),
        settings: Mutex::new(serde_json::Value::Null),
    });
    let handle = app.clone();
    let context = app.state::<AppContext>().inner().clone();
    let _ = context.background.spawn(Box::pin(async move {
        while let Some(input) = receive.recv().await {
            let state = handle.state::<State>();
            let (generation, status) = {
                let dictation = state.app_state.dictation.lock_or_recover();
                (
                    state.app_state.recording_id.load(Ordering::SeqCst),
                    status(dictation.status),
                )
            };
            let control = handle.state::<Control>();
            let (effect, recording, locked) = {
                let mut controller = control.controller.lock_or_recover();
                controller.observe(generation, status, Origin::Toggle);
                let effect = controller.input(input);
                (effect, controller.keyboard_recording(), controller.locked)
            };
            crate::keyboard::set_recording_state(recording);
            let _ = handle.emit("dictation-lock-changed", locked);
            if let Some(effect) = effect {
                execute(&handle, effect).await;
            }
        }
    }));
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
pub(crate) fn send(app: &tauri::AppHandle, input: Input) {
    if let Some(control) = app.try_state::<Control>() {
        let _ = control.send.send(input);
    }
}
pub(crate) fn started(app: &tauri::AppHandle, generation: u64, origin: &str) {
    let control = app.state::<Control>();
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
pub(crate) fn level(app: &tauri::AppHandle, generation: u64, level: f32) {
    if let Some(control) = app.try_state::<Control>() {
        let _ = control.send.send(Input::Level {
            generation,
            level: level as f64,
            at_ms: control.epoch.elapsed().as_millis() as u64,
        });
    }
}
async fn execute(app: &tauri::AppHandle, effect: Effect) {
    use crate::commands::recording;
    match effect {
        Effect::Start {
            origin,
            gesture_token,
        } => {
            let settings = app.state::<Control>().settings.lock_or_recover().clone();
            let smart_auto = if settings["smartAutoMicrophoneEnabled"] == true {
                serde_json::from_value(serde_json::json!({
                    "approvedDeviceIds": settings["smartAutoApprovedDeviceIds"],
                    "preferredDeviceIds": settings["smartAutoPreferredDeviceIds"],
                    "allowContinuity": settings["smartAutoAllowContinuity"],
                    "requireRecentSignal": settings["smartAutoProbeEnabled"],
                }))
                .ok()
            } else {
                None
            };
            let device = settings["microphone"]
                .as_str()
                .filter(|id| *id != "system_default" && !id.is_empty())
                .map(str::to_owned);
            let result = recording::start_native_recording(
                app.clone(),
                app.state::<State>(),
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
            if result.is_err() {
                let _ = app.emit("dictation-control-error", "Could not start dictation.");
            }
        }
        Effect::Cancel { generation } => {
            let _ = recording::cancel_native_recording(
                app.clone(),
                app.state::<State>(),
                Some(generation),
            )
            .await;
        }
        Effect::Stop { generation, auto } => {
            // Transcription may take seconds. Keep receiving Escape and levels;
            // the existing pipeline rechecks this exact generation at every await.
            let handle = app.clone();
            let context = app.state::<AppContext>().inner().clone();
            let _ = context.background.spawn(Box::pin(async move {
                let trigger = if auto {
                    crate::audio_lifecycle::DictationStopTrigger::AutoStop
                } else {
                    crate::audio_lifecycle::DictationStopTrigger::HoldRelease
                };
                let _ = recording::stop_native_recording_for(
                    handle.clone(),
                    handle.state::<State>(),
                    Some(generation),
                    trigger,
                )
                .await;
            }));
        }
        Effect::Score { generation, pause } => {
            let handle = app.clone();
            let context = app.state::<AppContext>().inner().clone();
            let _ = context.background.spawn(Box::pin(async move {
                let answer =
                    crate::commands::smart_turn::check_smart_turn(handle.state::<State>()).await;
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
}

/// Settings migration remains 1.5. Accept only a complete v7 native-confirmed
/// snapshot; older/unknown versions wait for the existing frontend migration.
pub(crate) async fn configure(
    app: &tauri::AppHandle,
    settings: serde_json::Value,
) -> Result<(), String> {
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
    let configured = settings["model"]
        .as_str()
        .is_some_and(|model| crate::model_runtime::model_definition(model).is_ok());
    let installed = settings["model"].as_str().is_some_and(|model| {
        crate::commands::models::check_specific_model_exists(app.state::<State>(), model.into())
    });
    let accessibility = crate::injector::is_accessibility_enabled();
    let control = app.state::<Control>();
    let gate = murmur_core::dictation::LaunchGate {
        configured,
        onboarding_complete: settings["launchOnboardingComplete"]
            .as_bool()
            .unwrap_or(false),
        model_installed: installed,
        accessibility,
        disabled: settings["disabled"].as_bool().unwrap_or(true),
    };
    {
        let mut controller = control.controller.lock_or_recover();
        controller.gate = gate;
        controller.silence_ms = settings["autoStopSilenceMs"]
            .as_u64()
            .filter(|ms| [0, 1500, 2500, 4000].contains(ms))
            .unwrap_or(0);
        controller.smart_enabled = settings["smartAutoStopEnabled"].as_bool().unwrap_or(false);
    }
    *control.settings.lock_or_recover() = settings.clone();
    crate::commands::keyboard::set_app_disabled(app.clone(), gate.disabled)?;
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

#[tauri::command]
pub(crate) async fn configure_dictation_control(
    app: tauri::AppHandle,
    settings: serde_json::Value,
) -> Result<(), String> {
    configure(&app, settings).await
}
#[tauri::command]
pub(crate) fn toggle_dictation_lock(app: tauri::AppHandle) {
    send(&app, Input::Lock);
}

pub(crate) async fn refresh(app: &tauri::AppHandle) {
    let settings = app.state::<Control>().settings.lock_or_recover().clone();
    if !settings.is_null() {
        let _ = configure(app, settings).await;
    }
}
