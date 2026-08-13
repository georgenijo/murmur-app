//! Declared environment variables for the voice-query CLI (#550).
//!
//! `spawn_user_cli` forwards a fixed, fail-closed allowlist and nothing else,
//! which is right for secrets but wrong for the handful of pairs a provider CLI
//! genuinely needs to find its own configuration (`CLAUDE_CONFIG_DIR`,
//! `CODEX_HOME`, …). This module is the narrow, explicit exception: the user
//! names each pair, the pairs are validated here, and they are layered
//! *underneath* the inherited allowlist so nothing declared can shadow `HOME`
//! or any other allowlist key.
//!
//! The pairs live in a Rust-owned `query-env.json` (0600) rather than the
//! frontend settings blob, so they are never mirrored into localStorage and are
//! not readable by any webview that has not been handed them. Values are stored
//! in plain text: this surface is for configuration, not credentials, and
//! Settings says so. Accepting secrets needs a Keychain-backed design first
//! (see `docs/decisions/DECISIONS.md`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::Manager;

pub(crate) const MAX_DECLARED_VARIABLES: usize = 16;
const MAX_NAME_BYTES: usize = 128;
const MAX_VALUE_BYTES: usize = 4096;
const MAX_FILE_BYTES: u64 = 128 * 1024;
const FILE_NAME: &str = "query-env.json";
const MAIN_WINDOW_LABEL: &str = "main";

/// Prefixes that let a declared pair inject code into the child process rather
/// than configure it. Refused outright, on every platform.
const DENIED_PREFIXES: [&str; 2] = ["DYLD_", "LD_"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DeclaredEnvVar {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct DeclaredEnvFile {
    #[serde(default)]
    variables: Vec<DeclaredEnvVar>,
}

fn require_main_window(label: &str) -> Result<(), String> {
    if label == MAIN_WINDOW_LABEL {
        Ok(())
    } else {
        Err(
            "Voice Query environment variables are only available from the main window."
                .to_string(),
        )
    }
}

fn valid_name(name: &str) -> bool {
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

/// Validate the declared set as a whole and return it as spawn-ready pairs.
///
/// Every refusal names the offending variable so Settings can point at the row
/// the user has to fix, and never echoes the value.
pub(crate) fn validate(variables: &[DeclaredEnvVar]) -> Result<Vec<(String, String)>, String> {
    if variables.len() > MAX_DECLARED_VARIABLES {
        return Err(format!(
            "Voice Query accepts at most {MAX_DECLARED_VARIABLES} declared environment variables."
        ));
    }
    let mut pairs: Vec<(String, String)> = Vec::with_capacity(variables.len());
    for variable in variables {
        let name = variable.name.trim();
        if name.is_empty() {
            return Err("Every declared environment variable needs a name.".to_string());
        }
        if name.len() > MAX_NAME_BYTES || !valid_name(name) {
            return Err(format!(
                "“{name}” is not a valid environment variable name. Use letters, digits, and underscores."
            ));
        }
        if crate::managed_child::USER_CLI_ENVIRONMENT_ALLOWLIST.contains(&name) {
            return Err(format!(
                "{name} is forwarded by Murmur itself and cannot be redeclared."
            ));
        }
        if DENIED_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix))
        {
            return Err(format!(
                "{name} can change which code the CLI loads and is not allowed."
            ));
        }
        if pairs.iter().any(|(existing, _)| existing == name) {
            return Err(format!("{name} is declared more than once."));
        }
        if variable.value.len() > MAX_VALUE_BYTES {
            return Err(format!(
                "The value for {name} exceeds the {MAX_VALUE_BYTES} byte limit."
            ));
        }
        if variable.value.contains('\0') {
            return Err(format!("The value for {name} contains a null byte."));
        }
        pairs.push((name.to_string(), variable.value.clone()));
    }
    Ok(pairs)
}

fn store_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Durable data directory unavailable: {e}"))?;
    Ok(dir.join(FILE_NAME))
}

fn read_file(path: &Path) -> Result<Vec<DeclaredEnvVar>, String> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("Failed to read Voice Query environment: {e}")),
    };
    if metadata.len() > MAX_FILE_BYTES {
        return Err("The Voice Query environment file is too large to read.".to_string());
    }
    let contents = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read Voice Query environment: {e}"))?;
    let parsed: DeclaredEnvFile = serde_json::from_str(&contents)
        .map_err(|_| "The Voice Query environment file is not valid JSON.".to_string())?;
    // A hand-edited or tampered file is not trusted any more than the UI is.
    validate(&parsed.variables)?;
    Ok(parsed.variables)
}

