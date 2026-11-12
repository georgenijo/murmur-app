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
/// A grant seen here is remembered with a fingerprint of the running build
/// (content-free marker file). Accessibility later reading as missing under a
/// *different* build is the signature of a stale entry: the switch is still ON in
/// System Settings for an older build, but no longer matches this binary.
/// `request_accessibility_permission` uses that to recover in one step.
///
/// A process that saw access missing and then granted keeps failing Accessibility
/// queries until it restarts, so that transition schedules one relaunch while
/// Murmur is idle.
#[tauri::command]
pub fn check_accessibility_permission(app: tauri::AppHandle) -> bool {
    let trusted = injector::is_accessibility_enabled();
    #[cfg(target_os = "macos")]
    note_accessibility_observation(&app, trusted);
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

/// Serializes accessibility request/reset work so a fast double click cannot
/// run two prompt or relaunch sequences.
static ACCESSIBILITY_REQUEST_IN_FLIGHT: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// This process observed Accessibility missing at least once.
static ACCESSIBILITY_SEEN_MISSING: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// The one automatic relaunch after a grant arrived mid-run has been scheduled.
static GRANT_RELAUNCH_SCHEDULED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Marker written before a reset-driven relaunch so the new process issues the
/// first prompt itself, instead of leaving the user on a pane with no Murmur row.
const ACCESSIBILITY_RELAUNCH_MARKER: &str = "accessibility-reset-pending";

/// Marker holding the fingerprint of the build that last saw Accessibility granted.
const ACCESSIBILITY_GRANTED_MARKER: &str = "accessibility-granted-once";

const RESTART_BUSY_MESSAGE: &str = "Finish your recording, meeting or transcription first. Murmur restarts to finish a permission reset.";
const RESTART_UNAVAILABLE_MESSAGE: &str =
    "Resetting a permission restarts Murmur, which only works from the installed app.";

/// What gates a relaunch. A relaunch ends the process, so it must never run over
/// a recording, a transcription, or a meeting, and only a packaged `.app` can be
/// relaunched (a dev session would reset the wrong identity and restart outside
/// its tooling).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RelaunchGate {
    can_relaunch: bool,
    busy: Option<&'static str>,
}

impl RelaunchGate {
    fn open(&self) -> bool {
        self.can_relaunch && self.busy.is_none()
    }
}

/// Why an explicit reset must not run now.
fn reset_refusal(gate: RelaunchGate) -> Option<&'static str> {
    if !gate.can_relaunch {
        Some(RESTART_UNAVAILABLE_MESSAGE)
    } else if gate.busy.is_some() {
        Some(RESTART_BUSY_MESSAGE)
    } else {
        None
    }
}

/// A recorded grant is stale when it was seen under a different build than the
/// one running now. Same build and still missing means access was turned off or
/// removed, which an ordinary prompt or pane handles without a restart.
fn grant_is_stale(recorded: Option<&str>, current: Option<&str>) -> bool {
    matches!((recorded, current), (Some(recorded), Some(current)) if recorded != current)
}

/// Whether to relaunch once because access went from missing to granted inside
/// this process: its Accessibility queries keep failing until it restarts.
fn should_relaunch_after_grant(
    seen_missing: bool,
    trusted_now: bool,
    already_scheduled: bool,
    can_relaunch: bool,
) -> bool {
    seen_missing && trusted_now && !already_scheduled && can_relaunch
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AccessibilityRequestFacts {
    trusted: bool,
    prompt_issued: bool,
    stale_grant: bool,
    gate: RelaunchGate,
}

/// What one accessibility request does. Exactly one user-visible surface opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AccessibilityRequestPlan {
    /// Already trusted; nothing to show.
    AlreadyGranted,
    /// A grant seen under an older build is missing now: the entry left in System
    /// Settings is stale. Reset it and relaunch so the new process can list a
    /// fresh one, rather than sending the user to a switch that cannot work.
    ResetAndRelaunch,
    /// Same stale entry, but Murmur is busy: open the pane and leave the reset for
    /// when the work is done.
    OpenPaneRestartDeferred,
    /// First request in this process: register the app with the system prompt,
    /// then surface exactly one of the macOS dialog or the Settings pane.
    PromptSurface,
    /// The prompt was already issued, so it cannot do anything more; only open
    /// the pane.
    OpenPane,
}

