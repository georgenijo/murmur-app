use crate::{audio, injector};

#[cfg(target_os = "macos")]
pub(crate) fn open_system_preference_pane(pane: &str) -> Result<(), String> {
    std::process::Command::new("open")
        .arg(format!(
            "x-apple.systempreferences:com.apple.preference.security?{}",
            pane
        ))
        .spawn()
        .map_err(|e| format!("Failed to open System Settings: {}", e))?;
    Ok(())
}

#[tauri::command]
pub fn open_system_preferences() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        open_system_preference_pane("Privacy_Microphone")
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("System preferences shortcut not supported on this platform".to_string())
    }
}

/// Open the macOS pane that owns Screen & System Audio Recording access.
/// CATap permission is surfaced there on supported macOS releases.
#[tauri::command]
pub fn open_system_audio_preferences() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        open_system_preference_pane("Privacy_ScreenCapture")
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("System Audio settings are only available on macOS".to_string())
    }
}

/// Check if accessibility permission is granted (macOS)
///
/// A grant seen here is remembered (content-free marker file). Accessibility
/// later reading as missing is then the signature of a stale entry: the switch is
/// still ON in System Settings for an older build, but no longer matches this
/// binary. `request_accessibility_permission` uses that to recover in one step.
#[tauri::command]
pub fn check_accessibility_permission(app: tauri::AppHandle) -> bool {
    let trusted = injector::is_accessibility_enabled();
    #[cfg(target_os = "macos")]
    if trusted {
        remember_accessibility_grant(&app);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = &app;
    trusted
}

/// Whether this process has already issued the system Accessibility prompt.
///
/// macOS decides, per process, whether to list the app in Privacy & Security →
/// Accessibility (and whether to show its own dialog) from the first TCC state it
/// sees. A `tccutil reset` made inside a running process is never picked up: the
/// app is not re-listed and no dialog appears. So a reset only works as
/// "reset, relaunch, prompt from the new process".
static ACCESSIBILITY_PROMPT_ISSUED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Marker written before a reset-driven relaunch so the new process issues the
/// first prompt itself, instead of leaving the user on a pane with no Murmur row.
const ACCESSIBILITY_RELAUNCH_MARKER: &str = "accessibility-reset-pending";

/// Marker recording that Accessibility was observed granted at least once.
const ACCESSIBILITY_GRANTED_MARKER: &str = "accessibility-granted-once";

#[cfg(target_os = "macos")]
fn accessibility_marker(app: &tauri::AppHandle, name: &str) -> Option<std::path::PathBuf> {
    use tauri::Manager;
    app.path().app_data_dir().ok().map(|dir| dir.join(name))
}

#[cfg(target_os = "macos")]
fn remember_accessibility_grant(app: &tauri::AppHandle) {
    let Some(marker) = accessibility_marker(app, ACCESSIBILITY_GRANTED_MARKER) else {
        return;
    };
    if marker.exists() {
        return;
    }
    if let Some(dir) = marker.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(marker, b"1");
}

/// What one accessibility request does. Exactly one user-visible surface opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AccessibilityRequestPlan {
    /// Already trusted; nothing to show.
    AlreadyGranted,
    /// Access was granted before and is missing now: the entry left in System
    /// Settings is stale. Reset it and relaunch so the new process can list a
    /// fresh one, rather than sending the user to a switch that cannot work.
    ResetAndRelaunch,
    /// First request in this process: register the app with the system prompt,
    /// then surface exactly one of the macOS dialog or the Settings pane.
    PromptSurface,
    /// The prompt was already issued, so it cannot do anything more; only open
    /// the pane.
    OpenPane,
}

fn plan_accessibility_request(
    trusted: bool,
    prompt_issued: bool,
    previously_granted: bool,
) -> AccessibilityRequestPlan {
    if trusted {
        AccessibilityRequestPlan::AlreadyGranted
    } else if previously_granted {
        AccessibilityRequestPlan::ResetAndRelaunch
    } else if prompt_issued {
        AccessibilityRequestPlan::OpenPane
    } else {
        AccessibilityRequestPlan::PromptSurface
    }
}

