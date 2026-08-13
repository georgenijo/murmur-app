use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::fs;
use std::path::Path;

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInstallEnvironment {
    app_translocated: bool,
}

const CANARY_ENV: &str = "MURMUR_UPDATER_CANARY";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdaterCanaryRequest {
    pub action: String,
    pub result: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdaterCanaryState {
    pub path: Option<String>,
    pub result: Option<serde_json::Value>,
}

fn canary_path_from_env<F>(get: F) -> Option<String>
where
    F: FnOnce(&str) -> Option<std::ffi::OsString>,
{
    get(CANARY_ENV)
        .filter(|path| !path.is_empty())
        .map(|path| path.to_string_lossy().into_owned())
}

fn canary_path() -> Option<String> {
    canary_path_from_env(|key| std::env::var_os(key))
}

fn read_canary_result(path: &Path) -> Option<serde_json::Value> {
    fs::read_to_string(path)
        .ok()
        .and_then(|contents| serde_json::from_str(&contents).ok())
}

fn write_canary_result(path: &Path, result: &serde_json::Value) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Canary result path has no parent directory".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create canary result directory: {error}"))?;
    let temporary = path.with_extension(format!("json.tmp-{}", std::process::id()));
    let bytes = serde_json::to_vec_pretty(result)
        .map_err(|error| format!("Could not encode canary result: {error}"))?;
    fs::write(&temporary, bytes)
        .map_err(|error| format!("Could not write canary result: {error}"))?;
    fs::rename(&temporary, path)
        .map_err(|error| format!("Could not publish canary result: {error}"))
}

/// Read or atomically update the opt-in OTA canary result file.
///
/// The environment variable is intentionally read by Rust: WebKit has no
/// ambient process environment, and an absent variable returns an inert state.
#[tauri::command]
pub fn updater_canary(request: UpdaterCanaryRequest) -> Result<UpdaterCanaryState, String> {
    let path = canary_path();
    if request.action == "write" {
        let path = path
            .as_deref()
            .ok_or_else(|| "Updater canary is not enabled".to_string())?;
        let result = request
            .result
            .ok_or_else(|| "Canary write requires a result".to_string())?;
        write_canary_result(Path::new(path), &result)?;
    } else if request.action != "read" {
        return Err(format!("Unknown updater canary action: {}", request.action));
    }
    let result = path
        .as_deref()
        .and_then(|path| read_canary_result(Path::new(path)));
    Ok(UpdaterCanaryState { path, result })
}

fn is_app_translocated(path: &Path) -> bool {
    path.components()
        .any(|component| component.as_os_str() == OsStr::new("AppTranslocation"))
}

#[tauri::command]
pub fn get_update_install_environment() -> Result<UpdateInstallEnvironment, String> {
    let executable = std::env::current_exe()
        .map_err(|_| "Could not verify the update installation location".to_string())?;
    let app_translocated = is_app_translocated(&executable);

    Ok(UpdateInstallEnvironment { app_translocated })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_gatekeeper_app_translocation_path() {
        let path = Path::new(
            "/private/var/folders/example/T/AppTranslocation/UUID/d/Murmur.app/Contents/MacOS/ui",
        );
        assert!(is_app_translocated(path));
    }

    #[test]
    fn accepts_normal_applications_path() {
        let path = Path::new("/Applications/Murmur.app/Contents/MacOS/ui");
        assert!(!is_app_translocated(path));
    }

    #[test]
    fn does_not_match_similar_component_names() {
        let path = Path::new("/tmp/MyAppTranslocation/Murmur.app/Contents/MacOS/ui");
        assert!(!is_app_translocated(path));
    }

    #[test]
    fn canary_path_is_inert_when_environment_variable_is_absent() {
        assert_eq!(canary_path_from_env(|_| None), None);
    }

    #[test]
    fn canary_path_ignores_empty_environment_variable() {
        assert_eq!(
            canary_path_from_env(|_| Some(std::ffi::OsString::new())),
            None
        );
    }
}
