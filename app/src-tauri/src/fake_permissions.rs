//! Dev-only fake permission provider for native onboarding tests (#972).
//!
//! Computer Use cannot click the macOS TCC alerts (a system process owns them),
//! and enabling Accessibility needs an admin password. With this provider the
//! permission commands onboarding uses answer from a script instead of TCC, so a
//! native run can walk every grant and denial path without a system dialog.
//!
//! Compiled only with the `fake-permissions` cargo feature, which only
//! `npm run tauri:dev:fake-permissions` enables, and refused in any build without
//! debug assertions (see the `compile_error!` in `lib.rs`). Even then it is idle
//! until `MURMUR_FAKE_PERMISSIONS` is set at process start. It never calls
//! `tccutil` and never changes a real grant: a faked permission's reset command
//! is refused instead.
//!
//! The script is JSON, inline or in a file, or a preset name. See
//! `docs/DEVELOPMENT.md` ("Fake permissions for native onboarding tests").

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::Deserialize;

pub(crate) const ENV_VAR: &str = "MURMUR_FAKE_PERMISSIONS";

/// Refusal for reset commands while a permission is faked.
pub(crate) const RESET_REFUSED: &str =
    "Fake permissions are active (MURMUR_FAKE_PERMISSIONS): reset is disabled and no real grant was changed.";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Permission {
    Microphone,
    Accessibility,
    SystemAudio,
    Calendar,
}

impl Permission {
    const ALL: [Permission; 4] = [
        Permission::Microphone,
        Permission::Accessibility,
        Permission::SystemAudio,
        Permission::Calendar,
    ];

    fn name(self) -> &'static str {
        match self {
            Permission::Microphone => "microphone",
            Permission::Accessibility => "accessibility",
            Permission::SystemAudio => "systemAudio",
            Permission::Calendar => "calendar",
        }
    }
}

/// What the fake reports for a permission right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum FakeState {
    NotDetermined,
    Granted,
    Denied,
    Restricted,
}

/// How the simulated user answers a dialog or a Settings visit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Outcome {
    Granted,
    Denied,
    NoAnswer,
}

