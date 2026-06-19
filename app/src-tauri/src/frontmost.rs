//! Frontmost-app detection used by per-app dictation profiles.
//!
//! Reuses the osascript approach from `injector.rs` to ask System Events for
//! the bundle identifier of whatever app currently owns the keyboard focus.
//! Detection is best-effort: any failure (osascript missing, permission denied,
//! empty result) returns `None` so callers fall back to global behaviour.

/// Return the bundle identifier of the frontmost macOS app, e.g.
/// `"com.apple.Terminal"`. Returns `None` on any failure so the caller can fall
/// back to the global auto-paste setting.
#[cfg(target_os = "macos")]
pub fn frontmost_bundle_id() -> Option<String> {
    use std::process::Command;

    let output = Command::new("osascript")
        .arg("-e")
        .arg(r#"tell application "System Events" to get bundle identifier of first process whose frontmost is true"#)
        .output()
        .ok()?;

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

/// Resolve the effective auto-paste setting for the frontmost app.
///
/// Given the global `auto_paste` value, the detected frontmost `bundle_id`
/// (if any), and the configured profiles, returns the override from the first
/// matching profile that sets one, otherwise the global value. Kept pure so it
/// can be unit-tested without invoking osascript.
pub fn resolve_auto_paste(
    auto_paste: bool,
    bundle_id: Option<&str>,
    profiles: &[crate::state::AppProfile],
) -> bool {
    let Some(bundle_id) = bundle_id else {
        return auto_paste;
    };
    for profile in profiles {
        if profile.bundle_id == bundle_id {
            if let Some(override_value) = profile.auto_paste_override {
                return override_value;
            }
        }
    }
    auto_paste
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppProfile;

    fn profile(bundle_id: &str, auto_paste_override: Option<bool>) -> AppProfile {
        AppProfile {
            bundle_id: bundle_id.to_string(),
            label: bundle_id.to_string(),
            auto_paste_override,
        }
    }

    #[test]
    fn no_frontmost_uses_global() {
        let profiles = vec![profile("com.apple.Terminal", Some(false))];
        assert!(resolve_auto_paste(true, None, &profiles));
        assert!(!resolve_auto_paste(false, None, &profiles));
    }

    #[test]
    fn unmatched_app_uses_global() {
        let profiles = vec![profile("com.apple.Terminal", Some(false))];
        assert!(resolve_auto_paste(true, Some("com.apple.Safari"), &profiles));
    }

    #[test]
    fn matching_profile_override_wins() {
        let profiles = vec![profile("com.apple.Terminal", Some(false))];
        assert!(!resolve_auto_paste(true, Some("com.apple.Terminal"), &profiles));
    }

    #[test]
    fn matching_profile_can_force_on() {
        let profiles = vec![profile("com.googlecode.iterm2", Some(true))];
        assert!(resolve_auto_paste(false, Some("com.googlecode.iterm2"), &profiles));
    }

    #[test]
    fn null_override_falls_through_to_global() {
        let profiles = vec![profile("com.apple.Terminal", None)];
        assert!(resolve_auto_paste(true, Some("com.apple.Terminal"), &profiles));
        assert!(!resolve_auto_paste(false, Some("com.apple.Terminal"), &profiles));
    }

    #[test]
    fn first_matching_override_wins() {
        let profiles = vec![
            profile("com.apple.Terminal", None),
            profile("com.apple.Terminal", Some(false)),
        ];
        // First profile has no override, so we fall through to the second.
        assert!(!resolve_auto_paste(true, Some("com.apple.Terminal"), &profiles));
    }
}
