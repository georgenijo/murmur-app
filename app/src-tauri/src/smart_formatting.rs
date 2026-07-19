//! Deterministic prose formatting and bounded same-utterance backtracking.
//!
//! Rules in this module require explicit spoken structure. Ambiguous prose is
//! returned unchanged, and no rule reads external context or performs I/O.

const MAX_BACKTRACK_REPLACEMENT_WORDS: usize = 4;
const MAX_BACKTRACK_REPLACEMENT_CHARS: usize = 64;
const MAX_LIST_ITEMS: usize = 10;
const MAX_LIST_ITEM_WORDS: usize = 24;
const MAX_PAIRED_CONTENT_CHARS: usize = 240;
const MAX_SMART_FORMATTING_INPUT_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone)]
struct WordSpan {
    start: usize,
    end: usize,
    lower: String,
}

/// Apply the complete deterministic prose-formatting grammar.
pub(crate) fn format_smart_prose(input: &str) -> String {
    if input.trim().is_empty() || input.len() > MAX_SMART_FORMATTING_INPUT_BYTES {
        return input.to_string();
    }

    let mut output = apply_bounded_backtrack(input);
    output = format_explicit_email(&output);
    output = format_explicit_url(&output);
    output = replace_paired_markers(&output);
    output = replace_spoken_markers(&output);
    format_spoken_enumeration(&output)
}

fn apply_bounded_backtrack(input: &str) -> String {
    const MARKERS: &[(&str, bool)] = &[
        ("actually, make that", false),
        // Without the spoken comma, require a separator before the cue. This
        // keeps ordinary prose such as "I can actually make that change" from
        // being mistaken for a correction.
        ("actually make that", true),
        ("i mean", true),
        ("or rather", true),
        ("rather", true),
    ];

    let lower = input.to_ascii_lowercase();
    let mut selected: Option<(usize, usize, bool)> = None;
    for (marker, requires_punctuation) in MARKERS {
        for (start, _) in lower.match_indices(marker) {
            if !is_phrase_boundary(&lower, start, marker.len()) {
                continue;
            }
            if *requires_punctuation && !has_correction_separator(input, start) {
                continue;
            }
            let end = start + marker.len();
            if selected.is_none_or(|(selected_start, _, _)| start > selected_start) {
                selected = Some((start, end, *requires_punctuation));
            }
        }
    }

    let Some((marker_start, marker_end, _)) = selected else {
        return input.to_string();
    };
    let replacement_raw = input[marker_end..]
        .trim_start_matches(|ch: char| ch.is_whitespace() || matches!(ch, ',' | ':' | '-' | '—'))
        .trim();
    let replacement =
        replacement_raw.trim_end_matches(|ch: char| matches!(ch, '.' | ',' | ';' | '!' | '?'));
    let replacement_words = replacement.split_whitespace().count();
    if replacement.is_empty()
        || replacement.len() > MAX_BACKTRACK_REPLACEMENT_CHARS
        || !(1..=MAX_BACKTRACK_REPLACEMENT_WORDS).contains(&replacement_words)
        || replacement.contains(['\n', '\r'])
        || replacement.to_ascii_lowercase().starts_with("is ")
        || replacement.to_ascii_lowercase().starts_with("that ")
        || replacement.to_ascii_lowercase().starts_with("than ")
    {
        return input.to_string();
    }

    let mut prefix = input[..marker_start].trim_end().to_string();
    trim_correction_separator(&mut prefix);
    let Some((word_start, word_end)) = last_word_range(&prefix) else {
        return input.to_string();
    };
    let abandoned = &prefix[word_start..word_end];
    if abandoned.eq_ignore_ascii_case(replacement) {
        return input.to_string();
    }

    // This intentionally replaces only the final abandoned term. If a
    // multi-word replacement begins with the preceding term, applying it would
    // duplicate that term ("next Friday" -> "next next Monday"). Fail closed
    // instead of guessing at a wider abandoned phrase.
    if replacement_words > 1 {
        let before_abandoned = prefix[..word_start].trim_end();
        if let Some((previous_start, previous_end)) = last_word_range(before_abandoned) {
            let previous = &before_abandoned[previous_start..previous_end];
            let replacement_first = replacement.split_whitespace().next().unwrap_or_default();
            if previous.eq_ignore_ascii_case(replacement_first) {
                return input.to_string();
            }
        }
    }

    prefix.replace_range(word_start..word_end, replacement);
    while prefix.ends_with(|ch: char| ch.is_whitespace()) {
        prefix.pop();
    }
    if !prefix.ends_with(['.', '!', '?']) {
        prefix.push('.');
    }
    prefix
}

