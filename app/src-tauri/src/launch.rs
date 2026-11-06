//! Content-free launch timing and conservative native preparation.
//!
//! Native startup applies a committed v7 configuration before arming control.
//! Unknown/legacy snapshots still use the existing frontend migration path.
use crate::MutexExt;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    OnceLock,
};
use std::time::Instant;
use tauri::Manager;

static PROCESS_START: OnceLock<Instant> = OnceLock::new();
static FRONTEND_CONFIGURING: AtomicBool = AtomicBool::new(false);
static MODEL_READY: AtomicBool = AtomicBool::new(false);
static FRONTEND_HYDRATED: AtomicBool = AtomicBool::new(false);

pub fn process_start() {
    PROCESS_START.get_or_init(Instant::now);
}

pub(crate) fn milestone(event_code: &'static str) {
    let launch_elapsed_ms = if event_code == "system.launch_process_start" {
        0
    } else {
        PROCESS_START
            .get_or_init(Instant::now)
            .elapsed()
            .as_millis() as u64
    };
    tracing::info!(target: "system", event_code, launch_elapsed_ms, "Launch milestone");
}

pub(crate) fn frontend_configuring() {
    FRONTEND_CONFIGURING.store(true, Ordering::SeqCst);
}

pub(crate) fn model_ready() {
    if !MODEL_READY.swap(true, Ordering::SeqCst) {
        milestone("system.launch_model_ready");
    }
}

pub(crate) fn frontend_hydrated() {
    if !FRONTEND_HYDRATED.swap(true, Ordering::SeqCst) {
        milestone("system.launch_frontend_hydrated");
    }
}

#[derive(Debug, PartialEq)]
struct Preparation {
    disabled: bool,
    model: String,
}

/// Do not duplicate the frontend's migrations or guess at unknown settings.
/// Older installations acquire the durable onboarding mirror on their next
/// frontend boot and use the existing frontend-driven path for this launch.
fn preparation(blob: &str, _accessibility: bool) -> Option<Preparation> {
    let value: serde_json::Value = serde_json::from_str(blob).ok()?;
    if value.get("settingsVersion")?.as_u64()? != 7
        || !value.get("launchOnboardingComplete")?.as_bool()?
    {
        return None;
    }
    let disabled = value.get("disabled")?.as_bool()?;
    let model = value.get("model")?.as_str()?;
    crate::model_runtime::model_definition(model).ok()?;
    // These are the exact frontend picker IDs. Invalid snapshots fall back to
    // frontend validation, rather than choosing a different native hotkey.
    let key = value.get("doubleTapKey")?.as_str()?;
    if !crate::keyboard::is_dictation_key_id(key)
        || !matches!(
            value.get("recordingMode")?.as_str()?,
            "hold_down" | "double_tap" | "both"
        )
    {
        return None;
    }
    Some(Preparation {
        disabled,
        model: model.into(),
    })
}

pub(crate) fn prepare(app: &tauri::AppHandle) {
    let Ok(Some(blob)) = crate::commands::settings_store::load_settings_blob(app.clone()) else {
        return;
    };
    let Some(plan) = preparation(&blob, crate::injector::is_accessibility_enabled()) else {
        return;
    };
    crate::keyboard::set_app_disabled(plan.disabled);
    let revision = crate::dictation_control::accept_configuration(app);
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let Ok(settings) = serde_json::from_str::<serde_json::Value>(&blob) else {
            return;
        };
        // The persisted v7 snapshot was confirmed by the current loader. Apply
        // the existing backend configuration path before arming native control.
        if crate::commands::recording::configure_dictation(
            settings.clone(),
            handle.clone(),
            handle.state::<crate::State>(),
        )
        .await
        .is_ok()
        {
            if crate::dictation_control::configure_accepted(&handle, settings, revision) != Ok(true)
            {
                return;
            }
            plan.schedule_warmup(|model| {
                let model = model.to_owned();
                let configuration = crate::audio::prepare::configuration_generation();
                let warm_handle = handle.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    let state = warm_handle.state::<crate::State>();
                    if crate::keyboard::is_app_disabled() {
                        return;
                    }
                    *state.app_state.last_transcription_at.lock_or_recover() = Some(Instant::now());
                    let _ = state.app_state.model_runtime.prepare_startup_if(
                        Some(&warm_handle),
                        &model,
                        || {
                            !crate::keyboard::is_app_disabled()
                                && crate::audio::prepare::configuration_generation()
                                    == configuration
                        },
                    );
                });
            });
        }
    });
}

pub(crate) fn warmup_allowed(disabled: bool, model: &str) -> bool {
    !disabled
        && crate::model_runtime::model_definition(model)
            .is_ok_and(|definition| definition.warm_on_startup)
}

pub(crate) fn schedule_warmup(disabled: bool, model: &str, warm: impl FnOnce(&str)) {
    if warmup_allowed(disabled, model) {
        warm(model);
    }
}

impl Preparation {
    // Startup and backend configuration share the same guarded decision.
    fn schedule_warmup(&self, warm: impl FnOnce(&str)) {
        schedule_warmup(self.disabled, &self.model, warm);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn settings() -> serde_json::Value {
        serde_json::json!({"settingsVersion":7,"launchOnboardingComplete":true,
            "disabled":false,"model":"base.en","doubleTapKey":"shift_l","recordingMode":"hold_down"})
    }
    #[test]
    fn rust_start_requires_the_same_gates_and_valid_persisted_settings() {
        let settings = settings();
        assert!(preparation(&settings.to_string(), true).is_some());
        assert!(preparation(&settings.to_string(), false).is_some());
        assert!(preparation("invalid", true).is_none());
        for (key, value) in [
            ("launchOnboardingComplete", serde_json::json!(false)),
            ("launchOnboardingComplete", serde_json::Value::Null),
            ("disabled", serde_json::json!("false")),
            ("settingsVersion", serde_json::json!(6)),
            ("settingsVersion", serde_json::json!(8)),
            ("model", serde_json::json!("unknown-model")),
            ("doubleTapKey", serde_json::json!("unknown-key")),
            ("recordingMode", serde_json::json!("hotkey")),
        ] {
            let mut invalid = settings.clone();
            invalid[key] = value;
            assert!(preparation(&invalid.to_string(), true).is_none(), "{key}");
        }
    }
    #[test]
    fn rust_start_obeys_disabled_and_catalog_warm_policy() {
        use std::cell::Cell;
        let models = Cell::new(0);
        let mut settings = settings();
        settings["model"] = serde_json::json!("parakeet-tdt-0.6b-v3-coreml");
        settings["disabled"] = serde_json::json!(true);
        let disabled = preparation(&settings.to_string(), true).unwrap();
        assert!(disabled.disabled);
        disabled.schedule_warmup(|_| models.set(models.get() + 1));
        assert_eq!(models.get(), 0);
        settings["disabled"] = serde_json::json!(false);
        preparation(&settings.to_string(), true)
            .unwrap()
            .schedule_warmup(|_| models.set(models.get() + 1));
        assert_eq!(models.get(), 1);
        settings["model"] = serde_json::json!("base.en");
        preparation(&settings.to_string(), true)
            .unwrap()
            .schedule_warmup(|_| models.set(models.get() + 1));
        assert_eq!(models.get(), 1);
    }
}
