use crate::dictation_context::builtin_mode;
use crate::frontmost;
use crate::state::{DictationState, MurmurMode};
use crate::{MutexExt, State};
use serde::Serialize;
use tauri::{Emitter, Manager};

const BUILTIN_MODE_IDS: [&str; 7] = [
    "builtin.everyday",
    "builtin.messages",
    "builtin.email",
    "builtin.notes",
    "builtin.technical",
    "builtin.terminal",
    "builtin.verbatim",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModeSource {
    Manual,
    AppBinding,
    Temporary,
}

impl ModeSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::AppBinding => "app_binding",
            Self::Temporary => "temporary",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModeRuntimeStatus {
    pub id: String,
    pub name: String,
    pub source: ModeSource,
}

fn mode_by_id(dictation: &DictationState, id: &str) -> Option<MurmurMode> {
    builtin_mode(id).or_else(|| {
        dictation
            .modes
            .iter()
            .find(|mode| mode.id == id && mode.enabled)
            .cloned()
    })
}

fn available_modes(dictation: &DictationState) -> Vec<MurmurMode> {
    BUILTIN_MODE_IDS
        .iter()
        .filter_map(|id| builtin_mode(id))
        .chain(dictation.modes.iter().filter(|mode| mode.enabled).cloned())
        .collect()
}

fn bound_mode_id<'a>(dictation: &'a DictationState, bundle_id: Option<&str>) -> Option<&'a str> {
    let id = bundle_id.and_then(|bundle_id| {
        dictation
            .app_profiles
            .iter()
            .find(|profile| profile.bundle_id == bundle_id)
            .and_then(|profile| profile.mode_id.as_deref())
    })?;
    mode_by_id(dictation, id).is_some().then_some(id)
}

fn resolved_status(dictation: &DictationState, bundle_id: Option<&str>) -> ModeRuntimeStatus {
    let temporary = dictation
        .temporary_mode_bundle_id
        .as_deref()
        .zip(dictation.temporary_mode_id.as_deref())
        .filter(|(bundle, _)| Some(*bundle) == bundle_id)
        .and_then(|(_, id)| mode_by_id(dictation, id).map(|mode| (mode, ModeSource::Temporary)));
    let bound = bound_mode_id(dictation, bundle_id)
        .and_then(|id| mode_by_id(dictation, id))
        .map(|mode| (mode, ModeSource::AppBinding));
    let manual = mode_by_id(dictation, &dictation.manual_mode_id)
        .or_else(|| builtin_mode("builtin.everyday"))
        .expect("built-in Everyday Mode must exist");
    let (mode, source) = temporary.or(bound).unwrap_or((manual, ModeSource::Manual));
    ModeRuntimeStatus {
        id: mode.id,
        name: mode.name,
        source,
    }
}

fn observe_bundle(dictation: &mut DictationState, bundle_id: Option<&str>) -> ModeRuntimeStatus {
    if dictation.temporary_mode_bundle_id.as_deref() != bundle_id {
        dictation.temporary_mode_id = None;
        dictation.temporary_mode_bundle_id = None;
    }
    resolved_status(dictation, bundle_id)
}

fn publish(app: &tauri::AppHandle, status: &ModeRuntimeStatus) {
    super::tray::set_mode_menu_status(status);
    let _ = app.emit("mode-runtime-changed", status);
}

fn current_bundle_id() -> Option<String> {
    frontmost::query_frontmost_app_identity().bundle_id
}

#[tauri::command]
pub fn get_mode_runtime_status(state: tauri::State<'_, State>) -> ModeRuntimeStatus {
    let bundle_id = current_bundle_id();
    let mut dictation = state.app_state.dictation.lock_or_recover();
    observe_bundle(&mut dictation, bundle_id.as_deref())
}

