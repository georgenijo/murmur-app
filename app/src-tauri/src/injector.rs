use arboard::Clipboard;

/// Copy text to clipboard and optionally simulate Cmd+V paste.
/// `delay_ms` controls the pause before pasting (window focus settling).
/// On paste failure, retries once after a 100ms backoff.
pub fn inject_text(text: &str, auto_paste: bool, delay_ms: u64) -> Result<(), String> {
    tracing::info!(target: "pipeline", "inject_text called with auto_paste={}, delay_ms={}, text_len={}", auto_paste, delay_ms, text.len());

    // Skip if text is empty
    if text.trim().is_empty() {
        tracing::info!(target: "pipeline", "inject_text: text is empty, skipping");
        return Ok(());
    }

    let mut clipboard = Clipboard::new()
        .map_err(|e| format!("Failed to access clipboard: {}", e))?;

    // Copy transcription to clipboard
    clipboard.set_text(text)
        .map_err(|e| format!("Failed to copy to clipboard: {}", e))?;
    tracing::info!(target: "pipeline", "inject_text: text copied to clipboard");

    // If auto-paste is disabled, we're done
    if !auto_paste {
        return Ok(());
    }

    {
        use std::thread;
        use std::time::Duration;

        // Check accessibility permission before attempting paste simulation (macOS only)
        if !is_accessibility_enabled() {
            tracing::warn!(target: "pipeline", "inject_text: accessibility permission not granted — text in clipboard only");
            return Ok(());
        }

        // Wait for window focus to settle
        thread::sleep(Duration::from_millis(delay_ms));

        // Guard against pasting when nothing editable is focused (e.g. Finder
        // desktop). A synthetic Cmd+V there drops a stray .textClipping file
        // instead of pasting. Only skip when we POSITIVELY determine the focused
        // element is non-editable; on any uncertainty we allow the paste so the
        // common "a field is focused" case is never broken. See
        // `focused_field_state` for the false-negative bias.
        if focused_field_state() == FocusedFieldState::NonEditable {
            tracing::warn!(target: "pipeline", "inject_text: focused element is not an editable text field — skipping paste, text in clipboard only");
            return Err("No editable text field is focused".to_string());
        }

        // Simulate paste keystroke, retry once on failure
        match simulate_paste() {
            Ok(()) => Ok(()),
            Err(first_err) => {
                tracing::warn!(target: "pipeline", "inject_text: first paste attempt failed: {}, retrying in 100ms", first_err);
                thread::sleep(Duration::from_millis(100));
                simulate_paste().map_err(|retry_err| {
                    format!("Auto-paste failed after retry: {}", retry_err)
                })
            }
        }
    }
}

/// Simulate Cmd+V keystroke using osascript (most reliable on macOS Sonoma/Sequoia)
#[cfg(target_os = "macos")]
fn simulate_paste() -> Result<(), String> {
    use std::process::Command;

    tracing::info!(target: "pipeline", "simulate_paste: using osascript to simulate Cmd+V");

    let output = Command::new("osascript")
        .arg("-e")
        .arg(r#"tell application "System Events" to keystroke "v" using command down"#)
        .output()
        .map_err(|e| format!("Failed to run osascript: {}", e))?;

    if output.status.success() {
        tracing::info!(target: "pipeline", "simulate_paste: completed successfully");
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("osascript failed: {}", stderr))
    }
}

/// Result of inspecting whatever UI element currently owns keyboard focus.
///
/// `Unknown` covers every uncertain case (AX query failed, permission denied,
/// empty/unrecognised role). The paste guard treats `Unknown` exactly like
/// `Editable` — it only skips the paste on a positive `NonEditable` reading.
///
/// On non-macOS builds the `Editable`/`NonEditable` variants are only
/// constructed by `classify_focused_role`, which there is test-only.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FocusedFieldState {
    /// Focused element is a clearly editable text control — paste is safe.
    Editable,
    /// Focused element is positively non-editable (button, image, desktop, …).
    NonEditable,
    /// Could not determine — default to allowing the paste (current behavior).
    Unknown,
}