fn has_correction_separator(input: &str, marker_start: usize) -> bool {
    let prefix = input[..marker_start].trim_end();
    prefix.ends_with([',', ';', '—'])
        || prefix.ends_with(" -")
        || prefix.to_ascii_lowercase().ends_with("em dash")
}

fn trim_correction_separator(prefix: &mut String) {
    while prefix.ends_with(|ch: char| ch.is_whitespace()) {
        prefix.pop();
    }
    let lower = prefix.to_ascii_lowercase();
    if lower.ends_with("em dash") {
        prefix.truncate(prefix.len() - "em dash".len());
    }
    while prefix.ends_with(|ch: char| ch.is_whitespace() || matches!(ch, ',' | ';' | '-' | '—')) {
        prefix.pop();
    }
}

fn last_word_range(input: &str) -> Option<(usize, usize)> {
    let end = input
        .char_indices()
        .rev()
        .find(|(_, ch)| ch.is_alphanumeric())
        .map(|(index, ch)| index + ch.len_utf8())?;
    let start = input[..end]
        .char_indices()
        .rev()
        .find(|(_, ch)| !ch.is_alphanumeric() && !matches!(ch, '\'' | '-'))
        .map_or(0, |(index, ch)| index + ch.len_utf8());
    Some((start, end))
}

fn format_spoken_enumeration(input: &str) -> String {
    const ORDINALS: &[&str] = &[
        "first", "second", "third", "fourth", "fifth", "sixth", "seventh", "eighth", "ninth",
        "tenth",
    ];

    let words = word_spans(input);
    let Some(first_index) = words.iter().position(|word| word.lower == "first") else {
        return input.to_string();
    };
    let prefix = input[..words[first_index].start].trim();
    if !valid_list_prefix(prefix) {
        return input.to_string();
    }

    let mut markers = vec![(1usize, first_index)];
    let mut expected_rank = 2usize;
    for (index, word) in words.iter().enumerate().skip(first_index + 1) {
        let Some(rank) = ORDINALS
            .iter()
            .position(|ordinal| *ordinal == word.lower)
            .map(|rank| rank + 1)
        else {
            continue;
        };
        if rank != expected_rank || rank > MAX_LIST_ITEMS {
            return input.to_string();
        }
        markers.push((rank, index));
        expected_rank += 1;
    }
    if markers.len() < 2 {
        return input.to_string();
    }

    let mut items = Vec::with_capacity(markers.len());
    for (position, (_, word_index)) in markers.iter().enumerate() {
        let start = words[*word_index].end;
        let end = markers
            .get(position + 1)
            .map_or(input.len(), |(_, next_index)| words[*next_index].start);
        let item = trim_list_item(&input[start..end]);
        let word_count = item.split_whitespace().count();
        if item.is_empty() || !(1..=MAX_LIST_ITEM_WORDS).contains(&word_count) {
            return input.to_string();
        }
        items.push(capitalize_first(item));
    }

    let mut output = String::new();
    if !prefix.is_empty() {
        output.push_str(prefix.trim_end_matches(|ch: char| matches!(ch, ':' | ',' | ';')));
        output.push(':');
        output.push('\n');
    }
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            output.push('\n');
        }
        output.push_str(&(index + 1).to_string());
        output.push_str(". ");
        output.push_str(item);
    }
    output
}

fn valid_list_prefix(prefix: &str) -> bool {
    if prefix.is_empty() {
        return true;
    }
    let normalized = prefix
        .trim_end_matches(|ch: char| matches!(ch, ':' | ',' | ';'))
        .to_ascii_lowercase();
    normalized.ends_with(" are")
        || normalized.ends_with(" include")
        || normalized.ends_with(" includes")
        || normalized.ends_with(" follows")
        || normalized.ends_with(" following")
        || normalized.ends_with(" priorities")
        || normalized.ends_with(" tasks")
        || normalized.ends_with(" steps")
        || normalized.ends_with(" reasons")
        || normalized.ends_with(" items")
        || normalized.ends_with(" goals")
}

