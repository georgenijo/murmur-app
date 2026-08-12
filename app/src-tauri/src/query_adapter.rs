//! Incremental provider-output adapters for Voice Query (#551).
//!
//! Claude and Codex emit JSON Lines when their presets request structured
//! output. The adapters extract only assistant text, typed failures, and
//! content-free usage numbers. Custom and providers without a structured
//! contract retain the original raw-stdout behavior.
//!
//! Structured parsing is deliberately fail-safe. The complete structured
//! byte stream is retained until completion; if any non-empty line is
//! malformed or unexpected, one `Replace` update swaps any text emitted so
//! far for those original bytes and all later bytes stream as raw text. That
//! gives the caller an exact, non-duplicated raw fallback without ever making
//! malformed provider JSON fail the query pass.

use crate::query_provider::QueryProviderId;

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct QueryUsage {
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) reasoning_output_tokens: u64,
    pub(crate) cached_input_tokens: u64,
    pub(crate) cache_creation_input_tokens: u64,
    pub(crate) cost_usd: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AnswerUpdate {
    Append(String),
    Replace(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderFailure {
    pub(crate) detail: Option<String>,
}

#[derive(Debug, Default)]
pub(crate) struct AdapterCompletion {
    pub(crate) updates: Vec<AnswerUpdate>,
    pub(crate) usage: Option<QueryUsage>,
    pub(crate) failure: Option<ProviderFailure>,
    pub(crate) used_structured_output: bool,
}

/// The single output seam used by the query runner. Process spawning, argv,
/// timeouts, and cancellation remain generic and provider-independent.
pub(crate) struct VoiceQueryAdapter {
    inner: AdapterKind,
}

enum AdapterKind {
    Raw(RawAdapter),
    JsonLines(JsonLinesAdapter),
}

impl VoiceQueryAdapter {
    pub(crate) fn new(provider: QueryProviderId, max_output_bytes: usize) -> Self {
        let inner = match provider {
            QueryProviderId::Claude => AdapterKind::JsonLines(JsonLinesAdapter::new(
                StructuredProvider::Claude,
                max_output_bytes,
            )),
            QueryProviderId::Codex => AdapterKind::JsonLines(JsonLinesAdapter::new(
                StructuredProvider::Codex,
                max_output_bytes,
            )),
            QueryProviderId::Grok | QueryProviderId::Cursor | QueryProviderId::Custom => {
                AdapterKind::Raw(RawAdapter::default())
            }
        };
        Self { inner }
    }

    pub(crate) fn push_stdout(&mut self, bytes: &[u8]) -> Result<Vec<AnswerUpdate>, &'static str> {
        match &mut self.inner {
            AdapterKind::Raw(adapter) => Ok(adapter.push(bytes)),
            AdapterKind::JsonLines(adapter) => adapter.push(bytes),
        }
    }

    pub(crate) fn finish(&mut self) -> Result<AdapterCompletion, &'static str> {
        match &mut self.inner {
            AdapterKind::Raw(adapter) => Ok(AdapterCompletion {
                updates: adapter.finish(),
                ..AdapterCompletion::default()
            }),
            AdapterKind::JsonLines(adapter) => adapter.finish(),
        }
    }
}

#[derive(Default)]
struct Utf8Chunks {
    pending: Vec<u8>,
}

impl Utf8Chunks {
    fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        match std::str::from_utf8(&self.pending) {
            Ok(text) => {
                let text = text.to_string();
                self.pending.clear();
                text
            }
            Err(error) => {
                let valid = error.valid_up_to();
                if error.error_len().is_none() {
                    let text = String::from_utf8_lossy(&self.pending[..valid]).into_owned();
                    self.pending.drain(..valid);
                    text
                } else {
                    let text = String::from_utf8_lossy(&self.pending).into_owned();
                    self.pending.clear();
                    text
                }
            }
        }
    }

    fn finish(&mut self) -> String {
        let text = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        text
    }
}

#[derive(Default)]
struct RawAdapter {
    decoder: Utf8Chunks,
}