/// The macOS prompt carries its own "Open System Settings" button, so Murmur
/// opens the pane itself only when no system dialog is on screen.
fn should_open_settings_pane(system_dialog_visible: bool) -> bool {
    !system_dialog_visible
}

/// Window owners macOS uses for its permission dialogs.
#[cfg(target_os = "macos")]
const SYSTEM_PERMISSION_DIALOG_OWNERS: [&str; 2] =
    ["universalAccessAuthWarn", "UserNotificationCenter"];

/// Whether a macOS permission dialog is currently on screen. Reads only window
/// owner names (no screen content, no Screen Recording permission).
#[cfg(target_os = "macos")]
fn system_permission_dialog_visible() -> bool {
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
    use core_foundation::string::CFString;
    use core_graphics::window::{
        copy_window_info, kCGNullWindowID, kCGWindowListExcludeDesktopElements,
        kCGWindowListOptionOnScreenOnly,
    };

    let Some(windows) = copy_window_info(
        kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
        kCGNullWindowID,
    ) else {
        return false;
    };
    let owner_key = CFString::from_static_string("kCGWindowOwnerName");
    windows.iter().any(|entry| {
        let dictionary: CFDictionary<CFString, CFType> =
            unsafe { CFDictionary::wrap_under_get_rule(*entry as CFDictionaryRef) };
        dictionary
            .find(&owner_key)
            .and_then(|owner| owner.downcast::<CFString>())
            .is_some_and(|owner| {
                SYSTEM_PERMISSION_DIALOG_OWNERS.contains(&owner.to_string().as_str())
            })
    })
}

#[cfg(target_os = "macos")]
fn run_tccutil_reset(service: &str) -> Result<String, String> {
    let bundle_id = current_bundle_identifier()
        .ok_or_else(|| "Could not determine the app's bundle identifier".to_string())?;
    tracing::info!(
        target: "system",
        "resetting {} TCC entry for bundle id {}",
        service,
        bundle_id
    );
    let output = std::process::Command::new("tccutil")
        .args(["reset", service, &bundle_id])
        .output()
        .map_err(|e| format!("Failed to run tccutil: {}", e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::warn!(
            target: "system",
            "tccutil reset {} failed for {}: {}",
            service,
            bundle_id,
            stderr.trim()
        );
        return Err(format!("tccutil reset failed: {}", stderr.trim()));
    }
    Ok(bundle_id)
}

/// Issue the system prompt (registers the app in the Accessibility list) and
/// surface exactly one thing: the macOS dialog, or the Settings pane when the
/// dialog did not appear. Returns the outcome code for the UI.
#[cfg(target_os = "macos")]
fn prompt_and_surface_accessibility() -> Result<&'static str, String> {
    ACCESSIBILITY_PROMPT_ISSUED.store(true, std::sync::atomic::Ordering::SeqCst);
    if injector::request_accessibility_prompt() {
        return Ok("granted");
    }
    // The dialog is presented asynchronously by the system.
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(900);
    let mut dialog_visible = false;
    while std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if system_permission_dialog_visible() {
            dialog_visible = true;
            break;
        }
    }
    if should_open_settings_pane(dialog_visible) {
        open_system_preference_pane("Privacy_Accessibility")?;
        Ok("opened_settings")
    } else {
        Ok("prompted")
    }
}

