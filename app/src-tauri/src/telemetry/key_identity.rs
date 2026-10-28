//! Keyboard key-identity privacy boundary.
//!
//! A key pressed while a hotkey is held is usually the next character the user
//! is typing, so its identity is content. Telemetry may name only the
//! configured hotkey; every other key is a bounded class. This module enforces
//! that in every sink, independently of call sites: structured events, the
//! pretty `app.log`, and JSONL lines persisted by an older build.

use std::borrow::Cow;
use std::ops::Range;

use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::registry::LookupSpan;

/// rdev `Key` names (Debug form) that are unambiguous in any text. Modifiers
/// and F1–F20 are absent: they are the configurable hotkeys and reveal
/// nothing the user typed.
const DISTINCT_KEY_NAMES: &[&str] = &[
    "KeyA",
    "KeyB",
    "KeyC",
    "KeyD",
    "KeyE",
    "KeyF",
    "KeyG",
    "KeyH",
    "KeyI",
    "KeyJ",
    "KeyK",
    "KeyL",
    "KeyM",
    "KeyN",
    "KeyO",
    "KeyP",
    "KeyQ",
    "KeyR",
    "KeyS",
    "KeyT",
    "KeyU",
    "KeyV",
    "KeyW",
    "KeyX",
    "KeyY",
    "KeyZ",
    "Num0",
    "Num1",
    "Num2",
    "Num3",
    "Num4",
    "Num5",
    "Num6",
    "Num7",
    "Num8",
    "Num9",
    "Kp0",
    "Kp1",
    "Kp2",
    "Kp3",
    "Kp4",
    "Kp5",
    "Kp6",
    "Kp7",
    "Kp8",
    "Kp9",
    "KpReturn",
    "KpMinus",
    "KpPlus",
    "KpMultiply",
    "KpDivide",
    "KpDelete",
    "SemiColon",
    "BackQuote",
    "BackSlash",
    "IntlBackslash",
    "LeftBracket",
    "RightBracket",
    "CapsLock",
    "PageUp",
    "PageDown",
    "UpArrow",
    "DownArrow",
    "LeftArrow",
    "RightArrow",
    "PrintScreen",
    "ScrollLock",
    "NumLock",
    "F21",
    "F22",
    "F23",
    "F24",
];

/// rdev `Key` names that are also ordinary words. They count as key identity
/// in the keyboard stream, or anywhere in parentheses such as `Some(Name)`.
const WORD_KEY_NAMES: &[&str] = &[
    "Backspace",
    "Delete",
    "End",
    "Escape",
    "Home",
    "Return",
    "Space",
    "Tab",
    "Pause",
    "Minus",
    "Equal",
    "Quote",
    "Comma",
    "Dot",
    "Slash",
    "Insert",
    "Function",
];

/// Replaces a key identity inside otherwise-retained text.
const REDACTED_KEY: &str = "[key]";

/// Characters that delimit a key-name token. Paths and model names such as
/// `/models/KeyA/model.bin` or `vendor/KeyA-7B` are one token, not a key.
fn is_token_delimiter(c: char) -> bool {
    c.is_whitespace() || matches!(c, '(' | ')' | ',' | '=' | ':' | '"' | '\'')
}

/// Byte ranges in `text` that name a non-hotkey key: a distinct rdev key name,
/// a raw `Unknown(<code>)`, or a word-like key name in the keyboard stream or
/// in parentheses such as `Some(End)` or `KeyPress(Return)`.
fn key_identity_spans(text: &str, keyboard_stream: bool) -> Vec<Range<usize>> {
    let mut spans = Vec::new();
    let mut start = 0;
    for (end, c) in text
        .char_indices()
        .chain(std::iter::once((text.len(), ' ')))
    {
        if !is_token_delimiter(c) {
            continue;
        }
        let token = &text[start..end];
        let rest = &text[end..];
        if token == "Unknown" {
            let code = rest
                .strip_prefix('(')
                .and_then(|rest| rest.split_once(')'))
                .map(|(code, _)| code);
            if code.is_some_and(|code| !code.is_empty() && code.bytes().all(|b| b.is_ascii_digit()))
            {
                spans.push(start..end + code.map_or(0, str::len) + 2);
            }
        } else if DISTINCT_KEY_NAMES.contains(&token)
            || (WORD_KEY_NAMES.contains(&token)
                && (keyboard_stream || (text[..start].ends_with('(') && rest.starts_with(')'))))
        {
            spans.push(start..end);
        }
        start = end + c.len_utf8();
    }
    spans
}

