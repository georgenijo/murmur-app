//! Frontmost-app detection used by per-app dictation profiles.
//!
//! The primary query uses `NSWorkspace` directly. Transient unavailable/empty
//! results are retried briefly before the existing System Events AppleScript is
//! used once as a bounded compatibility fallback. The first successful sample
//! is returned to the caller and becomes part of its immutable recording
//! context; failures remain global-only and deny app-specific context reads.

use serde::Serialize;

const MAX_RUNNING_APPLICATIONS: usize = 64;

/// Privacy-bounded data exposed to the Settings picker. Process identifiers,
/// paths, launch arguments, window titles, and document state never cross the
/// command boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunningApplication {
    pub bundle_id: String,
    pub name: String,
}

/// Content-free identity frozen into a live recording. The process identifier
/// never crosses the Tauri command boundary or enters history/telemetry; it is
/// used only to keep a delayed auto-paste bound to its original application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontmostAppIdentity {
    pub bundle_id: Option<String>,
    pub process_id: Option<i32>,
}

/// User-visible metadata frozen for one Voice Query pass. This type must never
/// be logged or emitted on a broadcast event: app names and window titles can
/// reveal document content.
#[derive(Clone)]
pub struct FrontmostContextMetadata {
    pub app_name: String,
    pub window_title: Option<String>,
}

#[derive(Debug)]
struct RunningApplicationCandidate {
    bundle_id: Option<String>,
    name: Option<String>,
    regular: bool,
    current_process: bool,
}

fn bounded_running_applications(
    candidates: impl IntoIterator<Item = RunningApplicationCandidate>,
) -> Vec<RunningApplication> {
    let mut applications = candidates
        .into_iter()
        .filter(|candidate| candidate.regular && !candidate.current_process)
        .filter_map(|candidate| {
            let bundle_id = candidate.bundle_id?.trim().to_string();
            if bundle_id.is_empty() {
                return None;
            }
            let name = candidate.name.unwrap_or_default().trim().to_string();
            Some(RunningApplication {
                name: if name.is_empty() {
                    bundle_id.clone()
                } else {
                    name
                },
                bundle_id,
            })
        })
        .collect::<Vec<_>>();
    applications.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| {
                left.bundle_id
                    .to_lowercase()
                    .cmp(&right.bundle_id.to_lowercase())
            })
    });
    let mut seen = std::collections::HashSet::new();
    applications.retain(|application| seen.insert(application.bundle_id.to_lowercase()));
    applications.truncate(MAX_RUNNING_APPLICATIONS);
    applications
}

/// Return a bounded, ephemeral list for Settings. The caller owns the only
/// copy; this module does not cache or log app names or bundle identifiers.
#[tauri::command]
#[cfg(target_os = "macos")]
pub fn list_running_applications() -> Vec<RunningApplication> {
    use objc2_app_kit::{NSApplicationActivationPolicy, NSWorkspace};

    let current_pid = std::process::id() as i32;
    let candidates = NSWorkspace::sharedWorkspace()
        .runningApplications()
        .iter()
        .map(|application| RunningApplicationCandidate {
            bundle_id: application
                .bundleIdentifier()
                .map(|value| value.to_string()),
            name: application.localizedName().map(|value| value.to_string()),
            regular: application.activationPolicy() == NSApplicationActivationPolicy::Regular,
            current_process: application.processIdentifier() == current_pid,
        })
        .collect::<Vec<_>>();
    bounded_running_applications(candidates)
}

/// Unsupported test targets retain the command surface without probing
/// platform process state.
#[tauri::command]
#[cfg(not(target_os = "macos"))]
pub fn list_running_applications() -> Vec<RunningApplication> {
    Vec::new()
}

#[cfg(any(target_os = "macos", test))]
use std::time::Duration;

#[cfg(any(target_os = "macos", test))]
const MAX_NATIVE_ATTEMPTS: usize = 3;
#[cfg(any(target_os = "macos", test))]
const NATIVE_RETRY_DELAY: Duration = Duration::from_millis(10);

#[cfg(any(target_os = "macos", test))]
type QueryResult = Result<Option<String>, ()>;

#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DetectionSource {
    None,
    Native,
    Osascript,
}