impl RawAdapter {
    fn push(&mut self, bytes: &[u8]) -> Vec<AnswerUpdate> {
        let text = self.decoder.push(bytes);
        (!text.is_empty())
            .then_some(AnswerUpdate::Append(text))
            .into_iter()
            .collect()
    }

    fn finish(&mut self) -> Vec<AnswerUpdate> {
        let text = self.decoder.finish();
        (!text.is_empty())
            .then_some(AnswerUpdate::Append(text))
            .into_iter()
            .collect()
    }
}

#[derive(Debug, Clone, Copy)]
enum StructuredProvider {
    Claude,
    Codex,
}

struct JsonLinesAdapter {
    provider: StructuredProvider,
    max_output_bytes: usize,
    total_output_bytes: usize,
    raw_bytes: Vec<u8>,
    pending_line: Vec<u8>,
    raw_fallback: Option<RawAdapter>,
    emitted_answer: bool,
    usage: Option<QueryUsage>,
    failure: Option<ProviderFailure>,
}

impl JsonLinesAdapter {
    fn new(provider: StructuredProvider, max_output_bytes: usize) -> Self {
        Self {
            provider,
            max_output_bytes,
            total_output_bytes: 0,
            raw_bytes: Vec::new(),
            pending_line: Vec::new(),
            raw_fallback: None,
            emitted_answer: false,
            usage: None,
            failure: None,
        }
    }

    fn push(&mut self, bytes: &[u8]) -> Result<Vec<AnswerUpdate>, &'static str> {
        self.total_output_bytes = self
            .total_output_bytes
            .checked_add(bytes.len())
            .filter(|total| *total <= self.max_output_bytes)
            .ok_or("output_too_large")?;

        if let Some(raw) = &mut self.raw_fallback {
            return Ok(raw.push(bytes));
        }