/// `text` with every non-hotkey key identity replaced by `[key]`.
fn redact_key_identity(text: &str, keyboard_stream: bool) -> Cow<'_, str> {
    let spans = key_identity_spans(text, keyboard_stream);
    if spans.is_empty() {
        return Cow::Borrowed(text);
    }
    let mut redacted = String::with_capacity(text.len());
    let mut last = 0;
    for span in spans {
        redacted.push_str(&text[last..span.start]);
        redacted.push_str(REDACTED_KEY);
        last = span.end;
    }
    redacted.push_str(&text[last..]);
    Cow::Owned(redacted)
}

/// Exact keyboard-stream string schema. Key identity is limited to the
/// configured hotkey's settings id; every other key is a bounded class.
fn is_safe_keyboard_string(key: &str, value: &str) -> bool {
    let inner = value
        .strip_prefix("Some(")
        .and_then(|rest| rest.strip_suffix(')'))
        .unwrap_or(value);
    match key {
        "event_code" => super::canonical_event_code(value).is_some(),
        "reason" => matches!(
            value,
            "held_too_long"
                | "second_tap_expired"
                | "combo_cancelled"
                | "single_short_tap_noop"
                | "processing_skipped"
                | "other_key"
                | "released"
        ),
        "detector" => matches!(value, "double_tap" | "hold_down"),
        "mode" => matches!(
            value,
            "DoubleTap" | "HoldDown" | "Both" | "double_tap" | "hold_down" | "both"
        ),
        "state" | "double_tap_state" | "hold_state" => matches!(
            inner,
            "None" | "Idle" | "WaitingFirstUp" | "WaitingSecondDown" | "WaitingSecondUp" | "Held"
        ),
        "event_kind" => matches!(
            value,
            "key_press"
                | "key_release"
                | "button_press"
                | "button_release"
                | "mouse_move"
                | "wheel"
        ),
        "key_class" => matches!(
            value,
            "none" | "modifier" | "function" | "navigation" | "editing" | "character" | "other"
        ),
        "target_key" => {
            matches!(value, "none" | "other")
                || crate::keyboard::is_dictation_key_id(value)
                || crate::keyboard::is_transform_or_query_key_id(value)
        }
        "modifier_edge" => matches!(value, "press" | "release" | "not_modifier"),
        "caller_file" => {
            value.ends_with(".rs")
                && value
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_/.".contains(&b))
        }
        _ => false,
    }
}

/// Redact key identity from every string field. The keyboard stream keeps
/// only its exact schema; returns true when that schema was applied and no
/// other stream rule should run.
pub(super) fn sanitize_fields(
    stream: &str,
    obj: &mut serde_json::Map<String, serde_json::Value>,
) -> bool {
    if stream == "keyboard" {
        obj.retain(|key, value| match value.as_str() {
            Some(value) => is_safe_keyboard_string(key, value),
            None => true,
        });
        return true;
    }
    for value in obj.values_mut() {
        if let serde_json::Value::String(text) = value {
            if let Cow::Owned(redacted) = redact_key_identity(text, false) {
                *text = redacted;
            }
        }
    }
    false
}

/// The summary with any key identity redacted.
pub(super) fn bounded_summary(stream: &str, summary: String) -> String {
    match redact_key_identity(&summary, stream == "keyboard") {
        Cow::Owned(redacted) => redacted,
        Cow::Borrowed(_) => summary,
    }
}

/// Re-apply the key-identity boundary to one persisted `AppEvent` JSONL line,
/// which an older build may have written. Unchanged and unparseable lines are
/// returned as-is, so callers keep their own framing and offsets.
pub(crate) fn sanitize_persisted_line(line: &str) -> Cow<'_, str> {
    let Ok(serde_json::Value::Object(mut event)) = serde_json::from_str(line) else {
        return Cow::Borrowed(line);
    };
    let stream = event
        .get("stream")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let mut changed = false;
    if let Some(data) = event
        .get_mut("data")
        .and_then(serde_json::Value::as_object_mut)
    {
        let before = data.clone();
        sanitize_fields(&stream, data);
        changed |= *data != before;
    }
    if let Some(summary) = event.get_mut("summary") {
        if let Some(text) = summary.as_str() {
            let bounded = bounded_summary(&stream, text.to_string());
            if bounded != text {
                *summary = serde_json::Value::String(bounded);
                changed = true;
            }
        }
    }
    if changed {
        Cow::Owned(serde_json::Value::Object(event).to_string())
    } else {
        Cow::Borrowed(line)
    }
}