fn plan_accessibility_request(facts: AccessibilityRequestFacts) -> AccessibilityRequestPlan {
    if facts.trusted {
        return AccessibilityRequestPlan::AlreadyGranted;
    }
    if facts.stale_grant && facts.gate.can_relaunch {
        return if facts.gate.busy.is_none() {
            AccessibilityRequestPlan::ResetAndRelaunch
        } else {
            AccessibilityRequestPlan::OpenPaneRestartDeferred
        };
    }
    fallback_accessibility_plan(facts.prompt_issued)
}

/// The ordinary prompt-or-pane path, also the fallback when a reset fails.
fn fallback_accessibility_plan(prompt_issued: bool) -> AccessibilityRequestPlan {
    if prompt_issued {
        AccessibilityRequestPlan::OpenPane
    } else {
        AccessibilityRequestPlan::PromptSurface
    }
}

/// The macOS prompt carries its own "Open System Settings" button, so Murmur
/// opens the pane itself only when no system dialog appeared for this request.
fn should_open_settings_pane(system_dialog_appeared: bool) -> bool {
    !system_dialog_appeared
}

/// Consume the post-reset marker. True exactly once per relaunch, so the new
/// process asks once and a later launch cannot loop.
fn take_relaunch_marker(data_dir: &std::path::Path) -> bool {
    std::fs::remove_file(data_dir.join(ACCESSIBILITY_RELAUNCH_MARKER)).is_ok()
}

/// Holds the accessibility request lock until dropped.
struct AccessibilityRequestGuard;

impl AccessibilityRequestGuard {
    fn acquire() -> Option<Self> {
        (!ACCESSIBILITY_REQUEST_IN_FLIGHT.swap(true, std::sync::atomic::Ordering::SeqCst))
            .then_some(Self)
    }
}

