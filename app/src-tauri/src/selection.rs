//! Accessibility (AX) capture of the current text selection, used by the
//! transform pipeline (issue #312, PR-B1). macOS-only in practice — every
//! function that touches the AX APIs is `#[cfg(target_os = "macos")]` — but
//! the module compiles cross-platform so the pure classification logic
//! (`classify_selection`, `length_bucket`, `is_secure_subrole`/`is_secure_role`)
//! is unit-testable without Accessibility permission or a running AX server.
//!
//! Fails closed: any ambiguity about whether reading is safe (secure field,
//! Accessibility not granted, oversized selection, AX query failure) produces
//! an error and nothing beyond the minimum needed to classify it is read.
//!
//! Privacy: `TransformSnapshot.text` must NEVER be logged, sent over telemetry,
//! or serialized wholesale to the frontend. Only `length_bucket(...)` and the
//! `SelectionError`/outcome enums are safe to log — see `log_capture_outcome`.
//!
//! This duplicates a small amount of the raw AX FFI scaffolding already in
//! `injector.rs` (CFString conversion, `AXUIElementCreateApplication`, the
//! per-element messaging timeout) rather than refactoring it into a shared
//! module. `injector.rs`'s AX path backs the paste-guard (`focused_field_state`)
//! and is exercised by its own test suite; keeping this capture path
//! self-contained avoids any risk of regressing that already-reviewed code
//! for this PR. A follow-up can consolidate if a third AX caller appears.
//!
//! No command wires `capture_selection` to the frontend yet — issue #312's
//! transform *pipeline* (capture -> LLM -> review -> apply) is a later PR in
//! the series. This module allows dead_code accordingly.

#![allow(dead_code)]

use std::time::Instant;

/// Hard cap on captured selection size (UTF-8 bytes). Selections larger than
/// this are refused outright (fail closed) rather than truncated — a silent
/// truncation could feed the transform pipeline an incomplete or misleading
/// context.
pub const MAX_SELECTION_BYTES: usize = 16384;

/// Screen-space bounding rectangle for the current selection, in the same
/// coordinate space `AXBoundsForRange` reports (top-left origin, points).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Immutable snapshot of a captured selection, owned app-side.
///
/// NEVER serialize this wholesale to the frontend and NEVER log `text` — only
/// length buckets and outcome enums (see `length_bucket`, `log_capture_outcome`).
#[derive(Clone)]
pub struct TransformSnapshot {
    pub bundle_id: Option<String>,
    pub pid: i32,
    pub text: String,
    pub range: Option<(usize, usize)>,
    pub bounds: Option<Rect>,
    pub captured_at: Instant,
}

impl std::fmt::Debug for TransformSnapshot {
    /// Manual impl instead of `#[derive(Debug)]`: the never-log rule on
    /// `text` (see the module doc comment) needs to be structural, not just a
    /// convention every future call site has to remember. Prints the length
    /// bucket instead of the raw text.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TransformSnapshot")
            .field("bundle_id", &self.bundle_id)
            .field("pid", &self.pid)
            .field("text_length_bucket", &length_bucket(self.text.len()))
            .field("range", &self.range)
            .field("bounds", &self.bounds)
            .field("captured_at", &self.captured_at)
            .finish()
    }
}

/// Reasons `capture_selection` can fail. Every variant is loggable on its own
/// (no payload) — never attach the selection text to any of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionError {
    /// Accessibility permission is not granted app-wide.
    AccessibilityDenied,
    /// The focused element is a secure/password field. Nothing beyond the
    /// role/subrole was read.
    SecureField,
    /// No frontmost application, no focused element, or the focused element
    /// reports an empty selection.
    NoSelection,
    /// Selection exceeds `MAX_SELECTION_BYTES` UTF-8 bytes. Refused outright,
    /// never truncated.
    TooLarge,
    /// Native AX query failed or is unavailable. No osascript fallback in v1
    /// (unlike `injector::focused_field_state`) — a native failure is a hard
    /// error here.
    AxUnavailable,
    /// The secure-field check itself errored (non-benign AXSubrole/AXRole
    /// status, or the messaging timeout could not be armed on the focused
    /// element). We could not prove the focused element is NOT a password
    /// field, so this is terminal for the clipboard fallback — only the AX
    /// retry loop (which re-runs the full check) may try again (issue #334).
    SecureCheckFailed,
}

impl SelectionError {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AccessibilityDenied => "accessibility_denied",
            Self::SecureField => "secure_field",
            Self::NoSelection => "no_selection",
            Self::TooLarge => "too_large",
            Self::AxUnavailable => "ax_unavailable",
            Self::SecureCheckFailed => "secure_check_failed",
        }
    }
}

/// Whether an AX-path failure may be retried by the warm-up retry loop in
/// `capture_selection`. Retrying is always safe — a retry re-runs the full
/// secure-field check and reads nothing unless that check passes — so this
/// includes `SecureCheckFailed` (Chromium's lazy AX tree times out the first
/// subrole queries; the retries are what warm it).
fn retry_eligible(error: SelectionError) -> bool {
    matches!(
        error,
        SelectionError::NoSelection
            | SelectionError::AxUnavailable
            | SelectionError::SecureCheckFailed
    )
}

/// Whether an AX-path failure may fall back to the clipboard capture.
/// Fail-closed set (issue #334): a positively detected `SecureField`, missing
/// Accessibility permission, and — critically — an errored secure-field check
/// (`SecureCheckFailed`) never fall back: if we could not prove the focused
/// element is not a password field, we do not post a synthetic Cmd+C at it.
fn fallback_eligible(error: SelectionError) -> bool {
    matches!(
        error,
        SelectionError::NoSelection | SelectionError::AxUnavailable
    )
}

/// Capture-granularity fallback decision (issue #334). `fallback_eligible`
/// judges only the FINAL attempt's error, but the secure-check error must be
/// sticky across the whole retry ladder: if ANY attempt reached a focused
/// element and then failed to complete the secure-field check, a later
/// attempt failing somewhere shallower (e.g. the focused-element query timing
/// out → `AxUnavailable`) must not launder the capture back into fallback
/// eligibility. Chromium's 25ms timeouts fail at nondeterministic query
/// points across warm-up attempts, so this laundering path is real, not
/// theoretical.
fn fallback_allowed(final_error: SelectionError, secure_check_errored: bool) -> bool {
    fallback_eligible(final_error) && !secure_check_errored
}

/// Bucket a byte length for privacy-safe logging. Never log the raw length or
/// text — only which bucket it falls in.
pub fn length_bucket(bytes: usize) -> &'static str {
    match bytes {
        0 => "0",
        1..=16 => "1-16",
        17..=64 => "17-64",
        65..=256 => "65-256",
        257..=1024 => "257-1024",
        1025..=4096 => "1025-4096",
        4097..=16384 => "4097-16384",
        _ => ">16384",
    }
}