fn write_file(path: &Path, variables: &[DeclaredEnvVar]) -> Result<(), String> {
    let directory = path
        .parent()
        .ok_or_else(|| "Durable data directory unavailable.".to_string())?;
    std::fs::create_dir_all(directory)
        .map_err(|e| format!("Failed to create durable data directory: {e}"))?;
    let blob = serde_json::to_string(&DeclaredEnvFile {
        variables: variables.to_vec(),
    })
    .map_err(|e| format!("Failed to encode Voice Query environment: {e}"))?;
    let temp = path.with_file_name(format!(".{FILE_NAME}.murmur-tmp"));
    if let Err(e) = std::fs::write(&temp, blob) {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("Failed to write Voice Query environment: {e}"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o600)) {
            let _ = std::fs::remove_file(&temp);
            return Err(format!("Failed to protect Voice Query environment: {e}"));
        }
    }
    if let Err(e) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("Failed to publish Voice Query environment: {e}"));
    }
    Ok(())
}

/// Spawn-ready declared pairs. A corrupt or refused file yields no pairs rather
/// than failing the query: the CLI still runs with the inherited allowlist.
pub(crate) fn spawn_pairs(app: &tauri::AppHandle) -> Vec<(String, String)> {
    let Ok(path) = store_path(app) else {
        return Vec::new();
    };
    match read_file(&path).and_then(|variables| validate(&variables)) {
        Ok(pairs) => pairs,
        Err(_) => {
            tracing::warn!(
                target: "query",
                event_code = "query.declared_env_unusable",
                "declared voice-query environment could not be used"
            );
            Vec::new()
        }
    }
}

#[tauri::command]
pub(crate) fn load_query_env_vars(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
) -> Result<Vec<DeclaredEnvVar>, String> {
    require_main_window(window.label())?;
    read_file(&store_path(&app)?)
}

#[tauri::command]
pub(crate) fn save_query_env_vars(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    variables: Vec<DeclaredEnvVar>,
) -> Result<(), String> {
    require_main_window(window.label())?;
    validate(&variables)?;
    write_file(&store_path(&app)?, &variables)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declared(name: &str, value: &str) -> DeclaredEnvVar {
        DeclaredEnvVar {
            name: name.to_string(),
            value: value.to_string(),
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "murmur_query_env_test_{}_{tag}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn accepts_the_provider_configuration_pairs_presets_suggest() {
        let pairs = validate(&[
            declared("CLAUDE_CONFIG_DIR", "/Users/someone/.claude"),
            declared("CODEX_HOME", "/Users/someone/.codex"),
        ])
        .unwrap();
        assert_eq!(
            pairs,
            vec![
                (
                    "CLAUDE_CONFIG_DIR".to_string(),
                    "/Users/someone/.claude".to_string()
                ),
                (
                    "CODEX_HOME".to_string(),
                    "/Users/someone/.codex".to_string()
                ),
            ]
        );
    }

    #[test]
    fn refuses_home_and_every_other_allowlist_key() {
        for key in crate::managed_child::USER_CLI_ENVIRONMENT_ALLOWLIST {
            let error = validate(&[declared(key, "/tmp/anything")]).unwrap_err();
            assert!(error.contains(key), "{error}");
        }
    }

    #[test]
    fn refuses_dynamic_linker_injection_and_malformed_names() {
        assert!(validate(&[declared("DYLD_INSERT_LIBRARIES", "/tmp/evil.dylib")]).is_err());
        assert!(validate(&[declared("LD_PRELOAD", "/tmp/evil.so")]).is_err());
        assert!(validate(&[declared("1BAD", "x")]).is_err());
        assert!(validate(&[declared("HAS SPACE", "x")]).is_err());
        assert!(validate(&[declared("HAS=EQUALS", "x")]).is_err());
        assert!(validate(&[declared("", "x")]).is_err());
    }

    #[test]
    fn refuses_duplicates_and_oversized_entries() {
        assert!(validate(&[declared("CODEX_HOME", "/a"), declared("CODEX_HOME", "/b")]).is_err());
        assert!(validate(&[declared("CODEX_HOME", &"a".repeat(MAX_VALUE_BYTES + 1))]).is_err());
        assert!(validate(&[declared("CODEX_HOME", "a\0b")]).is_err());
        let too_many: Vec<DeclaredEnvVar> = (0..=MAX_DECLARED_VARIABLES)
            .map(|index| declared(&format!("VAR_{index}"), "x"))
            .collect();
        assert!(validate(&too_many).is_err());
    }

    #[test]
    fn round_trips_through_an_owner_only_file() {
        let dir = temp_dir("round_trip");
        let path = dir.join(FILE_NAME);
        assert_eq!(read_file(&path).unwrap(), Vec::new());

        let variables = vec![declared("CLAUDE_CONFIG_DIR", "/Users/someone/.claude")];
        write_file(&path, &variables).unwrap();
        assert_eq!(read_file(&path).unwrap(), variables);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }

        write_file(&path, &[]).unwrap();
        assert_eq!(read_file(&path).unwrap(), Vec::new());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_tampered_file_is_refused_rather_than_trusted() {
        let dir = temp_dir("tampered");
        let path = dir.join(FILE_NAME);
        std::fs::write(
            &path,
            r#"{"variables":[{"name":"HOME","value":"/tmp/evil"}]}"#,
        )
        .unwrap();
        assert!(read_file(&path).is_err());
        std::fs::write(&path, "not json at all").unwrap();
        assert!(read_file(&path).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