fn trim_list_item(item: &str) -> &str {
    item.trim_matches(|ch: char| {
        ch.is_whitespace() || matches!(ch, ',' | ';' | ':' | '.' | '!' | '?' | '-' | '—')
    })
}

fn capitalize_first(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut capitalized = false;
    for ch in input.chars() {
        if !capitalized && ch.is_alphabetic() {
            output.extend(ch.to_uppercase());
            capitalized = true;
        } else {
            output.push(ch);
        }
    }
    output
}

fn format_explicit_email(input: &str) -> String {
    let words = word_spans(input);
    let cue_indices = words
        .iter()
        .enumerate()
        .filter(|(_, word)| word.lower == "email")
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if cue_indices.is_empty() {
        return input.to_string();
    }

    for at_index in (1..words.len()).rev() {
        if words[at_index].lower != "at" || at_index + 3 >= words.len() {
            continue;
        }
        let mut local_start = at_index - 1;
        while local_start >= 2
            && matches!(
                words[local_start - 1].lower.as_str(),
                "dot" | "underscore" | "dash" | "hyphen" | "plus"
            )
        {
            local_start -= 2;
        }
        let domain = &words[at_index + 1..];
        let Some(last_domain_offset) = domain.iter().rposition(|word| word.lower == "dot") else {
            continue;
        };
        if last_domain_offset == 0 || last_domain_offset + 1 >= domain.len() {
            continue;
        }
        let end_index = at_index + 1 + last_domain_offset + 1;
        if end_index - local_start + 1 > 12 {
            continue;
        }
        let Some(cue_index) = cue_indices
            .iter()
            .copied()
            .rev()
            .find(|cue| *cue < local_start)
        else {
            continue;
        };
        if local_start - cue_index > 8
            || input[words[cue_index].end..words[local_start].start]
                .contains(['.', '!', '?', '\n', '\r'])
        {
            continue;
        }
        let Some(address) =
            build_email_address(&words[local_start..=end_index], at_index - local_start)
        else {
            continue;
        };
        let mut output = String::with_capacity(input.len());
        output.push_str(&input[..words[local_start].start]);
        output.push_str(&address);
        output.push_str(&input[words[end_index].end..]);
        return output;
    }
    input.to_string()
}

fn build_email_address(words: &[WordSpan], at_index: usize) -> Option<String> {
    if at_index == 0 || at_index + 3 > words.len() || words[at_index].lower != "at" {
        return None;
    }
    let mut output = String::new();
    for word in &words[..at_index] {
        if !append_address_token(&mut output, &word.lower, true) {
            return None;
        }
    }
    if output.is_empty() {
        return None;
    }
    output.push('@');
    let domain_start = output.len();
    let mut dots = 0usize;
    for word in &words[at_index + 1..] {
        if word.lower == "dot" {
            if output.len() == domain_start || output.ends_with('.') {
                return None;
            }
            output.push('.');
            dots += 1;
        } else if !append_address_token(&mut output, &word.lower, false) {
            return None;
        }
    }
    (dots > 0 && !output.ends_with('.')).then_some(output)
}

fn append_address_token(output: &mut String, token: &str, local: bool) -> bool {
    let separator = match token {
        "dot" if local => Some('.'),
        "underscore" if local => Some('_'),
        "dash" | "hyphen" => Some('-'),
        "plus" if local => Some('+'),
        _ => None,
    };
    if let Some(separator) = separator {
        if output.is_empty() || output.ends_with(['.', '_', '-', '+', '@']) {
            return false;
        }
        output.push(separator);
        return true;
    }
    if token.is_empty() || !token.chars().all(|ch| ch.is_ascii_alphanumeric()) {
        return false;
    }
    output.push_str(token);
    true
}