/// Reset the stale Accessibility entry and relaunch so the new process lists a
/// fresh one. The relaunched process issues the prompt itself.
#[cfg(target_os = "macos")]
fn reset_accessibility_and_relaunch(app: &tauri::AppHandle) -> Result<&'static str, String> {
    run_tccutil_reset("Accessibility")?;
    // The stale entry is gone; stop treating the next request as stale.
    if let Some(marker) = accessibility_marker(app, ACCESSIBILITY_GRANTED_MARKER) {
        let _ = std::fs::remove_file(marker);
    }
    let marker = accessibility_marker(app, ACCESSIBILITY_RELAUNCH_MARKER)
        .ok_or_else(|| "Could not locate the app data folder".to_string())?;
    if let Some(dir) = marker.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&marker, b"1").map_err(|e| format!("Could not schedule the restart: {e}"))?;
    relaunch_to_apply_reset(app.clone());
    Ok("restarting")
}

/// Ask for accessibility access with one clear action.
///
/// Returns "granted" | "prompted" (the macOS dialog is showing; its own button
/// opens System Settings) | "opened_settings" | "restarting" (a stale entry from
/// an earlier grant was reset; Murmur relaunches and asks again). Never raises
/// the dialog and the Settings pane together.
#[tauri::command]
pub async fn request_accessibility_permission(app: tauri::AppHandle) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        tauri::async_runtime::spawn_blocking(move || {
            let trusted = injector::is_accessibility_enabled();
            let prompt_issued =
                ACCESSIBILITY_PROMPT_ISSUED.load(std::sync::atomic::Ordering::SeqCst);
            let previously_granted = accessibility_marker(&app, ACCESSIBILITY_GRANTED_MARKER)
                .is_some_and(|marker| marker.exists());
            match plan_accessibility_request(trusted, prompt_issued, previously_granted) {
                AccessibilityRequestPlan::AlreadyGranted => Ok("granted".to_string()),
                AccessibilityRequestPlan::ResetAndRelaunch => {
                    reset_accessibility_and_relaunch(&app).map(str::to_string)
                }
                AccessibilityRequestPlan::OpenPane => {
                    open_system_preference_pane("Privacy_Accessibility")?;
                    Ok("opened_settings".to_string())
                }
                AccessibilityRequestPlan::PromptSurface => {
                    prompt_and_surface_accessibility().map(str::to_string)
                }
            }
        })
        .await
        .map_err(|e| format!("Accessibility request failed: {}", e))?
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Ok("granted".to_string())
    }
}

/// Run the first accessibility request of a process that was relaunched by
/// `reset_accessibility_permission`. Called once the app is up.
#[cfg(target_os = "macos")]
pub(crate) fn resume_accessibility_request_after_relaunch(data_dir: &std::path::Path) {
    let marker = data_dir.join(ACCESSIBILITY_RELAUNCH_MARKER);
    if std::fs::remove_file(&marker).is_err() {
        return;
    }
    tauri::async_runtime::spawn_blocking(|| {
        // Let the main window finish appearing so the dialog has an active app.
        std::thread::sleep(std::time::Duration::from_millis(1500));
        if injector::is_accessibility_enabled() {
            return;
        }
        if let Err(error) = prompt_and_surface_accessibility() {
            tracing::warn!(target: "system", "post-reset accessibility request failed: {error}");
        }
    });
}

/// Read the running process's bundle identifier (macOS).
///
/// Returns the *runtime* bundle id (e.g. the dev bundle `Local Dictation Dev`),
/// which is what TCC actually keys Accessibility entries on — not the static
/// identifier from `tauri.conf.json`, which can be stale for rebuilt/dev bundles.
#[cfg(target_os = "macos")]
pub(crate) fn current_bundle_identifier() -> Option<String> {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2_foundation::NSString;

    unsafe {
        let cls = AnyClass::get(c"NSBundle")?;
        let bundle: *mut AnyObject = msg_send![cls, mainBundle];
        if bundle.is_null() {
            return None;
        }
        let ident: *const NSString = msg_send![bundle, bundleIdentifier];
        if ident.is_null() {
            return None;
        }
        let s = (*ident).to_string();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    }
}