/// Log the outcome of a capture attempt. Only length buckets and outcome
/// enums ever reach the log line — the selection text itself never does.
pub fn log_capture_outcome(result: &Result<TransformSnapshot, SelectionError>) {
    log_capture_outcome_for_pass(result, 0);
}

/// Correlated production variant of [`log_capture_outcome`].
pub fn log_capture_outcome_for_pass(
    result: &Result<TransformSnapshot, SelectionError>,
    transform_pass_id: u64,
) {
    match result {
        Ok(snapshot) => {
            tracing::info!(
                target: "transform",
                transform_pass_id,
                outcome = "ok",
                length_bucket = length_bucket(snapshot.text.len()),
                has_range = snapshot.range.is_some(),
                has_bounds = snapshot.bounds.is_some(),
                "selection captured"
            );
        }
        Err(error) => {
            tracing::info!(
                target: "transform",
                transform_pass_id,
                outcome = error.as_str(),
                "selection capture failed"
            );
        }
    }
}

/// Subrole reported by a secure/password text field. Checked BEFORE any other
/// attribute is read — a positive match reads nothing else.
fn is_secure_subrole(subrole: &str) -> bool {
    matches!(subrole.trim(), "AXSecureTextField")
}

/// Role string that independently indicates a secure field even without a
/// matching subrole. Defense in depth — `AXSubrole` is the primary signal,
/// but some apps surface the secure marker only on `AXRole`.
fn is_secure_role(role: &str) -> bool {
    matches!(role.trim(), "AXSecureTextField")
}

/// `kAXErrorNoValue` (ApplicationServices/HIServices `AXError.h`) — the
/// element genuinely has no value for the requested attribute.
const AX_ERROR_NO_VALUE: i32 = -25212;
/// `kAXErrorAttributeUnsupported` — the attribute doesn't apply to this
/// element at all.
const AX_ERROR_ATTRIBUTE_UNSUPPORTED: i32 = -25205;
/// `kAXErrorCannotComplete` — generic failure. Notably what the 25ms
/// messaging timeout (`AX_QUERY_TIMEOUT_SECONDS` in `native`) surfaces as.
/// This is NOT benign: an ambiguous timeout must fail closed, never be read
/// as "no subrole/role".
const AX_ERROR_CANNOT_COMPLETE: i32 = -25204;

/// Classify a raw `AXError` status code returned when reading `AXSubrole` or
/// `AXRole` during the secure-field check, deciding whether it's safe to
/// continue past that check. Fails closed: only "the element has no value
/// for this attribute" and "this attribute doesn't apply to this element" are
/// benign — every other status, including the query timeout, must abort the
/// capture (`SelectionError::AxUnavailable`) rather than silently fall
/// through to reading the selection. Pure so it's unit-testable without
/// Accessibility permission or a running AX server.
fn is_benign_role_query_error(status: i32) -> bool {
    matches!(status, AX_ERROR_NO_VALUE | AX_ERROR_ATTRIBUTE_UNSUPPORTED)
}

/// Pure classification of a raw AX text read into either "usable" (`Ok`) or a
/// `SelectionError`. Factored out from AX I/O so it's testable without
/// Accessibility permission or a running AX server.
fn classify_selection(text: &str) -> Result<(), SelectionError> {
    if text.is_empty() {
        return Err(SelectionError::NoSelection);
    }
    if text.len() > MAX_SELECTION_BYTES {
        return Err(SelectionError::TooLarge);
    }
    Ok(())
}