/// `app.log` formatter: the default layout, with the key-identity boundary
/// applied to the message and fields.
pub(super) struct PrettyFormat;

impl<S, N> FormatEvent<S, N> for PrettyFormat
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        _ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &tracing::Event<'_>,
    ) -> std::fmt::Result {
        let meta = event.metadata();
        let stream = meta.target();
        let mut visitor = super::JsonVisitor::new();
        event.record(&mut visitor);
        sanitize_fields(stream, &mut visitor.fields);
        let message = bounded_summary(stream, visitor.message.unwrap_or_default());
        write!(
            writer,
            "{} {:>5} {stream}: {message}",
            chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true),
            meta.level(),
        )?;
        for (key, value) in &visitor.fields {
            match value {
                // Quoted and escaped, like the default formatter, so a value
                // cannot forge a log line or blur field boundaries.
                serde_json::Value::String(value) => write!(writer, " {key}={value:?}")?,
                value => write!(writer, " {key}={value}")?,
            }
        }
        writeln!(writer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rdev::{EventType, Key};
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::layer::{Context, SubscriberExt};
    use tracing_subscriber::{Layer, Registry};

    const LEAKS: &[&str] = &[
        "KeyI",
        "KeyA",
        "Num4",
        "SemiColon",
        "Slash",
        "Escape",
        "Unknown(",
    ];

    fn assert_no_leak(output: &str) {
        for leak in LEAKS {
            assert!(!output.contains(leak), "{leak} leaked: {output}");
        }
    }

    /// Events a current or future call site could write.
    fn emit_leaky_events() {
        tracing::info!(
            target: "keyboard",
            reason = "combo_cancelled",
            target_key = "shift_l",
            key_class = "character",
            is_target_key = false,
            event_type = ?EventType::KeyPress(Key::KeyI),
            key = ?Some(Key::SemiColon),
            "keyboard detector rejected sequence"
        );
        tracing::info!(target: "keyboard", "Escape pressed — emitting escape-cancel");
        tracing::info!(target: "keyboard", "saw {:?}", EventType::KeyRelease(Key::Num4));
        tracing::info!(target: "system", key = ?Some(Key::Slash), "typed KeyA then Num4");
        tracing::info!(
            target: "system",
            key = ?Some(Key::Unknown(63)),
            event_type = ?EventType::KeyPress(Key::Unknown(63)),
            "raw code Unknown(63)"
        );
        // Paths and model names that merely contain a key name survive.
        tracing::info!(
            target: "system",
            model = "/models/KeyA/model.bin",
            "loaded vendor/KeyA-7B"
        );
        // Ordinary words are not key identity outside the keyboard stream.
        tracing::info!(target: "system", phase = "End", "Return to idle");
    }

    #[derive(Clone, Default)]
    struct SharedBuf(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for SharedBuf {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn pretty_log_sink_never_names_a_typed_key() {
        let buf = SharedBuf::default();
        let writer = buf.clone();
        let layer = tracing_subscriber::fmt::layer()
            .event_format(PrettyFormat)
            .with_writer(move || writer.clone())
            .with_ansi(false);
        tracing::subscriber::with_default(Registry::default().with(layer), emit_leaky_events);

        let output = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
        assert_eq!(output.lines().count(), 7, "{output}");
        for line in output
            .lines()
            .filter(|line| !line.contains("loaded vendor/"))
        {
            assert_no_leak(line);
        }
        assert!(output.contains("INFO keyboard: keyboard detector rejected sequence"));
        assert!(output.contains(r#"reason="combo_cancelled""#), "{output}");
        assert!(output.contains(r#"key_class="character""#), "{output}");
        assert!(output.contains("is_target_key=false"), "{output}");
        assert!(output.contains(r#"target_key="shift_l""#), "{output}");
        assert!(output.contains("keyboard: [key] pressed"), "{output}");
        assert!(
            output.contains("system: typed [key] then [key]"),
            "{output}"
        );
        assert!(
            output.contains(
                r#"system: raw code [key] event_type="KeyPress([key])" key="Some([key])""#
            ),
            "{output}"
        );
        assert!(
            output.contains(r#"system: loaded vendor/KeyA-7B model="/models/KeyA/model.bin""#),
            "{output}"
        );
        assert!(
            output.contains(r#"system: Return to idle phase="End""#),
            "{output}"
        );
    }

    #[test]
    fn structured_event_sink_never_names_a_typed_key_in_any_build() {
        #[derive(Clone, Default)]
        struct Capture(Arc<Mutex<Vec<String>>>, bool);

        impl<S: tracing::Subscriber> Layer<S> for Capture {
            fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
                let stream = event.metadata().target();
                let (summary, data) = super::super::sanitized_event_payload(stream, event, self.1);
                self.0
                    .lock()
                    .unwrap()
                    .push(serde_json::json!({ "summary": summary, "data": data }).to_string());
            }
        }

        for debug_build in [true, false] {
            let layer = Capture(Arc::default(), debug_build);
            let captured = Arc::clone(&layer.0);
            tracing::subscriber::with_default(Registry::default().with(layer), emit_leaky_events);
            let events = captured.lock().unwrap().clone();
            assert_eq!(events.len(), 7);
            for event in &events[..5] {
                assert_no_leak(event);
            }
            assert!(!events[0].contains("KeyPress"), "{events:?}");
            assert!(events[4].contains(r#""key":"Some([key])""#), "{events:?}");
            assert!(
                events[5].contains(r#""model":"/models/KeyA/model.bin""#),
                "{events:?}"
            );
            assert!(
                events[0].contains(r#""key_class":"character""#),
                "{events:?}"
            );
            assert!(
                events[0].contains(r#""target_key":"shift_l""#),
                "{events:?}"
            );
            assert!(events[6].contains(r#""phase":"End""#), "{events:?}");
        }
    }

    #[test]
    fn hotkey_arming_cancel_keeps_its_content_free_schema() {
        for reason in ["other_key", "released"] {
            let mut data = serde_json::json!({
                "event_code": "keyboard.hotkey_arming_cancelled",
                "mode": "hold_down",
                "reason": reason,
                "held_ms": 81,
            });
            let expected = data.clone();
            assert!(sanitize_fields("keyboard", data.as_object_mut().unwrap()));
            assert_eq!(data, expected);
        }
    }

    #[test]
    fn keyboard_fields_keep_only_the_exact_schema() {
        let mut data = serde_json::json!({
            "reason": "combo_cancelled",
            "detector": "hold_down",
            "state": "Held",
            "double_tap_state": "Some(WaitingFirstUp)",
            "hold_state": "None",
            "mode": "Both",
            "target_key": "shift_l",
            "event_kind": "key_press",
            "key_class": "character",
            "is_target_key": false,
            "elapsed_ms": 42,
            "event_name": "Some(\"i\")",
            "raw": "Num4",
            "unknown": "KeyRelease(Unknown(63))",
            // Known fields must not smuggle a key identity either.
            "event_code": "KeyPress(KeyI)",
            "modifier_edge": "Some(KeyA)",
        });
        let obj = data.as_object_mut().unwrap();
        assert!(sanitize_fields("keyboard", obj));
        assert_eq!(
            data,
            serde_json::json!({
                "reason": "combo_cancelled",
                "detector": "hold_down",
                "state": "Held",
                "double_tap_state": "Some(WaitingFirstUp)",
                "hold_state": "None",
                "mode": "Both",
                "target_key": "shift_l",
                "event_kind": "key_press",
                "key_class": "character",
                "is_target_key": false,
                "elapsed_ms": 42,
            })
        );
    }

    #[test]
    fn other_streams_redact_only_key_shaped_tokens() {
        let data = serde_json::json!({
            "event_type": "KeyPress(Slash)",
            "key": "Some(KeyA)",
            "wrapped_word": "Some(Return)",
            "unknown": "Some(Unknown(63))",
            "bare": "Num4",
            "phase": "End",
            "label": "Home",
            "model": "/models/KeyA/model.bin",
            "elapsed_ms": 7,
        });
        for stream in ["system", "audio", "pipeline", "transform", "ui"] {
            let mut data = data.clone();
            assert!(!sanitize_fields(stream, data.as_object_mut().unwrap()));
            assert_eq!(
                data,
                serde_json::json!({
                    "event_type": "KeyPress([key])",
                    "key": "Some([key])",
                    "wrapped_word": "Some([key])",
                    "unknown": "Some([key])",
                    "bare": "[key]",
                    "phase": "End",
                    "label": "Home",
                    "model": "/models/KeyA/model.bin",
                    "elapsed_ms": 7,
                }),
                "{stream}"
            );
        }
    }

    #[test]
    fn key_names_count_only_as_delimited_tokens() {
        for (text, redacted) in [
            ("KeyA", "[key]"),
            ("(KeyA)", "([key])"),
            ("a,KeyA,b", "a,[key],b"),
            ("key=KeyA", "key=[key]"),
            ("key:KeyA", "key:[key]"),
            (r#""KeyA""#, r#""[key]""#),
            ("'Num4'", "'[key]'"),
            ("typed SemiColon then Num4", "typed [key] then [key]"),
            ("Some(Unknown(63))", "Some([key])"),
            ("code Unknown(7) end", "code [key] end"),
        ] {
            assert_eq!(redact_key_identity(text, false), redacted, "{text}");
        }
        for text in [
            "/models/KeyA/model.bin",
            "vendor/KeyA-7B",
            "KeyA_x",
            "KeyA.bin",
            "xKeyA",
            "KeyA1",
            "Num4-bit",
            "Unknown(device)",
            "Unknown()",
            "Return to idle",
            "End",
        ] {
            assert!(
                matches!(redact_key_identity(text, false), Cow::Borrowed(_)),
                "{text}"
            );
        }
        // Word-like key names count in the keyboard stream or in parentheses.
        assert_eq!(redact_key_identity("Escape pressed", true), "[key] pressed");
        assert_eq!(redact_key_identity("Some(End)", false), "Some([key])");
        assert_eq!(
            redact_key_identity("Unknown(device) KeyPress(Return)", false),
            "Unknown(device) KeyPress([key])"
        );
    }

    #[test]
    fn pretty_log_quotes_and_escapes_string_fields() {
        let buf = SharedBuf::default();
        let writer = buf.clone();
        let layer = tracing_subscriber::fmt::layer()
            .event_format(PrettyFormat)
            .with_writer(move || writer.clone())
            .with_ansi(false);
        tracing::subscriber::with_default(Registry::default().with(layer), || {
            tracing::info!(
                target: "system",
                detail = "a\nZ  INFO forged: line",
                label = "has spaces x=1",
                "event"
            );
        });

        let output = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
        assert_eq!(output.lines().count(), 1, "{output}");
        assert!(
            output.contains(r#"detail="a\nZ  INFO forged: line" label="has spaces x=1""#),
            "{output}"
        );
    }

    #[test]
    fn persisted_backlog_is_sanitized_line_by_line() {
        // Lines exactly as v0.51.6–v0.52.2 wrote them.
        let legacy = [
            r#"{"timestamp":"t1","stream":"keyboard","level":"info","summary":"Structured event","data":{"reason":"combo_cancelled","mode":"Both","event_type":"KeyPress(KeyI)","key":"Some(KeyI)","hold_state":"Some(Held)"}}"#,
            r#"{"timestamp":"t2","stream":"keyboard","level":"info","summary":"Escape pressed — emitting escape-cancel","data":{}}"#,
            r#"{"timestamp":"t3","stream":"keyboard","level":"info","summary":"Structured event","data":{"detector":"double_tap","target_key":"Some(ShiftLeft)","event_type":"KeyPress(Num4)","elapsed_ms":12}}"#,
        ];
        let clean = r#"{"timestamp":"t4","stream":"system","level":"info","summary":"Return to idle","data":{"phase":"End"}}"#;

        for line in legacy {
            let sanitized = sanitize_persisted_line(line);
            assert!(matches!(sanitized, Cow::Owned(_)), "{line}");
            assert_no_leak(&sanitized);
            assert!(!sanitized.contains('\n'));
            let event: super::super::AppEvent = serde_json::from_str(&sanitized).unwrap();
            assert_eq!(event.stream, "keyboard");
        }
        let first = sanitize_persisted_line(legacy[0]);
        assert!(first.contains(r#""reason":"combo_cancelled""#), "{first}");
        assert!(first.contains(r#""hold_state":"Some(Held)""#), "{first}");
        assert!(first.contains(r#""timestamp":"t1""#), "{first}");

        assert!(matches!(sanitize_persisted_line(clean), Cow::Borrowed(_)));
        assert!(matches!(
            sanitize_persisted_line("not json"),
            Cow::Borrowed(_)
        ));
    }
}