        self.raw_bytes.extend_from_slice(bytes);
        self.pending_line.extend_from_slice(bytes);
        let mut updates = Vec::new();
        while let Some(newline) = self.pending_line.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.pending_line.drain(..=newline).collect();
            match self.parse_line(&line) {
                Some(mut parsed) => updates.append(&mut parsed),
                None => {
                    updates.push(self.enable_raw_fallback());
                    break;
                }
            }
        }
        Ok(updates)
    }

    fn finish(&mut self) -> Result<AdapterCompletion, &'static str> {
        let mut updates = Vec::new();
        if let Some(raw) = &mut self.raw_fallback {
            updates.extend(raw.finish());
            return Ok(AdapterCompletion {
                updates,
                ..AdapterCompletion::default()
            });
        }

        if !self.pending_line.is_empty() {
            let line = std::mem::take(&mut self.pending_line);
            match self.parse_line(&line) {
                Some(mut parsed) => updates.append(&mut parsed),
                None => {
                    updates.push(self.enable_raw_fallback());
                    if let Some(raw) = &mut self.raw_fallback {
                        updates.extend(raw.finish());
                    }
                    return Ok(AdapterCompletion {
                        updates,
                        ..AdapterCompletion::default()
                    });
                }
            }
        }

        self.raw_bytes.clear();
        Ok(AdapterCompletion {
            updates,
            usage: self.usage.take(),
            failure: self.failure.take(),
            used_structured_output: true,
        })
    }

    fn parse_line(&mut self, bytes: &[u8]) -> Option<Vec<AnswerUpdate>> {
        let line = bytes.strip_suffix(b"\n").unwrap_or(bytes);
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.iter().all(u8::is_ascii_whitespace) {
            return Some(Vec::new());
        }
        let value: serde_json::Value = serde_json::from_slice(line).ok()?;
        match self.provider {
            StructuredProvider::Claude => self.parse_claude(&value),
            StructuredProvider::Codex => self.parse_codex(&value),
        }
    }

    fn parse_claude(&mut self, value: &serde_json::Value) -> Option<Vec<AnswerUpdate>> {
        let event_type = value.get("type")?.as_str()?;
        match event_type {
            "system" | "assistant" | "user" | "rate_limit_event" => Some(Vec::new()),
            "stream_event" => self.parse_claude_stream_event(value.get("event")?),
            "result" => {
                if let Some(usage) = usage_from(value.get("usage"), value.get("total_cost_usd")) {
                    self.usage = Some(usage);
                }
                let subtype = value.get("subtype").and_then(serde_json::Value::as_str);
                let failed = value
                    .get("is_error")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false)
                    || subtype.is_some_and(|subtype| subtype.starts_with("error"));
                let result = value
                    .get("result")
                    .and_then(serde_json::Value::as_str)
                    .filter(|result| !result.is_empty());
                if failed {
                    self.failure = Some(ProviderFailure {
                        detail: result.map(str::to_string),
                    });
                    Some(Vec::new())
                } else if !self.emitted_answer {
                    let result = result?;
                    self.emitted_answer = true;
                    Some(vec![AnswerUpdate::Append(result.to_string())])
                } else {
                    Some(Vec::new())
                }
            }
            "error" => {
                self.failure = Some(ProviderFailure {
                    detail: error_message(value),
                });
                Some(Vec::new())
            }
            _ => None,
        }
    }

    fn parse_claude_stream_event(
        &mut self,
        event: &serde_json::Value,
    ) -> Option<Vec<AnswerUpdate>> {
        let event_type = event.get("type")?.as_str()?;
        match event_type {
            "message_start"
            | "content_block_start"
            | "content_block_stop"
            | "message_delta"
            | "message_stop"
            | "ping" => Some(Vec::new()),
            "content_block_delta" => {
                let delta = event.get("delta")?;
                let delta_type = delta.get("type")?.as_str()?;
                match delta_type {
                    "text_delta" => {
                        let text = delta.get("text")?.as_str()?;
                        if text.is_empty() {
                            Some(Vec::new())
                        } else {
                            self.emitted_answer = true;
                            Some(vec![AnswerUpdate::Append(text.to_string())])
                        }
                    }
                    "thinking_delta" | "signature_delta" | "input_json_delta"
                    | "citations_delta" => Some(Vec::new()),
                    _ => None,
                }
            }
            "error" => {
                self.failure = Some(ProviderFailure {
                    detail: error_message(event),
                });
                Some(Vec::new())
            }
            _ => None,
        }
    }

    fn parse_codex(&mut self, value: &serde_json::Value) -> Option<Vec<AnswerUpdate>> {
        let event_type = value.get("type")?.as_str()?;
        match event_type {
            "thread.started" | "turn.started" | "item.started" | "item.updated" => Some(Vec::new()),
            "item.completed" => {
                let item = value.get("item")?;
                let item_type = item.get("type")?.as_str()?;
                if item_type != "agent_message" {
                    return Some(Vec::new());
                }
                let text = item.get("text")?.as_str()?;
                if text.is_empty() {
                    Some(Vec::new())
                } else {
                    self.emitted_answer = true;
                    Some(vec![AnswerUpdate::Append(text.to_string())])
                }
            }
            "turn.completed" => {
                if let Some(usage) = usage_from(value.get("usage"), None) {
                    self.usage = Some(usage);
                }
                Some(Vec::new())
            }
            "turn.failed" | "error" => {
                self.failure = Some(ProviderFailure {
                    detail: error_message(value),
                });
                Some(Vec::new())
            }
            _ => None,
        }
    }

    fn enable_raw_fallback(&mut self) -> AnswerUpdate {
        let mut raw = RawAdapter::default();
        let text = raw.decoder.push(&self.raw_bytes);
        self.raw_bytes.clear();
        self.pending_line.clear();
        self.usage = None;
        self.failure = None;
        self.emitted_answer = false;
        self.raw_fallback = Some(raw);
        AnswerUpdate::Replace(text)
    }
}