/// Classify an Accessibility role string as an editable text control.
///
/// Returns `true` for roles that accept typed/pasted text (text fields, text
/// areas, combo/search boxes, etc.) and `false` for everything else. Kept pure
/// (no I/O) so it can be unit-tested without invoking osascript. Matching is
/// exact against the AX role constants reported by System Events.
///
/// Reached only via `classify_focused_role` (macOS) or unit tests; suppress the
/// dead-code lint on non-macOS non-test builds.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn is_editable_text_role(role: &str) -> bool {
    matches!(
        role.trim(),
        "AXTextField"
            | "AXTextArea"
            | "AXComboBox"
            | "AXSearchField"
            | "AXSecureTextField"
            | "AXTokenField"
    )
}

/// Determine whether the frontmost app currently has an editable text element
/// focused, returning a tri-state so callers can apply a false-negative bias.
///
/// Asks System Events for the AX role of `AXFocusedUIElement` of the frontmost
/// process. Any failure (osascript missing, permission denied, no focused
/// element, unrecognised role) yields `Unknown` so the caller can default to
/// allowing the paste — we only ever return `NonEditable` when the role is read
/// successfully AND is positively not an editable text control.
#[cfg(target_os = "macos")]
fn focused_field_state() -> FocusedFieldState {
    use std::process::Command;

    // `missing value` (AppleScript's null) is returned when there is no focused
    // element; we map both that and the empty string to Unknown below.
    let script = r#"tell application "System Events"
    set frontApp to first process whose frontmost is true
    try
        set focused to value of attribute "AXFocusedUIElement" of frontApp
        return role of focused
    on error
        return ""
    end try
end tell"#;

    let output = match Command::new("osascript").arg("-e").arg(script).output() {
        Ok(output) => output,
        Err(e) => {
            tracing::warn!(target: "pipeline", "focused_field_state: failed to run osascript: {}", e);
            return FocusedFieldState::Unknown;
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::warn!(target: "pipeline", "focused_field_state: osascript failed: {}", stderr.trim());
        return FocusedFieldState::Unknown;
    }

    let role = String::from_utf8_lossy(&output.stdout).trim().to_string();
    classify_focused_role(&role)
}

/// Non-macOS platforms have no AX focus concept; never skip the paste here.
#[cfg(not(target_os = "macos"))]
fn focused_field_state() -> FocusedFieldState {
    FocusedFieldState::Unknown
}

/// Map an AX role string (as emitted by `focused_field_state`'s osascript) to a
/// `FocusedFieldState`. Pure, so it is exercised directly by unit tests.
///
/// An empty/`missing value` result means "no focused element / query failed" →
/// `Unknown`. A recognised editable role → `Editable`. Any other non-empty role
/// → `NonEditable`.
///
/// Only invoked from the macOS `focused_field_state`; on other platforms it is
/// reached solely via unit tests, so suppress the dead-code lint there.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn classify_focused_role(role: &str) -> FocusedFieldState {
    let role = role.trim();
    if role.is_empty() || role == "missing value" {
        return FocusedFieldState::Unknown;
    }
    if is_editable_text_role(role) {
        FocusedFieldState::Editable
    } else {
        FocusedFieldState::NonEditable
    }
}

/// Simulate Ctrl+V keystroke on Linux, supporting both X11 (xdotool) and Wayland (wtype).
/// Detects Wayland via WAYLAND_DISPLAY; falls back gracefully when tools are not installed.
#[cfg(target_os = "linux")]
fn simulate_paste() -> Result<(), String> {
    simulate_paste_linux(
        |key| std::env::var_os(key),
        |program, args| std::process::Command::new(program).args(args).output(),
    )
}