/// Relaunch Murmur so a TCC reset takes effect. A running process keeps the
/// authorization state it last saw: after `tccutil reset` it still reports a
/// denial, and `requestAccess` answers from that cached state without asking
/// again. Only a new process sees the reset, which is why the user previously
/// had to quit and reset a second time by hand.
///
/// Runs off the calling thread so the command can answer first.
#[cfg(target_os = "macos")]
fn relaunch_to_apply_reset(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(300));
        // Start the new process through Launch Services, as a normal launch, so
        // macOS attributes it to the app bundle. A detached shell waits for this
        // process to exit first, so the two never overlap.
        if let Some(bundle) = running_app_bundle() {
            let launched = std::process::Command::new("/bin/sh")
                .arg("-c")
                .arg("sleep 1; exec /usr/bin/open \"$0\"")
                .arg(&bundle)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .is_ok();
            if launched {
                app.exit(0);
                return;
            }
        }
        app.restart();
    });
}

/// The `.app` this process runs from, when it runs from one.
#[cfg(target_os = "macos")]
fn running_app_bundle() -> Option<std::path::PathBuf> {
    let executable = std::env::current_exe().ok()?;
    let bundle = executable.ancestors().nth(3)?;
    (bundle.extension()? == "app").then(|| bundle.to_path_buf())
}

/// Reset this app's stale macOS Accessibility TCC entry and bring up a fresh one.
///
/// Troubleshooting action for the case where System Settings lists the app under
/// Accessibility (often with its switch ON, from an older build) but the running
/// build still reports access missing. Resets ONLY the current bundle identifier
/// via `tccutil reset Accessibility <bundle-id>` — never all apps. A running
/// process cannot re-list itself after a reset (see
/// `ACCESSIBILITY_PROMPT_ISSUED`), so Murmur relaunches and prompts from the new
/// process. Returns "granted" | "restarting".
#[tauri::command]
pub async fn reset_accessibility_permission(app: tauri::AppHandle) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        tauri::async_runtime::spawn_blocking(move || {
            if injector::is_accessibility_enabled() {
                return Ok("granted".to_string());
            }
            reset_accessibility_and_relaunch(&app).map(str::to_string)
        })
        .await
        .map_err(|e| format!("Accessibility reset failed: {}", e))?
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Err("Accessibility reset is only supported on macOS".to_string())
    }
}

/// Request microphone permission (opens System Settings on macOS)
#[tauri::command]
pub fn request_microphone_permission() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        open_system_preference_pane("Privacy_Microphone")
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(())
    }
}

/// Ask macOS for microphone access from THIS process, without opening the device.
///
/// Calls `AVCaptureDevice.requestAccessForMediaType:completionHandler:`. When the
/// status is `notDetermined` this shows the system dialog and registers the app in
/// the Microphone pane; when it is already determined the completion fires at once
/// and no dialog appears. It never instantiates a capture unit, so it cannot duck
/// other apps' audio (issue #177) or light the orange indicator.
///
/// The request must come from the process whose status we later read. A grant
/// made through a prompt that the capture helper raised leaves this process
/// reporting `notDetermined` until it restarts, which made every later recording
/// look like a lost grant. Requests are coalesced while one is outstanding.
/// Fire-and-forget: callers observe the outcome through
/// `check_microphone_permission_status`.
#[cfg(target_os = "macos")]
pub(crate) fn request_microphone_access_from_this_process() -> Result<(), String> {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, Bool};
    use objc2_foundation::NSString;

    static REQUEST_IN_FLIGHT: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);
    if REQUEST_IN_FLIGHT.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return Ok(());
    }
    unsafe {
        let Some(cls) = AnyClass::get(c"AVCaptureDevice") else {
            REQUEST_IN_FLIGHT.store(false, std::sync::atomic::Ordering::SeqCst);
            return Err("AVCaptureDevice is unavailable".to_string());
        };
        // AVMediaTypeAudio == @"soun"
        let media = NSString::from_str("soun");
        // The completion handler runs on an arbitrary dispatch queue after the
        // user answers; the block only logs, all state reads go through the
        // polling commands.
        let handler = block2::RcBlock::new(|granted: Bool| {
            REQUEST_IN_FLIGHT.store(false, std::sync::atomic::Ordering::SeqCst);
            tracing::info!(
                target: "system",
                event_code = "system.microphone_access_request_completed",
                granted = granted.as_bool(),
                "microphone access request completed"
            );
        });
        let _: () = msg_send![
            cls,
            requestAccessForMediaType: &*media,
            completionHandler: &*handler
        ];
    }
    Ok(())
}