fn usage_from(
    usage: Option<&serde_json::Value>,
    top_level_cost: Option<&serde_json::Value>,
) -> Option<QueryUsage> {
    let usage = usage?.as_object()?;
    let value = |key: &str| usage.get(key).and_then(serde_json::Value::as_u64);
    let input_tokens = value("input_tokens");
    let output_tokens = value("output_tokens");
    let reasoning_output_tokens = value("reasoning_output_tokens");
    let cached_input_tokens =
        value("cached_input_tokens").or_else(|| value("cache_read_input_tokens"));
    let cache_creation_input_tokens =
        value("cache_creation_input_tokens").or_else(|| value("cache_write_input_tokens"));
    let cost_usd = top_level_cost
        .and_then(serde_json::Value::as_f64)
        .filter(|cost| cost.is_finite() && *cost >= 0.0);
    if input_tokens.is_none()
        && output_tokens.is_none()
        && reasoning_output_tokens.is_none()
        && cached_input_tokens.is_none()
        && cache_creation_input_tokens.is_none()
        && cost_usd.is_none()
    {
        return None;
    }
    Some(QueryUsage {
        input_tokens: input_tokens.unwrap_or(0),
        output_tokens: output_tokens.unwrap_or(0),
        reasoning_output_tokens: reasoning_output_tokens.unwrap_or(0),
        cached_input_tokens: cached_input_tokens.unwrap_or(0),
        cache_creation_input_tokens: cache_creation_input_tokens.unwrap_or(0),
        cost_usd,
    })
}

