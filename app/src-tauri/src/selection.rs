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
}

impl SelectionError {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AccessibilityDenied => "accessibility_denied",
            Self::SecureField => "secure_field",
            Self::NoSelection => "no_selection",
            Self::TooLarge => "too_large",
            Self::AxUnavailable => "ax_unavailable",
        }
    }
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
    match result {
        Ok(snapshot) => {
            tracing::info!(
                target: "transform",
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
) -> Result<TransformSnapshot, SelectionError> {
    if !crate::injector::is_accessibility_enabled() {
        return Err(SelectionError::AccessibilityDenied);
    }

    #[cfg(target_os = "macos")]
    {
        type AxReply = (
            Result<TransformSnapshot, SelectionError>,
            Option<(i32, Option<String>)>,
        );
        let (tx, rx) = tokio::sync::oneshot::channel::<AxReply>();
        app_handle
            .run_on_main_thread(move || {
                let _ = tx.send((
                    native::capture_selection_native(),
                    native::frontmost_pid_bundle(),
                ));
            })
            .map_err(|_| SelectionError::AxUnavailable)?;
        let (ax_result, frontmost) = rx
            .await
            .unwrap_or((Err(SelectionError::AxUnavailable), None));

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
            // never leak. `AccessibilityDenied` and a positively detected
            // `SecureField` stay hard-blocked with no fallback.
            (
                Err(err @ (SelectionError::NoSelection | SelectionError::AxUnavailable)),
                Some((pid, bundle_id)),
            ) => {
                tracing::info!(
                    target: "transform",
                    ax_outcome = err.as_str(),
                    "AX capture incomplete — attempting clipboard fallback"
                );
                tokio::task::spawn_blocking(move || {
                    clipboard_fallback::capture_via_clipboard(pid, bundle_id)
                })
                .await
                .unwrap_or(Err(err))
            }
            (result, _) => result,
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
/// Strategy: snapshot the current clipboard text, overwrite it with a unique
/// sentinel, post a synthetic Cmd+C, and poll until the clipboard no longer
/// holds the sentinel (or a 300ms deadline passes — with nothing selected,
/// Cmd+C is a no-op and the sentinel never changes). The user's original
/// clipboard text is restored afterwards. Limitations, documented in the
/// feature doc: a non-text clipboard (image/file) cannot be snapshotted by
/// `arboard` and therefore cannot be restored, and the captured snapshot has
/// no AX range/bounds (the popover centers; apply uses the paste fallback).
#[cfg(target_os = "macos")]
mod clipboard_fallback {
    use super::{SelectionError, TransformSnapshot};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    const POLL_INTERVAL: Duration = Duration::from_millis(20);
    const POLL_DEADLINE: Duration = Duration::from_millis(300);

    static CAPTURE_COUNTER: AtomicU64 = AtomicU64::new(0);

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

    /// Poll `read` until it returns text that is not `sentinel`, up to
    /// `deadline`. Injected reader so the loop is unit-testable without a real
    /// clipboard.
    pub(super) fn poll_for_selection<F>(
        sentinel: &str,
        mut read: F,
        deadline: Duration,
        interval: Duration,
    ) -> Option<String>
    where
        F: FnMut() -> Result<String, String>,
    {
        let started = Instant::now();
        loop {
            if let Ok(text) = read() {
                if text != sentinel {
                    return Some(text);
                }
            }
            if started.elapsed() >= deadline {
                return None;
            }
            std::thread::sleep(interval);
        }
    }

    pub(super) fn capture_via_clipboard(
        pid: i32,
        bundle_id: Option<String>,
    ) -> Result<TransformSnapshot, SelectionError> {
        let original = crate::injector::read_clipboard_text().ok();
        let sentinel = sentinel();
        if crate::injector::write_clipboard_text(&sentinel).is_err() {
            return Err(SelectionError::NoSelection);
        }

        let copied = match post_copy_keystroke() {
            Ok(()) => poll_for_selection(
                &sentinel,
                || crate::injector::read_clipboard_text(),
                POLL_DEADLINE,
                POLL_INTERVAL,
            ),
            Err(_) => None,
        };

        // Restore the user's clipboard. If the original was non-text (image/
        // file) there is nothing to restore; on a failed capture at least
        // clear our sentinel rather than leaving it behind.
        match &original {
            Some(text) => {
                let _ = crate::injector::write_clipboard_text(text);
            }
            None => {
                if copied.is_none() {
                    let _ = crate::injector::write_clipboard_text("");
                }
            }
        }

        let text = copied.ok_or(SelectionError::NoSelection)?;
        super::classify_selection(&text)?;
        tracing::info!(
            target: "transform",
            outcome = "ok",
            via = "clipboard_fallback",
            length_bucket = super::length_bucket(text.len()),
            "selection captured via clipboard fallback"
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

    fn copy_attribute_string(element: AXUIElementRef, name: &str) -> Result<String, SelectionError> {
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
        set_timeout(focused.0)?;

        // FIRST: secure-field check. A positive match reads NOTHING else. Fails
        // closed: an AX error here (including the 25ms messaging timeout) is
        // NOT treated as "no subrole/role" — only a benign "no value"/
        // "attribute unsupported" status is safe to continue past. Any other
        // error aborts the capture entirely (`AxUnavailable`) rather than
        // silently falling through to read the selection.
        match copy_attribute_string_status(focused.0, "AXSubrole") {
            Ok(subrole) => {
                if is_secure_subrole(&subrole) {
                    return Err(SelectionError::SecureField);
                }
            }
            Err(status) if super::is_benign_role_query_error(status) => {}
            Err(_) => return Err(SelectionError::AxUnavailable),
        }
        match copy_attribute_string_status(focused.0, "AXRole") {
            Ok(role) => {
                if is_secure_role(&role) {
                    return Err(SelectionError::SecureField);
                }
            }
            Err(status) if super::is_benign_role_query_error(status) => {}
            Err(_) => return Err(SelectionError::AxUnavailable),
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
    fn clipboard_poll_returns_first_non_sentinel_text() {
        use std::time::Duration;
        let reads = std::cell::RefCell::new(vec![
            Ok("SENTINEL".to_string()),
            Ok("SENTINEL".to_string()),
            Ok("copied selection".to_string()),
        ]);
        let result = super::clipboard_fallback::poll_for_selection(
            "SENTINEL",
            || reads.borrow_mut().remove(0),
            Duration::from_millis(500),
            Duration::from_millis(1),
        );
        assert_eq!(result, Some("copied selection".to_string()));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn clipboard_poll_times_out_when_sentinel_never_changes() {
        use std::time::Duration;
        // Nothing selected: Cmd+C is a no-op, the sentinel never changes, and
        // the poll must time out rather than return the sentinel as "text".
        let result = super::clipboard_fallback::poll_for_selection(
            "SENTINEL",
            || Ok("SENTINEL".to_string()),
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
            Err("busy".to_string()),
            Ok("copied".to_string()),
        ]);
        let result = super::clipboard_fallback::poll_for_selection(
            "SENTINEL",
            || reads.borrow_mut().remove(0),
            Duration::from_millis(500),
            Duration::from_millis(1),
        );
        assert_eq!(result, Some("copied".to_string()));
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
