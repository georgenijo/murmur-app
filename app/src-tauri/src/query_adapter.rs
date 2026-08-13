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

use crate::query_provider::{QueryProviderId, MAX_STDERR_BYTES};
use serde::Serialize;

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
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
pub(crate) enum ProviderFailureKind {
    Authentication,
    Provider,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderFailure {
    pub(crate) kind: ProviderFailureKind,
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

    /// True once a structured provider has degraded to its exact raw stdout.
    /// Raw-only providers return false because their output is the declared
    /// contract, while structured fallback may contain echoed prompt/context.
    pub(crate) fn used_structured_raw_fallback(&self) -> bool {
        match &self.inner {
            AdapterKind::Raw(_) => false,
            AdapterKind::JsonLines(adapter) => adapter.used_raw_fallback,
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
    used_raw_fallback: bool,
    emitted_answer: bool,
    terminal_seen: bool,
    claude_assistant_answer: Option<String>,
    usage: Option<QueryUsage>,
    failure: Option<ProviderFailure>,
}

const MAX_PROVIDER_DETAIL_BYTES: usize = MAX_STDERR_BYTES;

impl JsonLinesAdapter {
    fn new(provider: StructuredProvider, max_output_bytes: usize) -> Self {
        Self {
            provider,
            max_output_bytes,
            total_output_bytes: 0,
            raw_bytes: Vec::new(),
            pending_line: Vec::new(),
            raw_fallback: None,
            used_raw_fallback: false,
            emitted_answer: false,
            terminal_seen: false,
            claude_assistant_answer: None,
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

        // A successfully parsed prefix is not a complete structured response.
        // If the provider dies before its typed terminal frame, atomically
        // replace any optimistic deltas with the exact raw stream. This keeps
        // truncation and wrapper/version mismatches on the legacy-safe path.
        if !self.terminal_seen {
            return Ok(self.finish_as_raw_fallback());
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
            "system" => validate_claude_system(value).map(|()| Vec::new()),
            "assistant" => self.parse_claude_assistant(value),
            "user" => validate_claude_user(value).map(|()| Vec::new()),
            "rate_limit_event" => validate_rate_limit_event(value).map(|()| Vec::new()),
            "tool_progress" => validate_tool_progress(value).map(|()| Vec::new()),
            "tool_use_summary" => validate_tool_use_summary(value).map(|()| Vec::new()),
            "prompt_suggestion" => validate_prompt_suggestion(value).map(|()| Vec::new()),
            "auth_status" => validate_auth_status(value).map(|()| Vec::new()),
            "conversation_reset" => validate_conversation_reset(value).map(|()| Vec::new()),
            "stream_event" => {
                validate_identity(value)?;
                validate_nullable_string(value.get("parent_tool_use_id"))?;
                self.parse_claude_stream_event(value.get("event")?)
            }
            "result" => self.parse_claude_result(value),
            "error" => {
                let detail = error_message(value)?;
                let kind = failure_kind_from_error_value(value);
                self.record_failure(kind, Some(detail));
                self.terminal_seen = true;
                Some(Vec::new())
            }
            _ => None,
        }
    }

    fn parse_claude_assistant(&mut self, value: &serde_json::Value) -> Option<Vec<AnswerUpdate>> {
        validate_identity(value)?;
        let message = value.get("message")?.as_object()?;
        let text = assistant_text(message)?;
        validate_nullable_string(value.get("parent_tool_use_id"))?;
        if let Some(aborted) = value.get("aborted") {
            aborted.as_bool()?;
        }
        let error = match value.get("error") {
            None | Some(serde_json::Value::Null) => None,
            Some(error) => Some(error.as_str()?),
        };
        if let Some(error) = error {
            let kind = claude_assistant_failure_kind(error)?;
            let detail = if text.is_empty() { error } else { &text };
            self.record_failure(kind, Some(detail.to_string()));
        } else if !text.is_empty() {
            // With partial messages enabled, the final assistant envelope and
            // stream deltas contain the same text. Hold this as a fallback and
            // emit it only if no text delta (or result text) was available.
            self.claude_assistant_answer = Some(text);
        }
        Some(Vec::new())
    }

    fn parse_claude_result(&mut self, value: &serde_json::Value) -> Option<Vec<AnswerUpdate>> {
        validate_identity(value)?;
        let subtype = value.get("subtype")?.as_str()?;
        if !matches!(
            subtype,
            "success"
                | "error_during_execution"
                | "error_max_turns"
                | "error_max_budget_usd"
                | "error_max_structured_output_retries"
        ) {
            return None;
        }
        let is_error = value.get("is_error")?.as_bool()?;
        let result = optional_string(value.get("result"))?;
        if subtype == "success" && result.is_none() {
            return None;
        }
        let errors = optional_string_array(value.get("errors"))?;
        validate_optional_usage(value.get("usage"))?;
        validate_optional_nonnegative_number(value.get("total_cost_usd"))?;

        if let Some(usage) = usage_from(value.get("usage"), value.get("total_cost_usd")) {
            self.usage = Some(usage);
        }
        self.terminal_seen = true;

        let failed = is_error || subtype != "success" || !errors.is_empty();
        if failed {
            let mut details = errors;
            if let Some(result) = result.filter(|result| !result.is_empty()) {
                details.push(result.to_string());
            }
            if details.is_empty() {
                details.push(subtype.to_string());
            }
            let detail = bounded_join(details.iter().map(String::as_str));
            let kind = detail
                .as_deref()
                .map(claude_detail_failure_kind)
                .unwrap_or(ProviderFailureKind::Provider);
            self.record_failure(kind, detail);
            return Some(Vec::new());
        }

        if self.emitted_answer {
            return Some(Vec::new());
        }
        let answer = result
            .filter(|result| !result.is_empty())
            .map(str::to_string)
            .or_else(|| self.claude_assistant_answer.take());
        if let Some(answer) = answer.filter(|answer| !answer.is_empty()) {
            self.emitted_answer = true;
            Some(vec![AnswerUpdate::Append(answer)])
        } else {
            Some(Vec::new())
        }
    }

    fn parse_claude_stream_event(
        &mut self,
        event: &serde_json::Value,
    ) -> Option<Vec<AnswerUpdate>> {
        let event_type = event.get("type")?.as_str()?;
        match event_type {
            "message_start" => {
                assistant_text(event.get("message")?.as_object()?)?;
                Some(Vec::new())
            }
            "content_block_start" => {
                event.get("index")?.as_u64()?;
                validate_content_block(event.get("content_block")?)?;
                Some(Vec::new())
            }
            "content_block_stop" => {
                event.get("index")?.as_u64()?;
                Some(Vec::new())
            }
            "message_delta" => {
                let delta = event.get("delta")?.as_object()?;
                validate_nullable_string(delta.get("stop_reason"))?;
                validate_nullable_string(delta.get("stop_sequence"))?;
                validate_stream_usage(event.get("usage")?)?;
                Some(Vec::new())
            }
            "message_stop" | "ping" => Some(Vec::new()),
            "content_block_delta" => {
                event.get("index")?.as_u64()?;
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
                    "thinking_delta" => {
                        delta.get("thinking")?.as_str()?;
                        match delta.get("estimated_tokens") {
                            Some(serde_json::Value::Null) => {}
                            Some(value) => {
                                value.as_u64()?;
                            }
                            None => return None,
                        }
                        Some(Vec::new())
                    }
                    "signature_delta" => {
                        delta.get("signature")?.as_str()?;
                        Some(Vec::new())
                    }
                    "input_json_delta" => {
                        delta.get("partial_json")?.as_str()?;
                        Some(Vec::new())
                    }
                    "citations_delta" => {
                        delta.get("citation")?.as_object()?;
                        Some(Vec::new())
                    }
                    "compaction_delta" => {
                        validate_nullable_string(delta.get("content"))?;
                        validate_nullable_string(delta.get("encrypted_content"))?;
                        Some(Vec::new())
                    }
                    _ => None,
                }
            }
            "error" => {
                let detail = error_message(event)?;
                let kind = failure_kind_from_error_value(event);
                self.record_failure(kind, Some(detail));
                self.terminal_seen = true;
                Some(Vec::new())
            }
            _ => None,
        }
    }

    fn parse_codex(&mut self, value: &serde_json::Value) -> Option<Vec<AnswerUpdate>> {
        let event_type = value.get("type")?.as_str()?;
        match event_type {
            "thread.started" => {
                nonempty_string(value.get("thread_id")?)?;
                Some(Vec::new())
            }
            "turn.started" => Some(Vec::new()),
            "item.started" | "item.updated" => {
                validate_codex_item(value.get("item")?)?;
                Some(Vec::new())
            }
            "item.completed" => {
                let item = validate_codex_item(value.get("item")?)?;
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
                validate_codex_usage(value.get("usage")?)?;
                if let Some(usage) = usage_from(value.get("usage"), None) {
                    self.usage = Some(usage);
                }
                self.terminal_seen = true;
                Some(Vec::new())
            }
            "turn.failed" => {
                let detail = error_message(value)?;
                self.terminal_seen = true;
                let kind = codex_detail_failure_kind(&detail);
                self.record_failure(kind, Some(detail));
                Some(Vec::new())
            }
            "error" => {
                let detail = error_message(value)?;
                self.terminal_seen = true;
                let kind = codex_detail_failure_kind(&detail);
                self.record_failure(kind, Some(detail));
                Some(Vec::new())
            }
            _ => None,
        }
    }

    fn enable_raw_fallback(&mut self) -> AnswerUpdate {
        self.used_raw_fallback = true;
        let mut raw = RawAdapter::default();
        let text = raw.decoder.push(&self.raw_bytes);
        self.raw_bytes.clear();
        self.pending_line.clear();
        self.usage = None;
        self.failure = None;
        self.emitted_answer = false;
        self.terminal_seen = false;
        self.claude_assistant_answer = None;
        self.raw_fallback = Some(raw);
        AnswerUpdate::Replace(text)
    }

    fn finish_as_raw_fallback(&mut self) -> AdapterCompletion {
        self.used_raw_fallback = true;
        let text = String::from_utf8_lossy(&self.raw_bytes).into_owned();
        let mut updates = vec![AnswerUpdate::Replace(text)];
        if !self.pending_line.is_empty() {
            let tail = String::from_utf8_lossy(&self.pending_line).into_owned();
            if !tail.is_empty() {
                updates.push(AnswerUpdate::Append(tail));
            }
        }
        self.raw_bytes.clear();
        self.pending_line.clear();
        self.usage = None;
        self.failure = None;
        self.emitted_answer = false;
        self.terminal_seen = false;
        self.claude_assistant_answer = None;
        AdapterCompletion {
            updates,
            ..AdapterCompletion::default()
        }
    }

    fn record_failure(&mut self, kind: ProviderFailureKind, detail: Option<String>) {
        let detail = detail.and_then(|detail| bounded_join(std::iter::once(detail.as_str())));
        match &mut self.failure {
            Some(existing) => {
                if kind == ProviderFailureKind::Authentication {
                    existing.kind = ProviderFailureKind::Authentication;
                }
                existing.detail = bounded_join(
                    existing
                        .detail
                        .as_deref()
                        .into_iter()
                        .chain(detail.as_deref()),
                );
            }
            None => self.failure = Some(ProviderFailure { kind, detail }),
        }
    }
}

fn validate_identity(value: &serde_json::Value) -> Option<()> {
    nonempty_string(value.get("session_id")?)?;
    if let Some(uuid) = value.get("uuid") {
        nonempty_string(uuid)?;
    }
    Some(())
}

fn validate_nullable_string(value: Option<&serde_json::Value>) -> Option<()> {
    match value? {
        serde_json::Value::Null | serde_json::Value::String(_) => Some(()),
        _ => None,
    }
}

fn nonempty_string(value: &serde_json::Value) -> Option<&str> {
    value.as_str().filter(|value| !value.is_empty())
}

fn validate_claude_system(value: &serde_json::Value) -> Option<()> {
    validate_identity(value)?;
    match nonempty_string(value.get("subtype")?)? {
        "init" => {
            nonempty_string(value.get("model")?)?;
            nonempty_string(value.get("claude_code_version")?)?;
            string_array(value.get("tools")?)?;
        }
        "api_retry" => {
            value.get("attempt")?.as_u64()?;
            value.get("max_retries")?.as_u64()?;
            validate_nonnegative_number(value.get("retry_delay_ms")?)?;
            match value.get("error_status")? {
                serde_json::Value::Null => {}
                value => {
                    value.as_u64()?;
                }
            }
            claude_assistant_failure_kind(value.get("error")?.as_str()?)?;
        }
        "hook_started" => {
            nonempty_string(value.get("hook_id")?)?;
            nonempty_string(value.get("hook_name")?)?;
            nonempty_string(value.get("hook_event")?)?;
        }
        "hook_response" => {
            nonempty_string(value.get("hook_id")?)?;
            nonempty_string(value.get("hook_name")?)?;
            nonempty_string(value.get("hook_event")?)?;
            value.get("output")?.as_str()?;
            value.get("stdout")?.as_str()?;
            value.get("stderr")?.as_str()?;
            match value.get("exit_code")? {
                serde_json::Value::Null => {}
                exit_code => {
                    exit_code.as_i64()?;
                }
            }
            nonempty_string(value.get("outcome")?)?;
        }
        "status" => {
            nonempty_string(value.get("status")?)?;
        }
        "thinking_tokens" => {
            value.get("estimated_tokens")?.as_u64()?;
            value.get("estimated_tokens_delta")?.as_u64()?;
        }
        _ => return None,
    }
    Some(())
}

fn validate_claude_user(value: &serde_json::Value) -> Option<()> {
    validate_identity(value)?;
    validate_nullable_string(value.get("parent_tool_use_id"))?;
    let message = value.get("message")?.as_object()?;
    if message.get("role")?.as_str()? != "user" {
        return None;
    }
    match message.get("content")? {
        serde_json::Value::String(_) | serde_json::Value::Array(_) => Some(()),
        _ => None,
    }
}

fn validate_rate_limit_event(value: &serde_json::Value) -> Option<()> {
    validate_identity(value)?;
    let info = value.get("rate_limit_info")?.as_object()?;
    match info.get("status")?.as_str()? {
        "allowed" | "allowed_warning" | "rejected" => Some(()),
        _ => None,
    }
}

fn validate_tool_progress(value: &serde_json::Value) -> Option<()> {
    validate_identity(value)?;
    nonempty_string(value.get("tool_use_id")?)?;
    nonempty_string(value.get("tool_name")?)?;
    validate_nullable_string(value.get("parent_tool_use_id"))?;
    validate_nonnegative_number(value.get("elapsed_time_seconds")?)?;
    Some(())
}

fn validate_tool_use_summary(value: &serde_json::Value) -> Option<()> {
    validate_identity(value)?;
    value.get("summary")?.as_str()?;
    string_array(value.get("preceding_tool_use_ids")?)?;
    Some(())
}

fn validate_prompt_suggestion(value: &serde_json::Value) -> Option<()> {
    validate_identity(value)?;
    value.get("suggestion")?.as_str()?;
    Some(())
}

fn validate_auth_status(value: &serde_json::Value) -> Option<()> {
    validate_identity(value)?;
    value.get("isAuthenticating")?.as_bool()?;
    string_array(value.get("output")?)?;
    match value.get("error") {
        None | Some(serde_json::Value::Null) => Some(()),
        Some(error) => error.as_str().map(|_| ()),
    }
}

fn validate_conversation_reset(value: &serde_json::Value) -> Option<()> {
    validate_identity(value)?;
    nonempty_string(value.get("new_conversation_id")?)?;
    Some(())
}

fn assistant_text(message: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    if message.get("type")?.as_str()? != "message" || message.get("role")?.as_str()? != "assistant"
    {
        return None;
    }
    nonempty_string(message.get("id")?)?;
    nonempty_string(message.get("model")?)?;
    let blocks = message.get("content")?.as_array()?;
    let mut answer = String::new();
    for block in blocks {
        validate_content_block(block)?;
        if block.get("type")?.as_str()? == "text" {
            answer.push_str(block.get("text")?.as_str()?);
        }
    }
    Some(answer)
}

fn validate_content_block(value: &serde_json::Value) -> Option<()> {
    let block = value.as_object()?;
    match block.get("type")?.as_str()? {
        "text" => {
            block.get("text")?.as_str()?;
        }
        "thinking" => {
            block.get("thinking")?.as_str()?;
            block.get("signature")?.as_str()?;
        }
        "redacted_thinking" => {
            block.get("data")?.as_str()?;
        }
        "tool_use" | "server_tool_use" => {
            nonempty_string(block.get("id")?)?;
            nonempty_string(block.get("name")?)?;
            block.get("input")?;
        }
        "web_search_tool_result"
        | "web_fetch_tool_result"
        | "advisor_tool_result"
        | "code_execution_tool_result"
        | "bash_code_execution_tool_result"
        | "text_editor_code_execution_tool_result"
        | "tool_search_tool_result"
        | "mcp_tool_result" => {
            nonempty_string(block.get("tool_use_id")?)?;
            block.get("content")?;
        }
        "mcp_tool_use" => {
            nonempty_string(block.get("id")?)?;
            nonempty_string(block.get("name")?)?;
            nonempty_string(block.get("server_name")?)?;
            block.get("input")?;
        }
        "container_upload" => {
            nonempty_string(block.get("file_id")?)?;
        }
        "compaction" => {
            validate_nullable_string(block.get("content"))?;
            validate_nullable_string(block.get("encrypted_content"))?;
        }
        "fallback" => {
            block.get("from")?.as_object()?;
            block.get("to")?.as_object()?;
            block.get("trigger")?;
        }
        _ => return None,
    }
    Some(())
}

fn optional_string(value: Option<&serde_json::Value>) -> Option<Option<&str>> {
    match value {
        None | Some(serde_json::Value::Null) => Some(None),
        Some(value) => value.as_str().map(Some),
    }
}

fn string_array(value: &serde_json::Value) -> Option<Vec<&str>> {
    value
        .as_array()?
        .iter()
        .map(serde_json::Value::as_str)
        .collect()
}

fn optional_string_array(value: Option<&serde_json::Value>) -> Option<Vec<String>> {
    match value {
        None | Some(serde_json::Value::Null) => Some(Vec::new()),
        Some(value) => string_array(value).map(|values| {
            values
                .into_iter()
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect()
        }),
    }
}

fn validate_nonnegative_number(value: &serde_json::Value) -> Option<()> {
    value
        .as_f64()
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|_| ())
}

fn validate_optional_nonnegative_number(value: Option<&serde_json::Value>) -> Option<()> {
    match value {
        None | Some(serde_json::Value::Null) => Some(()),
        Some(value) => validate_nonnegative_number(value),
    }
}

fn validate_optional_usage(value: Option<&serde_json::Value>) -> Option<()> {
    let Some(value) = value else {
        return Some(());
    };
    if value.is_null() {
        return Some(());
    }
    let usage = value.as_object()?;
    for key in [
        "input_tokens",
        "output_tokens",
        "reasoning_output_tokens",
        "cached_input_tokens",
        "cache_read_input_tokens",
        "cache_creation_input_tokens",
        "cache_write_input_tokens",
    ] {
        if let Some(value) = usage.get(key) {
            value.as_u64()?;
        }
    }
    Some(())
}

fn validate_stream_usage(value: &serde_json::Value) -> Option<()> {
    let usage = value.as_object()?;
    usage.get("output_tokens")?.as_u64()?;
    for key in [
        "input_tokens",
        "cache_creation_input_tokens",
        "cache_read_input_tokens",
    ] {
        if let Some(value) = usage.get(key) {
            if !value.is_null() {
                value.as_u64()?;
            }
        }
    }
    Some(())
}

fn validate_codex_usage(value: &serde_json::Value) -> Option<()> {
    let usage = value.as_object()?;
    for key in ["input_tokens", "cached_input_tokens", "output_tokens"] {
        usage.get(key)?.as_u64()?;
    }
    validate_optional_usage(Some(value))
}

fn validate_codex_item(
    value: &serde_json::Value,
) -> Option<&serde_json::Map<String, serde_json::Value>> {
    let item = value.as_object()?;
    nonempty_string(item.get("id")?)?;
    match item.get("type")?.as_str()? {
        "agent_message" | "reasoning" => {
            item.get("text")?.as_str()?;
        }
        "command_execution" => {
            item.get("command")?.as_str()?;
            item.get("aggregated_output")?.as_str()?;
            validate_enum_string(
                item.get("status")?,
                &["in_progress", "completed", "failed", "declined"],
            )?;
        }
        "file_change" => {
            item.get("changes")?.as_array()?;
            validate_enum_string(item.get("status")?, &["in_progress", "completed", "failed"])?;
        }
        "mcp_tool_call" => {
            nonempty_string(item.get("server")?)?;
            nonempty_string(item.get("tool")?)?;
            validate_enum_string(item.get("status")?, &["in_progress", "completed", "failed"])?;
        }
        "collab_tool_call" => {
            item.get("tool")?.as_str()?;
            nonempty_string(item.get("sender_thread_id")?)?;
            string_array(item.get("receiver_thread_ids")?)?;
            validate_enum_string(item.get("status")?, &["in_progress", "completed", "failed"])?;
        }
        "web_search" => {
            item.get("query")?.as_str()?;
            item.get("action")?.as_object()?;
        }
        "todo_list" => {
            item.get("items")?.as_array()?;
        }
        "error" => {
            item.get("message")?.as_str()?;
        }
        _ => return None,
    }
    Some(item)
}

fn validate_enum_string(value: &serde_json::Value, allowed: &[&str]) -> Option<()> {
    allowed.contains(&value.as_str()?).then_some(())
}

fn claude_assistant_failure_kind(error: &str) -> Option<ProviderFailureKind> {
    match error {
        "authentication_failed" | "oauth_org_not_allowed" => {
            Some(ProviderFailureKind::Authentication)
        }
        "billing_error" | "rate_limit" | "overloaded" | "invalid_request" | "model_not_found"
        | "server_error" | "unknown" | "max_output_tokens" => Some(ProviderFailureKind::Provider),
        _ => None,
    }
}

fn claude_detail_failure_kind(detail: &str) -> ProviderFailureKind {
    let lower = detail.to_ascii_lowercase();
    if lower.contains("authentication_failed")
        || lower.contains("oauth_org_not_allowed")
        || crate::query_provider::is_auth_failure(QueryProviderId::Claude, detail, "")
    {
        ProviderFailureKind::Authentication
    } else {
        ProviderFailureKind::Provider
    }
}

fn codex_detail_failure_kind(detail: &str) -> ProviderFailureKind {
    if crate::query_provider::is_auth_failure(QueryProviderId::Codex, detail, "") {
        ProviderFailureKind::Authentication
    } else {
        ProviderFailureKind::Provider
    }
}

fn failure_kind_from_error_value(value: &serde_json::Value) -> ProviderFailureKind {
    let error_code = value
        .get("error")
        .and_then(|error| {
            error.as_str().or_else(|| {
                error
                    .get("type")
                    .or_else(|| error.get("code"))
                    .and_then(serde_json::Value::as_str)
            })
        })
        .or_else(|| value.get("code").and_then(serde_json::Value::as_str));
    if error_code.is_some_and(|code| {
        matches!(
            code,
            "authentication_failed"
                | "oauth_org_not_allowed"
                | "authentication_error"
                | "unauthorized"
        )
    }) {
        return ProviderFailureKind::Authentication;
    }
    error_message(value)
        .as_deref()
        .map(claude_detail_failure_kind)
        .unwrap_or(ProviderFailureKind::Provider)
}

fn bounded_join<'a>(parts: impl Iterator<Item = &'a str>) -> Option<String> {
    let mut joined = String::new();
    for part in parts.filter(|part| !part.is_empty()) {
        if !joined.is_empty() {
            push_bounded(&mut joined, "\n");
        }
        push_bounded(&mut joined, part);
        if joined.len() == MAX_PROVIDER_DETAIL_BYTES {
            break;
        }
    }
    (!joined.is_empty()).then_some(joined)
}

fn push_bounded(target: &mut String, value: &str) {
    let remaining = MAX_PROVIDER_DETAIL_BYTES.saturating_sub(target.len());
    if remaining == 0 {
        return;
    }
    let mut end = remaining.min(value.len());
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    target.push_str(&value[..end]);
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
            value.get("error").and_then(|error| {
                error
                    .as_str()
                    .or_else(|| error.get("message").and_then(serde_json::Value::as_str))
            })
        })
        .filter(|message| !message.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn apply(answer: &mut String, updates: Vec<AnswerUpdate>) {
        for update in updates {
            match update {
                AnswerUpdate::Append(text) => answer.push_str(&text),
                AnswerUpdate::Replace(text) => *answer = text,
            }
        }
    }

    fn line(value: serde_json::Value) -> String {
        format!("{value}\n")
    }

    fn claude_stream_event(event: serde_json::Value) -> serde_json::Value {
        json!({
            "type": "stream_event",
            "event": event,
            "parent_tool_use_id": null,
            "uuid": "event-uuid",
            "session_id": "private-session"
        })
    }

    fn claude_result(
        subtype: &str,
        is_error: bool,
        result: Option<&str>,
        errors: Option<Vec<&str>>,
    ) -> serde_json::Value {
        let mut value = json!({
            "type": "result",
            "subtype": subtype,
            "is_error": is_error,
            "usage": {"input_tokens": 1, "output_tokens": 2},
            "total_cost_usd": 0.0,
            "uuid": "result-uuid",
            "session_id": "private-session"
        });
        if let Some(result) = result {
            value["result"] = json!(result);
        }
        if let Some(errors) = errors {
            value["errors"] = json!(errors);
        }
        value
    }

    fn claude_assistant(content: serde_json::Value) -> serde_json::Value {
        json!({
            "type": "assistant",
            "message": {
                "id": "message-id",
                "type": "message",
                "role": "assistant",
                "model": "claude-sonnet",
                "content": content
            },
            "parent_tool_use_id": null,
            "uuid": "assistant-uuid",
            "session_id": "private-session"
        })
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
        assert!(!adapter.used_structured_raw_fallback());
    }

    #[test]
    fn claude_stream_extracts_only_text_deltas_and_final_usage() {
        let mut adapter = VoiceQueryAdapter::new(QueryProviderId::Claude, 8192);
        let stream = [
            line(json!({
                "type": "system",
                "subtype": "init",
                "model": "claude-sonnet",
                "claude_code_version": "2.1.228",
                "tools": [],
                "uuid": "system-uuid",
                "session_id": "private-session"
            })),
            line(claude_assistant(json!([
                {"type": "thinking", "thinking": "hidden", "signature": "opaque"},
                {"type": "text", "text": "Hello world"}
            ]))),
            line(claude_stream_event(json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": {"type": "text_delta", "text": "Hello "}
            }))),
            line(claude_stream_event(json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": {"type": "text_delta", "text": "world"}
            }))),
            line(json!({
                "type": "result",
                "subtype": "success",
                "is_error": false,
                "result": "Hello world",
                "total_cost_usd": 0.012,
                "usage": {
                    "input_tokens": 12,
                    "cache_creation_input_tokens": 3,
                    "cache_read_input_tokens": 4,
                    "output_tokens": 5
                },
                "uuid": "result-uuid",
                "session_id": "private-session"
            })),
        ]
        .concat();
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
                line(claude_result(
                    "success",
                    false,
                    Some("complete answer"),
                    None,
                ))
                .as_bytes(),
            )
            .unwrap();
        assert_eq!(
            updates,
            vec![AnswerUpdate::Append("complete answer".into())]
        );
        assert!(adapter.finish().unwrap().used_structured_output);
    }

    #[test]
    fn claude_assistant_message_supplies_text_when_result_text_is_empty() {
        let mut adapter = VoiceQueryAdapter::new(QueryProviderId::Claude, 4096);
        let assistant = line(claude_assistant(json!([
            {"type": "tool_use", "id": "tool-id", "name": "Read", "input": {}},
            {"type": "text", "text": "assistant fallback"}
        ])));
        assert!(adapter
            .push_stdout(assistant.as_bytes())
            .unwrap()
            .is_empty());
        let result = line(claude_result("success", false, Some(""), None));
        assert_eq!(
            adapter.push_stdout(result.as_bytes()).unwrap(),
            vec![AnswerUpdate::Append("assistant fallback".into())]
        );
        assert!(adapter.finish().unwrap().used_structured_output);
    }

    #[test]
    fn current_claude_non_answer_frames_are_validated_and_ignored() {
        let mut adapter = VoiceQueryAdapter::new(QueryProviderId::Claude, 16 * 1024);
        let frames = [
            json!({
                "type": "user",
                "message": {"role": "user", "content": "private prompt"},
                "parent_tool_use_id": null,
                "uuid": "user-uuid",
                "session_id": "private-session"
            }),
            json!({
                "type": "rate_limit_event",
                "rate_limit_info": {"status": "allowed"},
                "uuid": "rate-uuid",
                "session_id": "private-session"
            }),
            json!({
                "type": "tool_progress",
                "tool_use_id": "tool-id",
                "tool_name": "Read",
                "parent_tool_use_id": null,
                "elapsed_time_seconds": 0.25,
                "uuid": "progress-uuid",
                "session_id": "private-session"
            }),
            json!({
                "type": "tool_use_summary",
                "summary": "hidden",
                "preceding_tool_use_ids": ["tool-id"],
                "uuid": "summary-uuid",
                "session_id": "private-session"
            }),
            json!({
                "type": "auth_status",
                "isAuthenticating": false,
                "output": [],
                "uuid": "auth-uuid",
                "session_id": "private-session"
            }),
            json!({
                "type": "prompt_suggestion",
                "suggestion": "private follow-up",
                "uuid": "suggestion-uuid",
                "session_id": "private-session"
            }),
            json!({
                "type": "conversation_reset",
                "new_conversation_id": "conversation-id",
                "uuid": "reset-uuid",
                "session_id": "private-session"
            }),
        ];
        for frame in frames {
            assert!(adapter
                .push_stdout(line(frame).as_bytes())
                .unwrap()
                .is_empty());
        }
        assert!(adapter
            .push_stdout(line(claude_result("success", false, Some("answer"), None)).as_bytes())
            .unwrap()
            .contains(&AnswerUpdate::Append("answer".into())));
        assert!(adapter.finish().unwrap().used_structured_output);
    }

    #[test]
    fn current_claude_hook_and_status_frames_never_replace_the_answer_with_jsonl() {
        let mut adapter = VoiceQueryAdapter::new(QueryProviderId::Claude, 16 * 1024);
        let private_hook_output = "PRIVATE_STARTUP_HOOK_PROMPT";
        let frames = [
            json!({
                "type": "system",
                "subtype": "hook_started",
                "hook_id": "hook-id",
                "hook_name": "SessionStart:startup",
                "hook_event": "SessionStart",
                "uuid": "hook-started-uuid",
                "session_id": "private-session"
            }),
            json!({
                "type": "system",
                "subtype": "hook_response",
                "hook_id": "hook-id",
                "hook_name": "SessionStart:startup",
                "hook_event": "SessionStart",
                "output": private_hook_output,
                "stdout": private_hook_output,
                "stderr": "",
                "exit_code": 0,
                "outcome": "success",
                "uuid": "hook-response-uuid",
                "session_id": "private-session"
            }),
            json!({
                "type": "system",
                "subtype": "status",
                "status": "requesting",
                "uuid": "status-uuid",
                "session_id": "private-session"
            }),
            json!({
                "type": "system",
                "subtype": "thinking_tokens",
                "estimated_tokens": 50,
                "estimated_tokens_delta": 50,
                "uuid": "tokens-uuid",
                "session_id": "private-session"
            }),
        ];
        let mut answer = String::new();
        for frame in frames {
            apply(
                &mut answer,
                adapter.push_stdout(line(frame).as_bytes()).unwrap(),
            );
        }
        apply(
            &mut answer,
            adapter
                .push_stdout(
                    line(claude_result("success", false, Some("Clean answer"), None)).as_bytes(),
                )
                .unwrap(),
        );
        let completion = adapter.finish().unwrap();
        apply(&mut answer, completion.updates);

        assert_eq!(answer, "Clean answer");
        assert!(!answer.contains(private_hook_output));
        assert!(!answer.contains("hook_response"));
        assert!(completion.used_structured_output);
        assert!(!adapter.used_structured_raw_fallback());
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
                line(claude_result(
                    "error_during_execution",
                    true,
                    None,
                    Some(vec!["Not authenticated. Run /login."]),
                ))
                .as_bytes(),
            )
            .unwrap();
        assert_eq!(
            claude.finish().unwrap().failure,
            Some(ProviderFailure {
                kind: ProviderFailureKind::Authentication,
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
                kind: ProviderFailureKind::Provider,
                detail: Some("model unavailable".into())
            })
        );
    }

    #[test]
    fn claude_assistant_error_enum_preserves_typed_auth_failure() {
        let mut adapter = VoiceQueryAdapter::new(QueryProviderId::Claude, 4096);
        let mut assistant = claude_assistant(json!([]));
        assistant["error"] = json!("authentication_failed");
        adapter.push_stdout(line(assistant).as_bytes()).unwrap();
        adapter
            .push_stdout(
                line(claude_result(
                    "error_during_execution",
                    true,
                    None,
                    Some(vec!["credential rejected"]),
                ))
                .as_bytes(),
            )
            .unwrap();
        assert_eq!(
            adapter.finish().unwrap().failure,
            Some(ProviderFailure {
                kind: ProviderFailureKind::Authentication,
                detail: Some("authentication_failed\ncredential rejected".into()),
            })
        );
    }

    #[test]
    fn top_level_provider_errors_are_terminal_and_typed() {
        let mut claude = VoiceQueryAdapter::new(QueryProviderId::Claude, 1024);
        claude
            .push_stdout(
                line(json!({
                    "type": "error",
                    "error": {
                        "type": "authentication_error",
                        "message": "credential unavailable"
                    }
                }))
                .as_bytes(),
            )
            .unwrap();
        assert_eq!(
            claude.finish().unwrap().failure.unwrap().kind,
            ProviderFailureKind::Authentication
        );

        let mut codex = VoiceQueryAdapter::new(QueryProviderId::Codex, 1024);
        codex
            .push_stdout(line(json!({"type": "error", "message": "fatal stream error"})).as_bytes())
            .unwrap();
        assert_eq!(
            codex.finish().unwrap().failure.unwrap().kind,
            ProviderFailureKind::Provider
        );
    }

    #[test]
    fn typed_terminal_failure_outweighs_an_earlier_answer() {
        let mut claude = VoiceQueryAdapter::new(QueryProviderId::Claude, 4096);
        let partial = line(claude_stream_event(json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": {"type": "text_delta", "text": "partial answer"}
        })));
        claude.push_stdout(partial.as_bytes()).unwrap();
        claude
            .push_stdout(
                line(claude_result(
                    "error_during_execution",
                    true,
                    None,
                    Some(vec!["provider failed"]),
                ))
                .as_bytes(),
            )
            .unwrap();
        let completion = claude.finish().unwrap();
        assert!(completion.used_structured_output);
        assert_eq!(
            completion.failure,
            Some(ProviderFailure {
                kind: ProviderFailureKind::Provider,
                detail: Some("provider failed".into()),
            })
        );
    }

    #[test]
    fn malformed_json_replaces_prior_deltas_with_complete_raw_stdout_once() {
        let mut adapter = VoiceQueryAdapter::new(QueryProviderId::Claude, 4096);
        let first = line(claude_stream_event(json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": {"type": "text_delta", "text": "answer"}
        })));
        let malformed = "not-json\n";
        let tail = "raw-tail";
        let mut answer = String::new();
        apply(&mut answer, adapter.push_stdout(first.as_bytes()).unwrap());
        assert_eq!(answer, "answer");
        apply(
            &mut answer,
            adapter.push_stdout(malformed.as_bytes()).unwrap(),
        );
        assert!(adapter.used_structured_raw_fallback());
        assert_eq!(answer, format!("{first}{malformed}"));
        apply(&mut answer, adapter.push_stdout(tail.as_bytes()).unwrap());
        apply(&mut answer, adapter.finish().unwrap().updates);
        assert_eq!(answer, format!("{first}{malformed}{tail}"));
        assert_eq!(answer.matches(malformed).count(), 1);
    }

    #[test]
    fn valid_structured_prefix_without_terminal_frame_falls_back_at_eof() {
        let claude_raw = line(claude_stream_event(json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": {"type": "text_delta", "text": "partial"}
        })));
        let mut claude = VoiceQueryAdapter::new(QueryProviderId::Claude, 4096);
        assert_eq!(
            claude.push_stdout(claude_raw.as_bytes()).unwrap(),
            vec![AnswerUpdate::Append("partial".into())]
        );
        let completion = claude.finish().unwrap();
        assert_eq!(completion.updates, vec![AnswerUpdate::Replace(claude_raw)]);
        assert!(!completion.used_structured_output);
        assert!(claude.used_structured_raw_fallback());

        let codex_raw = line(json!({
            "type": "item.completed",
            "item": {"id": "item-1", "type": "agent_message", "text": "partial"}
        }));
        let mut codex = VoiceQueryAdapter::new(QueryProviderId::Codex, 4096);
        assert_eq!(
            codex.push_stdout(codex_raw.as_bytes()).unwrap(),
            vec![AnswerUpdate::Append("partial".into())]
        );
        let completion = codex.finish().unwrap();
        assert_eq!(completion.updates, vec![AnswerUpdate::Replace(codex_raw)]);
        assert!(!completion.used_structured_output);
        assert!(codex.used_structured_raw_fallback());
    }

    #[test]
    fn incomplete_terminal_eof_raw_replacement_preserves_split_utf8() {
        let mut raw = line(claude_stream_event(json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": {"type": "text_delta", "text": "partial"}
        })))
        .into_bytes();
        raw.extend_from_slice(&[0xf0, 0x9f]);
        let mut adapter = VoiceQueryAdapter::new(QueryProviderId::Claude, 4096);
        let updates = adapter.push_stdout(&raw).unwrap();
        assert_eq!(updates, vec![AnswerUpdate::Append("partial".into())]);
        assert_eq!(
            adapter.finish().unwrap().updates,
            vec![
                AnswerUpdate::Replace(String::from_utf8_lossy(&raw[..raw.len() - 2]).into_owned()),
                AnswerUpdate::Append("�".into()),
            ]
        );
    }

    #[test]
    fn malformed_known_frames_degrade_to_raw_instead_of_being_ignored() {
        let malformed_claude = line(json!({
            "type": "tool_progress",
            "tool_use_id": "tool-id",
            "tool_name": "Read",
            "parent_tool_use_id": null,
            "elapsed_time_seconds": "not-a-number",
            "uuid": "progress-uuid",
            "session_id": "private-session"
        }));
        let mut claude = VoiceQueryAdapter::new(QueryProviderId::Claude, 4096);
        assert_eq!(
            claude.push_stdout(malformed_claude.as_bytes()).unwrap(),
            vec![AnswerUpdate::Replace(malformed_claude)]
        );

        let malformed_codex = line(json!({
            "type": "item.completed",
            "item": {"id": "item-1", "type": "agent_message"}
        }));
        let mut codex = VoiceQueryAdapter::new(QueryProviderId::Codex, 4096);
        assert_eq!(
            codex.push_stdout(malformed_codex.as_bytes()).unwrap(),
            vec![AnswerUpdate::Replace(malformed_codex)]
        );
    }

    #[test]
    fn provider_detail_is_utf8_safe_and_bounded() {
        let detail = "🦀".repeat(MAX_PROVIDER_DETAIL_BYTES);
        let bounded = bounded_join(std::iter::once(detail.as_str())).unwrap();
        assert!(bounded.len() <= MAX_PROVIDER_DETAIL_BYTES);
        assert!(std::str::from_utf8(bounded.as_bytes()).is_ok());
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