fn error_message(value: &serde_json::Value) -> Option<String> {
    value
        .get("message")
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            value
                .get("error")
                .and_then(|error| error.get("message"))
                .and_then(serde_json::Value::as_str)
        })
        .filter(|message| !message.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(answer: &mut String, updates: Vec<AnswerUpdate>) {
        for update in updates {
            match update {
                AnswerUpdate::Append(text) => answer.push_str(&text),
                AnswerUpdate::Replace(text) => *answer = text,
            }
        }
    }

    #[test]
    fn raw_adapter_preserves_split_utf8() {
        let mut adapter = VoiceQueryAdapter::new(QueryProviderId::Custom, 1024);
        let bytes = "hello 🦀".as_bytes();
        let mut answer = String::new();
        apply(&mut answer, adapter.push_stdout(&bytes[..8]).unwrap());
        apply(&mut answer, adapter.push_stdout(&bytes[8..]).unwrap());
        apply(&mut answer, adapter.finish().unwrap().updates);
        assert_eq!(answer, "hello 🦀");
    }

    #[test]
    fn claude_stream_extracts_only_text_deltas_and_final_usage() {
        let mut adapter = VoiceQueryAdapter::new(QueryProviderId::Claude, 4096);
        let stream = concat!(
            "{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"secret\"}\n",
            "{\"type\":\"stream_event\",\"event\":{\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello \"}}}\n",
            "{\"type\":\"stream_event\",\"event\":{\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"world\"}}}\n",
            "{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"Hello world\",\"total_cost_usd\":0.012,\"usage\":{\"input_tokens\":12,\"cache_creation_input_tokens\":3,\"cache_read_input_tokens\":4,\"output_tokens\":5}}\n"
        );
        let mut answer = String::new();
        for bytes in stream.as_bytes().chunks(17) {
            apply(&mut answer, adapter.push_stdout(bytes).unwrap());
        }
        let completion = adapter.finish().unwrap();
        apply(&mut answer, completion.updates);
        assert_eq!(answer, "Hello world");
        assert!(completion.used_structured_output);
        assert_eq!(
            completion.usage,
            Some(QueryUsage {
                input_tokens: 12,
                output_tokens: 5,
                reasoning_output_tokens: 0,
                cached_input_tokens: 4,
                cache_creation_input_tokens: 3,
                cost_usd: Some(0.012),
            })
        );
        assert_eq!(completion.failure, None);
    }

    #[test]
    fn claude_result_supplies_answer_when_no_partial_delta_arrives() {
        let mut adapter = VoiceQueryAdapter::new(QueryProviderId::Claude, 1024);
        let updates = adapter
            .push_stdout(
                br#"{"type":"result","subtype":"success","is_error":false,"result":"complete answer","usage":{"input_tokens":1,"output_tokens":2}}
"#,
            )
            .unwrap();
        assert_eq!(
            updates,
            vec![AnswerUpdate::Append("complete answer".into())]
        );
        assert!(adapter.finish().unwrap().used_structured_output);
    }

    #[test]
    fn codex_extracts_agent_message_and_usage() {
        let mut adapter = VoiceQueryAdapter::new(QueryProviderId::Codex, 4096);
        let stream = concat!(
            "{\"type\":\"thread.started\",\"thread_id\":\"private\"}\r\n",
            "{\"type\":\"item.completed\",\"item\":{\"id\":\"item_0\",\"type\":\"reasoning\",\"text\":\"hidden\"}}\n",
            "{\"type\":\"item.completed\",\"item\":{\"id\":\"item_1\",\"type\":\"agent_message\",\"text\":\"Clean answer\"}}\n",
            "{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":21,\"cached_input_tokens\":8,\"cache_write_input_tokens\":2,\"output_tokens\":13,\"reasoning_output_tokens\":5}}\n"
        );
        let mut answer = String::new();
        apply(&mut answer, adapter.push_stdout(stream.as_bytes()).unwrap());
        let completion = adapter.finish().unwrap();
        apply(&mut answer, completion.updates);
        assert_eq!(answer, "Clean answer");
        assert_eq!(
            completion.usage,
            Some(QueryUsage {
                input_tokens: 21,
                output_tokens: 13,
                reasoning_output_tokens: 5,
                cached_input_tokens: 8,
                cache_creation_input_tokens: 2,
                cost_usd: None,
            })
        );
    }

    #[test]
    fn structured_errors_are_typed_even_when_the_process_can_exit_zero() {
        let mut claude = VoiceQueryAdapter::new(QueryProviderId::Claude, 1024);
        claude
            .push_stdout(
                br#"{"type":"result","subtype":"error_during_execution","is_error":true,"result":"Not authenticated. Run /login."}
"#,
            )
            .unwrap();
        assert_eq!(
            claude.finish().unwrap().failure,
            Some(ProviderFailure {
                detail: Some("Not authenticated. Run /login.".into())
            })
        );

        let mut codex = VoiceQueryAdapter::new(QueryProviderId::Codex, 1024);
        codex
            .push_stdout(
                br#"{"type":"turn.failed","error":{"message":"model unavailable"}}
"#,
            )
            .unwrap();
        assert_eq!(
            codex.finish().unwrap().failure,
            Some(ProviderFailure {
                detail: Some("model unavailable".into())
            })
        );
    }

    #[test]
    fn malformed_json_replaces_prior_deltas_with_complete_raw_stdout_once() {
        let mut adapter = VoiceQueryAdapter::new(QueryProviderId::Claude, 4096);
        let first = "{\"type\":\"stream_event\",\"event\":{\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"answer\"}}}\n";
        let malformed = "not-json\n";
        let tail = "raw-tail";
        let mut answer = String::new();
        apply(&mut answer, adapter.push_stdout(first.as_bytes()).unwrap());
        assert_eq!(answer, "answer");
        apply(
            &mut answer,
            adapter.push_stdout(malformed.as_bytes()).unwrap(),
        );
        assert_eq!(answer, format!("{first}{malformed}"));
        apply(&mut answer, adapter.push_stdout(tail.as_bytes()).unwrap());
        apply(&mut answer, adapter.finish().unwrap().updates);
        assert_eq!(answer, format!("{first}{malformed}{tail}"));
        assert_eq!(answer.matches(malformed).count(), 1);
    }

    #[test]
    fn unexpected_json_also_degrades_to_raw_text() {
        let raw = "{\"unexpected\":true}\n";
        let mut adapter = VoiceQueryAdapter::new(QueryProviderId::Codex, 1024);
        assert_eq!(
            adapter.push_stdout(raw.as_bytes()).unwrap(),
            vec![AnswerUpdate::Replace(raw.into())]
        );
        assert!(!adapter.finish().unwrap().used_structured_output);
    }

    #[test]
    fn structured_raw_archive_keeps_the_existing_output_bound() {
        let mut adapter = VoiceQueryAdapter::new(QueryProviderId::Claude, 8);
        assert_eq!(adapter.push_stdout(b"123456789"), Err("output_too_large"));
    }
}