impl Drop for AccessibilityRequestGuard {
    fn drop(&mut self) {
        ACCESSIBILITY_REQUEST_IN_FLIGHT.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

#[cfg(target_os = "macos")]
fn accessibility_marker(app: &tauri::AppHandle, name: &str) -> Option<std::path::PathBuf> {
    use tauri::Manager;
    app.path().app_data_dir().ok().map(|dir| dir.join(name))
}

/// Identifies the running build: version, executable size and modification time.
/// A rebuild, re-sign, or update changes it; reading a TCC database is not needed.
#[cfg(target_os = "macos")]
fn build_fingerprint() -> Option<String> {
    let metadata = std::fs::metadata(std::env::current_exe().ok()?).ok()?;
    let modified = metadata
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    Some(format!(
        "{}:{}:{}",
        env!("CARGO_PKG_VERSION"),
        metadata.len(),
        modified
    ))
}

#[cfg(target_os = "macos")]
fn remember_accessibility_grant(app: &tauri::AppHandle) {
    let (Some(marker), Some(fingerprint)) = (
        accessibility_marker(app, ACCESSIBILITY_GRANTED_MARKER),
        build_fingerprint(),
    ) else {
        return;
    };
    if std::fs::read_to_string(&marker).is_ok_and(|recorded| recorded == fingerprint) {
        return;
    }
    if let Some(dir) = marker.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(marker, fingerprint);
}

/// Record an Accessibility reading from the status poll. A reading of "granted"
/// after "missing" in the same process schedules the one automatic relaunch.
#[cfg(target_os = "macos")]
fn note_accessibility_observation(app: &tauri::AppHandle, trusted: bool) {
    use std::sync::atomic::Ordering;
    if !trusted {
        ACCESSIBILITY_SEEN_MISSING.store(true, Ordering::SeqCst);
        return;
    }
    remember_accessibility_grant(app);
    if should_relaunch_after_grant(
        ACCESSIBILITY_SEEN_MISSING.load(Ordering::SeqCst),
        true,
        GRANT_RELAUNCH_SCHEDULED.load(Ordering::SeqCst),
        running_app_bundle().is_some(),
    ) && !GRANT_RELAUNCH_SCHEDULED.swap(true, Ordering::SeqCst)
    {
        tracing::info!(
            target: "system",
            event_code = "system.accessibility_grant_relaunch_scheduled",
            "Accessibility was granted while running; relaunching when idle"
        );
        relaunch_when_idle(app.clone(), None);
    }
}

/// The live gate for a relaunch: packaged app, and no recording, transcription,
/// meeting, preview, or benchmark in progress.
#[cfg(target_os = "macos")]
fn relaunch_gate(app: &tauri::AppHandle) -> RelaunchGate {
    use tauri::Manager;
    RelaunchGate {
        can_relaunch: running_app_bundle().is_some(),
        busy: app
            .try_state::<crate::State>()
            .and_then(|state| crate::commands::recording::dictation_prepare_refusal(&state)),
    }
}

/// Window owners macOS uses for its permission dialogs.
#[cfg(target_os = "macos")]
const SYSTEM_PERMISSION_DIALOG_OWNERS: [&str; 2] =
    ["universalAccessAuthWarn", "UserNotificationCenter"];

/// How many macOS permission dialog windows are on screen. Reads only window
/// owner names (no screen content, no Screen Recording permission).
#[cfg(target_os = "macos")]
fn system_permission_dialog_windows() -> usize {
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
        return 0;
    };
    let owner_key = CFString::from_static_string("kCGWindowOwnerName");
    windows
        .iter()
        .filter(|entry| {
            let dictionary: CFDictionary<CFString, CFType> =
                unsafe { CFDictionary::wrap_under_get_rule(**entry as CFDictionaryRef) };
            dictionary
                .find(&owner_key)
                .and_then(|owner| owner.downcast::<CFString>())
                .is_some_and(|owner| {
                    SYSTEM_PERMISSION_DIALOG_OWNERS.contains(&owner.to_string().as_str())
                })
        })
        .count()
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
/// surface exactly one thing: the macOS dialog that this request raised, or the
/// Settings pane when none appeared. Returns the outcome code for the UI.
#[cfg(target_os = "macos")]
fn prompt_and_surface_accessibility() -> Result<&'static str, String> {
    ACCESSIBILITY_PROMPT_ISSUED.store(true, std::sync::atomic::Ordering::SeqCst);
    // A dialog that was already on screen (any other permission alert) must not
    // be mistaken for this request's.
    let windows_before = system_permission_dialog_windows();
    if injector::request_accessibility_prompt() {
        return Ok("granted");
    }
    // The dialog is presented asynchronously by the system.
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(1500);
    let mut appeared = false;
    while std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if system_permission_dialog_windows() > windows_before {
            appeared = true;
            break;
        }
    }
    if should_open_settings_pane(appeared) {
        open_system_preference_pane("Privacy_Accessibility")?;
        Ok("opened_settings")
    } else {
        Ok("prompted")
    }
}

/// Reset the stale Accessibility entry and relaunch so the new process lists a
/// fresh one. The relaunched process issues the prompt itself. Fails only when
/// `tccutil` does; nothing has changed then.
#[cfg(target_os = "macos")]
fn reset_accessibility_and_relaunch(app: &tauri::AppHandle) -> Result<&'static str, String> {
    run_tccutil_reset("Accessibility")?;
    // The stale entry is gone; stop treating the next request as stale.
    if let Some(marker) = accessibility_marker(app, ACCESSIBILITY_GRANTED_MARKER) {
        let _ = std::fs::remove_file(marker);
    }
    // Best effort: without it the new process just waits for the user's click.
    if let Some(marker) = accessibility_marker(app, ACCESSIBILITY_RELAUNCH_MARKER) {
        if let Some(dir) = marker.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(error) = std::fs::write(&marker, b"1") {
            tracing::warn!(target: "system", "could not schedule the post-reset prompt: {error}");
        }
    }
    relaunch_when_idle(app.clone(), Some(std::time::Duration::from_secs(3)));
    Ok("restarting")
}