fn format_explicit_url(input: &str) -> String {
    let trimmed = input.trim();
    let lower = trimmed.to_ascii_lowercase();
    let body = if lower.starts_with("url ") {
        &trimmed["url ".len()..]
    } else if lower.starts_with("web address ") {
        &trimmed["web address ".len()..]
    } else {
        return input.to_string();
    };
    let body = body.trim_end_matches(|ch: char| matches!(ch, '.' | ',' | ';' | '!' | '?'));
    let words = body
        .split_whitespace()
        .map(|word| word.to_ascii_lowercase())
        .collect::<Vec<_>>();
    if words.len() < 3 || words.len() > 20 || !words.iter().any(|word| word == "dot") {
        return input.to_string();
    }
    let Some(last_dot) = words.iter().rposition(|word| word == "dot") else {
        return input.to_string();
    };
    if last_dot + 1 >= words.len()
        || !words[last_dot + 1]
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric())
        || words
            .get(last_dot + 2)
            .is_some_and(|token| !matches!(token.as_str(), "slash" | "colon"))
    {
        return input.to_string();
    }

    let mut output = String::new();
    let mut index = 0;
    if starts_with_words(&words, 0, &["https", "colon", "slash", "slash"]) {
        output.push_str("https://");
        index = 4;
    } else if starts_with_words(&words, 0, &["http", "colon", "slash", "slash"]) {
        output.push_str("http://");
        index = 4;
    }
    let mut dots = 0usize;
    while index < words.len() {
        match words[index].as_str() {
            "dot" if !output.is_empty() && !output.ends_with(['.', '/', ':', '-']) => {
                output.push('.');
                dots += 1;
            }
            "slash" if !output.is_empty() && !output.ends_with('/') => output.push('/'),
            "colon" if !output.is_empty() && !output.ends_with(':') => output.push(':'),
            "dash" | "hyphen" if !output.is_empty() && !output.ends_with('-') => output.push('-'),
            token if token.chars().all(|ch| ch.is_ascii_alphanumeric()) => output.push_str(token),
            _ => return input.to_string(),
        }
        index += 1;
    }
    if dots == 0 || output.ends_with(['.', '/', ':', '-']) {
        return input.to_string();
    }
    output
}