#[cfg(target_os = "linux")]
fn simulate_paste_linux<F, G>(env_get: F, mut runner: G) -> Result<(), String>
where
    F: Fn(&str) -> Option<std::ffi::OsString>,
    G: FnMut(&str, &[&str]) -> std::io::Result<std::process::Output>,
{
    let is_wayland = env_get("WAYLAND_DISPLAY")
        .map(|v| !v.is_empty())
        .unwrap_or(false);

    let wayland_candidates: [(&str, &[&str]); 2] = [
        ("wtype", &["-M", "ctrl", "-k", "v"]),
        ("xdotool", &["key", "ctrl+v"]),
    ];
    let x11_candidates: [(&str, &[&str]); 1] = [("xdotool", &["key", "ctrl+v"])];
    let candidates: &[(&str, &[&str])] = if is_wayland {
        &wayland_candidates
    } else {
        &x11_candidates
    };

    for (program, args) in candidates {
        tracing::info!(
            target: "pipeline",
            "simulate_paste: trying {} ({})",
            program,
            if is_wayland { "Wayland" } else { "X11" }
        );
        match runner(program, args) {
            Ok(output) if output.status.success() => {
                tracing::info!(target: "pipeline", "simulate_paste: {} completed successfully", program);
                return Ok(());
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(format!("{} failed: {}", program, stderr));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::warn!(
                    target: "pipeline",
                    "simulate_paste: {} not installed, trying next fallback",
                    program
                );
            }
            Err(e) => {
                return Err(format!("Failed to run {}: {}", program, e));
            }
        }
    }

    tracing::warn!(
        target: "pipeline",
        "simulate_paste: no paste tool available (install xdotool or wtype) — text remains in clipboard"
    );
    Ok(())
}

