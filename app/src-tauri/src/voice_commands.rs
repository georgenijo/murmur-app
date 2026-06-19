//! Post-transcription voice command transforms.
//!
//! Spoken command tokens (e.g. "new line", "scratch that") are rewritten into
//! their effect before the text reaches the clipboard/auto-paste. This runs as
//! a pure string transform after transcription and is gated behind the
//! `voiceCommandsEnabled` setting.

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

    #[test]
    fn disabled_returns_text_unchanged() {
        let input = "say new line period now";
        assert_eq!(apply_voice_commands(input, false), input);
    }

    #[test]
    fn new_line_inserts_newline() {
        assert_eq!(apply_voice_commands("hello new line world", true), "hello\nworld");
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
        assert_eq!(apply_voice_commands("really question mark", true), "really?");
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
        assert_eq!(apply_voice_commands("hello New Line world", true), "hello\nworld");
        assert_eq!(apply_voice_commands("done PERIOD", true), "done.");
    }

    #[test]
    fn word_boundary_avoids_false_positives() {
        // "newline" is one word — must not be split.
        assert_eq!(apply_voice_commands("newline", true), "newline");
        // "periodic" contains "period" but isn't the command.
        assert_eq!(apply_voice_commands("periodic table", true), "periodic table");
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
}