#[cfg(any(target_os = "macos", test))]
impl DetectionSource {
    const fn code(self) -> u64 {
        match self {
            Self::None => 0,
            Self::Native => 1,
            Self::Osascript => 2,
        }
    }
}

#[cfg(any(target_os = "macos", test))]
#[derive(Debug, PartialEq, Eq)]
struct DetectionResult {
    bundle_id: Option<String>,
    source: DetectionSource,
    retry_count: usize,
}

#[cfg(any(target_os = "macos", test))]
impl DetectionResult {
    fn outcome_code(&self) -> u64 {
        u64::from(self.bundle_id.is_some())
    }
}

#[cfg(any(target_os = "macos", test))]
fn normalized_bundle_id(result: QueryResult) -> Option<String> {
    result.ok().flatten().and_then(|bundle_id| {
        let bundle_id = bundle_id.trim();
        (!bundle_id.is_empty()).then(|| bundle_id.to_string())
    })
}

#[cfg(any(target_os = "macos", test))]
fn detect_with<N, F, S>(mut native: N, mut fallback: F, mut sleep: S) -> DetectionResult
where
    N: FnMut() -> QueryResult,
    F: FnMut() -> QueryResult,
    S: FnMut(Duration),
{
    for attempt in 0..MAX_NATIVE_ATTEMPTS {
        if let Some(bundle_id) = normalized_bundle_id(native()) {
            return DetectionResult {
                bundle_id: Some(bundle_id),
                source: DetectionSource::Native,
                retry_count: attempt,
            };
        }

        if attempt + 1 < MAX_NATIVE_ATTEMPTS {
            sleep(NATIVE_RETRY_DELAY);
        }
    }

    let retry_count = MAX_NATIVE_ATTEMPTS.saturating_sub(1);
    if let Some(bundle_id) = normalized_bundle_id(fallback()) {
        DetectionResult {
            bundle_id: Some(bundle_id),
            source: DetectionSource::Osascript,
            retry_count,
        }
    } else {
        DetectionResult {
            bundle_id: None,
            source: DetectionSource::None,
            retry_count,
        }
    }
}

#[cfg(target_os = "macos")]
fn native_frontmost_bundle_id() -> QueryResult {
    use objc2_app_kit::NSWorkspace;

    let app = NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .ok_or(())?;
    Ok(app.bundleIdentifier().map(|value| value.to_string()))
}

#[cfg(target_os = "macos")]
fn osascript_frontmost_bundle_id() -> QueryResult {
    let output = crate::injector::run_osascript_with_timeout(
        r#"tell application "System Events" to get bundle identifier of first process whose frontmost is true"#,
    )
    .map_err(|_| ())?;

    if !output.status.success() {
        return Err(());
    }

    Ok(Some(String::from_utf8_lossy(&output.stdout).into_owned()))
}

/// Return the bundle identifier of the first frontmost macOS app observed by
/// the bounded detector. Returns `None` on total failure so the caller resolves
/// a global-only dictation context.
#[cfg(target_os = "macos")]
pub fn frontmost_bundle_id() -> Option<String> {
    let started = std::time::Instant::now();
    let result = detect_with(
        native_frontmost_bundle_id,
        osascript_frontmost_bundle_id,
        std::thread::sleep,
    );
    tracing::info!(
        target: "pipeline",
        outcome_code = result.outcome_code(),
        retry_count = result.retry_count as u64,
        source_code = result.source.code(),
        elapsed_ms = started.elapsed().as_millis() as u64,
        "frontmost app detection completed"
    );
    result.bundle_id
}

/// Capture the frontmost bundle and PID as one privacy-bounded recording input.
/// The bounded bundle detector retains its compatibility fallback. A PID is
/// accepted only when a second native sample still reports the same bundle,
/// preventing a focus transition from pairing identities from different apps.
#[cfg(target_os = "macos")]
pub fn frontmost_app_identity() -> FrontmostAppIdentity {
    use objc2_app_kit::NSWorkspace;

    let bundle_id = frontmost_bundle_id();
    let process_id = NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .and_then(|application| {
            let native_bundle = application
                .bundleIdentifier()
                .map(|value| value.to_string());
            (native_bundle == bundle_id).then(|| application.processIdentifier())
        });
    FrontmostAppIdentity {
        bundle_id,
        process_id,
    }
}

