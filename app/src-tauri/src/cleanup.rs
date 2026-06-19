//! Rule-based transcript cleanup applied before injection.
//!
//! Pure, deterministic text tidying — no LLM, no network. Each rule is
//! deliberately conservative: it only touches filler tokens it recognizes and
//! never drops a real word. Filler removal and capitalization are independently
//! toggleable so users can opt into only the parts they trust.

/// Options controlling which cleanup rules run.
///
/// The spacing/punctuation normalisation and immediate-duplicate collapse
/// always run (they're safe and reversible in effect); only the two
/// potentially-aggressive rules are gated.
#[derive(Debug, Clone, Copy)]
pub struct CleanupOptions {
    /// Remove standalone filler tokens ("um", "uh", "er", "hmm").
    pub remove_filler: bool,
    /// Capitalize sentence starts.
    pub capitalize: bool,
}

impl Default for CleanupOptions {
    fn default() -> Self {
        Self { remove_filler: true, capitalize: true }
    }
}

/// Filler tokens removed when they appear as standalone words. Kept short and
/// unambiguous so we never strip a real word (e.g. "Erm" the name, or "her").
const FILLER_TOKENS: &[&str] = &["um", "uh", "er", "hmm", "uhh", "umm", "erm"];

/// Clean a transcript according to `opts`.
///
/// Pipeline order (each step operates on the previous step's output):
/// 1. remove standalone filler tokens (if enabled),
/// 2. collapse immediate duplicate words ("the the" -> "the"),
/// 3. normalize whitespace and spacing around punctuation,
/// 4. capitalize sentence starts (if enabled).
///
/// Terminal punctuation present in the input is preserved.
pub fn clean_transcript(text: &str, opts: CleanupOptions) -> String {
    if text.trim().is_empty() {
        return String::new();
    }

    let mut out = text.to_string();

    if opts.remove_filler {
        out = remove_filler(&out);
    }

    out = collapse_duplicates(&out);
    out = normalize_spacing(&out);

    if opts.capitalize {
        out = capitalize_sentences(&out);
    }

    out
}

/// Strip a single trailing punctuation char from a token, returning the bare
/// word and the punctuation (if any). Only handles the sentence/clause marks
/// we care about so we don't mangle contractions or hyphenated words.
fn split_trailing_punct(token: &str) -> (&str, &str) {
    if let Some(last) = token.chars().last() {
        if matches!(last, '.' | ',' | '!' | '?' | ';' | ':') {
            let idx = token.len() - last.len_utf8();
            return (&token[..idx], &token[idx..]);
        }
    }
    (token, "")
}

/// Compare two tokens for the duplicate-collapse rule: case-insensitive on the
/// word, ignoring trailing punctuation. "The the." collapses to a single key.
fn word_key(token: &str) -> String {
    let (word, _) = split_trailing_punct(token);
    word.to_lowercase()
}

/// Closed-class function words whose immediate repetition is overwhelmingly a
/// recognition/stutter artifact rather than real speech. Only immediate
/// duplicates of these words collapse.
///
/// This preserves the "never drop a real word" guarantee for legitimate
/// consecutive repeats that English genuinely produces — e.g. "had had no
/// effect", "the things that that man said", "all that I had had" — because
/// those content/relative words are *not* on this list. The list is kept
/// deliberately small and excludes any word that can validly double:
///   - "that" is excluded ("that that" is valid),
///   - "had"/"is"/"do"/"has" are excluded (valid as repeated auxiliaries).
const COLLAPSIBLE_STUTTER_WORDS: &[&str] = &[
    "i", "the", "a", "an", "and", "to", "of", "it", "in", "on",
    "we", "you", "so", "but", "for", "with", "my", "he", "she", "they",
];

/// Remove filler tokens that stand alone as whole words. A token is filler only
/// when, stripped of trailing punctuation, it case-insensitively matches a known
/// filler. Punctuation attached to a removed filler is dropped with it.
fn remove_filler(text: &str) -> String {
    let kept: Vec<&str> = text
        .split_whitespace()
        .filter(|token| {
            let (word, _) = split_trailing_punct(token);
            let lower = word.to_lowercase();
            !FILLER_TOKENS.contains(&lower.as_str())
        })
        .collect();
    kept.join(" ")
}

/// Collapse immediate duplicate *stutter* words ("the the" -> "the",
/// "I I" -> "I"). Comparison is case-insensitive and punctuation-insensitive;
/// the *first* occurrence is kept so any trailing punctuation on it survives.
///
/// Collapsing is intentionally conservative: only a repeat whose word is in
/// [`COLLAPSIBLE_STUTTER_WORDS`] is dropped. A legitimate doubled content word
/// such as "had had" or "that that" is left untouched, honoring the module's
/// "never drop a real word" guarantee.
fn collapse_duplicates(text: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut prev_key: Option<String> = None;
    for token in text.split_whitespace() {
        let key = word_key(token);
        let is_repeat = !key.is_empty() && prev_key.as_deref() == Some(key.as_str());
        // Only collapse an immediate repeat when the word is a known stutter
        // artifact; real repeated words (e.g. "had had") are preserved.
        if is_repeat && COLLAPSIBLE_STUTTER_WORDS.contains(&key.as_str()) {
            continue;
        }
        out.push(token);
        prev_key = Some(key);
    }
    out.join(" ")
}