/// Capture the current AX text selection.
///
/// AX calls must run on the main thread (same constraint as
/// `injector::inject_text`'s focus query), so this dispatches the native work
/// via `run_on_main_thread` and awaits the result on a oneshot channel —
/// mirroring the dispatch pattern already used for `inject_text` in
/// `commands/recording.rs`.
///
/// Checks Accessibility up front (fail fast with a distinct error) before
/// dispatching to the main thread at all.
///
/// Clipboard fallback (issue #329): Chromium/Electron webviews (Brave, Chrome,
/// Slack, …) often expose no `AXSelectedText` — or fail/time out the AX
/// queries entirely — even with a live visible selection. `NoSelection` and
/// `AxUnavailable` fall back to a simulated Cmd+C against a sentinel-primed
/// clipboard (see `clipboard_fallback` and the safety rationale at the match
/// arm below). `SecureField` and `AccessibilityDenied` stay fail-closed with
/// no fallback.
pub async fn capture_selection(
    app_handle: &tauri::AppHandle,
    transform_pass_id: u64,
) -> Result<TransformSnapshot, SelectionError> {
    if !crate::injector::is_accessibility_enabled() {
        crate::transform_trace::capture_attempt(
            transform_pass_id,
            0,
            SelectionError::AccessibilityDenied.as_str(),
            0,
        );
        crate::transform_trace::capture_path(
            transform_pass_id,
            "preflight",
            SelectionError::AccessibilityDenied.as_str(),
            0,
            None,
        );
        return Err(SelectionError::AccessibilityDenied);
    }

    #[cfg(target_os = "macos")]
    {
        let capture_started = Instant::now();
        type AxReply = (
            Result<TransformSnapshot, SelectionError>,
            Option<(i32, Option<String>)>,
        );
        // Chromium builds its accessibility tree lazily: the FIRST AX queries
        // against a Chromium app fail or time out, and the very act of
        // querying flips its "assistive client present" switch and starts the
        // tree build. Observed live on Brave (issue #329): three presses
        // failed with ax_unavailable, the fourth succeeded with full
        // range+bounds. So retry the AX capture a couple of times with a
        // warm-up gap before falling back — the retries run while the user is
        // still holding the key/speaking, so the latency is invisible, and an
        // AX capture is strictly better than the clipboard fallback (it has
        // range + bounds for anchoring and AX write-back).
        const AX_ATTEMPTS: u32 = 3;
        const AX_RETRY_GAP: std::time::Duration = std::time::Duration::from_millis(250);

        let mut ax_result = Err(SelectionError::AxUnavailable);
        let mut frontmost: Option<(i32, Option<String>)> = None;
        // Sticky across attempts (issue #334): once any attempt errors the
        // secure-field check, the whole capture is barred from the clipboard
        // fallback — see `fallback_allowed`.
        let mut secure_check_errored = false;
        for attempt in 0..AX_ATTEMPTS {
            if attempt > 0 {
                tokio::time::sleep(AX_RETRY_GAP).await;
            }
            let attempt_started = Instant::now();
            let (tx, rx) = tokio::sync::oneshot::channel::<AxReply>();
            app_handle
                .run_on_main_thread(move || {
                    let _ = tx.send((
                        native::capture_selection_native(),
                        native::frontmost_pid_bundle(),
                    ));
                })
                .map_err(|_| SelectionError::AxUnavailable)?;
            let (result, fm) = rx
                .await
                .unwrap_or((Err(SelectionError::AxUnavailable), None));
            ax_result = result;
            frontmost = fm;
            let attempt_outcome = match &ax_result {
                Ok(_) => "ok",
                Err(error) => error.as_str(),
            };
            crate::transform_trace::capture_attempt(
                transform_pass_id,
                attempt + 1,
                attempt_outcome,
                attempt_started.elapsed().as_millis() as u64,
            );
            if matches!(ax_result, Err(SelectionError::SecureCheckFailed)) {
                secure_check_errored = true;
            }
            match ax_result {
                // Retry only the "AX couldn't produce a selection" cases
                // (including an errored secure-field check, which a retry
                // re-runs from scratch) — success and the fail-closed errors
                // (SecureField, AccessibilityDenied, TooLarge) are final.
                Err(err) if retry_eligible(err) => {
                    if attempt + 1 < AX_ATTEMPTS {
                        tracing::info!(
                            target: "transform",
                            transform_pass_id,
                            attempt = attempt + 1,
                            "AX capture incomplete — retrying after warm-up gap"
                        );
                    }
                }
                _ => break,
            }
        }

        match (ax_result, frontmost) {
            // AX couldn't produce a selection + a known frontmost app: try the
            // clipboard fallback off the main thread (it sleeps while polling).
            //
            // `NoSelection`: the secure-field checks passed benignly and the
            // element simply exposes no `AXSelectedText`.
            //
            // `AxUnavailable`: the AX queries themselves failed — Chromium
            // browsers routinely time out the 25ms messaging deadline or fail
            // the focused-element query outright (the same -25204/-25212
            // behavior documented in `injector::focused_field_state`), so the
            // secure-field check never completed. Falling back is still safe:
            // the fallback only simulates the user's own Cmd+C gesture and
            // reads nothing via AX. Secure fields refuse Copy system-wide
            // (NSSecureTextField disables it at the framework level; browsers
            // block password-field copy), so against a secure field the
            // sentinel never changes and the fallback times out — it can fail,
            // never leak. `AccessibilityDenied`, a positively detected
            // `SecureField`, and an errored secure-field check
            // (`SecureCheckFailed`, issue #334) stay hard-blocked with no
            // fallback: if the check itself failed we could not prove the
            // focused element is not a password field, so no synthetic Cmd+C.
            // The bar is sticky across the retry ladder (`fallback_allowed`):
            // an errored secure check on ANY attempt bars the fallback even if
            // a later attempt fails shallower (`AxUnavailable`).
            (Err(err), Some((pid, bundle_id))) if fallback_allowed(err, secure_check_errored) => {
                tracing::info!(
                    target: "transform",
                    transform_pass_id,
                    ax_outcome = err.as_str(),
                    "AX capture incomplete — attempting clipboard fallback"
                );
                tokio::task::spawn_blocking(move || {
                    clipboard_fallback::capture_via_clipboard(
                        pid,
                        bundle_id,
                        err,
                        transform_pass_id,
                    )
                })
                .await
                .unwrap_or(Err(err))
            }
            (result, _) => {
                let outcome = match &result {
                    Ok(_) => "ok",
                    Err(error) => error.as_str(),
                };
                let length_bucket = result
                    .as_ref()
                    .ok()
                    .map(|snapshot| length_bucket(snapshot.text.len()));
                crate::transform_trace::capture_path(
                    transform_pass_id,
                    "ax_attempt",
                    outcome,
                    capture_started.elapsed().as_millis() as u64,
                    length_bucket,
                );
                result
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = app_handle;
        Err(SelectionError::AxUnavailable)
    }
}

/// Clipboard-based selection capture (issue #329), used when the AX path
/// returned `NoSelection` (secure-field checks passed benignly, no
/// `AXSelectedText` exposed) or `AxUnavailable` (AX queries failed/timed out —
/// Chromium/Electron webviews are the main case; see the safety rationale at
/// the `capture_selection` match arm: this only reproduces the user's own
/// Cmd+C gesture, which secure fields refuse system-wide).
///
/// Known limitation: capture runs while the user physically holds the
/// transform key, and some apps (observed: Brave) read the hardware modifier
/// state rather than the synthetic event's flags — seeing Cmd+Opt+C instead
/// of Cmd+C, which is not Copy. The AX retry loop in `capture_selection` is
/// therefore the primary browser path (Chromium's AX tree warms after the
/// first queries); this fallback is the last resort for genuinely AX-less
/// targets whose copy handling honors the event flags.
///
/// Strategy: snapshot the full pasteboard (every item, every type — images
/// and files survive, issue #335), overwrite it with a unique text sentinel,
/// record the pasteboard `changeCount`, post a synthetic Cmd+C, and poll until
/// the pasteboard shows exactly one ownership change past the sentinel write
/// with non-sentinel text (or a 300ms deadline passes — with nothing selected,
/// Cmd+C is a no-op and the count never moves). More than one change means a
/// third-party writer interleaved and the text is rejected. The user's
/// pasteboard is restored in full afterwards, success or failure. Remaining
/// limitation: the captured snapshot has no AX range/bounds (the popover
/// centers; apply uses the paste fallback).
#[cfg(target_os = "macos")]
mod clipboard_fallback {
    use super::{SelectionError, TransformSnapshot};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    const POLL_INTERVAL: Duration = Duration::from_millis(20);
    const POLL_DEADLINE: Duration = Duration::from_millis(300);

    static CAPTURE_COUNTER: AtomicU64 = AtomicU64::new(0);

    /// Full-fidelity pasteboard snapshot: every item with every type's raw
    /// data, so images/files/rich text survive the sentinel dance (issue #335
    /// defect A — the previous text-only snapshot destroyed any non-text
    /// clipboard). Content never leaves this module and is never logged.
    pub(super) type PasteboardSnapshot = Vec<Vec<(String, Vec<u8>)>>;

    /// NSPasteboard access. arboard also talks to NSPasteboard from arbitrary
    /// threads, and this module already runs inside `spawn_blocking`, so
    /// off-main use here matches the existing clipboard I/O in `injector`.
    mod pasteboard {
        use super::PasteboardSnapshot;
        use objc2_app_kit::{NSPasteboard, NSPasteboardItem};
        use objc2_foundation::{NSArray, NSData, NSString};

        /// Monotonic change counter for the general pasteboard. Bumps once
        /// per ownership change (i.e. once per Copy / write).
        pub(in super::super) fn change_count() -> isize {
            let pb = NSPasteboard::generalPasteboard();
            pb.changeCount()
        }

        pub(in super::super) fn snapshot() -> PasteboardSnapshot {
            let pb = NSPasteboard::generalPasteboard();
            let Some(items) = pb.pasteboardItems() else {
                return Vec::new();
            };
            items
                .iter()
                .map(|item| {
                    item.types()
                        .iter()
                        .filter_map(|ty| {
                            let data = item.dataForType(&ty)?;
                            Some((ty.to_string(), data.to_vec()))
                        })
                        .collect()
                })
                .collect()
        }

        pub(in super::super) fn restore(snapshot: &PasteboardSnapshot, transform_pass_id: u64) {
            let pb = NSPasteboard::generalPasteboard();
            pb.clearContents();
            if snapshot.is_empty() {
                return;
            }
            let items: Vec<_> = snapshot
                .iter()
                .map(|entry| {
                    let item = NSPasteboardItem::new();
                    for (ty, data) in entry {
                        let ns_type = NSString::from_str(ty);
                        let ns_data = NSData::with_bytes(data);
                        item.setData_forType(&ns_data, &ns_type);
                    }
                    objc2::runtime::ProtocolObject::from_retained(item)
                })
                .collect();
            let array = NSArray::from_retained_slice(&items);
            if !pb.writeObjects(&array) {
                // Content-free by design: item/type counts only.
                tracing::warn!(
                    target: "transform",
                    transform_pass_id,
                    items = snapshot.len(),
                    "pasteboard restore write was refused"
                );
            }
        }
    }

    /// Test-only shims: the `pasteboard` submodule is private; these expose
    /// snapshot/restore to the unit tests without widening the API.
    #[cfg(test)]
    pub(super) fn pasteboard_snapshot_for_tests() -> PasteboardSnapshot {
        pasteboard::snapshot()
    }

    #[cfg(test)]
    pub(super) fn pasteboard_restore_for_tests(snapshot: &PasteboardSnapshot) {
        pasteboard::restore(snapshot, 0)
    }

    /// Unique-per-attempt sentinel. Never derived from clipboard or selection
    /// content — safe to compare, never logged with content around it.
    fn sentinel() -> String {
        format!(
            "murmur-transform-capture-{}-{}",
            std::process::id(),
            CAPTURE_COUNTER.fetch_add(1, Ordering::Relaxed)
        )
    }

    /// Post a synthetic Cmd+C, mirroring `injector`'s Cmd+V CGEvent shape
    /// (down, 3ms hardware-like gap, up).
    fn post_copy_keystroke() -> Result<(), String> {
        use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation, KeyCode};
        use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

        let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
            .map_err(|_| "could not create CGEvent source".to_string())?;
        let key_down = CGEvent::new_keyboard_event(source.clone(), KeyCode::ANSI_C, true)
            .map_err(|_| "could not create Cmd+C key-down event".to_string())?;
        let key_up = CGEvent::new_keyboard_event(source, KeyCode::ANSI_C, false)
            .map_err(|_| "could not create Cmd+C key-up event".to_string())?;
        key_down.set_flags(CGEventFlags::CGEventFlagCommand);
        key_up.set_flags(CGEventFlags::CGEventFlagCommand);
        key_down.post(CGEventTapLocation::HID);
        std::thread::sleep(Duration::from_millis(3));
        key_up.post(CGEventTapLocation::HID);
        Ok(())
    }

    /// Poll `read` until the pasteboard shows exactly the one ownership change
    /// our synthetic Cmd+C should produce, up to `deadline`. `baseline` is the
    /// `changeCount` observed immediately after the sentinel write; the only
    /// accepted outcome is `baseline + 1` with non-sentinel text, re-checked
    /// after the text read so a foreign write racing the read is caught.
    /// A count that ever moves past `baseline + 1` means a third-party writer
    /// (clipboard manager, Universal Clipboard push) interleaved — that text
    /// is NOT the user's selection and is rejected outright (issue #335
    /// defect B). Injected reader so the loop is unit-testable without a real
    /// clipboard.
    ///
    /// Residual limitation (documented in the feature doc): `changeCount`
    /// counts changes but cannot attribute a writer, so when the synthetic
    /// Cmd+C is swallowed (Chromium reads hardware modifier state and sees
    /// Cmd+Opt+C) and EXACTLY ONE foreign write lands inside the 300ms
    /// window, that single write is indistinguishable from the copy and is
    /// accepted. No NSPasteboard API closes this fully; the guard reduces the
    /// exposure from "any write, any time in the window" to "a single write
    /// that also stops writing before the re-check".
    pub(super) fn poll_for_copy<F>(
        sentinel: &str,
        baseline: isize,
        mut read: F,
        deadline: Duration,
        interval: Duration,
    ) -> Option<String>
    where
        F: FnMut() -> (isize, Result<String, String>),
    {
        let started = Instant::now();
        loop {
            let (count, text) = read();
            if count > baseline + 1 {
                return None;
            }
            if count == baseline + 1 {
                match text {
                    Ok(text) if text != sentinel => {
                        let (count_after, _) = read();
                        if count_after == baseline + 1 {
                            return Some(text);
                        }
                        return None;
                    }
                    // Count moved but the sentinel is still (or again) the
                    // text: a foreign writer re-wrote it. Unusable.
                    Ok(_) => return None,
                    // The one change happened but no text is readable yet
                    // (transient read error, or Copy produced non-text data).
                    // Keep polling until the deadline.
                    Err(_) => {}
                }
            }
            if started.elapsed() >= deadline {
                return None;
            }
            std::thread::sleep(interval);
        }
    }

    /// The error reported when the fallback itself fails: the AX path's
    /// terminal error, preserved verbatim (issue #336). `AxUnavailable` must
    /// stay "couldn't read the selection" — never collapse to the misleading
    /// `NoSelection` ("Select some text first") when text IS selected but the
    /// synthetic Cmd+C was swallowed (Chromium sees Cmd+Opt+C while the
    /// transform key is held). `NoSelection` is reserved for the case where
    /// the AX path itself said the selection was empty.
    pub(super) fn fallback_failure_error(ax_outcome: SelectionError) -> SelectionError {
        ax_outcome
    }

    /// `ax_outcome` is the terminal error of the AX path that triggered this
    /// fallback. When the fallback itself fails, that original error is what
    /// the caller reports (issue #336) — see `fallback_failure_error`.
    pub(super) fn capture_via_clipboard(
        pid: i32,
        bundle_id: Option<String>,
        ax_outcome: SelectionError,
        transform_pass_id: u64,
    ) -> Result<TransformSnapshot, SelectionError> {
        let started = Instant::now();
        let original = pasteboard::snapshot();
        let sentinel = sentinel();
        if crate::injector::write_clipboard_text(&sentinel).is_err() {
            // arboard clears the pasteboard BEFORE writing, so a failed
            // sentinel write can still have destroyed the contents — restore
            // the snapshot (harmless if the pasteboard was never touched).
            pasteboard::restore(&original, transform_pass_id);
            crate::transform_trace::capture_path(
                transform_pass_id,
                "clipboard_fallback",
                "sentinel_write_failed",
                started.elapsed().as_millis() as u64,
                None,
            );
            return Err(fallback_failure_error(ax_outcome));
        }
        let baseline = pasteboard::change_count();

        let copied = match post_copy_keystroke() {
            Ok(()) => poll_for_copy(
                &sentinel,
                baseline,
                || {
                    (
                        pasteboard::change_count(),
                        crate::injector::read_clipboard_text(),
                    )
                },
                POLL_DEADLINE,
                POLL_INTERVAL,
            ),
            Err(_) => None,
        };

        // Restore the user's pasteboard in full fidelity — every item, every
        // type — whether or not the capture succeeded. An empty snapshot
        // restores to an empty (cleared) pasteboard, which also removes our
        // sentinel on the failure path.
        pasteboard::restore(&original, transform_pass_id);

        let text = match copied {
            Some(text) => text,
            None => {
                crate::transform_trace::capture_path(
                    transform_pass_id,
                    "clipboard_fallback",
                    ax_outcome.as_str(),
                    started.elapsed().as_millis() as u64,
                    None,
                );
                return Err(fallback_failure_error(ax_outcome));
            }
        };
        if let Err(error) = super::classify_selection(&text) {
            crate::transform_trace::capture_path(
                transform_pass_id,
                "clipboard_fallback",
                error.as_str(),
                started.elapsed().as_millis() as u64,
                None,
            );
            return Err(error);
        }
        crate::transform_trace::capture_path(
            transform_pass_id,
            "clipboard_fallback",
            "ok",
            started.elapsed().as_millis() as u64,
            Some(super::length_bucket(text.len())),
        );
        Ok(TransformSnapshot {
            bundle_id,
            pid,
            text,
            // No AX range/bounds from a clipboard capture: the popover
            // centers (anchor None) and apply uses the paste fallback.
            range: None,
            bounds: None,
            captured_at: Instant::now(),
        })
    }
}

#[cfg(target_os = "macos")]
mod native {
    use super::{is_secure_role, is_secure_subrole, Rect, SelectionError, TransformSnapshot};
    use objc2_app_kit::NSWorkspace;
    use std::ffi::{c_char, c_void, CStr, CString};
    use std::time::Instant;

    type AXUIElementRef = *const c_void;
    type CFTypeRef = *const c_void;
    type CFIndex = isize;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
        fn AXUIElementCopyAttributeValue(
            element: AXUIElementRef,
            attribute: CFTypeRef,
            value: *mut CFTypeRef,
        ) -> i32;
        fn AXUIElementCopyParameterizedAttributeValue(
            element: AXUIElementRef,
            attribute: CFTypeRef,
            parameter: CFTypeRef,
            value: *mut CFTypeRef,
        ) -> i32;
        fn AXUIElementSetMessagingTimeout(element: AXUIElementRef, timeout: f32) -> i32;
        fn AXValueGetType(value: CFTypeRef) -> u32;
        fn AXValueGetValue(value: CFTypeRef, value_type: u32, value_ptr: *mut c_void) -> bool;
        fn CFStringCreateWithCString(
            allocator: CFTypeRef,
            string: *const c_char,
            encoding: u32,
        ) -> CFTypeRef;
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

    const AX_SUCCESS: i32 = 0;
    const AX_QUERY_TIMEOUT_SECONDS: f32 = 0.025;
    const UTF8_ENCODING: u32 = 0x0800_0100;
    // kAXValueCFRangeType / kAXValueCGRectType from the AXValueType enum
    // (ApplicationServices / HIServices AXValue.h).
    const AX_VALUE_CGRECT_TYPE: u32 = 3;
    const AX_VALUE_CFRANGE_TYPE: u32 = 4;

    #[repr(C)]
    #[derive(Debug, Clone, Copy)]
    struct CFRange {
        location: CFIndex,
        length: CFIndex,
    }

    #[repr(C)]
    #[derive(Debug, Clone, Copy)]
    struct CGPoint {
        x: f64,
        y: f64,
    }

    #[repr(C)]
    #[derive(Debug, Clone, Copy)]
    struct CGSize {
        width: f64,
        height: f64,
    }

    #[repr(C)]
    #[derive(Debug, Clone, Copy)]
    struct CGRect {
        origin: CGPoint,
        size: CGSize,
    }

    /// RAII guard releasing any CFTypeRef (including AXUIElementRef, which
    /// follows normal CF retain/release semantics) obtained via a Copy/Create
    /// rule API. Avoids the manual "release on every early-return path"
    /// bookkeeping the equivalent injector.rs code has to do by hand.
    struct CFGuard(CFTypeRef);
    impl Drop for CFGuard {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CFRelease(self.0) };
            }
        }
    }

    fn cfstring(s: &str) -> Result<CFGuard, SelectionError> {
        let c = CString::new(s).map_err(|_| SelectionError::AxUnavailable)?;
        let raw = unsafe { CFStringCreateWithCString(std::ptr::null(), c.as_ptr(), UTF8_ENCODING) };
        if raw.is_null() {
            return Err(SelectionError::AxUnavailable);
        }
        Ok(CFGuard(raw))
    }

    fn cfstring_to_string(value: CFTypeRef) -> Result<String, SelectionError> {
        let length = unsafe { CFStringGetLength(value) };
        let max_size = unsafe { CFStringGetMaximumSizeForEncoding(length, UTF8_ENCODING) };
        if max_size <= 0 {
            return Ok(String::new());
        }
        let mut buffer = vec![0 as c_char; (max_size + 1) as usize];
        let converted = unsafe {
            CFStringGetCString(
                value,
                buffer.as_mut_ptr(),
                buffer.len() as CFIndex,
                UTF8_ENCODING,
            )
        };
        if !converted {
            return Err(SelectionError::AxUnavailable);
        }
        Ok(unsafe { CStr::from_ptr(buffer.as_ptr()) }
            .to_string_lossy()
            .into_owned())
    }

    fn set_timeout(element: AXUIElementRef) -> Result<(), SelectionError> {
        let status = unsafe { AXUIElementSetMessagingTimeout(element, AX_QUERY_TIMEOUT_SECONDS) };
        if status != AX_SUCCESS {
            return Err(SelectionError::AxUnavailable);
        }
        Ok(())
    }

    /// Copy an attribute value (Copy Rule — caller owns and must release it),
    /// returning the raw `AXError` status code on failure so callers that
    /// need to distinguish "benign" (no value / attribute unsupported) from
    /// "fatal" (e.g. the messaging timeout) errors can do so — see
    /// `super::is_benign_role_query_error`.
    fn copy_attribute_raw(element: AXUIElementRef, name: &str) -> Result<CFGuard, i32> {
        let attr = cfstring(name).map_err(|_| super::AX_ERROR_CANNOT_COMPLETE)?;
        let mut value: CFTypeRef = std::ptr::null();
        let status = unsafe { AXUIElementCopyAttributeValue(element, attr.0, &mut value) };
        if status != AX_SUCCESS || value.is_null() {
            if !value.is_null() {
                unsafe { CFRelease(value) };
            }
            return Err(status);
        }
        Ok(CFGuard(value))
    }

    /// Copy an attribute value (Copy Rule — caller owns and must release it).
    fn copy_attribute(element: AXUIElementRef, name: &str) -> Result<CFGuard, SelectionError> {
        copy_attribute_raw(element, name).map_err(|_| SelectionError::AxUnavailable)
    }

    fn copy_attribute_string(
        element: AXUIElementRef,
        name: &str,
    ) -> Result<String, SelectionError> {
        let value = copy_attribute(element, name)?;
        cfstring_to_string(value.0)
    }

    /// Like `copy_attribute_string`, but preserves the raw `AXError` status
    /// code on failure instead of collapsing it to `SelectionError`. Used only
    /// by the secure-field checks in `capture_selection_native`, which must
    /// fail closed on anything other than a benign "no value"/"unsupported"
    /// status (see `super::is_benign_role_query_error`).
    fn copy_attribute_string_status(element: AXUIElementRef, name: &str) -> Result<String, i32> {
        let value = copy_attribute_raw(element, name)?;
        cfstring_to_string(value.0).map_err(|_| super::AX_ERROR_CANNOT_COMPLETE)
    }

    fn decode_range(value: CFTypeRef) -> Option<(usize, usize)> {
        if unsafe { AXValueGetType(value) } != AX_VALUE_CFRANGE_TYPE {
            return None;
        }
        let mut range = CFRange {
            location: 0,
            length: 0,
        };
        let ok = unsafe {
            AXValueGetValue(
                value,
                AX_VALUE_CFRANGE_TYPE,
                &mut range as *mut CFRange as *mut c_void,
            )
        };
        if !ok || range.location < 0 || range.length < 0 {
            return None;
        }
        Some((
            range.location as usize,
            (range.location + range.length) as usize,
        ))
    }

    fn decode_rect(value: CFTypeRef) -> Option<Rect> {
        if unsafe { AXValueGetType(value) } != AX_VALUE_CGRECT_TYPE {
            return None;
        }
        let mut rect = CGRect {
            origin: CGPoint { x: 0.0, y: 0.0 },
            size: CGSize {
                width: 0.0,
                height: 0.0,
            },
        };
        let ok = unsafe {
            AXValueGetValue(
                value,
                AX_VALUE_CGRECT_TYPE,
                &mut rect as *mut CGRect as *mut c_void,
            )
        };
        if !ok {
            return None;
        }
        Some(Rect {
            x: rect.origin.x,
            y: rect.origin.y,
            width: rect.size.width,
            height: rect.size.height,
        })
    }

    /// Query the parameterized `AXBoundsForRange` attribute using an already
    /// AX-native range value (i.e. the exact `AXValueRef` read back from
    /// `AXSelectedTextRange`) as the parameter. Bounds are optional per spec —
    /// any failure here yields `None`, never an error for the whole capture.
    fn query_bounds_for_range(element: AXUIElementRef, range_value: CFTypeRef) -> Option<Rect> {
        let attr = cfstring("AXBoundsForRange").ok()?;
        let mut value: CFTypeRef = std::ptr::null();
        let status = unsafe {
            AXUIElementCopyParameterizedAttributeValue(element, attr.0, range_value, &mut value)
        };
        if status != AX_SUCCESS || value.is_null() {
            if !value.is_null() {
                unsafe { CFRelease(value) };
            }
            return None;
        }
        decode_rect(CFGuard(value).0)
    }

    /// Frontmost app's (pid, bundle id), read on the main thread alongside the
    /// AX capture so the clipboard fallback (issue #329) can attribute its
    /// snapshot without touching NSWorkspace off the main thread.
    pub(super) fn frontmost_pid_bundle() -> Option<(i32, Option<String>)> {
        let frontmost = NSWorkspace::sharedWorkspace().frontmostApplication()?;
        Some((
            frontmost.processIdentifier(),
            frontmost.bundleIdentifier().map(|value| value.to_string()),
        ))
    }

    pub(super) fn capture_selection_native() -> Result<TransformSnapshot, SelectionError> {
        let frontmost = NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .ok_or(SelectionError::AxUnavailable)?;
        let pid = frontmost.processIdentifier();
        let bundle_id = frontmost.bundleIdentifier().map(|value| value.to_string());

        let app = unsafe { AXUIElementCreateApplication(pid) };
        if app.is_null() {
            return Err(SelectionError::AxUnavailable);
        }
        let _app_guard = CFGuard(app);
        set_timeout(app)?;

        let focused = copy_attribute(app, "AXFocusedUIElement")?;
        // From here on we hold a focused element whose secure-ness is not yet
        // established. Any failure to complete the secure-field check —
        // including failing to arm the messaging timeout it depends on — is
        // `SecureCheckFailed`, which the caller treats as terminal for the
        // clipboard fallback (issue #334): retry-eligible, never fall-back.
        if set_timeout(focused.0).is_err() {
            return Err(SelectionError::SecureCheckFailed);
        }

        // FIRST: secure-field check. A positive match reads NOTHING else. Fails
        // closed: an AX error here (including the 25ms messaging timeout) is
        // NOT treated as "no subrole/role" — only a benign "no value"/
        // "attribute unsupported" status is safe to continue past. Any other
        // error aborts the capture entirely (`SecureCheckFailed`) rather than
        // silently falling through to read the selection or falling over to
        // the clipboard fallback.
        match copy_attribute_string_status(focused.0, "AXSubrole") {
            Ok(subrole) => {
                if is_secure_subrole(&subrole) {
                    return Err(SelectionError::SecureField);
                }
            }
            Err(status) if super::is_benign_role_query_error(status) => {}
            Err(_) => return Err(SelectionError::SecureCheckFailed),
        }
        match copy_attribute_string_status(focused.0, "AXRole") {
            Ok(role) => {
                if is_secure_role(&role) {
                    return Err(SelectionError::SecureField);
                }
            }
            Err(status) if super::is_benign_role_query_error(status) => {}
            Err(_) => return Err(SelectionError::SecureCheckFailed),
        }

        let text = copy_attribute_string(focused.0, "AXSelectedText").unwrap_or_default();
        super::classify_selection(&text)?;

        let range_value = copy_attribute(focused.0, "AXSelectedTextRange").ok();
        let range = range_value.as_ref().and_then(|guard| decode_range(guard.0));
        let bounds = range_value
            .as_ref()
            .and_then(|guard| query_bounds_for_range(focused.0, guard.0));

        Ok(TransformSnapshot {
            bundle_id,
            pid,
            text,
            range,
            bounds,
            captured_at: Instant::now(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_bucket_covers_boundaries() {
        assert_eq!(length_bucket(0), "0");
        assert_eq!(length_bucket(1), "1-16");
        assert_eq!(length_bucket(16), "1-16");
        assert_eq!(length_bucket(17), "17-64");
        assert_eq!(length_bucket(64), "17-64");
        assert_eq!(length_bucket(65), "65-256");
        assert_eq!(length_bucket(256), "65-256");
        assert_eq!(length_bucket(257), "257-1024");
        assert_eq!(length_bucket(1024), "257-1024");
        assert_eq!(length_bucket(1025), "1025-4096");
        assert_eq!(length_bucket(4096), "1025-4096");
        assert_eq!(length_bucket(4097), "4097-16384");
        assert_eq!(length_bucket(16384), "4097-16384");
        assert_eq!(length_bucket(16385), ">16384");
        assert_eq!(length_bucket(usize::MAX), ">16384");
    }

    #[test]
    fn classify_empty_selection_is_no_selection() {
        assert_eq!(classify_selection(""), Err(SelectionError::NoSelection));
    }

    #[test]
    fn classify_within_cap_is_ok() {
        assert_eq!(classify_selection("hello"), Ok(()));
        let exactly_at_cap = "a".repeat(MAX_SELECTION_BYTES);
        assert_eq!(classify_selection(&exactly_at_cap), Ok(()));
    }

    #[test]
    fn classify_over_cap_is_too_large_and_never_truncates() {
        let over_cap = "a".repeat(MAX_SELECTION_BYTES + 1);
        assert_eq!(classify_selection(&over_cap), Err(SelectionError::TooLarge));
    }

    #[test]
    fn classify_multibyte_selection_uses_utf8_byte_length_not_char_count() {
        // Each 'é' is 2 UTF-8 bytes; MAX_SELECTION_BYTES/2 + 1 chars exceeds the
        // byte cap even though the char count alone would not.
        let over_cap_multibyte = "é".repeat(MAX_SELECTION_BYTES / 2 + 1);
        assert!(over_cap_multibyte.chars().count() <= MAX_SELECTION_BYTES);
        assert_eq!(
            classify_selection(&over_cap_multibyte),
            Err(SelectionError::TooLarge)
        );
    }

    #[test]
    fn secure_subrole_is_detected() {
        assert!(is_secure_subrole("AXSecureTextField"));
        assert!(is_secure_subrole("  AXSecureTextField  "));
        assert!(!is_secure_subrole("AXTextField"));
        assert!(!is_secure_subrole(""));
    }

    #[test]
    fn secure_role_is_detected() {
        assert!(is_secure_role("AXSecureTextField"));
        assert!(!is_secure_role("AXTextField"));
        assert!(!is_secure_role("AXGroup"));
    }

    #[test]
    fn selection_error_outcome_strings_are_stable_and_content_free() {
        let cases = [
            (SelectionError::AccessibilityDenied, "accessibility_denied"),
            (SelectionError::SecureField, "secure_field"),
            (SelectionError::NoSelection, "no_selection"),
            (SelectionError::TooLarge, "too_large"),
            (SelectionError::AxUnavailable, "ax_unavailable"),
            (SelectionError::SecureCheckFailed, "secure_check_failed"),
        ];
        for (error, expected) in cases {
            assert_eq!(error.as_str(), expected);
        }
    }

    #[test]
    fn benign_role_query_errors_allow_continuing() {
        assert!(is_benign_role_query_error(AX_ERROR_NO_VALUE));
        assert!(is_benign_role_query_error(AX_ERROR_ATTRIBUTE_UNSUPPORTED));
    }

    #[test]
    fn other_role_query_errors_fail_closed() {
        // The messaging timeout must NOT be treated as benign — this is the
        // exact fail-open bug this classification exists to prevent.
        assert!(!is_benign_role_query_error(AX_ERROR_CANNOT_COMPLETE));
        assert!(!is_benign_role_query_error(-1));
        assert!(!is_benign_role_query_error(i32::MIN));
        assert!(!is_benign_role_query_error(i32::MAX));
    }

    #[test]
    fn transform_snapshot_debug_redacts_text_but_keeps_length_bucket() {
        let snapshot = TransformSnapshot {
            bundle_id: Some("com.example.app".to_string()),
            pid: 123,
            text: "super secret selection text".to_string(),
            range: Some((0, 5)),
            bounds: None,
            captured_at: Instant::now(),
        };
        let debug = format!("{:?}", snapshot);
        assert!(!debug.contains("super secret selection text"));
        assert!(!debug.contains("secret"));
        assert!(debug.contains("17-64"));
        assert!(debug.contains("com.example.app"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn clipboard_poll_returns_text_from_the_single_copy_change() {
        use std::time::Duration;
        // Sequence: Cmd+C hasn't landed yet (count still at baseline), then
        // the copy lands (baseline+1) with real text, then the post-read
        // re-check still sees baseline+1 → accept.
        let reads = std::cell::RefCell::new(vec![
            (10, Ok("SENTINEL".to_string())),
            (10, Ok("SENTINEL".to_string())),
            (11, Ok("copied selection".to_string())),
            (11, Ok("copied selection".to_string())),
        ]);
        let result = super::clipboard_fallback::poll_for_copy(
            "SENTINEL",
            10,
            || reads.borrow_mut().remove(0),
            Duration::from_millis(500),
            Duration::from_millis(1),
        );
        assert_eq!(result, Some("copied selection".to_string()));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn clipboard_poll_times_out_when_nothing_is_copied() {
        use std::time::Duration;
        // Nothing selected: Cmd+C is a no-op, the change count never moves,
        // and the poll must time out rather than return the sentinel as text.
        let result = super::clipboard_fallback::poll_for_copy(
            "SENTINEL",
            10,
            || (10, Ok("SENTINEL".to_string())),
            Duration::from_millis(20),
            Duration::from_millis(1),
        );
        assert_eq!(result, None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn clipboard_poll_survives_transient_read_errors() {
        use std::time::Duration;
        let reads = std::cell::RefCell::new(vec![
            (11, Err("busy".to_string())),
            (11, Ok("copied".to_string())),
            (11, Ok("copied".to_string())),
        ]);
        let result = super::clipboard_fallback::poll_for_copy(
            "SENTINEL",
            10,
            || reads.borrow_mut().remove(0),
            Duration::from_millis(500),
            Duration::from_millis(1),
        );
        assert_eq!(result, Some("copied".to_string()));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn clipboard_poll_rejects_foreign_writer_that_bumps_past_our_copy() {
        use std::time::Duration;
        // Issue #335 defect B: a clipboard manager / Universal Clipboard push
        // interleaving with our Cmd+C moves the change count more than one
        // step past the sentinel write. That text is NOT the user's selection
        // — it must be rejected, not ingested.
        let result = super::clipboard_fallback::poll_for_copy(
            "SENTINEL",
            10,
            || (12, Ok("INJECTED-1234".to_string())),
            Duration::from_millis(500),
            Duration::from_millis(1),
        );
        assert_eq!(result, None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn clipboard_poll_rejects_foreign_write_racing_the_text_read() {
        use std::time::Duration;
        // The copy lands (baseline+1) but a foreign writer bumps the count
        // between our text read and the re-check — the text we read can no
        // longer be trusted to be the copy's.
        let reads = std::cell::RefCell::new(vec![
            (11, Ok("could be anyone's".to_string())),
            (12, Ok("INJECTED".to_string())),
        ]);
        let result = super::clipboard_fallback::poll_for_copy(
            "SENTINEL",
            10,
            || reads.borrow_mut().remove(0),
            Duration::from_millis(500),
            Duration::from_millis(1),
        );
        assert_eq!(result, None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn clipboard_poll_rejects_sentinel_rewritten_by_foreign_writer() {
        use std::time::Duration;
        // Count moved one step but the text is still the sentinel: someone
        // re-wrote our sentinel. Unusable — never returned as a selection.
        let result = super::clipboard_fallback::poll_for_copy(
            "SENTINEL",
            10,
            || (11, Ok("SENTINEL".to_string())),
            Duration::from_millis(20),
            Duration::from_millis(1),
        );
        assert_eq!(result, None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn pasteboard_snapshot_restores_non_text_content_in_full() {
        // Issue #335 defect A: a non-text clipboard (image/file) must survive
        // the fallback's sentinel dance. Round-trip real NSPasteboard items
        // carrying a custom binary type alongside text, through a destructive
        // overwrite, and verify both types come back byte-identical. The
        // user's real pasteboard is snapshotted first and restored at the end.
        let user_pasteboard = super::clipboard_fallback::pasteboard_snapshot_for_tests();

        let fixture: super::clipboard_fallback::PasteboardSnapshot = vec![vec![
            (
                "public.utf8-plain-text".to_string(),
                b"fixture text".to_vec(),
            ),
            (
                "com.murmur.test.binary".to_string(),
                vec![0u8, 159, 146, 150, 255],
            ),
        ]];
        super::clipboard_fallback::pasteboard_restore_for_tests(&fixture);

        // Destroy it the way the fallback does: a plain text write.
        crate::injector::write_clipboard_text("sentinel-destroys-clipboard").unwrap();

        // Restore the fixture snapshot and read back both types.
        super::clipboard_fallback::pasteboard_restore_for_tests(&fixture);
        let round_tripped = super::clipboard_fallback::pasteboard_snapshot_for_tests();

        // Put the user's pasteboard back before asserting.
        super::clipboard_fallback::pasteboard_restore_for_tests(&user_pasteboard);

        assert_eq!(round_tripped.len(), 1);
        let types: Vec<&str> = round_tripped[0].iter().map(|(ty, _)| ty.as_str()).collect();
        assert!(types.contains(&"public.utf8-plain-text"));
        assert!(types.contains(&"com.murmur.test.binary"));
        for (ty, data) in &fixture[0] {
            let restored = round_tripped[0]
                .iter()
                .find(|(t, _)| t == ty)
                .map(|(_, d)| d);
            assert_eq!(restored, Some(data), "type {ty} did not round-trip");
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn fallback_failure_preserves_the_ax_paths_terminal_error() {
        // Issue #336: when the AX path ended in AxUnavailable and the
        // clipboard fallback then failed too, the user must see "couldn't
        // read the selection" — not "Select some text first" over a visible
        // selection. NoSelection survives only when AX itself reported empty.
        assert_eq!(
            super::clipboard_fallback::fallback_failure_error(SelectionError::AxUnavailable),
            SelectionError::AxUnavailable
        );
        assert_eq!(
            super::clipboard_fallback::fallback_failure_error(SelectionError::NoSelection),
            SelectionError::NoSelection
        );
    }

    #[test]
    fn secure_check_failure_is_never_fallback_eligible() {
        // Issue #334: an errored secure-field check must fail closed — no
        // synthetic Cmd+C at an element we could not prove is not a password
        // field. Only the benign "AX couldn't produce a selection" outcomes
        // may fall back.
        assert!(!fallback_eligible(SelectionError::SecureCheckFailed));
        assert!(!fallback_eligible(SelectionError::SecureField));
        assert!(!fallback_eligible(SelectionError::AccessibilityDenied));
        assert!(!fallback_eligible(SelectionError::TooLarge));
        assert!(fallback_eligible(SelectionError::NoSelection));
        assert!(fallback_eligible(SelectionError::AxUnavailable));
    }

    #[test]
    fn secure_check_error_bars_fallback_for_the_whole_capture() {
        // Issue #334 (capture granularity): an errored secure check on ANY
        // retry attempt is sticky — a later attempt failing shallower
        // (AxUnavailable from a focused-element query timeout) must not
        // launder the capture back into fallback eligibility.
        assert!(!fallback_allowed(SelectionError::AxUnavailable, true));
        assert!(!fallback_allowed(SelectionError::NoSelection, true));
        assert!(!fallback_allowed(SelectionError::SecureCheckFailed, false));
        assert!(fallback_allowed(SelectionError::AxUnavailable, false));
        assert!(fallback_allowed(SelectionError::NoSelection, false));
    }

    #[test]
    fn secure_check_failure_is_retry_eligible_but_terminal_errors_are_not() {
        // Retrying re-runs the full secure check, so it stays safe; the
        // fail-closed terminal errors must not be retried.
        assert!(retry_eligible(SelectionError::SecureCheckFailed));
        assert!(retry_eligible(SelectionError::NoSelection));
        assert!(retry_eligible(SelectionError::AxUnavailable));
        assert!(!retry_eligible(SelectionError::SecureField));
        assert!(!retry_eligible(SelectionError::AccessibilityDenied));
        assert!(!retry_eligible(SelectionError::TooLarge));
    }

    #[test]
    fn rect_is_a_plain_copyable_value() {
        let rect = Rect {
            x: 1.0,
            y: 2.0,
            width: 3.0,
            height: 4.0,
        };
        let copy = rect;
        assert_eq!(rect, copy);
    }
}
