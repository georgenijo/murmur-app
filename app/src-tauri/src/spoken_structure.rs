//! Deterministic rendering for explicitly spoken punctuation and structure.
//!
//! This module is the single implementation of token matching, whitespace
//! attachment, ASR-punctuation arbitration, paired delimiters, and the
//! sentence-boundary-aware `scratch that` action. Higher-level prose grammars
//! (email, URL, enumeration, and backtracking) remain in `smart_formatting`.

use base64::Engine;

const MAX_INPUT_BYTES: usize = 16 * 1024;
const MAX_PAIRED_CONTENT_CHARS: usize = 240;
const PROTECTED_PREFIX: &str = "\u{e000}murmur-structure:";
const PROTECTED_SUFFIX: char = '\u{e001}';

pub(crate) const BASIC_COMMAND_PHRASES: &[&str] = &[
    "new paragraph",
    "new line",
    "scratch that",
    "open paren",
    "close paren",
    "question mark",
    "period",
    "comma",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SpokenStructurePolicy {
    Off,
    Basic,
    Extended,
    Union,
}

impl SpokenStructurePolicy {
    pub(crate) fn resolve(voice_commands_enabled: bool, smart_formatting_enabled: bool) -> Self {
        match (voice_commands_enabled, smart_formatting_enabled) {
            (false, false) => Self::Off,
            (true, false) => Self::Basic,
            (false, true) => Self::Extended,
            (true, true) => Self::Union,
        }
    }

    pub(crate) fn is_enabled(self) -> bool {
        self != Self::Off
    }

    fn includes_extended(self) -> bool {
        matches!(self, Self::Extended | Self::Union)
    }
}

#[derive(Clone, Copy)]
enum Marker {
    Break(&'static str),
    Punctuation(&'static str),
    Infix(&'static str),
    Tight(&'static str),
    OpenBracket(&'static str),
    CloseBracket(&'static str),
    ScratchThat,
}

const BASIC_MARKERS: &[(&str, Marker)] = &[
    ("new paragraph", Marker::Break("\n\n")),
    ("new line", Marker::Break("\n")),
    ("scratch that", Marker::ScratchThat),
    ("open paren", Marker::OpenBracket("(")),
    ("close paren", Marker::CloseBracket(")")),
    ("question mark", Marker::Punctuation("?")),
    ("period", Marker::Punctuation(".")),
    ("comma", Marker::Punctuation(",")),
];

const EXTENDED_MARKERS: &[(&str, Marker)] = &[
    ("new paragraph", Marker::Break("\n\n")),
    ("new line", Marker::Break("\n")),
    ("exclamation mark", Marker::Punctuation("!")),
    ("question mark", Marker::Punctuation("?")),
    ("semicolon", Marker::Punctuation(";")),
    ("colon", Marker::Punctuation(":")),
    ("period", Marker::Punctuation(".")),
    ("comma", Marker::Punctuation(",")),
    ("em dash", Marker::Infix("—")),
    ("en dash", Marker::Infix("–")),
    ("at sign", Marker::Infix("@")),
    ("hash sign", Marker::Infix("#")),
    ("number sign", Marker::Infix("#")),
    ("percent sign", Marker::Infix("%")),
    ("plus sign", Marker::Infix("+")),
    ("equals sign", Marker::Infix("=")),
    ("ampersand", Marker::Infix("&")),
    ("forward slash", Marker::Tight("/")),
    ("back slash", Marker::Tight("\\")),
    ("backslash", Marker::Tight("\\")),
    ("slash", Marker::Tight("/")),
    ("hyphen", Marker::Tight("-")),
];

const UNION_ONLY_MARKERS: &[(&str, Marker)] = &[
    ("scratch that", Marker::ScratchThat),
    ("open paren", Marker::OpenBracket("(")),
    ("close paren", Marker::CloseBracket(")")),
];

pub(crate) fn apply_spoken_structure(input: &str, policy: SpokenStructurePolicy) -> String {
    if !policy.is_enabled() || input.trim().is_empty() {
        return input.to_string();
    }
    if input.len() > MAX_INPUT_BYTES {
        return restore_protected_literals(input);
    }

    let paired = if policy.includes_extended() {
        replace_paired_markers(input, policy)
    } else {
        input.to_string()
    };
    replace_markers(&paired, policy)
}

/// Make command-generated text opaque to later spoken-structure scanning.
///
/// The placeholder exists only inside one in-memory pipeline run and is
/// restored by `apply_spoken_structure`, including its over-limit fail-closed
/// path. This prevents a snippet containing literal words such as `period` or
/// `new line` from being interpreted as fresh speech.
pub(crate) fn protect_literal_output(input: &str) -> String {
    if input.is_empty() || !contains_structural_phrase(input) {
        return input.to_string();
    }
    let encoded = base64::engine::general_purpose::STANDARD_NO_PAD.encode(input.as_bytes());
    format!("{PROTECTED_PREFIX}{encoded}{PROTECTED_SUFFIX}")
}

pub(crate) fn restore_literal_output(input: &str) -> String {
    restore_protected_literals(input)
}

fn contains_structural_phrase(input: &str) -> bool {
    const PAIRED_PHRASES: &[&str] = &[
        "open double quote",
        "close double quote",
        "open single quote",
        "close single quote",
        "open quote",
        "close quote",
        "open parenthesis",
        "close parenthesis",
    ];
    let lower = input.to_ascii_lowercase();
    BASIC_MARKERS
        .iter()
        .chain(EXTENDED_MARKERS)
        .map(|(phrase, _)| *phrase)
        .chain(PAIRED_PHRASES.iter().copied())
        .any(|phrase| find_bounded_phrase(&lower, phrase, 0).is_some())
}

fn replace_paired_markers(input: &str, policy: SpokenStructurePolicy) -> String {
    let quote_pairs = [
        ("open double quote", "close double quote", "\"", "\""),
        ("open single quote", "close single quote", "'", "'"),
        ("open quote", "close quote", "\"", "\""),
    ];
    let output = quote_pairs
        .iter()
        .fold(input.to_string(), |text, (open, close, left, right)| {
            replace_bounded_pair(&text, open, close, left, right)
        });

    // Basic Voice Commands historically allowed unpaired parentheses. Preserve
    // that behavior in Union mode; Extended-only mode remains fail-closed.
    if policy == SpokenStructurePolicy::Extended {
        [
            ("open parenthesis", "close parenthesis", "(", ")"),
            ("open paren", "close paren", "(", ")"),
        ]
        .iter()
        .fold(output, |text, (open, close, left, right)| {
            replace_bounded_pair(&text, open, close, left, right)
        })
    } else {
        output
    }
}

fn replace_bounded_pair(input: &str, open: &str, close: &str, left: &str, right: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let Some(open_start) = find_bounded_phrase(&lower, open, 0) else {
        return input.to_string();
    };
    let content_start = open_start + open.len();
    let Some(close_start) = find_bounded_phrase(&lower, close, content_start) else {
        return input.to_string();
    };
    let content = input[content_start..close_start].trim();
    if content.is_empty()
        || content.len() > MAX_PAIRED_CONTENT_CHARS
        || content.contains(['\n', '\r'])
    {
        return input.to_string();
    }
    let mut output = String::with_capacity(input.len());
    output.push_str(input[..open_start].trim_end());
    if !output.is_empty() && !output.ends_with([' ', '\n']) {
        output.push(' ');
    }
    output.push_str(left);
    output.push_str(content);
    output.push_str(right);
    let suffix = input[close_start + close.len()..].trim_start();
    if !suffix.is_empty() {
        output.push(' ');
        output.push_str(suffix);
    }
    output
}

fn replace_markers(input: &str, policy: SpokenStructurePolicy) -> String {
    let lower = input.to_ascii_lowercase();
    let mut output = String::with_capacity(input.len());
    let mut index = 0;
    let mut changed = false;
    let mut used_extended_semantics = false;

    while index < input.len() {
        if let Some((length, literal)) = protected_literal_at(input, index) {
            output.push_str(&literal);
            index += length;
            continue;
        }
        let Some((_, character)) = input[index..].char_indices().next() else {
            break;
        };
        let matched = marker_at(&lower, index, policy);
        if let Some((length, marker, extended_semantics)) = matched {
            changed = true;
            used_extended_semantics |= extended_semantics;
            remove_auto_punctuation_before_marker(
                &mut output,
                &input[..index],
                marker,
                extended_semantics,
            );
            apply_marker(&mut output, marker, extended_semantics);
            index += length;
            index += auto_punctuation_suffix_len(&input[index..], marker);
            if consumes_following_space(marker, extended_semantics)
                && input[index..].starts_with(' ')
            {
                index += 1;
            }
        } else {
            output.push(character);
            index += character.len_utf8();
        }
    }

    if changed && policy == SpokenStructurePolicy::Extended {
        output.trim().to_string()
    } else if changed && used_extended_semantics {
        output.trim_matches(' ').to_string()
    } else {
        output
    }
}

fn protected_literal_at(input: &str, index: usize) -> Option<(usize, String)> {
    let suffix = input.get(index..)?;
    let encoded = suffix.strip_prefix(PROTECTED_PREFIX)?;
    let encoded_end = encoded.find(PROTECTED_SUFFIX)?;
    let decoded = base64::engine::general_purpose::STANDARD_NO_PAD
        .decode(&encoded[..encoded_end])
        .ok()?;
    let literal = String::from_utf8(decoded).ok()?;
    let length = PROTECTED_PREFIX.len() + encoded_end + PROTECTED_SUFFIX.len_utf8();
    Some((length, literal))
}

fn restore_protected_literals(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut index = 0;
    while index < input.len() {
        if let Some((length, literal)) = protected_literal_at(input, index) {
            output.push_str(&literal);
            index += length;
            continue;
        }
        let Some(character) = input[index..].chars().next() else {
            break;
        };
        output.push(character);
        index += character.len_utf8();
    }
    output
}

fn marker_at(
    lower: &str,
    index: usize,
    policy: SpokenStructurePolicy,
) -> Option<(usize, Marker, bool)> {
    let tables: &[(&[(&str, Marker)], bool)] = match policy {
        SpokenStructurePolicy::Off => &[],
        SpokenStructurePolicy::Basic => &[(BASIC_MARKERS, false)],
        SpokenStructurePolicy::Extended => &[(EXTENDED_MARKERS, true)],
        SpokenStructurePolicy::Union => &[
            (EXTENDED_MARKERS, true),
            (UNION_ONLY_MARKERS, false),
        ],
    };

    for (markers, extended_semantics) in tables {
        for (phrase, marker) in *markers {
            if lower[index..].starts_with(phrase)
                && is_phrase_boundary(lower, index, phrase.len())
                && marker_allowed(lower, index, phrase)
            {
                return Some((phrase.len(), *marker, *extended_semantics));
            }
        }
    }
    None
}

fn apply_marker(output: &mut String, marker: Marker, extended_semantics: bool) {
    trim_inline_space(output, extended_semantics);
    match marker {
        Marker::Break(value) => {
            if extended_semantics {
                while output.ends_with('\n') && value == "\n\n" {
                    output.pop();
                }
            }
            output.push_str(value);
        }
        Marker::Punctuation(value) => {
            output.push_str(value);
            if extended_semantics {
                // Extended scanning consumes the source space after a marker.
                output.push(' ');
            }
        }
        Marker::Infix(value) => {
            if !output.is_empty() && !output.ends_with([' ', '\n']) {
                output.push(' ');
            }
            output.push_str(value);
            output.push(' ');
        }
        Marker::Tight(value) => output.push_str(value),
        Marker::OpenBracket(value) => {
            if !output.is_empty() && !output.ends_with('\n') {
                output.push(' ');
            }
            output.push_str(value);
        }
        Marker::CloseBracket(value) => output.push_str(value),
        Marker::ScratchThat => delete_previous_sentence(output),
    }
}

fn consumes_following_space(marker: Marker, extended_semantics: bool) -> bool {
    if extended_semantics {
        return true;
    }
    matches!(marker, Marker::Break(_) | Marker::OpenBracket(_))
}

fn trim_inline_space(output: &mut String, extended_semantics: bool) {
    if extended_semantics {
        while output.ends_with(' ') {
            output.pop();
        }
    } else if output.ends_with(' ') {
        output.pop();
    }
}

fn remove_auto_punctuation_before_marker(
    output: &mut String,
    input_prefix: &str,
    marker: Marker,
    extended_semantics: bool,
) {
    let Marker::Punctuation(spoken) = marker else {
        return;
    };
    let Some(spoken) = spoken.chars().next() else {
        return;
    };
    let Some(existing) = input_prefix.trim_end().chars().next_back() else {
        return;
    };
    if existing != spoken && !(is_terminal_punctuation(existing) && is_terminal_punctuation(spoken))
    {
        return;
    }

    trim_inline_space(output, extended_semantics);
    if output.ends_with(existing) {
        output.pop();
    }
}

fn auto_punctuation_suffix_len(suffix: &str, marker: Marker) -> usize {
    let Marker::Punctuation(spoken) = marker else {
        return 0;
    };
    let Some(spoken) = spoken.chars().next() else {
        return 0;
    };
    let Some(existing) = suffix.chars().next() else {
        return 0;
    };
    if existing == spoken
        || (is_terminal_punctuation(existing) && is_terminal_punctuation(spoken))
    {
        existing.len_utf8()
    } else {
        0
    }
}

fn marker_allowed(input_lower: &str, start: usize, phrase: &str) -> bool {
    if !matches!(phrase, "slash" | "forward slash") {
        return true;
    }

    // A failed explicit URL grammar must remain byte-for-byte unchanged.
    let trimmed = input_lower.trim_start();
    if trimmed.starts_with("url ") || trimmed.starts_with("web address ") {
        return false;
    }

    // The final CLI stage owns explicit inline `slash command <name>` cues.
    let after = input_lower[start + phrase.len()..].trim_start();
    !(after.starts_with("command")
        && after["command".len()..]
            .chars()
            .next()
            .is_none_or(|character| !character.is_alphanumeric()))
}

fn delete_previous_sentence(output: &mut String) {
    while output.ends_with(|character: char| character.is_whitespace()) {
        output.pop();
    }
    let boundary = output
        .char_indices()
        .rev()
        .find(|(_, character)| matches!(character, '.' | '!' | '?' | '\n'))
        .map(|(index, character)| index + character.len_utf8());
    match boundary {
        Some(boundary) => output.truncate(boundary),
        None => output.clear(),
    }
    while output.ends_with(' ') {
        output.pop();
    }
}

fn is_terminal_punctuation(character: char) -> bool {
    matches!(character, '.' | '!' | '?')
}

fn find_bounded_phrase(haystack_lower: &str, phrase: &str, from: usize) -> Option<usize> {
    haystack_lower[from..]
        .match_indices(phrase)
        .find_map(|(offset, _)| {
            let start = from + offset;
            is_phrase_boundary(haystack_lower, start, phrase.len()).then_some(start)
        })
}

fn is_phrase_boundary(haystack: &str, start: usize, length: usize) -> bool {
    let before = haystack[..start].chars().next_back();
    let after = haystack[start + length..].chars().next();
    before.is_none_or(|character| !character.is_alphanumeric())
        && after.is_none_or(|character| !character.is_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_resolution_is_explicit() {
        assert_eq!(
            SpokenStructurePolicy::resolve(false, false),
            SpokenStructurePolicy::Off
        );
        assert_eq!(
            SpokenStructurePolicy::resolve(true, false),
            SpokenStructurePolicy::Basic
        );
        assert_eq!(
            SpokenStructurePolicy::resolve(false, true),
            SpokenStructurePolicy::Extended
        );
        assert_eq!(
            SpokenStructurePolicy::resolve(true, true),
            SpokenStructurePolicy::Union
        );
    }

    #[test]
    fn basic_mode_preserves_legacy_voice_command_semantics() {
        assert_eq!(
            apply_spoken_structure(
                "hello comma world period new line bye",
                SpokenStructurePolicy::Basic,
            ),
            "hello, world.\nbye"
        );
        assert_eq!(
            apply_spoken_structure(
                "call open paren x close paren",
                SpokenStructurePolicy::Basic,
            ),
            "call (x)"
        );
        assert_eq!(
            apply_spoken_structure(
                "First sentence. Second sentence scratch that",
                SpokenStructurePolicy::Basic,
            ),
            "First sentence."
        );
    }

    #[test]
    fn extended_mode_formats_symbols_and_requires_paired_delimiters() {
        assert_eq!(
            apply_spoken_structure(
                "Say open quote ship it close quote period new paragraph Thanks exclamation mark",
                SpokenStructurePolicy::Extended,
            ),
            "Say \"ship it\".\n\nThanks!"
        );
        assert_eq!(
            apply_spoken_structure(
                "root forward slash users backslash george",
                SpokenStructurePolicy::Extended,
            ),
            "root/users\\george"
        );
        let unpaired = "Say open quote this stays literal";
        assert_eq!(
            apply_spoken_structure(unpaired, SpokenStructurePolicy::Extended),
            unpaired
        );
    }

    #[test]
    fn url_and_inline_cli_cues_remain_owned_by_their_authoritative_grammars() {
        for input in [
            "URL example dot com slash docs please",
            "web address example dot com forward slash docs please",
            "Use slash command chat",
        ] {
            assert_eq!(
                apply_spoken_structure(input, SpokenStructurePolicy::Extended),
                input
            );
        }
    }

    #[test]
    fn scratch_uses_boundaries_created_in_the_same_pass() {
        assert_eq!(
            apply_spoken_structure(
                "one period two scratch that",
                SpokenStructurePolicy::Union,
            ),
            "one."
        );
        assert_eq!(
            apply_spoken_structure(
                "hello new line world scratch that",
                SpokenStructurePolicy::Union,
            ),
            "hello\n"
        );
    }

    #[test]
    fn explicit_punctuation_wins_over_one_adjacent_asr_mark() {
        for (input, expected) in [
            ("I have one idea. period", "I have one idea."),
            ("Are we ready? question mark", "Are we ready?"),
            ("Are we ready. question mark.", "Are we ready?"),
            ("Count one comma, two comma, three period.", "Count one, two, three."),
            (
                "Really question mark exclamation mark",
                "Really?!",
            ),
        ] {
            assert_eq!(
                apply_spoken_structure(input, SpokenStructurePolicy::Extended),
                expected
            );
        }
    }

    #[test]
    fn arbitration_does_not_cross_newlines_or_consume_more_than_one_mark() {
        assert_eq!(
            apply_spoken_structure(
                "Ready?\nquestion mark",
                SpokenStructurePolicy::Extended,
            ),
            "Ready?\n?"
        );
        assert_eq!(
            apply_spoken_structure(
                "Ready?? question mark",
                SpokenStructurePolicy::Extended,
            ),
            "Ready??"
        );
        assert_eq!(
            apply_spoken_structure(
                "Wait… question mark",
                SpokenStructurePolicy::Extended,
            ),
            "Wait…?"
        );
    }

    #[test]
    fn unicode_case_mapping_cannot_misalign_ascii_phrase_scanning() {
        assert_eq!(
            apply_spoken_structure(
                "İstanbul period",
                SpokenStructurePolicy::Basic,
            ),
            "İstanbul."
        );
    }

    #[test]
    fn over_limit_input_fails_closed() {
        let input = format!("{} period", "a".repeat(MAX_INPUT_BYTES));
        assert_eq!(
            apply_spoken_structure(&input, SpokenStructurePolicy::Union),
            input
        );
    }

    #[test]
    fn formatted_output_is_idempotent() {
        for input in [
            "Really question mark exclamation mark",
            "one period two",
            "root forward slash users",
        ] {
            let once = apply_spoken_structure(input, SpokenStructurePolicy::Extended);
            assert_eq!(
                apply_spoken_structure(&once, SpokenStructurePolicy::Extended),
                once
            );
        }
    }

    #[test]
    fn protected_command_output_is_restored_without_reinterpretation() {
        let protected = protect_literal_output("literal period and new line");
        let input = format!("before {protected} after period");
        assert_eq!(
            apply_spoken_structure(&input, SpokenStructurePolicy::Union),
            "before literal period and new line after."
        );
    }

    #[test]
    fn protected_output_is_restored_on_the_over_limit_fail_closed_path() {
        let protected = protect_literal_output("literal period");
        let input = format!("{} {protected}", "a".repeat(MAX_INPUT_BYTES));
        let expected = format!("{} literal period", "a".repeat(MAX_INPUT_BYTES));
        assert_eq!(
            apply_spoken_structure(&input, SpokenStructurePolicy::Basic),
            expected
        );
    }
}