/// Normalize whitespace: collapse runs of spaces, remove spaces before
/// sentence/clause punctuation, and ensure a single space after punctuation
/// when another word follows.
fn normalize_spacing(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut prev_was_space = false;

    for ch in text.chars() {
        if ch.is_whitespace() {
            // Defer emitting whitespace until we know it isn't followed by
            // punctuation that should hug the previous word.
            prev_was_space = true;
            continue;
        }

        if matches!(ch, '.' | ',' | '!' | '?' | ';' | ':') {
            // Drop any pending space before punctuation.
            prev_was_space = false;
            result.push(ch);
            continue;
        }

        if prev_was_space && !result.is_empty() {
            result.push(' ');
        }
        prev_was_space = false;
        result.push(ch);
    }

    result.trim().to_string()
}

/// Capitalize the first alphabetic character of each sentence. Sentence
/// boundaries are the start of the string and the position after a terminal
/// `.`/`!`/`?`. Only the leading letter is touched — interior casing (acronyms,
/// proper nouns) is left untouched so we never corrupt real words.
fn capitalize_sentences(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut at_sentence_start = true;

    for ch in text.chars() {
        if at_sentence_start && ch.is_alphabetic() {
            for upper in ch.to_uppercase() {
                result.push(upper);
            }
            at_sentence_start = false;
            continue;
        }

        result.push(ch);

        if matches!(ch, '.' | '!' | '?') {
            at_sentence_start = true;
        } else if !ch.is_whitespace() {
            // A non-terminal, non-space char means we're inside a sentence.
            at_sentence_start = false;
        }
        // Whitespace between a terminator and the next letter keeps
        // `at_sentence_start` true.
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full() -> CleanupOptions {
        CleanupOptions { remove_filler: true, capitalize: true }
    }

    #[test]
    fn empty_input_returns_empty() {
        assert_eq!(clean_transcript("", full()), "");
        assert_eq!(clean_transcript("   ", full()), "");
    }

    #[test]
    fn removes_standalone_filler() {
        assert_eq!(
            clean_transcript("um hello uh world", full()),
            "Hello world"
        );
    }

    #[test]
    fn removes_filler_with_trailing_punct() {
        assert_eq!(clean_transcript("hello um, world", full()), "Hello world");
    }

    #[test]
    fn keeps_real_words_resembling_filler() {
        // "her" and "under" must survive — only exact filler tokens are dropped.
        assert_eq!(
            clean_transcript("her under the umbrella", full()),
            "Her under the umbrella"
        );
    }

    #[test]
    fn filler_disabled_keeps_tokens() {
        let opts = CleanupOptions { remove_filler: false, capitalize: false };
        assert_eq!(clean_transcript("um hello", opts), "um hello");
    }

    #[test]
    fn collapses_immediate_duplicates() {
        assert_eq!(clean_transcript("the the cat", full()), "The cat");
        assert_eq!(clean_transcript("I I think", full()), "I think");
    }

    #[test]
    fn collapses_case_insensitive_duplicates() {
        assert_eq!(clean_transcript("The the dog", full()), "The dog");
    }

    #[test]
    fn does_not_collapse_distinct_words() {
        assert_eq!(
            clean_transcript("had had no effect", full()).to_lowercase(),
            "had had no effect"
        );
        // Only *immediate* repeats collapse; "the cat the dog" stays intact.
        assert_eq!(clean_transcript("the cat the dog", full()), "The cat the dog");
    }

    #[test]
    fn normalizes_space_before_punctuation() {
        assert_eq!(
            clean_transcript("hello , world .", full()),
            "Hello, world."
        );
    }

    #[test]
    fn collapses_double_spaces() {
        assert_eq!(
            clean_transcript("hello    there  friend", full()),
            "Hello there friend"
        );
    }

    #[test]
    fn capitalizes_sentence_starts() {
        assert_eq!(
            clean_transcript("hello world. how are you?", full()),
            "Hello world. How are you?"
        );
    }

    #[test]
    fn capitalize_disabled_leaves_case() {
        let opts = CleanupOptions { remove_filler: false, capitalize: false };
        assert_eq!(clean_transcript("hello. world.", opts), "hello. world.");
    }

    #[test]
    fn preserves_terminal_punctuation() {
        assert_eq!(clean_transcript("done!", full()), "Done!");
        assert_eq!(clean_transcript("really?", full()), "Really?");
    }

    #[test]
    fn combined_real_world_example() {
        let input = "um so the the meeting is uh tomorrow .  see you  there";
        assert_eq!(
            clean_transcript(input, full()),
            "So the meeting is tomorrow. See you there"
        );
    }

    #[test]
    fn does_not_drop_interior_casing() {
        // Acronyms / proper nouns mid-sentence are untouched.
        assert_eq!(
            clean_transcript("call the FBI today.", full()),
            "Call the FBI today."
        );
    }

    #[test]
    fn filler_only_input_collapses_to_empty() {
        assert_eq!(clean_transcript("um uh hmm", full()), "");
    }

    #[test]
    fn independent_toggles() {
        // Filler removed, but capitalization left off.
        let opts = CleanupOptions { remove_filler: true, capitalize: false };
        assert_eq!(clean_transcript("um hello", opts), "hello");

        // Capitalization on, filler left in.
        let opts = CleanupOptions { remove_filler: false, capitalize: true };
        assert_eq!(clean_transcript("um hello", opts), "Um hello");
    }
}