/// Take ownership of an expected microphone prompt before the capture helper
/// can raise it, so the grant is recorded in the process that reads the status.
/// Unit tests must never raise a real system prompt, so they only count calls.
pub(crate) fn own_expected_microphone_prompt() {
    #[cfg(test)]
    {
        OWNED_PROMPT_REQUESTS.with(|count| count.set(count.get() + 1));
    }
    #[cfg(all(not(test), target_os = "macos"))]
    if let Err(error) = request_microphone_access_from_this_process() {
        tracing::warn!(target: "system", "microphone access request failed: {error}");
    }
}

#[cfg(test)]
thread_local! {
    static OWNED_PROMPT_REQUESTS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn owned_prompt_requests_for_test() -> u32 {
    OWNED_PROMPT_REQUESTS.with(std::cell::Cell::get)
}

/// Trigger the native macOS microphone permission prompt (TCC) in-flow.
#[tauri::command]
pub fn request_microphone_access() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        request_microphone_access_from_this_process()
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(())
    }
}

/// Reset this app's stale macOS Microphone TCC entry and relaunch.
///
/// Mirrors `reset_accessibility_permission` for the case where the running build
/// still reports the microphone as denied/missing while System Settings lists the
/// app (common after rebuilding a dev bundle or moving the .app — the binary
/// signature changes and the TCC entry goes stale). Resets ONLY the current bundle
/// identifier via `tccutil reset Microphone <bundle-id>` — never all apps.
///
/// The Microphone pane no longer lists the app afterward, so it is not opened,
/// and the running process cannot see the reset (see `relaunch_to_apply_reset`).
/// The relaunched process reads `notDetermined`, so the next dictation asks
/// macOS once, from the process that records the answer. Returns "restarting".
#[tauri::command]
pub async fn reset_microphone_permission(app: tauri::AppHandle) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        tauri::async_runtime::spawn_blocking(move || {
            run_tccutil_reset("Microphone")?;
            relaunch_to_apply_reset(app);
            Ok("restarting".to_string())
        })
        .await
        .map_err(|e| format!("Microphone reset failed: {}", e))?
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Err("Microphone reset is only supported on macOS".to_string())
    }
}