/// Read the display name and focused-window title only when the native
/// frontmost application still matches the identity already frozen by the
/// caller. A focus change makes context unavailable instead of pairing the
/// original app identity with a newer app's title.
#[cfg(target_os = "macos")]
pub async fn frontmost_context_metadata(
    app_handle: &tauri::AppHandle,
    identity: &FrontmostAppIdentity,
) -> Option<FrontmostContextMetadata> {
    let expected_pid = identity.process_id?;
    let expected_bundle = identity.bundle_id.clone();
    let (tx, rx) = tokio::sync::oneshot::channel();
    app_handle
        .run_on_main_thread(move || {
            let _ = tx.send(native_context_metadata(
                expected_pid,
                expected_bundle.as_deref(),
            ));
        })
        .ok()?;
    rx.await.ok().flatten()
}

#[cfg(target_os = "macos")]
fn native_context_metadata(
    expected_pid: i32,
    expected_bundle: Option<&str>,
) -> Option<FrontmostContextMetadata> {
    use objc2_app_kit::NSWorkspace;

    let app = NSWorkspace::sharedWorkspace().frontmostApplication()?;
    if app.processIdentifier() != expected_pid {
        return None;
    }
    let bundle = app.bundleIdentifier().map(|value| value.to_string());
    if bundle.as_deref() != expected_bundle {
        return None;
    }
    let app_name = app
        .localizedName()
        .map(|value| value.to_string())
        .filter(|value| !value.trim().is_empty())
        .or(bundle)?;
    Some(FrontmostContextMetadata {
        app_name,
        window_title: native_window_title(expected_pid),
    })
}

#[cfg(target_os = "macos")]
fn native_window_title(pid: i32) -> Option<String> {
    use std::ffi::{c_char, c_void, CStr, CString};

    type AXUIElementRef = *const c_void;
    type CFTypeRef = *const c_void;
    type CFIndex = isize;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
        fn AXUIElementSetMessagingTimeout(element: AXUIElementRef, timeout: f32) -> i32;
        fn AXUIElementCopyAttributeValue(
            element: AXUIElementRef,
            attribute: CFTypeRef,
            value: *mut CFTypeRef,
        ) -> i32;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFStringCreateWithCString(
            allocator: *const c_void,
            c_str: *const c_char,
            encoding: u32,
        ) -> CFTypeRef;
        fn CFStringGetCStringPtr(string: CFTypeRef, encoding: u32) -> *const c_char;
        fn CFStringGetLength(string: CFTypeRef) -> CFIndex;
        fn CFStringGetMaximumSizeForEncoding(length: CFIndex, encoding: u32) -> CFIndex;
        fn CFStringGetCString(
            string: CFTypeRef,
            buffer: *mut c_char,
            buffer_size: CFIndex,
            encoding: u32,
        ) -> bool;
        fn CFRelease(value: CFTypeRef);
    }

    const UTF8: u32 = 0x0800_0100;

    unsafe fn attribute(element: AXUIElementRef, name: &str) -> Option<CFTypeRef> {
        let name = CString::new(name).ok()?;
        let key = unsafe { CFStringCreateWithCString(std::ptr::null(), name.as_ptr(), UTF8) };
        if key.is_null() {
            return None;
        }
        let mut value: CFTypeRef = std::ptr::null();
        let status = unsafe { AXUIElementCopyAttributeValue(element, key, &mut value) };
        unsafe { CFRelease(key) };
        (status == 0 && !value.is_null()).then_some(value)
    }

    unsafe fn string(value: CFTypeRef) -> Option<String> {
        let direct = unsafe { CFStringGetCStringPtr(value, UTF8) };
        if !direct.is_null() {
            return Some(
                unsafe { CStr::from_ptr(direct) }
                    .to_string_lossy()
                    .into_owned(),
            );
        }
        let length = unsafe { CFStringGetLength(value) };
        let capacity = unsafe { CFStringGetMaximumSizeForEncoding(length, UTF8) } + 1;
        if capacity <= 1 {
            return None;
        }
        let mut buffer = vec![0_u8; capacity as usize];
        unsafe { CFStringGetCString(value, buffer.as_mut_ptr().cast(), capacity, UTF8) }.then(
            || {
                unsafe { CStr::from_ptr(buffer.as_ptr().cast()) }
                    .to_string_lossy()
                    .into_owned()
            },
        )
    }

    let application = unsafe { AXUIElementCreateApplication(pid) };
    if application.is_null() {
        return None;
    }
    let _ = unsafe { AXUIElementSetMessagingTimeout(application, 0.025) };
    let window = unsafe { attribute(application, "AXFocusedWindow") };
    unsafe { CFRelease(application) };
    let window = window?;
    let title = unsafe { attribute(window.cast(), "AXTitle") };
    unsafe { CFRelease(window) };
    let title = title?;
    let text = unsafe { string(title) };
    unsafe { CFRelease(title) };
    text.filter(|value| !value.trim().is_empty())
}