fn replace_paired_markers(input: &str) -> String {
    let pairs = [
        ("open double quote", "close double quote", "\"", "\""),
        ("open single quote", "close single quote", "'", "'"),
        ("open quote", "close quote", "\"", "\""),
        ("open parenthesis", "close parenthesis", "(", ")"),
        ("open paren", "close paren", "(", ")"),
    ];
    pairs
        .iter()
        .fold(input.to_string(), |text, (open, close, left, right)| {
            replace_bounded_pair(&text, open, close, left, right)
        })
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

#[derive(Clone, Copy)]
enum SpokenMarker {
    Break(&'static str),
    Punctuation(&'static str),
    Infix(&'static str),
    Tight(&'static str),
}

fn replace_spoken_markers(input: &str) -> String {
    const MARKERS: &[(&str, SpokenMarker)] = &[
        ("new paragraph", SpokenMarker::Break("\n\n")),
        ("new line", SpokenMarker::Break("\n")),
        ("exclamation mark", SpokenMarker::Punctuation("!")),
        ("question mark", SpokenMarker::Punctuation("?")),
        ("semicolon", SpokenMarker::Punctuation(";")),
        ("colon", SpokenMarker::Punctuation(":")),
        ("period", SpokenMarker::Punctuation(".")),
        ("comma", SpokenMarker::Punctuation(",")),
        ("em dash", SpokenMarker::Infix("—")),
        ("en dash", SpokenMarker::Infix("–")),
        ("at sign", SpokenMarker::Infix("@")),
        ("hash sign", SpokenMarker::Infix("#")),
        ("number sign", SpokenMarker::Infix("#")),
        ("percent sign", SpokenMarker::Infix("%")),
        ("plus sign", SpokenMarker::Infix("+")),
        ("equals sign", SpokenMarker::Infix("=")),
        ("ampersand", SpokenMarker::Infix("&")),
        ("hyphen", SpokenMarker::Tight("-")),
    ];

    let lower = input.to_ascii_lowercase();
    let mut output = String::with_capacity(input.len());
    let mut index = 0;
    let mut changed = false;
    while index < input.len() {
        let Some((_, ch)) = input[index..].char_indices().next() else {
            break;
        };
        let mut matched = None;
        for (phrase, marker) in MARKERS {
            if lower[index..].starts_with(phrase) && is_phrase_boundary(&lower, index, phrase.len())
            {
                matched = Some((phrase.len(), *marker));
                break;
            }
        }
        if let Some((length, marker)) = matched {
            changed = true;
            apply_spoken_marker(&mut output, marker);
            index += length;
            if input[index..].starts_with(' ') {
                index += 1;
            }
        } else {
            output.push(ch);
            index += ch.len_utf8();
        }
    }
    if changed {
        output.trim().to_string()
    } else {
        input.to_string()
    }
}

fn apply_spoken_marker(output: &mut String, marker: SpokenMarker) {
    while output.ends_with(' ') {
        output.pop();
    }
    match marker {
        SpokenMarker::Break(value) => {
            while output.ends_with('\n') && value == "\n\n" {
                output.pop();
            }
            output.push_str(value);
        }
        SpokenMarker::Punctuation(value) => {
            output.push_str(value);
            // The scanner consumes the source space after a spoken marker, so
            // restore word separation. Final trimming removes this at EOF, and
            // the next marker trims it before inserting a break or symbol.
            output.push(' ');
        }
        SpokenMarker::Infix(value) => {
            if !output.is_empty() && !output.ends_with([' ', '\n']) {
                output.push(' ');
            }
            output.push_str(value);
            output.push(' ');
        }
        SpokenMarker::Tight(value) => output.push_str(value),
    }
}

fn word_spans(input: &str) -> Vec<WordSpan> {
    let mut words = Vec::new();
    let mut start: Option<usize> = None;
    for (index, ch) in input.char_indices() {
        if ch.is_alphanumeric() || matches!(ch, '\'' | '_') {
            start.get_or_insert(index);
        } else if let Some(word_start) = start.take() {
            words.push(WordSpan {
                start: word_start,
                end: index,
                lower: input[word_start..index].to_lowercase(),
            });
        }
    }
    if let Some(word_start) = start {
        words.push(WordSpan {
            start: word_start,
            end: input.len(),
            lower: input[word_start..].to_lowercase(),
        });
    }
    words
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
    before.is_none_or(|ch| !ch.is_alphanumeric()) && after.is_none_or(|ch| !ch.is_alphanumeric())
}

fn starts_with_words(words: &[String], start: usize, phrase: &[&str]) -> bool {
    start + phrase.len() <= words.len()
        && words[start..start + phrase.len()]
            .iter()
            .zip(phrase)
            .all(|(actual, expected)| actual == expected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_clear_spoken_enumeration() {
        assert_eq!(
            format_smart_prose(
                "The three priorities are first reliability, second latency, third accuracy"
            ),
            "The three priorities are:\n1. Reliability\n2. Latency\n3. Accuracy"
        );
        assert_eq!(
            format_smart_prose("first red second green"),
            "1. Red\n2. Green"
        );
    }

    #[test]
    fn enumeration_requires_clear_consecutive_markers_and_bounded_items() {
        for prose in [
            "I first met George on second avenue",
            "The priorities are first reliability third accuracy",
            "The first release was better than the second",
            "The tasks are first second latency",
        ] {
            assert_eq!(format_smart_prose(prose), prose);
        }
        let long_item = format!("The tasks are first {} second done", "word ".repeat(25));
        assert_eq!(format_smart_prose(&long_item), long_item);
    }

    #[test]
    fn bounded_backtrack_replaces_only_the_abandoned_term() {
        assert_eq!(
            format_smart_prose("Ship it Friday—actually, make that Monday"),
            "Ship it Monday."
        );
        assert_eq!(
            format_smart_prose("Meet Friday, I mean Monday"),
            "Meet Monday."
        );
        assert_eq!(
            format_smart_prose("Keep the intro. Send it Friday, or rather Monday"),
            "Keep the intro. Send it Monday."
        );
    }

    #[test]
    fn backtrack_rejects_discourse_and_unbounded_replacements() {
        for prose in [
            "What I mean is that the result is stable",
            "Actually this is already correct",
            "Ship it Friday, I mean that the whole previous paragraph should be rewritten now",
            "I would rather stay home",
            "I can actually make that change today",
            "Meet Friday, rather than Monday",
            "Send it next Friday—actually, make that next Monday",
        ] {
            assert_eq!(format_smart_prose(prose), prose);
        }
    }

    #[test]
    fn formatting_has_an_explicit_whole_utterance_resource_bound() {
        let over_limit = format!(
            "Email address {} at example dot com",
            "a".repeat(MAX_SMART_FORMATTING_INPUT_BYTES)
        );
        assert_eq!(format_smart_prose(&over_limit), over_limit);
    }

    #[test]
    fn formats_email_only_with_explicit_email_context() {
        assert_eq!(
            format_smart_prose("Email George at george at example dot com"),
            "Email George at george@example.com"
        );
        assert_eq!(
            format_smart_prose("Email address jane dot doe plus work at example dot org"),
            "Email address jane.doe+work@example.org"
        );
        let prose = "George is at example dot com today";
        assert_eq!(format_smart_prose(prose), prose);
    }

    #[test]
    fn formats_url_only_with_leading_bounded_cue() {
        assert_eq!(
            format_smart_prose("URL https colon slash slash example dot com slash docs"),
            "https://example.com/docs"
        );
        assert_eq!(
            format_smart_prose("web address www dot example dot org"),
            "www.example.org"
        );
        let prose = "Visit example dot com when ready";
        assert_eq!(format_smart_prose(prose), prose);
        let trailing_prose = "URL example dot com please";
        assert_eq!(format_smart_prose(trailing_prose), trailing_prose);
    }

    #[test]
    fn email_url_and_pair_grammars_fail_closed_at_their_bounds() {
        let long_url = format!("URL example dot com slash {}", "segment ".repeat(20));
        assert_eq!(format_smart_prose(&long_url), long_url);

        let long_pair = format!(
            "Say open quote {} close quote",
            "content ".repeat(MAX_PAIRED_CONTENT_CHARS)
        );
        assert_eq!(format_smart_prose(&long_pair), long_pair);

        let long_email =
            "Email address one dot two dot three dot four dot five dot six at example dot com";
        assert_eq!(format_smart_prose(long_email), long_email);
    }

    #[test]
    fn explicit_punctuation_quotes_parentheses_and_paragraphs_are_deterministic() {
        assert_eq!(
            format_smart_prose(
                "Say open quote ship it close quote period new paragraph Thanks exclamation mark"
            ),
            "Say \"ship it\".\n\nThanks!"
        );
        assert_eq!(
            format_smart_prose("Use open paren optional close paren em dash carefully"),
            "Use (optional) — carefully"
        );
        assert_eq!(
            format_smart_prose("Set x plus sign y equals sign ten percent sign"),
            "Set x + y = ten %"
        );
        assert_eq!(format_smart_prose("one period two"), "one. two");
        let unpaired = "Say open quote this stays literal";
        assert_eq!(format_smart_prose(unpaired), unpaired);
    }

    #[test]
    fn ambiguous_prose_without_complete_explicit_grammar_is_unchanged() {
        for prose in [
            "My email address changed yesterday",
            "The URL was documented in the ticket",
            "She said open quote but never closed it",
            "The first release was second to none",
            "I would rather stay home",
        ] {
            assert_eq!(format_smart_prose(prose), prose);
        }
    }

    #[test]
    fn formatting_is_idempotent_for_all_golden_outputs() {
        let fixtures = [
            "The three priorities are first reliability, second latency, third accuracy",
            "Ship it Friday—actually, make that Monday",
            "Email George at george at example dot com",
            "URL https colon slash slash example dot com slash docs",
            "Say open quote ship it close quote period",
        ];
        for fixture in fixtures {
            let once = format_smart_prose(fixture);
            assert_eq!(format_smart_prose(&once), once, "fixture: {fixture}");
        }
    }

    #[test]
    fn unicode_and_existing_formatted_text_are_preserved() {
        let prose = "Café déjà vu — 東京";
        assert_eq!(format_smart_prose(prose), prose);
        let spaced = "  Preserve exact whitespace when no grammar activates  ";
        assert_eq!(format_smart_prose(spaced), spaced);
        let list = "Priorities:\n1. Reliability\n2. Latency";
        assert_eq!(format_smart_prose(list), list);
    }
}