/// Check if accessibility permission is granted (macOS)
pub fn is_accessibility_enabled() -> bool {
    #[cfg(target_os = "macos")]
    {
        extern "C" {
            fn AXIsProcessTrusted() -> bool;
        }
        unsafe { AXIsProcessTrusted() }
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::ffi::OsString;
    use std::io;
    use std::os::unix::process::ExitStatusExt;
    use std::process::{ExitStatus, Output};

    fn ok_output() -> io::Result<Output> {
        Ok(Output {
            status: ExitStatus::from_raw(0),
            stdout: vec![],
            stderr: vec![],
        })
    }

    fn fail_output(stderr: &str) -> io::Result<Output> {
        Ok(Output {
            status: ExitStatus::from_raw(1 << 8),
            stdout: vec![],
            stderr: stderr.as_bytes().to_vec(),
        })
    }

    fn not_found_err() -> io::Result<Output> {
        Err(io::Error::new(io::ErrorKind::NotFound, "not found"))
    }

    fn other_err() -> io::Result<Output> {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied"))
    }

    fn env_with(key: &str, val: &str) -> impl Fn(&str) -> Option<OsString> {
        let mut map: HashMap<String, OsString> = HashMap::new();
        map.insert(key.to_string(), OsString::from(val));
        move |k| map.get(k).cloned()
    }

    fn empty_env() -> impl Fn(&str) -> Option<OsString> {
        |_| None
    }

    #[test]
    fn x11_uses_xdotool_ctrl_v() {
        let calls: RefCell<Vec<(String, Vec<String>)>> = RefCell::new(Vec::new());
        let result = simulate_paste_linux(empty_env(), |program, args| {
            calls
                .borrow_mut()
                .push((program.to_string(), args.iter().map(|s| s.to_string()).collect()));
            ok_output()
        });
        assert!(result.is_ok());
        let calls = calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "xdotool");
        assert_eq!(calls[0].1, vec!["key", "ctrl+v"]);
    }

    #[test]
    fn x11_xdotool_not_installed_falls_back_silently() {
        let calls: RefCell<Vec<String>> = RefCell::new(Vec::new());
        let result = simulate_paste_linux(empty_env(), |program, _args| {
            calls.borrow_mut().push(program.to_string());
            not_found_err()
        });
        assert!(result.is_ok());
        let calls = calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0], "xdotool");
    }

    #[test]
    fn x11_xdotool_exit_failure_returns_err() {
        let result = simulate_paste_linux(empty_env(), |_program, _args| {
            fail_output("some error")
        });
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("xdotool failed"), "expected 'xdotool failed' in: {}", msg);
    }

    #[test]
    fn wayland_prefers_wtype() {
        let calls: RefCell<Vec<(String, Vec<String>)>> = RefCell::new(Vec::new());
        let result = simulate_paste_linux(env_with("WAYLAND_DISPLAY", "wayland-0"), |program, args| {
            calls
                .borrow_mut()
                .push((program.to_string(), args.iter().map(|s| s.to_string()).collect()));
            ok_output()
        });
        assert!(result.is_ok());
        let calls = calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "wtype");
        assert_eq!(calls[0].1, vec!["-M", "ctrl", "-k", "v"]);
    }

    #[test]
    fn wayland_falls_back_to_xdotool_when_wtype_missing() {
        let calls: RefCell<Vec<(String, Vec<String>)>> = RefCell::new(Vec::new());
        let result = simulate_paste_linux(env_with("WAYLAND_DISPLAY", "wayland-0"), |program, args| {
            calls
                .borrow_mut()
                .push((program.to_string(), args.iter().map(|s| s.to_string()).collect()));
            if program == "wtype" {
                not_found_err()
            } else {
                ok_output()
            }
        });
        assert!(result.is_ok());
        let calls = calls.borrow();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, "wtype");
        assert_eq!(calls[1].0, "xdotool");
        assert_eq!(calls[1].1, vec!["key", "ctrl+v"]);
    }

    #[test]
    fn wayland_both_missing_is_graceful_ok() {
        let calls: RefCell<Vec<String>> = RefCell::new(Vec::new());
        let result = simulate_paste_linux(env_with("WAYLAND_DISPLAY", "wayland-0"), |program, _args| {
            calls.borrow_mut().push(program.to_string());
            not_found_err()
        });
        assert!(result.is_ok());
        let calls = calls.borrow();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0], "wtype");
        assert_eq!(calls[1], "xdotool");
    }

    #[test]
    fn wayland_wtype_exit_failure_does_not_fall_back() {
        let calls: RefCell<Vec<String>> = RefCell::new(Vec::new());
        let result = simulate_paste_linux(env_with("WAYLAND_DISPLAY", "wayland-0"), |program, _args| {
            calls.borrow_mut().push(program.to_string());
            fail_output("boom")
        });
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("wtype failed"), "expected 'wtype failed' in: {}", msg);
        let calls = calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0], "wtype");
    }

    #[test]
    fn wayland_display_empty_treated_as_x11() {
        let calls: RefCell<Vec<String>> = RefCell::new(Vec::new());
        let result = simulate_paste_linux(env_with("WAYLAND_DISPLAY", ""), |program, _args| {
            calls.borrow_mut().push(program.to_string());
            ok_output()
        });
        assert!(result.is_ok());
        let calls = calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0], "xdotool");
    }

    #[test]
    fn non_notfound_io_error_surfaces() {
        let result = simulate_paste_linux(empty_env(), |_program, _args| other_err());
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("Failed to run xdotool"), "expected 'Failed to run xdotool' in: {}", msg);
    }
}

#[cfg(test)]
mod focus_tests {
    use super::*;

    #[test]
    fn editable_roles_are_editable() {
        for role in [
            "AXTextField",
            "AXTextArea",
            "AXComboBox",
            "AXSearchField",
            "AXSecureTextField",
            "AXTokenField",
        ] {
            assert!(is_editable_text_role(role), "{} should be editable", role);
        }
    }