/// Non-macOS platforms have no frontmost-app concept here; profiles are a no-op.
#[cfg(not(target_os = "macos"))]
pub fn frontmost_bundle_id() -> Option<String> {
    None
}

#[cfg(not(target_os = "macos"))]
pub fn frontmost_app_identity() -> FrontmostAppIdentity {
    FrontmostAppIdentity {
        bundle_id: None,
        process_id: None,
    }
}

#[cfg(not(target_os = "macos"))]
pub async fn frontmost_context_metadata(
    _app_handle: &tauri::AppHandle,
    _identity: &FrontmostAppIdentity,
) -> Option<FrontmostContextMetadata> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::collections::VecDeque;

    fn candidate(id: &str, name: &str) -> RunningApplicationCandidate {
        RunningApplicationCandidate {
            bundle_id: Some(id.to_string()),
            name: Some(name.to_string()),
            regular: true,
            current_process: false,
        }
    }

    #[test]
    fn running_app_picker_is_sorted_deduplicated_and_private_by_default() {
        let mut candidates = vec![
            candidate("com.example.zulu", "Zulu"),
            candidate("com.example.alpha", "Alpha"),
        ];
        candidates.push(candidate("COM.EXAMPLE.03", "Duplicate"));
        candidates.push(candidate("com.example.03", "Elsewhere in sort order"));
        candidates.push(RunningApplicationCandidate {
            bundle_id: Some("com.example.menu".into()),
            name: Some("Menu helper".into()),
            regular: false,
            current_process: false,
        });
        candidates.push(RunningApplicationCandidate {
            bundle_id: Some("com.example.murmur".into()),
            name: Some("Murmur".into()),
            regular: true,
            current_process: true,
        });

        let applications = bounded_running_applications(candidates);

        assert_eq!(applications.len(), 3);
        assert_eq!(applications[0].name, "Alpha");
        assert_eq!(
            applications
                .iter()
                .filter(|app| app.bundle_id.eq_ignore_ascii_case("com.example.03"))
                .count(),
            1
        );
        assert!(applications
            .iter()
            .all(|app| app.bundle_id != "com.example.menu"));
        assert!(applications
            .iter()
            .all(|app| app.bundle_id != "com.example.murmur"));
    }

    #[test]
    fn running_app_picker_is_bounded() {
        let candidates = (0..80)
            .map(|index| {
                candidate(
                    &format!("com.example.{index:02}"),
                    &format!("App {index:02}"),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            bounded_running_applications(candidates).len(),
            MAX_RUNNING_APPLICATIONS
        );
    }

    #[test]
    fn running_app_payload_contains_only_picker_fields() {
        let payload = serde_json::to_value(RunningApplication {
            bundle_id: "com.apple.Terminal".into(),
            name: "Terminal".into(),
        })
        .expect("serialize picker payload");

        assert_eq!(payload.as_object().expect("object").len(), 2);
        assert_eq!(payload["bundleId"], "com.apple.Terminal");
        assert_eq!(payload["name"], "Terminal");
    }

    #[test]
    fn immediate_native_success_skips_retry_and_fallback() {
        let native_calls = Cell::new(0);
        let fallback_calls = Cell::new(0);
        let sleep_calls = Cell::new(0);

        let result = detect_with(
            || {
                native_calls.set(native_calls.get() + 1);
                Ok(Some(" com.apple.Terminal ".to_string()))
            },
            || {
                fallback_calls.set(fallback_calls.get() + 1);
                Ok(Some("fallback".to_string()))
            },
            |_| sleep_calls.set(sleep_calls.get() + 1),
        );

        assert_eq!(result.bundle_id.as_deref(), Some("com.apple.Terminal"));
        assert_eq!(result.source, DetectionSource::Native);
        assert_eq!(result.retry_count, 0);
        assert_eq!(native_calls.get(), 1);
        assert_eq!(fallback_calls.get(), 0);
        assert_eq!(sleep_calls.get(), 0);
    }

    #[test]
    fn transient_native_failures_retry_until_success() {
        let mut native_results = VecDeque::from([
            Err(()),
            Ok(Some("  ".to_string())),
            Ok(Some("com.todesktop.cursor".to_string())),
        ]);
        let fallback_calls = Cell::new(0);
        let sleep_calls = Cell::new(0);

        let result = detect_with(
            || native_results.pop_front().expect("bounded native attempt"),
            || {
                fallback_calls.set(fallback_calls.get() + 1);
                Err(())
            },
            |delay| {
                assert_eq!(delay, NATIVE_RETRY_DELAY);
                sleep_calls.set(sleep_calls.get() + 1);
            },
        );

        assert_eq!(result.bundle_id.as_deref(), Some("com.todesktop.cursor"));
        assert_eq!(result.source, DetectionSource::Native);
        assert_eq!(result.retry_count, 2);
        assert_eq!(fallback_calls.get(), 0);
        assert_eq!(sleep_calls.get(), 2);
    }

    #[test]
    fn fallback_succeeds_after_native_attempts_are_exhausted() {
        let native_calls = Cell::new(0);
        let fallback_calls = Cell::new(0);

        let result = detect_with(
            || {
                native_calls.set(native_calls.get() + 1);
                Err(())
            },
            || {
                fallback_calls.set(fallback_calls.get() + 1);
                Ok(Some("com.apple.Safari".to_string()))
            },
            |_| {},
        );

        assert_eq!(result.bundle_id.as_deref(), Some("com.apple.Safari"));
        assert_eq!(result.source, DetectionSource::Osascript);
        assert_eq!(result.retry_count, 2);
        assert_eq!(native_calls.get(), MAX_NATIVE_ATTEMPTS);
        assert_eq!(fallback_calls.get(), 1);
    }

    #[test]
    fn total_failure_is_bounded_and_deny_by_default() {
        let native_calls = Cell::new(0);
        let fallback_calls = Cell::new(0);
        let sleep_calls = Cell::new(0);

        let result = detect_with(
            || {
                native_calls.set(native_calls.get() + 1);
                Err(())
            },
            || {
                fallback_calls.set(fallback_calls.get() + 1);
                Ok(Some(String::new()))
            },
            |_| sleep_calls.set(sleep_calls.get() + 1),
        );

        assert_eq!(result.bundle_id, None);
        assert_eq!(result.source, DetectionSource::None);
        assert_eq!(result.outcome_code(), 0);
        assert_eq!(result.retry_count, 2);
        assert_eq!(native_calls.get(), MAX_NATIVE_ATTEMPTS);
        assert_eq!(fallback_calls.get(), 1);
        assert_eq!(sleep_calls.get(), MAX_NATIVE_ATTEMPTS - 1);
    }

    #[test]
    fn app_change_during_retry_uses_first_successful_sample() {
        let mut native_results = VecDeque::from([
            Err(()),
            Ok(Some("com.apple.Terminal".to_string())),
            Ok(Some("com.apple.Safari".to_string())),
        ]);

        let result = detect_with(
            || native_results.pop_front().expect("bounded native attempt"),
            || Err(()),
            |_| {},
        );

        assert_eq!(result.bundle_id.as_deref(), Some("com.apple.Terminal"));
        assert_eq!(result.retry_count, 1);
        assert_eq!(
            native_results.len(),
            1,
            "later focus changes are not sampled"
        );
    }

    #[test]
    fn first_success_is_immutable_even_if_the_app_would_change() {
        let mut native_results = VecDeque::from([
            Ok(Some("com.apple.Terminal".to_string())),
            Ok(Some("com.apple.Safari".to_string())),
        ]);

        let result = detect_with(
            || native_results.pop_front().expect("bounded native attempt"),
            || Err(()),
            |_| {},
        );

        assert_eq!(result.bundle_id.as_deref(), Some("com.apple.Terminal"));
        assert_eq!(result.retry_count, 0);
        assert_eq!(native_results.len(), 1, "detector must not re-read focus");
    }
}