/// Check microphone authorization status WITHOUT opening the device.
///
/// Reads `AVCaptureDevice.authorizationStatus(for: .audio)`. This only queries
/// TCC state — it never instantiates a capture/voice-processing audio unit, so
/// it cannot duck other apps' audio. (Issue #177: the previous getUserMedia probe
/// opened VPIO on every window focus, ducking other system audio each time.)
#[tauri::command]
pub fn check_microphone_permission() -> bool {
    #[cfg(target_os = "macos")]
    {
        use objc2::msg_send;
        use objc2::runtime::AnyClass;
        use objc2_foundation::NSString;

        // AVAuthorizationStatusAuthorized == 3
        const AUTHORIZED: isize = 3;
        unsafe {
            let Some(cls) = AnyClass::get(c"AVCaptureDevice") else {
                return false;
            };
            // AVMediaTypeAudio == @"soun"
            let media = NSString::from_str("soun");
            let status: isize = msg_send![cls, authorizationStatusForMediaType: &*media];
            status == AUTHORIZED
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

/// Map a raw macOS `AVAuthorizationStatus` value to a banner-state string.
///
/// Pure helper so the status-enum -> banner-state mapping is unit-testable
/// without a live TCC database. The returned string is consumed by the frontend
/// permissions banner, which distinguishes a hard "denied" (show red, offer the
/// reset path) from a transient "notDetermined"/"unknown" state (do NOT show a
/// hard denied banner — see issue #190, false-negatives after a dev rebuild or
/// app move when TCC drops the entry and the status reads not-determined).
///
/// `AVAuthorizationStatus` (AVFoundation):
///   0 = notDetermined, 1 = restricted, 2 = denied, 3 = authorized.
/// Any unexpected value (e.g. a future/probe-failure sentinel) maps to "unknown"
/// rather than "denied" so a transient probe glitch never hard-fails the banner.
fn mic_status_to_banner_state(status: isize) -> &'static str {
    match status {
        3 => "granted",
        2 | 1 => "denied",
        0 => "notDetermined",
        _ => "unknown",
    }
}

#[cfg(test)]
thread_local! {
    static MICROPHONE_STATUS_OVERRIDE: std::cell::Cell<Option<&'static str>> =
        const { std::cell::Cell::new(None) };
}

/// Test-only: report `status` from this thread instead of reading TCC.
#[cfg(test)]
pub(crate) fn override_microphone_permission_status_for_test(status: Option<&'static str>) {
    MICROPHONE_STATUS_OVERRIDE.with(|cell| cell.set(status));
}

/// Read the *current* microphone authorization status as a banner-state string.
///
/// Queries `AVCaptureDevice.authorizationStatus(for: .audio)` live at call-time
/// (never a cached value) and maps it via [`mic_status_to_banner_state`]. Unlike
/// the boolean [`check_microphone_permission`], this preserves the distinction
/// between a genuine "denied" and a transient "notDetermined"/"unknown" state so
/// the banner doesn't false-negative after a rebuild/move (issue #190).
///
/// Returns one of: "granted" | "denied" | "notDetermined" | "unknown".
/// Like the bool probe, this only reads TCC state — it never opens the device,
/// so it cannot duck other apps' audio (issue #177).
#[tauri::command]
pub fn check_microphone_permission_status() -> String {
    #[cfg(test)]
    if let Some(status) = MICROPHONE_STATUS_OVERRIDE.with(std::cell::Cell::get) {
        return status.to_string();
    }
    #[cfg(target_os = "macos")]
    {
        use objc2::msg_send;
        use objc2::runtime::AnyClass;
        use objc2_foundation::NSString;

        unsafe {
            let Some(cls) = AnyClass::get(c"AVCaptureDevice") else {
                // Class lookup failed: a probe glitch, not a real denial.
                return "unknown".to_string();
            };
            // AVMediaTypeAudio == @"soun"
            let media = NSString::from_str("soun");
            let status: isize = msg_send![cls, authorizationStatusForMediaType: &*media];
            mic_status_to_banner_state(status).to_string()
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        "granted".to_string()
    }
}

const AUDIO_INVENTORY_REQUEST_DENIED: &str = "Microphone inventory is unavailable.";

fn audio_inventory_request_allowed(window_label: &str) -> bool {
    window_label == "main"
}

async fn run_audio_inventory_read<T: Send + 'static>(
    read: impl FnOnce() -> T + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(read)
        .await
        .map_err(|_| AUDIO_INVENTORY_REQUEST_DENIED.to_string())
}

#[tauri::command]
pub async fn list_audio_devices(
    window: tauri::WebviewWindow,
) -> Result<Vec<audio::AudioDeviceDescriptor>, String> {
    if !audio_inventory_request_allowed(window.label()) {
        return Err(AUDIO_INVENTORY_REQUEST_DENIED.to_string());
    }
    drop(window);
    run_audio_inventory_read(audio::list_input_devices).await?
}

#[tauri::command]
pub async fn get_audio_input_inventory(
    window: tauri::WebviewWindow,
) -> Result<crate::audio_inventory::AudioInputInventorySnapshot, String> {
    if !audio_inventory_request_allowed(window.label()) {
        return Err(AUDIO_INVENTORY_REQUEST_DENIED.to_string());
    }
    drop(window);
    run_audio_inventory_read(crate::audio_inventory::get_inventory).await
}

#[cfg(test)]
mod tests {
    use super::{
        audio_inventory_request_allowed, mic_status_to_banner_state, plan_accessibility_request,
        run_audio_inventory_read, should_open_settings_pane, AccessibilityRequestPlan,
    };

    #[test]
    fn first_accessibility_request_prompts_and_never_opens_both() {
        assert_eq!(
            plan_accessibility_request(false, false, false),
            AccessibilityRequestPlan::PromptSurface
        );
        // The surface step opens the pane only when no system dialog is showing.
        assert!(!should_open_settings_pane(true));
        assert!(should_open_settings_pane(false));
    }

    #[test]
    fn repeat_accessibility_request_only_opens_the_pane() {
        // macOS ignores a second prompt in the same process, so a second click
        // must only open the pane.
        assert_eq!(
            plan_accessibility_request(false, true, false),
            AccessibilityRequestPlan::OpenPane
        );
    }

    #[test]
    fn granted_accessibility_request_shows_nothing() {
        for issued in [false, true] {
            for previously in [false, true] {
                assert_eq!(
                    plan_accessibility_request(true, issued, previously),
                    AccessibilityRequestPlan::AlreadyGranted
                );
            }
        }
    }

    #[test]
    fn a_missing_grant_that_once_worked_resets_and_relaunches() {
        // Listed-and-on in Settings but no longer matching this binary: one
        // action resets the entry and relaunches, whether or not this process
        // already prompted.
        for issued in [false, true] {
            assert_eq!(
                plan_accessibility_request(false, issued, true),
                AccessibilityRequestPlan::ResetAndRelaunch
            );
        }
    }

    #[test]
    fn audio_inventory_is_exactly_main_window_scoped() {
        assert!(audio_inventory_request_allowed("main"));
        for denied in ["overlay", "diagnostics", "transform-review", "MAIN", ""] {
            assert!(!audio_inventory_request_allowed(denied));
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn audio_inventory_read_runs_on_a_blocking_worker() {
        let caller = std::thread::current().id();
        let worker = run_audio_inventory_read(|| std::thread::current().id())
            .await
            .unwrap();
        assert_ne!(worker, caller);
    }

    #[test]
    fn authorized_status_maps_to_granted() {
        // AVAuthorizationStatusAuthorized
        assert_eq!(mic_status_to_banner_state(3), "granted");
    }

    #[test]
    fn denied_and_restricted_map_to_denied() {
        // AVAuthorizationStatusDenied
        assert_eq!(mic_status_to_banner_state(2), "denied");
        // AVAuthorizationStatusRestricted (e.g. MDM/parental controls) — still a
        // genuine block, so it must read as denied, not a transient state.
        assert_eq!(mic_status_to_banner_state(1), "denied");
    }

    #[test]
    fn not_determined_is_not_a_hard_denial() {
        // AVAuthorizationStatusNotDetermined: TCC has no entry yet (common right
        // after a rebuild/move). Must NOT collapse to "denied" (issue #190).
        let state = mic_status_to_banner_state(0);
        assert_eq!(state, "notDetermined");
        assert_ne!(state, "denied");
    }

    #[test]
    fn unexpected_values_map_to_unknown_not_denied() {
        // Any future/sentinel value from a probe glitch must degrade to "unknown",
        // never a hard "denied" — we never want to false-negative the banner.
        for v in [-1isize, 4, 99, isize::MAX, isize::MIN] {
            let state = mic_status_to_banner_state(v);
            assert_eq!(state, "unknown", "value {v} should map to unknown");
            assert_ne!(state, "denied", "value {v} must not map to denied");
        }
    }
}
