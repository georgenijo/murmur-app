//! Post-transcription voice command transforms.
//!
//! Spoken command tokens (e.g. "new line", "scratch that") are rewritten into
//! their effect before the text reaches the clipboard/auto-paste. This runs as
//! a pure string transform after transcription and is gated behind the
//! `voiceCommandsEnabled` setting.

use chrono::{DateTime, FixedOffset, Local};
use std::collections::HashSet;

use crate::knowledge_store::{
    KnowledgeDraft, KnowledgeEntry, KnowledgePayload, KnowledgeScope, VoiceCommandKind,
};

const MAX_EXPANDED_SNIPPET_CHARS: usize = 65_536;

pub(crate) const BUILTIN_COMMAND_PHRASES: &[&str] = &[
    "new paragraph",
    "new line",
    "scratch that",
    "open paren",
    "close paren",
    "question mark",
    "period",
    "comma",
];

pub(crate) fn is_builtin_phrase(normalized_phrase: &str) -> bool {
    BUILTIN_COMMAND_PHRASES
        .iter()
        .any(|phrase| crate::knowledge_store::normalize_key(phrase) == normalized_phrase)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedVoiceCommand {
    pub id: String,
    pub phrase: String,
    pub command_type: VoiceCommandKind,
    pub content: String,
    pub allow_clipboard_read: bool,
    pub app_scoped: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VoiceCommandApplication {
    pub text: String,
    pub matched: bool,
    pub clipboard_required: bool,
    pub clipboard_read: bool,
}

pub(crate) trait VoiceCommandRuntime {
    fn now(&self) -> DateTime<FixedOffset>;
    fn clipboard_text(&self) -> Result<String, String>;
}

pub(crate) struct SystemVoiceCommandRuntime;

impl VoiceCommandRuntime for SystemVoiceCommandRuntime {
    fn now(&self) -> DateTime<FixedOffset> {
        Local::now().fixed_offset()
    }

    fn clipboard_text(&self) -> Result<String, String> {
        crate::injector::read_clipboard_text()
    }
}

pub(crate) fn validate_snippet_template(
    template: &str,
    allow_clipboard_read: bool,
) -> Result<(), String> {
    let mut remaining = template;
    while let Some(start) = remaining.find("{{") {
        let after = &remaining[start + 2..];
        let end = after
            .find("}}")
            .ok_or_else(|| "Snippet variable is missing its closing braces.".to_string())?;
        let variable = after[..end].trim();
        if !matches!(variable, "date" | "time" | "clipboard") {
            return Err(format!(
                "Unsupported snippet variable '{{{{{variable}}}}}'. Use date, time, or clipboard."
            ));
        }
        if variable == "clipboard" && !allow_clipboard_read {
            return Err(
                "This snippet uses {{clipboard}}. Explicitly allow clipboard reading to save it."
                    .to_string(),
            );
        }
        remaining = &after[end + 2..];
    }
    if remaining.contains("}}") {
        return Err("Snippet variable has closing braces without an opening pair.".to_string());
    }
    Ok(())
}

pub(crate) fn commands_from_knowledge(entries: Vec<KnowledgeEntry>) -> Vec<ResolvedVoiceCommand> {
    let mut commands = Vec::<ResolvedVoiceCommand>::new();
    for entry in entries {
        let Some(metadata) = entry.voice_command else {
            continue;
        };
        let (phrase, content) = match entry.payload {
            KnowledgePayload::ReplacementRule {
                source,
                replacement,
            } if metadata.command_type == VoiceCommandKind::TextReplacement => {
                (source, replacement)
            }
            KnowledgePayload::Snippet { trigger, body }
                if metadata.command_type == VoiceCommandKind::Snippet =>
            {
                (trigger, body)
            }
            _ => continue,
        };
        let app_scoped = matches!(entry.scope, KnowledgeScope::App { .. });
        commands.push(ResolvedVoiceCommand {
            id: entry.id,
            phrase,
            command_type: metadata.command_type,
            content,
            allow_clipboard_read: metadata.allow_clipboard_read,
            app_scoped,
        });
    }
    let app_phrases = commands
        .iter()
        .filter(|command| command.app_scoped)
        .map(|command| crate::knowledge_store::normalize_key(&command.phrase))
        .collect::<HashSet<_>>();
    commands
        .into_iter()
        .filter(|command| {
            command.app_scoped
                || !app_phrases.contains(&crate::knowledge_store::normalize_key(&command.phrase))
        })
        .collect()
}

/// Apply the default voice-command map to `text`.
///
/// When `enabled` is false the text is returned unchanged. Matching is
/// case-insensitive and word-boundary aware (a command only fires when it sits
/// between non-alphanumeric boundaries), so "newline" or "comma-separated" are
/// left alone.
///
/// Supported commands:
/// - `new line` -> `\n`
/// - `new paragraph` -> `\n\n`
/// - `scratch that` -> delete the previous sentence
/// - `open paren` / `close paren` -> `(` / `)`
/// - `period` / `comma` / `question mark` -> punctuation attached to the prior word
pub fn apply_voice_commands(text: &str, enabled: bool) -> String {
    if !enabled {
        return text.to_string();
    }

    // Multi-word commands are matched before single-word ones so "new paragraph"
    // wins over "new" + "paragraph" and "question mark" isn't split.
    //
    // Each command carries a kind that controls how surrounding whitespace is
    // handled when it's spliced into the output.
    const COMMANDS: &[(&str, Command)] = &[
        ("new paragraph", Command::Replace("\n\n")),
        ("new line", Command::Replace("\n")),
        ("scratch that", Command::ScratchThat),
        ("open paren", Command::OpenBracket("(")),
        ("close paren", Command::CloseBracket(")")),
        ("question mark", Command::Punctuation("?")),
        ("period", Command::Punctuation(".")),
        ("comma", Command::Punctuation(",")),
    ];

    let lower = text.to_lowercase();
    let chars: Vec<char> = text.chars().collect();
    let lower_chars: Vec<char> = lower.chars().collect();

    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let mut matched = false;
        for (phrase, command) in COMMANDS {
            let phrase_chars: Vec<char> = phrase.chars().collect();
            if matches_at(&lower_chars, i, &phrase_chars) {
                command.apply(&mut out);
                i += phrase_chars.len();
                // Command kinds that splice tightly against the following word
                // (Replace, OpenBracket) must swallow the single inline space
                // that separated the command phrase from the next word, e.g.
                // "hello new line world" -> "hello\nworld" and
                // "open paren x" -> "(x". Punctuation/CloseBracket attach to the
                // prior word and must leave that space so the next word doesn't
                // collide ("one comma two" stays "one, two").
                if command.splices_tightly() && i < chars.len() && chars[i] == ' ' {
                    i += 1;
                }
                matched = true;
                break;
            }
        }
        if !matched {
            out.push(chars[i]);
            i += 1;
        }
    }

    out
}

/// Apply the built-in command map, then any user-defined `custom` commands.
///
/// Custom commands are literal `phrase -> replacement` substitutions, matched
/// case-insensitively on word boundaries (same matcher as the built-ins). They run
/// *after* the built-ins, so a user can extend the vocabulary without breaking the
/// defaults. Blank phrases are ignored. When `enabled` is false the text is
/// returned unchanged.
#[cfg(test)]
pub fn apply_voice_commands_with_custom(
    text: &str,
    enabled: bool,
    custom: &[(String, String)],
) -> String {
    if !enabled {
        return text.to_string();
    }
    let built = apply_voice_commands(text, true);
    if custom.is_empty() {
        return built;
    }
    let mut out = built;
    for (phrase, replacement) in custom {
        let phrase = phrase.trim();
        if phrase.is_empty() {
            continue;
        }
        out = replace_phrase(&out, phrase, replacement);
    }
    out
}

pub(crate) fn apply_voice_commands_with_resolved(
    text: &str,
    enabled: bool,
    commands: &[ResolvedVoiceCommand],
    runtime: &dyn VoiceCommandRuntime,
) -> VoiceCommandApplication {
    if !enabled {
        return VoiceCommandApplication {
            text: text.to_string(),
            matched: false,
            clipboard_required: false,
            clipboard_read: false,
        };
    }
    let mut out = apply_voice_commands(text, true);
    let mut matched = out != text;
    let mut clipboard_required = false;
    let mut clipboard_read = false;
    let now = runtime.now();
    for command in commands {
        if !contains_phrase(&out, &command.phrase) {
            continue;
        }
        matched = true;
        let replacement = match command.command_type {
            VoiceCommandKind::TextReplacement => Ok(command.content.clone()),
            VoiceCommandKind::Snippet => {
                let needs_clipboard = command.content.contains("{{clipboard}}");
                clipboard_required |= needs_clipboard;
                expand_snippet(command, &now, runtime).map(|(expanded, read)| {
                    clipboard_read |= read;
                    expanded
                })
            }
        };
        match replacement {
            Ok(replacement) => {
                out = replace_phrase(&out, &command.phrase, &replacement);
            }
            Err(_) => {
                tracing::warn!(
                    target: "pipeline",
                    command_type = ?command.command_type,
                    clipboard_required,
                    "voice command expansion skipped"
                );
            }
        }
    }
    VoiceCommandApplication {
        text: out,
        matched,
        clipboard_required,
        clipboard_read,
    }
}

fn expand_snippet(
    command: &ResolvedVoiceCommand,
    now: &DateTime<FixedOffset>,
    runtime: &dyn VoiceCommandRuntime,
) -> Result<(String, bool), String> {
    validate_snippet_template(&command.content, command.allow_clipboard_read)?;
    let mut expanded = command
        .content
        .replace("{{date}}", &now.format("%Y-%m-%d").to_string())
        .replace("{{time}}", &now.format("%H:%M").to_string());
    let mut clipboard_read = false;
    if expanded.contains("{{clipboard}}") {
        if !command.allow_clipboard_read {
            return Err("Clipboard reading is not allowed for this command.".to_string());
        }
        let clipboard = runtime.clipboard_text()?;
        if clipboard.chars().count() > MAX_EXPANDED_SNIPPET_CHARS {
            return Err("Clipboard text is too large for a snippet.".to_string());
        }
        expanded = expanded.replace("{{clipboard}}", &clipboard);
        clipboard_read = true;
    }
    if expanded.chars().count() > MAX_EXPANDED_SNIPPET_CHARS {
        return Err("Expanded snippet exceeds the 65,536-character limit.".to_string());
    }
    Ok((expanded, clipboard_read))
}

fn contains_phrase(text: &str, phrase: &str) -> bool {
    let lower = text.to_lowercase();
    let chars: Vec<char> = text.chars().collect();
    let lower_chars: Vec<char> = lower.chars().collect();
    let phrase_chars: Vec<char> = phrase.to_lowercase().chars().collect();
    if phrase_chars.is_empty() || lower_chars.len() != chars.len() {
        return false;
    }
    (0..chars.len()).any(|index| matches_at(&lower_chars, index, &phrase_chars))
}

pub(crate) fn preview_voice_command(
    draft: KnowledgeDraft,
    text: &str,
    read_clipboard: bool,
) -> Result<VoiceCommandApplication, String> {
    let metadata = draft
        .voice_command
        .ok_or_else(|| "Preview requires a typed Voice Command.".to_string())?;
    let (phrase, content) = match draft.payload {
        KnowledgePayload::ReplacementRule {
            source,
            replacement,
        } if metadata.command_type == VoiceCommandKind::TextReplacement => (source, replacement),
        KnowledgePayload::Snippet { trigger, body }
            if metadata.command_type == VoiceCommandKind::Snippet =>
        {
            validate_snippet_template(&body, metadata.allow_clipboard_read)?;
            (trigger, body)
        }
        _ => {
            return Err(
                "Voice command type does not match its stored knowledge payload.".to_string(),
            );
        }
    };
    let command = ResolvedVoiceCommand {
        id: draft.id.unwrap_or_else(|| "preview".to_string()),
        phrase,
        command_type: metadata.command_type,
        content,
        allow_clipboard_read: metadata.allow_clipboard_read,
        app_scoped: matches!(draft.scope, KnowledgeScope::App { .. }),
    };
    if is_builtin_phrase(&crate::knowledge_store::normalize_key(&command.phrase)) {
        return Err("That phrase is reserved by a built-in Voice Command.".to_string());
    }
    let runtime = PreviewVoiceCommandRuntime { read_clipboard };
    Ok(apply_voice_commands_with_resolved(
        text,
        true,
        &[command],
        &runtime,
    ))
}

struct PreviewVoiceCommandRuntime {
    read_clipboard: bool,
}

impl VoiceCommandRuntime for PreviewVoiceCommandRuntime {
    fn now(&self) -> DateTime<FixedOffset> {
        Local::now().fixed_offset()
    }

    fn clipboard_text(&self) -> Result<String, String> {
        if self.read_clipboard {
            crate::injector::read_clipboard_text()
        } else {
            Err("Clipboard preview was not explicitly requested.".to_string())
        }
    }
}

/// Replace every word-boundary, case-insensitive occurrence of `phrase` in `text`
/// with `replacement`. Mirrors the built-in matcher's lowercase-parallel scan.
fn replace_phrase(text: &str, phrase: &str, replacement: &str) -> String {
    let lower = text.to_lowercase();
    let chars: Vec<char> = text.chars().collect();
    let lower_chars: Vec<char> = lower.chars().collect();
    let phrase_chars: Vec<char> = phrase.to_lowercase().chars().collect();
    // Lowercasing can (rarely, for some Unicode) change length; bail to the
    // original in that case rather than risk an index mismatch.
    if phrase_chars.is_empty() || lower_chars.len() != chars.len() {
        return text.to_string();
    }

    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if matches_at(&lower_chars, i, &phrase_chars) {
            out.push_str(replacement);
            i += phrase_chars.len();
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// How a matched command rewrites the output buffer.
enum Command {
    /// Insert literal text (e.g. newline) verbatim.
    Replace(&'static str),
    /// Attach punctuation to the prior word: trim trailing space, then append.
    Punctuation(&'static str),
    /// Open bracket: ensure a leading space, append, then suppress the next space.
    OpenBracket(&'static str),
    /// Close bracket: attach to the prior word (trim trailing space), then append.
    CloseBracket(&'static str),
    /// Delete the previous sentence from the output buffer.
    ScratchThat,
}

impl Command {
    /// True when the command attaches directly to the *following* word, so the
    /// single inline space that separated the command phrase from that word
    /// should be consumed. `Replace` (newline) and `OpenBracket` both lead into
    /// the next word with no space; `Punctuation` and `CloseBracket` attach to
    /// the prior word and keep the space before the next one.
    fn splices_tightly(&self) -> bool {
        matches!(self, Command::Replace(_) | Command::OpenBracket(_))
    }

    fn apply(&self, out: &mut String) {
        match self {
            Command::Replace(s) => {
                // Trim a space we may have just emitted before the command word,
                // so "hello new line" -> "hello\n" rather than "hello \n".
                trim_trailing_inline_space(out);
                out.push_str(s);
            }
            Command::Punctuation(p) | Command::CloseBracket(p) => {
                trim_trailing_inline_space(out);
                out.push_str(p);
            }
            Command::OpenBracket(b) => {
                trim_trailing_inline_space(out);
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push(' ');
                }
                out.push_str(b);
            }
            Command::ScratchThat => {
                trim_trailing_inline_space(out);
                delete_previous_sentence(out);
            }
        }
    }
}

/// True if `phrase` occurs in `haystack` starting at `start`, bounded by
/// non-alphanumeric characters on both sides (word-boundary match).
fn matches_at(haystack: &[char], start: usize, phrase: &[char]) -> bool {
    if start + phrase.len() > haystack.len() {
        return false;
    }
    // Left boundary: start of input or a non-alphanumeric char before it.
    if start > 0 && haystack[start - 1].is_alphanumeric() {
        return false;
    }
    for (k, &pc) in phrase.iter().enumerate() {
        if haystack[start + k] != pc {
            return false;
        }
    }
    // Right boundary: end of input or a non-alphanumeric char after it.
    let after = start + phrase.len();
    if after < haystack.len() && haystack[after].is_alphanumeric() {
        return false;
    }
    true
}

/// Remove a single trailing space (but not a newline) from the buffer so
/// punctuation/brackets attach to the prior word.
fn trim_trailing_inline_space(out: &mut String) {
    if out.ends_with(' ') {
        out.pop();
    }
}

/// Delete the previous sentence from `out`, where a sentence is delimited by
/// `.`, `!`, `?`, or a newline. Leaves the delimiter (and any text before it)
/// intact, trimming trailing whitespace so the next word flows on cleanly.
fn delete_previous_sentence(out: &mut String) {
    // Drop trailing whitespace first so we look at real content.
    while out.ends_with(|c: char| c.is_whitespace()) {
        out.pop();
    }
    // Find the boundary of the prior sentence: the last sentence-ending
    // delimiter or newline still in the buffer.
    let boundary = out
        .char_indices()
        .rev()
        .find(|(_, c)| matches!(c, '.' | '!' | '?' | '\n'))
        .map(|(idx, c)| idx + c.len_utf8());
    match boundary {
        Some(b) => out.truncate(b),
        None => out.clear(),
    }
    // Trim any whitespace left between the delimiter and the next word.
    while out.ends_with(' ') {
        out.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::cell::Cell;

    struct FixedRuntime {
        now: DateTime<FixedOffset>,
        clipboard: Result<String, String>,
        reads: Cell<u32>,
    }

    impl VoiceCommandRuntime for FixedRuntime {
        fn now(&self) -> DateTime<FixedOffset> {
            self.now
        }

        fn clipboard_text(&self) -> Result<String, String> {
            self.reads.set(self.reads.get() + 1);
            self.clipboard.clone()
        }
    }

    fn fixed_runtime(clipboard: Result<&str, &str>) -> FixedRuntime {
        let zone = FixedOffset::west_opt(5 * 60 * 60).unwrap();
        FixedRuntime {
            now: zone.with_ymd_and_hms(2026, 7, 20, 9, 7, 0).unwrap(),
            clipboard: clipboard.map(str::to_string).map_err(str::to_string),
            reads: Cell::new(0),
        }
    }

    fn resolved(
        phrase: &str,
        command_type: VoiceCommandKind,
        content: &str,
        allow_clipboard_read: bool,
    ) -> ResolvedVoiceCommand {
        ResolvedVoiceCommand {
            id: phrase.to_string(),
            phrase: phrase.to_string(),
            command_type,
            content: content.to_string(),
            allow_clipboard_read,
            app_scoped: false,
        }
    }

    #[test]
    fn disabled_returns_text_unchanged() {
        let input = "say new line period now";
        assert_eq!(apply_voice_commands(input, false), input);
    }

    #[test]
    fn new_line_inserts_newline() {
        assert_eq!(
            apply_voice_commands("hello new line world", true),
            "hello\nworld"
        );
    }

    #[test]
    fn new_paragraph_inserts_double_newline() {
        assert_eq!(
            apply_voice_commands("first new paragraph second", true),
            "first\n\nsecond"
        );
    }

    #[test]
    fn new_paragraph_wins_over_new_line() {
        // "new paragraph" must match before the "new line"-style prefix logic.
        let out = apply_voice_commands("a new paragraph b", true);
        assert_eq!(out, "a\n\nb");
    }

    #[test]
    fn period_attaches_to_prior_word() {
        assert_eq!(apply_voice_commands("done period", true), "done.");
    }

    #[test]
    fn comma_attaches_to_prior_word() {
        assert_eq!(apply_voice_commands("one comma two", true), "one, two");
    }

    #[test]
    fn question_mark_attaches_to_prior_word() {
        assert_eq!(
            apply_voice_commands("really question mark", true),
            "really?"
        );
    }

    #[test]
    fn open_and_close_paren() {
        assert_eq!(
            apply_voice_commands("call open paren x close paren", true),
            "call (x)"
        );
    }

    #[test]
    fn case_insensitive_matching() {
        assert_eq!(
            apply_voice_commands("hello New Line world", true),
            "hello\nworld"
        );
        assert_eq!(apply_voice_commands("done PERIOD", true), "done.");
    }

    #[test]
    fn word_boundary_avoids_false_positives() {
        // "newline" is one word — must not be split.
        assert_eq!(apply_voice_commands("newline", true), "newline");
        // "periodic" contains "period" but isn't the command.
        assert_eq!(
            apply_voice_commands("periodic table", true),
            "periodic table"
        );
        // "commatesh" likewise.
        assert_eq!(apply_voice_commands("commando", true), "commando");
    }

    #[test]
    fn scratch_that_deletes_previous_sentence() {
        assert_eq!(
            apply_voice_commands("First sentence. Second sentence scratch that", true),
            "First sentence."
        );
    }

    #[test]
    fn scratch_that_with_no_prior_delimiter_clears_buffer() {
        assert_eq!(apply_voice_commands("just one line scratch that", true), "");
    }

    #[test]
    fn scratch_that_stops_at_newline() {
        let out = apply_voice_commands("line one new line line two scratch that", true);
        assert_eq!(out, "line one\n");
    }

    #[test]
    fn empty_input() {
        assert_eq!(apply_voice_commands("", true), "");
    }

    #[test]
    fn no_commands_passes_through() {
        let input = "just a normal sentence with words";
        assert_eq!(apply_voice_commands(input, true), input);
    }

    #[test]
    fn multiple_commands_in_sequence() {
        assert_eq!(
            apply_voice_commands("hello comma world period new line bye", true),
            "hello, world.\nbye"
        );
    }

    #[test]
    fn command_at_start_of_input() {
        // "period" at the very start has no prior word; it just emits the mark.
        assert_eq!(apply_voice_commands("period", true), ".");
    }

    fn pairs(p: &[(&str, &str)]) -> Vec<(String, String)> {
        p.iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    #[test]
    fn custom_disabled_returns_unchanged() {
        let custom = pairs(&[("my email", "me@example.com")]);
        assert_eq!(
            apply_voice_commands_with_custom("my email", false, &custom),
            "my email"
        );
    }

    #[test]
    fn custom_literal_replacement() {
        let custom = pairs(&[("my email", "me@example.com")]);
        assert_eq!(
            apply_voice_commands_with_custom("send to my email please", true, &custom),
            "send to me@example.com please"
        );
    }

    #[test]
    fn custom_is_case_insensitive_and_word_bounded() {
        let custom = pairs(&[("sig", "Best, Alex")]);
        // "sig" matches as a word, but not inside "signal".
        assert_eq!(
            apply_voice_commands_with_custom("SIG and signal", true, &custom),
            "Best, Alex and signal"
        );
    }

    #[test]
    fn custom_runs_after_builtins() {
        // Built-in "new line" fires first, then the custom replacement.
        let custom = pairs(&[("ticket", "JIRA-123")]);
        assert_eq!(
            apply_voice_commands_with_custom("ticket new line done", true, &custom),
            "JIRA-123\ndone"
        );
    }

    #[test]
    fn custom_blank_phrase_ignored() {
        let custom = pairs(&[("   ", "x")]);
        assert_eq!(
            apply_voice_commands_with_custom("hello world", true, &custom),
            "hello world"
        );
    }

    #[test]
    fn custom_empty_list_matches_builtin_only() {
        assert_eq!(
            apply_voice_commands_with_custom("done period", true, &[]),
            "done."
        );
    }

    #[test]
    fn multiline_snippet_renders_date_and_time_from_one_fixed_instant() {
        let runtime = fixed_runtime(Ok("unused"));
        let command = resolved(
            "insert standup",
            VoiceCommandKind::Snippet,
            "Yesterday:\n- done\nToday {{date}} {{time}}:\n- ship",
            false,
        );
        let applied =
            apply_voice_commands_with_resolved("insert standup", true, &[command], &runtime);
        assert_eq!(
            applied.text,
            "Yesterday:\n- done\nToday 2026-07-20 09:07:\n- ship"
        );
        assert!(applied.matched);
        assert_eq!(runtime.reads.get(), 0);
    }

    #[test]
    fn clipboard_is_read_only_after_a_permitted_phrase_matches() {
        let runtime = fixed_runtime(Ok("alpha\nbeta"));
        let command = resolved(
            "paste note",
            VoiceCommandKind::Snippet,
            "Note:\n{{clipboard}}",
            true,
        );
        let missed = apply_voice_commands_with_resolved(
            "ordinary prose",
            true,
            &[command.clone()],
            &runtime,
        );
        assert_eq!(missed.text, "ordinary prose");
        assert_eq!(runtime.reads.get(), 0);

        let applied = apply_voice_commands_with_resolved("paste note", true, &[command], &runtime);
        assert_eq!(applied.text, "Note:\nalpha\nbeta");
        assert!(applied.clipboard_required);
        assert!(applied.clipboard_read);
        assert_eq!(runtime.reads.get(), 1);
    }

    #[test]
    fn unavailable_clipboard_fails_closed_to_the_spoken_phrase() {
        let runtime = fixed_runtime(Err("unavailable"));
        let command = resolved(
            "paste note",
            VoiceCommandKind::Snippet,
            "{{clipboard}}",
            true,
        );
        let applied = apply_voice_commands_with_resolved(
            "before paste note after",
            true,
            &[command],
            &runtime,
        );
        assert_eq!(applied.text, "before paste note after");
        assert!(applied.matched);
        assert!(!applied.clipboard_read);
    }

    #[test]
    fn text_replacements_keep_variable_syntax_literal_and_allow_empty_output() {
        let runtime = fixed_runtime(Ok("secret"));
        let literal = resolved(
            "today token",
            VoiceCommandKind::TextReplacement,
            "{{date}}",
            false,
        );
        let empty = resolved("remove me", VoiceCommandKind::TextReplacement, "", false);
        assert_eq!(
            apply_voice_commands_with_resolved("today token", true, &[literal], &runtime).text,
            "{{date}}"
        );
        assert_eq!(
            apply_voice_commands_with_resolved("remove me", true, &[empty], &runtime).text,
            ""
        );
        assert_eq!(runtime.reads.get(), 0);
    }

    #[test]
    fn snippet_validation_rejects_unknown_or_implicit_clipboard_variables() {
        assert!(validate_snippet_template("{{weather}}", false).is_err());
        assert!(validate_snippet_template("{{clipboard}}", false).is_err());
        assert!(validate_snippet_template("{{clipboard}}", true).is_ok());
        assert!(validate_snippet_template("{{date", false).is_err());
    }
}
