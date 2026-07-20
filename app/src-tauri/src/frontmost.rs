//! Frontmost-app detection used by per-app dictation profiles.
//!
//! The primary query uses `NSWorkspace` directly. Transient unavailable/empty
//! results are retried briefly before the existing System Events AppleScript is
//! used once as a bounded compatibility fallback. The first successful sample
//! is returned to the caller and becomes part of its immutable recording
//! context; failures remain global-only and deny app-specific context reads.

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

/// Non-macOS platforms have no frontmost-app concept here; profiles are a no-op.
#[cfg(not(target_os = "macos"))]
pub fn frontmost_bundle_id() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::collections::VecDeque;

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