#[tauri::command]
pub fn cycle_mode(app: tauri::AppHandle, state: tauri::State<'_, State>) -> ModeRuntimeStatus {
    let bundle_id = current_bundle_id();
    let mut dictation = state.app_state.dictation.lock_or_recover();
    let current = observe_bundle(&mut dictation, bundle_id.as_deref());
    let modes = available_modes(&dictation);
    let next = modes
        .iter()
        .position(|mode| mode.id == current.id)
        .and_then(|index| modes.get((index + 1) % modes.len()))
        .or_else(|| modes.first())
        .expect("built-in Mode catalog must not be empty")
        .clone();
    let binding_active = bound_mode_id(&dictation, bundle_id.as_deref()).is_some();
    let manual_changed = if binding_active {
        dictation.temporary_mode_id = Some(next.id.clone());
        dictation.temporary_mode_bundle_id = bundle_id.clone();
        false
    } else {
        dictation.manual_mode_id = next.id.clone();
        dictation.temporary_mode_id = None;
        dictation.temporary_mode_bundle_id = None;
        true
    };
    let status = resolved_status(&dictation, bundle_id.as_deref());
    drop(dictation);
    if manual_changed {
        let _ = app.emit(
            "mode-manual-changed",
            serde_json::json!({ "modeId": status.id }),
        );
    }
    tracing::info!(target: "pipeline", source = status.source.as_str(), "native Mode cycled");
    publish(&app, &status);
    status
}

#[tauri::command]
pub fn clear_temporary_mode_override(
    app: tauri::AppHandle,
    state: tauri::State<'_, State>,
) -> ModeRuntimeStatus {
    let bundle_id = current_bundle_id();
    let mut dictation = state.app_state.dictation.lock_or_recover();
    dictation.temporary_mode_id = None;
    dictation.temporary_mode_bundle_id = None;
    let status = resolved_status(&dictation, bundle_id.as_deref());
    drop(dictation);
    publish(&app, &status);
    status
}

pub(crate) fn spawn_mode_watcher(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut previous: Option<ModeRuntimeStatus> = None;
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let bundle_id = current_bundle_id();
            let status = {
                let state = app.state::<State>();
                let mut dictation = state.app_state.dictation.lock_or_recover();
                observe_bundle(&mut dictation, bundle_id.as_deref())
            };
            if previous.as_ref() != Some(&status) {
                publish(&app, &status);
                previous = Some(status);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppProfile;

    #[test]
    fn app_binding_temporarily_overrides_and_leaving_restores_manual_mode() {
        let mut state = DictationState::default();
        state.manual_mode_id = "builtin.notes".into();
        state.app_profiles.push(AppProfile {
            bundle_id: "com.example.mail".into(),
            label: "Mail".into(),
            auto_paste_override: None,
            cleanup_override: None,
            cli_formatting_override: None,
            smart_formatting_override: None,
            writing_style: None,
            ide_context_enabled: false,
            ide_project_roots: Vec::new(),
            query_context_excluded: false,
            mode_id: Some("builtin.email".into()),
        });
        assert_eq!(
            resolved_status(&state, Some("com.example.mail")).id,
            "builtin.email"
        );
        state.temporary_mode_id = Some("builtin.verbatim".into());
        state.temporary_mode_bundle_id = Some("com.example.mail".into());
        assert_eq!(
            resolved_status(&state, Some("com.example.mail")).source,
            ModeSource::Temporary
        );
        let restored = observe_bundle(&mut state, Some("com.example.notes"));
        assert_eq!(restored.id, "builtin.notes");
        assert_eq!(restored.source, ModeSource::Manual);
        assert!(state.temporary_mode_id.is_none());
    }

    #[test]
    fn disabled_or_unknown_modes_fail_back_to_everyday() {
        let mut state = DictationState::default();
        state.manual_mode_id = "missing".into();
        let status = resolved_status(&state, None);
        assert_eq!(status.id, "builtin.everyday");
        assert_eq!(status.source, ModeSource::Manual);
    }
}
