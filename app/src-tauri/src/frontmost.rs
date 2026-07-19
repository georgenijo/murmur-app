//! Frontmost-app detection used by per-app dictation profiles.
//!
//! Reuses the osascript approach from `injector.rs` to ask System Events for
//! the bundle identifier of whatever app currently owns the keyboard focus.
//! Detection is best-effort: any failure (osascript missing, permission denied,
//! empty result) returns `None` so callers fall back to global behaviour.

/// Return the bundle identifier of the frontmost macOS app, e.g.
/// `"com.apple.Terminal"`. Returns `None` on any failure so the caller can fall
/// back to a global-only dictation context.
#[cfg(target_os = "macos")]
pub fn frontmost_bundle_id() -> Option<String> {
    let output = match crate::injector::run_osascript_with_timeout(
        r#"tell application "System Events" to get bundle identifier of first process whose frontmost is true"#,
    ) {
        Ok(output) => output,
        Err(error) => {
            tracing::warn!(target: "pipeline", "frontmost_bundle_id: {error}");
            return None;
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::warn!(target: "pipeline", "frontmost_bundle_id: osascript failed: {}", stderr.trim());
        return None;
    }

    let bundle_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if bundle_id.is_empty() {
        None
    } else {
        Some(bundle_id)
    }
}

/// Non-macOS platforms have no frontmost-app concept here; profiles are a no-op.
#[cfg(not(target_os = "macos"))]
pub fn frontmost_bundle_id() -> Option<String> {
    None
}
