use crate::browser_site::{self, BrowserSiteIdentity};
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
    SiteBinding,
    Temporary,
}

impl ModeSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::AppBinding => "app_binding",
            Self::SiteBinding => "site_binding",
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
    crate::dictation_context::enabled_mode(dictation, id)
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

fn site_mode_id<'a>(
    dictation: &'a DictationState,
    site: Option<&BrowserSiteIdentity>,
) -> Option<&'a str> {
    crate::dictation_context::enabled_site_mode_id(dictation, site)
}

fn resolved_status(
    dictation: &DictationState,
    bundle_id: Option<&str>,
    site: Option<&BrowserSiteIdentity>,
) -> ModeRuntimeStatus {
    let temporary = dictation
        .temporary_mode_bundle_id
        .as_deref()
        .zip(dictation.temporary_mode_id.as_deref())
        .filter(|(bundle, _)| Some(*bundle) == bundle_id)
        .and_then(|(_, id)| mode_by_id(dictation, id).map(|mode| (mode, ModeSource::Temporary)));
    let bound = bound_mode_id(dictation, bundle_id)
        .and_then(|id| mode_by_id(dictation, id))
        .map(|mode| (mode, ModeSource::AppBinding));
    let site_bound = site_mode_id(dictation, site)
        .and_then(|id| mode_by_id(dictation, id))
        .map(|mode| (mode, ModeSource::SiteBinding));
    let manual = mode_by_id(dictation, &dictation.manual_mode_id)
        .or_else(|| builtin_mode("builtin.everyday"))
        .expect("built-in Everyday Mode must exist");
    let (mode, source) = temporary
        .or(site_bound)
        .or(bound)
        .unwrap_or((manual, ModeSource::Manual));
    ModeRuntimeStatus {
        id: mode.id,
        name: mode.name,
        source,
    }
}

fn observe_context(
    dictation: &mut DictationState,
    bundle_id: Option<&str>,
    site: Option<&BrowserSiteIdentity>,
) -> ModeRuntimeStatus {
    if dictation.temporary_mode_bundle_id.as_deref() != bundle_id {
        dictation.temporary_mode_id = None;
        dictation.temporary_mode_bundle_id = None;
    }
    resolved_status(dictation, bundle_id, site)
}

fn publish(app: &tauri::AppHandle, status: &ModeRuntimeStatus) {
    super::tray::set_mode_menu_status(status);
    let _ = app.emit("mode-runtime-changed", status);
}

fn current_identity() -> frontmost::FrontmostAppIdentity {
    frontmost::query_frontmost_app_identity()
}

fn current_site(
    enabled: bool,
    identity: &frontmost::FrontmostAppIdentity,
) -> Option<BrowserSiteIdentity> {
    enabled
        .then(|| browser_site::query_for_identity(identity))
        .flatten()
}

#[tauri::command]
pub fn get_mode_runtime_status(state: tauri::State<'_, State>) -> ModeRuntimeStatus {
    let identity = current_identity();
    let enabled = state
        .app_state
        .dictation
        .lock_or_recover()
        .site_mode_lookup_enabled;
    let site = current_site(enabled, &identity);
    let mut dictation = state.app_state.dictation.lock_or_recover();
    observe_context(&mut dictation, identity.bundle_id.as_deref(), site.as_ref())
}

#[tauri::command]
pub fn cycle_mode(app: tauri::AppHandle, state: tauri::State<'_, State>) -> ModeRuntimeStatus {
    let identity = current_identity();
    let enabled = state
        .app_state
        .dictation
        .lock_or_recover()
        .site_mode_lookup_enabled;
    let site = current_site(enabled, &identity);
    let bundle_id = identity.bundle_id;
    let mut dictation = state.app_state.dictation.lock_or_recover();
    let current = observe_context(&mut dictation, bundle_id.as_deref(), site.as_ref());
    let modes = available_modes(&dictation);
    let next = modes
        .iter()
        .position(|mode| mode.id == current.id)
        .and_then(|index| modes.get((index + 1) % modes.len()))
        .or_else(|| modes.first())
        .expect("built-in Mode catalog must not be empty")
        .clone();
    let binding_active = site_mode_id(&dictation, site.as_ref()).is_some()
        || bound_mode_id(&dictation, bundle_id.as_deref()).is_some();
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
    let status = resolved_status(&dictation, bundle_id.as_deref(), site.as_ref());
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
    let identity = current_identity();
    let enabled = state
        .app_state
        .dictation
        .lock_or_recover()
        .site_mode_lookup_enabled;
    let site = current_site(enabled, &identity);
    let mut dictation = state.app_state.dictation.lock_or_recover();
    dictation.temporary_mode_id = None;
    dictation.temporary_mode_bundle_id = None;
    let status = resolved_status(&dictation, identity.bundle_id.as_deref(), site.as_ref());
    drop(dictation);
    publish(&app, &status);
    status
}