    #[test]
    fn non_editable_roles_are_not_editable() {
        for role in [
            "AXButton",
            "AXImage",
            "AXStaticText",
            "AXMenuItem",
            "AXCheckBox",
            "AXRadioButton",
            "AXLink",
            "AXScrollArea",
            "AXList",
            "AXGroup",
            "AXWindow",
            "AXSlider",
            "AXTable",
        ] {
            assert!(!is_editable_text_role(role), "{} should not be editable", role);
        }
    }

    #[test]
    fn unknown_or_empty_role_is_not_editable() {
        assert!(!is_editable_text_role(""));
        assert!(!is_editable_text_role("missing value"));
        assert!(!is_editable_text_role("AXSomethingNew"));
    }

    #[test]
    fn role_matching_ignores_surrounding_whitespace() {
        assert!(is_editable_text_role("  AXTextField  "));
        assert!(is_editable_text_role("AXTextArea\n"));
        assert!(!is_editable_text_role("  AXButton  "));
    }

    #[test]
    fn role_matching_is_case_sensitive() {
        // System Events reports the canonical AX casing; anything else is
        // treated conservatively as non-editable.
        assert!(!is_editable_text_role("axtextfield"));
        assert!(!is_editable_text_role("AXTEXTFIELD"));
    }

    #[test]
    fn classify_empty_or_missing_is_unknown() {
        assert_eq!(classify_focused_role(""), FocusedFieldState::Unknown);
        assert_eq!(classify_focused_role("   "), FocusedFieldState::Unknown);
        assert_eq!(classify_focused_role("missing value"), FocusedFieldState::Unknown);
    }

    #[test]
    fn classify_editable_role_is_editable() {
        assert_eq!(classify_focused_role("AXTextField"), FocusedFieldState::Editable);
        assert_eq!(classify_focused_role("AXTextArea\n"), FocusedFieldState::Editable);
        assert_eq!(classify_focused_role("AXSearchField"), FocusedFieldState::Editable);
    }

    #[test]
    fn classify_non_editable_role_is_non_editable() {
        assert_eq!(classify_focused_role("AXButton"), FocusedFieldState::NonEditable);
        assert_eq!(classify_focused_role("AXImage"), FocusedFieldState::NonEditable);
        // An unrecognised but non-empty role is treated as non-editable. This is
        // the only state that suppresses the paste, so it must be a positive read.
        assert_eq!(classify_focused_role("AXUnknownFutureRole"), FocusedFieldState::NonEditable);
    }
}

/// Trigger the macOS accessibility permission prompt.
/// Registers the app in System Settings > Privacy & Security > Accessibility
/// and shows the system dialog. Returns current trust status.
#[cfg(target_os = "macos")]
pub fn request_accessibility_prompt() -> bool {
    use std::ffi::c_void;

    #[repr(C)]
    struct Opaque([u8; 0]);

    extern "C" {
        fn AXIsProcessTrustedWithOptions(options: *const c_void) -> bool;
        static kAXTrustedCheckOptionPrompt: *const c_void;
        static kCFBooleanTrue: *const c_void;
        static kCFTypeDictionaryKeyCallBacks: Opaque;
        static kCFTypeDictionaryValueCallBacks: Opaque;
        fn CFDictionaryCreate(
            allocator: *const c_void,
            keys: *const *const c_void,
            values: *const *const c_void,
            num_values: isize,
            key_callbacks: *const c_void,
            value_callbacks: *const c_void,
        ) -> *const c_void;
        fn CFRelease(cf: *const c_void);
    }

    unsafe {
        let keys = [kAXTrustedCheckOptionPrompt];
        let values = [kCFBooleanTrue];
        let dict = CFDictionaryCreate(
            std::ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            1,
            &kCFTypeDictionaryKeyCallBacks as *const Opaque as *const c_void,
            &kCFTypeDictionaryValueCallBacks as *const Opaque as *const c_void,
        );
        if dict.is_null() {
            return false;
        }
        let trusted = AXIsProcessTrustedWithOptions(dict);
        CFRelease(dict);
        trusted
    }
}