/// Ask for accessibility access with one clear action.
///
/// Returns "granted" | "prompted" (the macOS dialog is showing; its own button
/// opens System Settings) | "opened_settings" | "restarting" (a stale entry from
/// an earlier build's grant was reset; Murmur relaunches and asks again) |
/// "restart_deferred" (stale, but Murmur is busy: the pane opened and the reset
/// waits) | "in_progress" (another request is still running). Never raises the
/// dialog and the Settings pane together.
#[tauri::command]
pub async fn request_accessibility_permission(app: tauri::AppHandle) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        tauri::async_runtime::spawn_blocking(move || {
            let Some(_guard) = AccessibilityRequestGuard::acquire() else {
                return Ok("in_progress".to_string());
            };
            let prompt_issued =
                ACCESSIBILITY_PROMPT_ISSUED.load(std::sync::atomic::Ordering::SeqCst);
            let recorded = accessibility_marker(&app, ACCESSIBILITY_GRANTED_MARKER)
                .and_then(|marker| std::fs::read_to_string(marker).ok());
            let facts = AccessibilityRequestFacts {
                trusted: injector::is_accessibility_enabled(),
                prompt_issued,
                stale_grant: grant_is_stale(recorded.as_deref(), build_fingerprint().as_deref()),
                gate: relaunch_gate(&app),
            };
            let surface = |plan| match plan {
                AccessibilityRequestPlan::OpenPane => {
                    open_system_preference_pane("Privacy_Accessibility")?;
                    Ok("opened_settings".to_string())
                }
                _ => prompt_and_surface_accessibility().map(str::to_string),
            };
            match plan_accessibility_request(facts) {
                AccessibilityRequestPlan::AlreadyGranted => Ok("granted".to_string()),
                AccessibilityRequestPlan::ResetAndRelaunch => {
                    match reset_accessibility_and_relaunch(&app) {
                        Ok(outcome) => Ok(outcome.to_string()),
                        Err(error) => {
                            // Never leave the main button replanning a reset that
                            // cannot work: forget the stale grant and take the
                            // ordinary prompt-or-pane path.
                            tracing::warn!(
                                target: "system",
                                "stale Accessibility reset failed, using the ordinary request: {error}"
                            );
                            if let Some(marker) =
                                accessibility_marker(&app, ACCESSIBILITY_GRANTED_MARKER)
                            {
                                let _ = std::fs::remove_file(marker);
                            }
                            surface(fallback_accessibility_plan(prompt_issued))
                        }
                    }
                }
                AccessibilityRequestPlan::OpenPaneRestartDeferred => {
                    open_system_preference_pane("Privacy_Accessibility")?;
                    Ok("restart_deferred".to_string())
                }
                plan @ (AccessibilityRequestPlan::PromptSurface
                | AccessibilityRequestPlan::OpenPane) => surface(plan),
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
    if !take_relaunch_marker(data_dir) {
        return;
    }
    tauri::async_runtime::spawn_blocking(|| {
        // Let the main window finish appearing so the dialog has an active app.
        std::thread::sleep(std::time::Duration::from_millis(1500));
        let Some(_guard) = AccessibilityRequestGuard::acquire() else {
            return;
        };
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

/// Wait for `pid` to exit (bounded), then open `bundle` through Launch Services;
/// if `opener` fails, run `exe` directly. Detached, so it outlives this process.
///
/// The wait matters: `open` on a bundle whose old instance is still shutting down
/// (meeting teardown alone can take seconds) only re-activates the dying process
/// and nothing relaunches.
#[cfg(target_os = "macos")]
fn spawn_relaunch_waiter(
    bundle: &std::path::Path,
    exe: &std::path::Path,
    pid: u32,
    opener: &std::path::Path,
) -> std::io::Result<std::process::Child> {
    const SCRIPT: &str = r#"i=0
while kill -0 "$3" 2>/dev/null && [ "$i" -lt 200 ]; do sleep 0.1; i=$((i+1)); done
"$1" "$0" || exec "$2""#;
    std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(SCRIPT)
        .arg(bundle)
        .arg(opener)
        .arg(exe)
        .arg(pid.to_string())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
}

/// End this process and start a fresh one.
#[cfg(target_os = "macos")]
fn relaunch_now(app: &tauri::AppHandle) {
    if let (Some(bundle), Ok(exe)) = (running_app_bundle(), std::env::current_exe()) {
        if spawn_relaunch_waiter(
            &bundle,
            &exe,
            std::process::id(),
            std::path::Path::new("/usr/bin/open"),
        )
        .is_ok()
        {
            app.exit(0);
            return;
        }
    }
    app.restart();
}

/// Relaunch Murmur once it is idle. A running process keeps the authorization
/// state it last saw: after `tccutil reset` it still reports a denial, and
/// `requestAccess` answers from that cached state without asking again. Only a
/// new process sees the reset, which is why the user previously had to quit and
/// reset a second time by hand.
///
/// Never relaunches over a recording, transcription, or meeting. With
/// `max_wait`, gives up (leaving the reset applied) if Murmur stays busy that
/// long; without it, waits as long as needed. Runs off the calling thread so the
/// command can answer first.
#[cfg(target_os = "macos")]
fn relaunch_when_idle(app: tauri::AppHandle, max_wait: Option<std::time::Duration>) {
    use std::time::{Duration, Instant};
    std::thread::spawn(move || {
        let started = Instant::now();
        std::thread::sleep(Duration::from_millis(1500));
        loop {
            if relaunch_gate(&app).open() {
                // Idle twice in a row, so a start in the gap is caught.
                std::thread::sleep(Duration::from_millis(400));
                if relaunch_gate(&app).open() {
                    relaunch_now(&app);
                    return;
                }
            }
            if !relaunch_gate(&app).can_relaunch
                || max_wait.is_some_and(|limit| started.elapsed() >= limit)
            {
                tracing::info!(
                    target: "system",
                    event_code = "system.permission_relaunch_skipped",
                    "relaunch skipped because Murmur stayed busy or cannot relaunch"
                );
                return;
            }
            std::thread::sleep(Duration::from_secs(2));
        }
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
/// process. Refused while Murmur is busy or running outside an installed app.
/// Returns "granted" | "restarting".
#[tauri::command]
pub async fn reset_accessibility_permission(app: tauri::AppHandle) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        tauri::async_runtime::spawn_blocking(move || {
            let Some(_guard) = AccessibilityRequestGuard::acquire() else {
                return Ok("in_progress".to_string());
            };
            if injector::is_accessibility_enabled() {
                return Ok("granted".to_string());
            }
            if let Some(refusal) = reset_refusal(relaunch_gate(&app)) {
                return Err(refusal.to_string());
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
/// and the running process cannot see the reset (see `relaunch_when_idle`).
/// Refused while Murmur is busy or running outside an installed app.
/// The relaunched process reads `notDetermined`, so the next dictation asks
/// macOS once, from the process that records the answer. Returns "restarting".
#[tauri::command]
pub async fn reset_microphone_permission(app: tauri::AppHandle) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        tauri::async_runtime::spawn_blocking(move || {
            if let Some(refusal) = reset_refusal(relaunch_gate(&app)) {
                return Err(refusal.to_string());
            }
            run_tccutil_reset("Microphone")?;
            relaunch_when_idle(app, Some(std::time::Duration::from_secs(3)));
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
        audio_inventory_request_allowed, fallback_accessibility_plan, grant_is_stale,
        mic_status_to_banner_state, plan_accessibility_request, reset_refusal,
        run_audio_inventory_read, should_open_settings_pane, should_relaunch_after_grant,
        take_relaunch_marker, AccessibilityRequestFacts, AccessibilityRequestGuard,
        AccessibilityRequestPlan, RelaunchGate,
    };

    const IDLE: RelaunchGate = RelaunchGate {
        can_relaunch: true,
        busy: None,
    };
    const BUSY: RelaunchGate = RelaunchGate {
        can_relaunch: true,
        busy: Some("recording"),
    };
    const DEV: RelaunchGate = RelaunchGate {
        can_relaunch: false,
        busy: None,
    };

    fn facts(
        trusted: bool,
        prompt_issued: bool,
        stale_grant: bool,
        gate: RelaunchGate,
    ) -> AccessibilityRequestFacts {
        AccessibilityRequestFacts {
            trusted,
            prompt_issued,
            stale_grant,
            gate,
        }
    }

    #[test]
    fn first_accessibility_request_prompts_and_never_opens_both() {
        assert_eq!(
            plan_accessibility_request(facts(false, false, false, IDLE)),
            AccessibilityRequestPlan::PromptSurface
        );
        // The surface step opens the pane only when no system dialog is showing.
        assert!(!should_open_settings_pane(true));
        assert!(should_open_settings_pane(false));
    }

    #[test]
    fn repeat_accessibility_request_only_opens_the_pane() {
        // macOS ignores a second prompt in the same process.
        assert_eq!(
            plan_accessibility_request(facts(false, true, false, IDLE)),
            AccessibilityRequestPlan::OpenPane
        );
    }

    #[test]
    fn granted_accessibility_request_shows_nothing() {
        for issued in [false, true] {
            for stale in [false, true] {
                for gate in [IDLE, BUSY, DEV] {
                    assert_eq!(
                        plan_accessibility_request(facts(true, issued, stale, gate)),
                        AccessibilityRequestPlan::AlreadyGranted
                    );
                }
            }
        }
    }

    #[test]
    fn a_stale_grant_resets_and_relaunches_only_when_idle_in_an_installed_app() {
        for issued in [false, true] {
            assert_eq!(
                plan_accessibility_request(facts(false, issued, true, IDLE)),
                AccessibilityRequestPlan::ResetAndRelaunch
            );
        }
    }

    #[test]
    fn a_stale_grant_while_busy_opens_the_pane_and_defers_the_restart() {
        assert_eq!(
            plan_accessibility_request(facts(false, false, true, BUSY)),
            AccessibilityRequestPlan::OpenPaneRestartDeferred
        );
    }

    #[test]
    fn a_dev_session_never_plans_a_reset_or_restart() {
        // Outside an installed .app the reset would target the wrong identity and
        // a restart would leave the dev tooling; use the ordinary request.
        assert_eq!(
            plan_accessibility_request(facts(false, false, true, DEV)),
            AccessibilityRequestPlan::PromptSurface
        );
        assert_eq!(
            plan_accessibility_request(facts(false, true, true, DEV)),
            AccessibilityRequestPlan::OpenPane
        );
    }

    #[test]
    fn a_failed_reset_falls_back_to_the_ordinary_request() {
        // The command forgets the stale grant when the reset fails, then takes
        // this path, so the main button is never stuck replanning a reset.
        assert_eq!(
            fallback_accessibility_plan(false),
            AccessibilityRequestPlan::PromptSurface
        );
        assert_eq!(
            fallback_accessibility_plan(true),
            AccessibilityRequestPlan::OpenPane
        );
        assert_eq!(
            plan_accessibility_request(facts(false, false, false, IDLE)),
            AccessibilityRequestPlan::PromptSurface
        );
    }

    #[test]
    fn explicit_resets_are_refused_while_busy_or_outside_an_installed_app() {
        assert_eq!(reset_refusal(IDLE), None);
        assert!(reset_refusal(BUSY).is_some_and(|m| m.contains("Finish")));
        assert!(reset_refusal(DEV).is_some_and(|m| m.contains("installed app")));
        // Not installed wins over busy: it can never succeed.
        let both = RelaunchGate {
            can_relaunch: false,
            busy: Some("meeting"),
        };
        assert!(reset_refusal(both).is_some_and(|m| m.contains("installed app")));
        assert!(IDLE.open());
        assert!(!BUSY.open());
        assert!(!DEV.open());
    }

    #[test]
    fn a_grant_is_stale_only_when_seen_under_a_different_build() {
        assert!(grant_is_stale(Some("0.65.0:10:1"), Some("0.65.0:11:2")));
        // Same build, still missing: access was switched off, not stale.
        assert!(!grant_is_stale(Some("0.65.0:10:1"), Some("0.65.0:10:1")));
        // Never granted, or fingerprint unreadable: nothing to call stale.
        assert!(!grant_is_stale(None, Some("0.65.0:10:1")));
        assert!(!grant_is_stale(Some("0.65.0:10:1"), None));
        assert!(!grant_is_stale(None, None));
    }

    #[test]
    fn a_grant_that_arrives_while_running_relaunches_once() {
        assert!(should_relaunch_after_grant(true, true, false, true));
        // Granted from the first reading: nothing was ever stale in this process.
        assert!(!should_relaunch_after_grant(false, true, false, true));
        // Not granted yet.
        assert!(!should_relaunch_after_grant(true, false, false, true));
        // Already scheduled: never loops.
        assert!(!should_relaunch_after_grant(true, true, true, true));
        // Dev session.
        assert!(!should_relaunch_after_grant(true, true, false, false));
    }

    fn scratch_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("murmur-permissions-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_relaunch_marker_is_consumed_exactly_once() {
        let dir = scratch_dir("marker");
        assert!(!take_relaunch_marker(&dir), "no marker, no request");
        std::fs::write(dir.join(super::ACCESSIBILITY_RELAUNCH_MARKER), b"1").unwrap();
        assert!(take_relaunch_marker(&dir));
        // A later launch finds nothing, so a relaunch can never loop.
        assert!(!take_relaunch_marker(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_request_guard_admits_one_request_at_a_time() {
        let first = AccessibilityRequestGuard::acquire().expect("first request runs");
        assert!(
            AccessibilityRequestGuard::acquire().is_none(),
            "a fast second click must not start a second sequence"
        );
        drop(first);
        assert!(AccessibilityRequestGuard::acquire().is_some());
    }

    #[cfg(target_os = "macos")]
    fn fake_opener(dir: &std::path::Path, body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("opener.sh");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[cfg(target_os = "macos")]
    fn wait_for(path: &std::path::Path, seconds: u64) -> bool {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(seconds);
        while std::time::Instant::now() < deadline {
            if path.exists() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        false
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_relaunch_waits_for_the_old_process_to_exit_before_opening() {
        let dir = scratch_dir("waits");
        let opened = dir.join("opened");
        let opener = fake_opener(&dir, &format!("echo \"$1\" > '{}'", opened.display()));
        let mut old = std::process::Command::new("sleep")
            .arg("2")
            .spawn()
            .unwrap();
        let bundle = dir.join("Murmur.app");
        let mut waiter = super::spawn_relaunch_waiter(
            &bundle,
            std::path::Path::new("/nonexistent"),
            old.id(),
            &opener,
        )
        .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1000));
        assert!(!opened.exists(), "opened while the old process was alive");
        old.wait().unwrap();
        assert!(wait_for(&opened, 5), "never opened after the old exit");
        assert!(std::fs::read_to_string(&opened)
            .unwrap()
            .contains("Murmur.app"));
        waiter.wait().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_failed_open_falls_back_to_running_the_executable() {
        let dir = scratch_dir("open-fails");
        let ran = dir.join("ran");
        let opener = fake_opener(&dir, "exit 1");
        let exe = dir.join("exe.sh");
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::write(&exe, format!("#!/bin/sh\ntouch '{}'\n", ran.display())).unwrap();
            std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        // A pid that is already gone: the wait ends at once.
        let mut gone = std::process::Command::new("true").spawn().unwrap();
        let pid = gone.id();
        gone.wait().unwrap();
        let mut waiter =
            super::spawn_relaunch_waiter(&dir.join("Murmur.app"), &exe, pid, &opener).unwrap();
        assert!(wait_for(&ran, 5), "executable fallback never ran");
        waiter.wait().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
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
