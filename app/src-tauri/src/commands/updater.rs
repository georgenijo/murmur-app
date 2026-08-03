use serde::Serialize;
use std::ffi::OsStr;
use std::path::Path;

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInstallEnvironment {
    app_translocated: bool,
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
}