impl Outcome {
    fn state(self) -> Option<FakeState> {
        match self {
            Outcome::Granted => Some(FakeState::Granted),
            Outcome::Denied => Some(FakeState::Denied),
            Outcome::NoAnswer => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Script {
    #[serde(default = "Script::default_start")]
    pub start: FakeState,
    /// Answer to the in-app request (the TCC dialog), given only while the
    /// permission is still `notDetermined`, as macOS does.
    #[serde(default = "Script::default_request")]
    pub request: Outcome,
    #[serde(default = "Script::default_request_delay")]
    pub request_delay_ms: u64,
    /// What the user does after an Open Settings action.
    #[serde(default = "Script::default_settings")]
    pub settings: Outcome,
    #[serde(default = "Script::default_settings_delay")]
    pub settings_delay_ms: u64,
}

impl Script {
    fn default_start() -> FakeState {
        FakeState::NotDetermined
    }
    fn default_request() -> Outcome {
        Outcome::Granted
    }
    fn default_request_delay() -> u64 {
        1_000
    }
    fn default_settings() -> Outcome {
        Outcome::Granted
    }
    fn default_settings_delay() -> u64 {
        2_000
    }
}

impl Default for Script {
    fn default() -> Self {
        Script {
            start: Self::default_start(),
            request: Self::default_request(),
            request_delay_ms: Self::default_request_delay(),
            settings: Self::default_settings(),
            settings_delay_ms: Self::default_settings_delay(),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Config {
    microphone: Option<Script>,
    accessibility: Option<Script>,
    system_audio: Option<Script>,
    calendar: Option<Script>,
    /// Open the real System Settings pane on an Open Settings action (default
    /// true), so the app loses focus and the return-to-Murmur path runs. Opening
    /// a pane changes no grant.
    open_settings_pane: Option<bool>,
}

struct Entry {
    script: Script,
    state: FakeState,
    /// Bumped by every change, so a delayed answer scheduled before a later
    /// change does not overwrite it.
    generation: u64,
}

pub(crate) struct Provider {
    entries: Mutex<HashMap<Permission, Entry>>,
    open_settings_pane: bool,
}

/// Parse a `MURMUR_FAKE_PERMISSIONS` value: a preset name, inline JSON, or the
/// path of a JSON file.
pub(crate) fn parse(value: &str) -> Result<Provider, String> {
    let value = value.trim();
    let config: Config = match value {
        "fresh" => preset(Script::default()),
        "granted" => preset(Script {
            start: FakeState::Granted,
            ..Script::default()
        }),
        "denied" => preset(Script {
            request: Outcome::Denied,
            ..Script::default()
        }),
        "no-answer" => preset(Script {
            request: Outcome::NoAnswer,
            settings: Outcome::NoAnswer,
            ..Script::default()
        }),
        json if json.starts_with('{') => {
            serde_json::from_str(json).map_err(|e| format!("{ENV_VAR}: invalid JSON: {e}"))?
        }
        path => {
            let text = std::fs::read_to_string(path)
                .map_err(|e| format!("{ENV_VAR}: cannot read script file: {e}"))?;
            serde_json::from_str(&text)
                .map_err(|e| format!("{ENV_VAR}: invalid JSON in script file: {e}"))?
        }
    };
    let mut entries = HashMap::new();
    for (permission, script) in [
        (Permission::Microphone, config.microphone),
        (Permission::Accessibility, config.accessibility),
        (Permission::SystemAudio, config.system_audio),
        (Permission::Calendar, config.calendar),
    ] {
        if let Some(script) = script {
            entries.insert(
                permission,
                Entry {
                    script,
                    state: script.start,
                    generation: 0,
                },
            );
        }
    }
    if entries.is_empty() {
        return Err(format!("{ENV_VAR}: the script fakes no permission"));
    }
    Ok(Provider {
        entries: Mutex::new(entries),
        open_settings_pane: config.open_settings_pane.unwrap_or(true),
    })
}

fn preset(script: Script) -> Config {
    Config {
        microphone: Some(script),
        accessibility: Some(script),
        system_audio: Some(script),
        calendar: Some(script),
        open_settings_pane: None,
    }
}

static PROVIDER: OnceLock<Option<Provider>> = OnceLock::new();

/// The process-wide provider, read from the environment once.
fn provider() -> Option<&'static Provider> {
    PROVIDER
        .get_or_init(|| {
            let value = std::env::var(ENV_VAR).ok()?;
            match parse(&value) {
                Ok(provider) => Some(provider),
                Err(error) => {
                    tracing::error!(target: "system", "fake permissions not active: {error}");
                    None
                }
            }
        })
        .as_ref()
}

/// Log once at startup when the provider is active. Content-free: only the
/// names of the faked permissions.
pub(crate) fn announce() {
    if let Some(provider) = provider() {
        let faked = provider.faked().join(", ");
        tracing::warn!(
            target: "system",
            "DEV FAKE PERMISSIONS ACTIVE ({ENV_VAR}): {faked} answer from a script, not macOS"
        );
    }
}

pub(crate) fn is_active() -> bool {
    provider().is_some()
}

/// Script for the dev marker shown in the main window.
pub(crate) const MARKER_SCRIPT: &str = r#"(function () {
  if (document.getElementById('murmur-fake-permissions-marker')) return;
  var add = function () {
    if (document.getElementById('murmur-fake-permissions-marker')) return;
    var el = document.createElement('div');
    el.id = 'murmur-fake-permissions-marker';
    el.setAttribute('role', 'status');
    el.textContent = 'DEV: fake permissions';
    el.style.cssText = 'position:fixed;right:8px;bottom:8px;z-index:2147483647;padding:2px 8px;border-radius:6px;background:#b91c1c;color:#fff;font:600 11px/18px -apple-system,system-ui,sans-serif;pointer-events:none;opacity:.9';
    document.body.appendChild(el);
  };
  if (document.body) add(); else document.addEventListener('DOMContentLoaded', add);
})();"#;

impl Provider {
    fn faked(&self) -> Vec<&'static str> {
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        Permission::ALL
            .iter()
            .filter(|p| entries.contains_key(p))
            .map(|p| p.name())
            .collect()
    }

    pub(crate) fn state(&self, permission: Permission) -> Option<FakeState> {
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.get(&permission).map(|entry| entry.state)
    }

    /// Start a request. `None` when `permission` is not faked.
    pub(crate) fn begin_request(&self, permission: Permission) -> Option<Request> {
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let entry = entries.get(&permission)?;
        let prompts = entry.state == FakeState::NotDetermined;
        Some(Request {
            permission,
            prompts,
            delay: if prompts {
                Duration::from_millis(entry.script.request_delay_ms)
            } else {
                Duration::ZERO
            },
            outcome: if prompts {
                entry.script.request
            } else {
                Outcome::NoAnswer
            },
            generation: entry.generation,
        })
    }

    /// Start an Open Settings visit. `None` when `permission` is not faked.
    pub(crate) fn begin_settings(&self, permission: Permission) -> Option<Request> {
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let entry = entries.get(&permission)?;
        Some(Request {
            permission,
            prompts: false,
            delay: Duration::from_millis(entry.script.settings_delay_ms),
            outcome: entry.script.settings,
            generation: entry.generation,
        })
    }

    /// Apply a finished request's answer unless something changed the
    /// permission since it began. Returns the state afterwards.
    pub(crate) fn complete(&self, request: &Request) -> FakeState {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let entry = entries
            .get_mut(&request.permission)
            .expect("a request only exists for a faked permission");
        if entry.generation == request.generation {
            if let Some(state) = request.outcome.state() {
                if state != entry.state {
                    entry.state = state;
                    entry.generation += 1;
                }
            }
        }
        entry.state
    }

    pub(crate) fn opens_settings_pane(&self) -> bool {
        self.open_settings_pane
    }
}

/// One scripted answer in flight.
#[derive(Debug)]
pub(crate) struct Request {
    permission: Permission,
    /// A dialog would show: the permission was still undecided.
    pub prompts: bool,
    pub delay: Duration,
    outcome: Outcome,
    generation: u64,
}

// -- Command hooks -----------------------------------------------------------
//
// Each returns `None` when the permission is not faked, so the caller falls
// through to the real macOS path.

pub(crate) fn state(permission: Permission) -> Option<FakeState> {
    provider()?.state(permission)
}

/// Answer a request after its scripted delay, for the commands that await the
/// answer (Calendar, System Audio).
pub(crate) async fn request_and_wait(permission: Permission) -> Option<FakeState> {
    let provider = provider()?;
    let request = provider.begin_request(permission)?;
    tracing::info!(target: "system", "fake permissions: {} request (prompts: {})", permission.name(), request.prompts);
    tokio::time::sleep(request.delay).await;
    Some(provider.complete(&request))
}

/// Start a request whose answer arrives later, like a TCC dialog. Returns
/// whether the dialog "showed" (the permission was undecided).
pub(crate) fn request_in_background(permission: Permission) -> Option<bool> {
    let provider = provider()?;
    let request = provider.begin_request(permission)?;
    let prompts = request.prompts;
    tracing::info!(target: "system", "fake permissions: {} request (prompts: {prompts})", permission.name());
    std::thread::spawn(move || {
        std::thread::sleep(request.delay);
        provider.complete(&request);
    });
    Some(prompts)
}

/// Simulate an Open Settings visit: after the scripted delay the user grants,
/// denies, or does nothing. Returns whether to open the real pane too.
pub(crate) fn open_settings(permission: Permission) -> Option<bool> {
    let provider = provider()?;
    let request = provider.begin_settings(permission)?;
    tracing::info!(target: "system", "fake permissions: {} Open Settings", permission.name());
    std::thread::spawn(move || {
        std::thread::sleep(request.delay);
        provider.complete(&request);
    });
    Some(provider.opens_settings_pane())
}

/// The fake's answer to `check_microphone_permission_status`.
pub(crate) fn microphone_banner_state(state: FakeState) -> &'static str {
    match state {
        FakeState::Granted => "granted",
        FakeState::Denied | FakeState::Restricted => "denied",
        FakeState::NotDetermined => "notDetermined",
    }
}

pub(crate) fn system_audio_state(
    state: FakeState,
) -> crate::meeting_capture::SystemAudioPermissionState {
    use crate::meeting_capture::SystemAudioPermissionState as S;
    match state {
        FakeState::Granted => S::Granted,
        FakeState::Denied | FakeState::Restricted => S::Denied,
        FakeState::NotDetermined => S::Unknown,
    }
}

pub(crate) fn system_audio_access(state: FakeState) -> crate::meeting_capture::SystemAudioAccess {
    crate::meeting_capture::SystemAudioAccess {
        permission: system_audio_state(state),
        capture_ready: state == FakeState::Granted,
        audio_flowing: false,
        needs_relaunch: false,
    }
}

pub(crate) fn calendar_state(state: FakeState) -> crate::calendar::CalendarPermissionStatus {
    use crate::calendar::CalendarPermissionStatus as S;
    match state {
        FakeState::Granted => S::Granted,
        FakeState::Denied => S::Denied,
        FakeState::Restricted => S::Restricted,
        FakeState::NotDetermined => S::NotDetermined,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instant(start: FakeState, request: Outcome, settings: Outcome) -> Script {
        Script {
            start,
            request,
            request_delay_ms: 0,
            settings,
            settings_delay_ms: 0,
        }
    }

    fn provider_with(permission: Permission, script: Script) -> Provider {
        let mut entries = HashMap::new();
        entries.insert(
            permission,
            Entry {
                script,
                state: script.start,
                generation: 0,
            },
        );
        Provider {
            entries: Mutex::new(entries),
            open_settings_pane: false,
        }
    }

    #[test]
    fn presets_fake_every_permission() {
        for name in ["fresh", "granted", "denied", "no-answer"] {
            let provider = parse(name).unwrap();
            assert_eq!(
                provider.faked(),
                ["microphone", "accessibility", "systemAudio", "calendar"]
            );
        }
        let granted = parse("granted").unwrap();
        assert_eq!(
            granted.state(Permission::Calendar),
            Some(FakeState::Granted)
        );
        let fresh = parse("fresh").unwrap();
        assert_eq!(
            fresh.state(Permission::Microphone),
            Some(FakeState::NotDetermined)
        );
    }

    #[test]
    fn inline_json_fakes_only_what_it_names_and_fills_defaults() {
        let provider = parse(
            r#"{"microphone":{"start":"denied","settings":"noAnswer"},"openSettingsPane":false}"#,
        )
        .unwrap();
        assert_eq!(provider.faked(), ["microphone"]);
        assert_eq!(
            provider.state(Permission::Microphone),
            Some(FakeState::Denied)
        );
        assert_eq!(provider.state(Permission::Accessibility), None);
        assert!(!provider.opens_settings_pane());
        let entries = provider.entries.lock().unwrap();
        let script = entries[&Permission::Microphone].script;
        assert_eq!(script.request, Outcome::Granted);
        assert_eq!(script.request_delay_ms, 1_000);
        assert_eq!(script.settings, Outcome::NoAnswer);
    }

    #[test]
    fn bad_scripts_are_refused() {
        assert!(parse("{").is_err());
        assert!(parse("{}").is_err(), "a script that fakes nothing");
        assert!(parse(r#"{"microphone":{"start":"maybe"}}"#).is_err());
        assert!(
            parse(r#"{"microphone":{"delay":5}}"#).is_err(),
            "unknown field"
        );
        assert!(parse("/nonexistent/murmur-fake-permissions.json").is_err());
    }

    #[test]
    fn script_file_is_read() {
        let dir = std::env::temp_dir().join(format!("murmur-fake-perm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("script.json");
        std::fs::write(&path, r#"{"calendar":{"start":"restricted"}}"#).unwrap();
        let provider = parse(path.to_str().unwrap()).unwrap();
        assert_eq!(
            provider.state(Permission::Calendar),
            Some(FakeState::Restricted)
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn undecided_request_takes_the_scripted_answer() {
        for (outcome, after) in [
            (Outcome::Granted, FakeState::Granted),
            (Outcome::Denied, FakeState::Denied),
            (Outcome::NoAnswer, FakeState::NotDetermined),
        ] {
            let provider = provider_with(
                Permission::Microphone,
                instant(FakeState::NotDetermined, outcome, Outcome::NoAnswer),
            );
            let request = provider.begin_request(Permission::Microphone).unwrap();
            assert!(request.prompts);
            assert_eq!(provider.complete(&request), after);
        }
    }

    #[test]
    fn decided_request_does_not_prompt_or_change() {
        let provider = provider_with(
            Permission::Microphone,
            instant(FakeState::Denied, Outcome::Granted, Outcome::NoAnswer),
        );
        let request = provider.begin_request(Permission::Microphone).unwrap();
        assert!(!request.prompts);
        assert_eq!(request.delay, Duration::ZERO);
        assert_eq!(provider.complete(&request), FakeState::Denied);
    }

    #[test]
    fn settings_visit_recovers_a_denial() {
        let provider = provider_with(
            Permission::Accessibility,
            instant(FakeState::Denied, Outcome::NoAnswer, Outcome::Granted),
        );
        let visit = provider.begin_settings(Permission::Accessibility).unwrap();
        assert_eq!(provider.complete(&visit), FakeState::Granted);
    }

    #[test]
    fn a_stale_answer_does_not_overwrite_a_newer_one() {
        let provider = provider_with(
            Permission::Calendar,
            instant(FakeState::NotDetermined, Outcome::Granted, Outcome::Denied),
        );
        let request = provider.begin_request(Permission::Calendar).unwrap();
        let visit = provider.begin_settings(Permission::Calendar).unwrap();
        assert_eq!(provider.complete(&visit), FakeState::Denied);
        assert_eq!(provider.complete(&request), FakeState::Denied);
    }

    #[test]
    fn unfaked_permissions_fall_through() {
        let provider = provider_with(Permission::Microphone, Script::default());
        assert!(provider.begin_request(Permission::Calendar).is_none());
        assert!(provider.begin_settings(Permission::SystemAudio).is_none());
    }

    /// Counterpart of `fake_permissions_default_tests`: the needles that a
    /// default build must not contain are present in this build.
    #[test]
    fn the_symbol_check_needles_are_in_a_fake_build() {
        let binary = std::fs::read(std::env::current_exe().unwrap()).unwrap();
        for needle in [
            "DEV FAKE PERMISSIONS ACTIVE",
            "murmur-fake-permissions-marker",
            "fake permissions not active: ",
            "Fake permissions are active (MURMUR_FAKE",
        ] {
            let needle = needle.as_bytes();
            assert!(
                binary.windows(needle.len()).any(|window| window == needle),
                "{:?} missing from the fake build",
                String::from_utf8_lossy(needle)
            );
        }
    }

    #[test]
    fn states_map_onto_each_command_type() {
        assert_eq!(microphone_banner_state(FakeState::Restricted), "denied");
        assert_eq!(
            microphone_banner_state(FakeState::NotDetermined),
            "notDetermined"
        );
        assert_eq!(
            serde_json::to_value(system_audio_access(FakeState::Granted)).unwrap(),
            serde_json::json!({"permission":"granted","captureReady":true,"audioFlowing":false,"needsRelaunch":false})
        );
        assert_eq!(
            serde_json::to_value(calendar_state(FakeState::Restricted)).unwrap(),
            "restricted"
        );
    }
}