pub(crate) fn spawn_mode_watcher(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut previous: Option<ModeRuntimeStatus> = None;
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let identity = current_identity();
            let status = {
                let state = app.state::<State>();
                let enabled = state
                    .app_state
                    .dictation
                    .lock_or_recover()
                    .site_mode_lookup_enabled;
                let site = current_site(enabled, &identity);
                let mut dictation = state.app_state.dictation.lock_or_recover();
                observe_context(&mut dictation, identity.bundle_id.as_deref(), site.as_ref())
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
    use crate::state::{AppProfile, BrowserSiteRule};

    #[test]
    fn app_binding_temporarily_overrides_and_leaving_restores_manual_mode() {
        let mut state = DictationState {
            manual_mode_id: "builtin.notes".into(),
            ..DictationState::default()
        };
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
            resolved_status(&state, Some("com.example.mail"), None).id,
            "builtin.email"
        );
        state.temporary_mode_id = Some("builtin.verbatim".into());
        state.temporary_mode_bundle_id = Some("com.example.mail".into());
        assert_eq!(
            resolved_status(&state, Some("com.example.mail"), None).source,
            ModeSource::Temporary
        );
        let restored = observe_context(&mut state, Some("com.example.notes"), None);
        assert_eq!(restored.id, "builtin.notes");
        assert_eq!(restored.source, ModeSource::Manual);
        assert!(state.temporary_mode_id.is_none());
    }

    #[test]
    fn disabled_or_unknown_modes_fail_back_to_everyday() {
        let state = DictationState {
            manual_mode_id: "missing".into(),
            ..DictationState::default()
        };
        let status = resolved_status(&state, None, None);
        assert_eq!(status.id, "builtin.everyday");
        assert_eq!(status.source, ModeSource::Manual);
    }

    #[test]
    fn exact_site_binding_outranks_app_and_leaving_restores_lower_precedence() {
        let mut state = DictationState {
            manual_mode_id: "builtin.notes".into(),
            site_mode_lookup_enabled: true,
            browser_site_rules: vec![BrowserSiteRule {
                id: "github".into(),
                browser_bundle_id: "com.apple.Safari".into(),
                host: "github.com".into(),
                mode_id: "builtin.technical".into(),
                enabled: true,
            }],
            ..DictationState::default()
        };
        state.app_profiles.push(AppProfile {
            bundle_id: "com.apple.Safari".into(),
            label: "Safari".into(),
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
        let github = BrowserSiteIdentity {
            browser_bundle_id: "com.apple.Safari".into(),
            host: "github.com".into(),
        };
        let site = resolved_status(&state, Some("com.apple.Safari"), Some(&github));
        assert_eq!(
            (site.id.as_str(), site.source),
            ("builtin.technical", ModeSource::SiteBinding)
        );
        let left_site = resolved_status(&state, Some("com.apple.Safari"), None);
        assert_eq!(
            (left_site.id.as_str(), left_site.source),
            ("builtin.email", ModeSource::AppBinding)
        );
        let left_browser = observe_context(&mut state, Some("com.example.Editor"), None);
        assert_eq!(
            (left_browser.id.as_str(), left_browser.source),
            ("builtin.notes", ModeSource::Manual)
        );
    }

    #[test]
    fn disabled_site_mode_falls_through_to_the_app_binding() {
        let mut disabled = builtin_mode("builtin.technical").unwrap();
        disabled.id = "mode.disabled".into();
        disabled.name = "Disabled".into();
        disabled.enabled = false;
        let state = DictationState {
            modes: vec![disabled],
            app_profiles: vec![AppProfile {
                bundle_id: "com.apple.Safari".into(),
                label: "Safari".into(),
                auto_paste_override: None,
                cleanup_override: None,
                cli_formatting_override: None,
                smart_formatting_override: None,
                writing_style: None,
                ide_context_enabled: false,
                ide_project_roots: Vec::new(),
                query_context_excluded: false,
                mode_id: Some("builtin.email".into()),
            }],
            site_mode_lookup_enabled: true,
            browser_site_rules: vec![BrowserSiteRule {
                id: "github".into(),
                browser_bundle_id: "com.apple.Safari".into(),
                host: "github.com".into(),
                mode_id: "mode.disabled".into(),
                enabled: true,
            }],
            ..DictationState::default()
        };
        let github = BrowserSiteIdentity {
            browser_bundle_id: "com.apple.Safari".into(),
            host: "github.com".into(),
        };

        let status = resolved_status(&state, Some("com.apple.Safari"), Some(&github));

        assert_eq!(
            (status.id.as_str(), status.source),
            ("builtin.email", ModeSource::AppBinding)
        );
        assert_eq!(
            crate::dictation_context::enabled_site_mode_id(&state, Some(&github)),
            None
        );
    }
}
