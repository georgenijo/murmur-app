//! Gated on-device homophone context pass (issue #833).
//!
//! Smart Correction's deterministic tiers cannot tell `cue` from `queue` or
//! `there` from `their` without sentence context. This optional tier asks
//! Apple's on-device FoundationModels language model to choose between a
//! small, fixed set of spellings, and only for a dictation that contains a
//! confusable token. A dictation without one never reaches the engine.
//!
//! The model never writes text. It answers `number: option` per flagged word,
//! Rust substitutes only those words, and a word-level alignment check rejects
//! any candidate that differs anywhere else. Timeouts, an unavailable OS or
//! model, and rejected answers all deliver the deterministic text. Nothing in
//! this module logs, persists, or transmits transcript text.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use crate::correction::{levenshtein, phonetic_key};

/// Hard end-to-end engine budget per dictation.
pub(crate) const ENGINE_BUDGET: Duration = Duration::from_millis(150);
/// More flagged words than this skips the pass instead of asking the engine.
const MAX_FLAGGED: usize = 6;
/// Longer transcripts skip the pass; the prompt stays small and fast.
const MAX_TEXT_CHARS: usize = 1_200;
/// Sounds-like vocabulary terms considered per dictation.
const MAX_VOCABULARY_TERMS: usize = 500;
const ANSWER_TOKENS_PER_WORD: u32 = 6;

pub(crate) const INSTRUCTIONS: &str = "Pick the correct spelling for each numbered word \
from its options, using the sentence. Answer one line per word: number=option. \
Never add other words. If unsure, pick the first option.";

/// Curated homophone groups. Matching is case-insensitive; the listed form is
/// the spelling written when the engine chooses it.
const CURATED_GROUPS: &[&[&str]] = &[
    &["cue", "queue"],
    &["cloud", "Claude"],
    &["CPO", "CPU"],
    &["there", "their", "they're"],
    &["Maine", "main"],
    &["cash", "cache"],
    &["sink", "sync"],
    &["root", "route"],
    &["whether", "weather"],
    &["principal", "principle"],
    &["lose", "loose"],
    &["your", "you're"],
];

fn curated_index() -> &'static HashMap<String, usize> {
    static INDEX: OnceLock<HashMap<String, usize>> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut index = HashMap::new();
        for (group, words) in CURATED_GROUPS.iter().enumerate() {
            for word in *words {
                index.insert(word.to_lowercase(), group);
            }
        }
        index
    })
}

fn normalize(word: &str) -> String {
    word.replace('\u{2019}', "'").to_lowercase()
}

/// One flagged word: its byte range in the text and the spellings the engine
/// may choose. `options[0]` is always the word exactly as recognized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Confusable {
    pub range: Range<usize>,
    pub options: Vec<String>,
    sentence_start: bool,
}

#[derive(Debug, Clone)]
struct Token {
    /// Whole whitespace-delimited token.
    range: Range<usize>,
    /// Word core without surrounding punctuation.
    core: Range<usize>,
}

fn tokenize(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut start = None;
    for (index, character) in text
        .char_indices()
        .chain(std::iter::once((text.len(), ' ')))
    {
        match (start, character.is_whitespace()) {
            (None, false) => start = Some(index),
            (Some(begin), true) => {
                let token = &text[begin..index];
                let is_word = |c: char| c.is_alphanumeric();
                let lead = token.find(is_word);
                let trail = token.rfind(is_word);
                let core = match (lead, trail) {
                    (Some(lead), Some(trail)) => {
                        let end = trail + token[trail..].chars().next().map_or(0, char::len_utf8);
                        begin + lead..begin + end
                    }
                    _ => index..index,
                };
                tokens.push(Token {
                    range: begin..index,
                    core,
                });
                start = None;
            }
            _ => {}
        }
    }
    tokens
}

fn is_sentence_start(text: &str, tokens: &[Token], index: usize) -> bool {
    let Some(previous) = index.checked_sub(1).map(|previous| &tokens[previous]) else {
        return true;
    };
    if text[previous.range.end..tokens[index].range.start].contains('\n') {
        return true;
    }
    text[previous.range.clone()]
        .trim_end_matches(['"', '\'', ')', ']', '\u{201d}', '\u{2019}'])
        .ends_with(['.', '!', '?'])
}

fn overlaps(range: &Range<usize>, protected: &[Range<usize>]) -> bool {
    protected
        .iter()
        .any(|span| span.start < range.end && range.start < span.end)
}

/// Single-word vocabulary terms eligible as sounds-like targets.
pub(crate) fn vocabulary_word(term: &str) -> Option<String> {
    let term = term.trim();
    let length = term.chars().count();
    ((3..=32).contains(&length) && term.chars().all(char::is_alphabetic)).then(|| term.to_string())
}

/// Find flagged words. Curated groups match case-insensitively. A user
/// vocabulary term flags a different word that sounds like it (same phonetic
/// key, small edit distance), which the deterministic fuzzy tier leaves alone
/// because it is an ordinary word. A word that already spells a vocabulary
/// term, or that an explicit alias wrote, is never flagged.
pub(crate) fn find_confusables(
    text: &str,
    vocabulary_terms: &[String],
    protected: &[Range<usize>],
) -> Vec<Confusable> {
    let tokens = tokenize(text);
    let curated = curated_index();
    let terms = vocabulary_terms
        .iter()
        .take(MAX_VOCABULARY_TERMS)
        .map(|term| (term.as_str(), normalize(term), phonetic_key(term)))
        .collect::<Vec<_>>();
    let mut flags = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        let core = &text[token.core.clone()];
        if core.is_empty()
            || !core
                .chars()
                .all(|c| c.is_alphabetic() || c == '\'' || c == '\u{2019}')
            || overlaps(&token.core, protected)
        {
            continue;
        }
        let lower = normalize(core);
        if terms.iter().any(|(_, term_lower, _)| *term_lower == lower) {
            continue;
        }
        let mut options = vec![core.to_string()];
        let mut push = |option: &str| {
            if !options
                .iter()
                .any(|known| normalize(known) == normalize(option))
            {
                options.push(option.to_string());
            }
        };
        if let Some(&group) = curated.get(&lower) {
            for word in CURATED_GROUPS[group] {
                push(word);
            }
        }
        let key = phonetic_key(core);
        if key.len() >= 2 && lower.chars().count() >= 3 {
            for (term, term_lower, term_key) in &terms {
                let distance = levenshtein(&lower, term_lower);
                if *term_key == key && distance <= 2 && distance <= (term_lower.len() / 3).max(1) {
                    push(term);
                }
            }
        }
        if options.len() >= 2 {
            flags.push(Confusable {
                range: token.core.clone(),
                options,
                sentence_start: is_sentence_start(text, &tokens, index),
            });
        }
    }
    flags
}

/// Prompt: the sentence with each flagged word replaced by its number, then
/// one options line per number.
pub(crate) fn build_prompt(text: &str, flags: &[Confusable]) -> String {
    let mut sentence = String::with_capacity(text.len() + 16);
    let mut cursor = 0;
    for (index, flag) in flags.iter().enumerate() {
        sentence.push_str(&text[cursor..flag.range.start]);
        sentence.push_str(&format!("[{}]", index + 1));
        cursor = flag.range.end;
    }
    sentence.push_str(&text[cursor..]);
    let sentence = sentence
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace('\0', "");
    let mut prompt = format!("Sentence: {sentence}");
    for (index, flag) in flags.iter().enumerate() {
        prompt.push_str(&format!("\n{}: {}", index + 1, flag.options.join("|")));
    }
    prompt
}

/// Parse `number: option` lines into a chosen option index per flag.
/// Any line that is not a valid choice rejects the whole answer.
pub(crate) fn parse_answer(answer: &str, flags: &[Confusable]) -> Option<Vec<usize>> {
    let mut choices: Vec<Option<usize>> = vec![None; flags.len()];
    let mut any = false;
    for line in answer.lines() {
        let line = line.trim().trim_start_matches(['-', '*', ' ']);
        if line.is_empty() {
            continue;
        }
        let line = line.trim_start_matches('[');
        let digits = line.chars().take_while(char::is_ascii_digit).count();
        let number = line[..digits].parse::<usize>().ok()?;
        let rest = line[digits..].trim_start_matches(']').trim_start();
        let rest = rest.strip_prefix([':', '=', '.', ')'])?;
        let choice = rest
            .trim()
            .trim_matches(['"', '`', '\u{201c}', '\u{201d}'])
            .trim_end_matches('.')
            .trim();
        let flag = flags.get(number.checked_sub(1)?)?;
        let option = flag
            .options
            .iter()
            .position(|option| normalize(option) == normalize(choice))?;
        match choices[number - 1] {
            Some(previous) if previous != option => return None,
            _ => choices[number - 1] = Some(option),
        }
        any = true;
    }
    any.then(|| choices.into_iter().map(|c| c.unwrap_or(0)).collect())
}

fn cased(option: &str, original: &str, sentence_start: bool) -> String {
    if normalize(option) == normalize(original) {
        return original.to_string();
    }
    if option.chars().any(char::is_uppercase) {
        return option.to_string();
    }
    let mut letters = original.chars().filter(|c| c.is_alphabetic());
    let first_upper = letters.next().is_some_and(char::is_uppercase);
    if first_upper && original.chars().count() > 1 && letters.all(char::is_uppercase) {
        return option.to_uppercase();
    }
    if first_upper && sentence_start {
        let mut chars = option.chars();
        return chars
            .next()
            .map(|first| first.to_uppercase().chain(chars).collect())
            .unwrap_or_default();
    }
    option.to_string()
}

/// One applied substitution: the original byte range and the replacement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Replacement {
    pub range: Range<usize>,
    pub text: String,
}

fn substitutions(text: &str, flags: &[Confusable], choices: &[usize]) -> Vec<Replacement> {
    flags
        .iter()
        .zip(choices)
        .filter(|(_, &choice)| choice != 0)
        .map(|(flag, &choice)| Replacement {
            range: flag.range.clone(),
            text: cased(
                &flag.options[choice],
                &text[flag.range.clone()],
                flag.sentence_start,
            ),
        })
        .filter(|replacement| replacement.text != text[replacement.range.clone()])
        .collect()
}

fn apply(text: &str, replacements: &[Replacement]) -> String {
    let mut output = String::with_capacity(text.len() + 8);
    let mut cursor = 0;
    for replacement in replacements {
        output.push_str(&text[cursor..replacement.range.start]);
        output.push_str(&replacement.text);
        cursor = replacement.range.end;
    }
    output.push_str(&text[cursor..]);
    output
}

/// Word-level alignment check: the candidate must keep every separator and
/// every unflagged token byte-for-byte, and a flagged token may differ only in
/// its word core, which must be one of that word's options.
pub(crate) fn only_flagged_tokens_changed(
    original: &str,
    candidate: &str,
    flags: &[Confusable],
) -> bool {
    let before = tokenize(original);
    let after = tokenize(candidate);
    if before.len() != after.len() {
        return false;
    }
    let separator = |text: &str, tokens: &[Token], index: usize| {
        let start = index
            .checked_sub(1)
            .map_or(0, |previous| tokens[previous].range.end);
        let end = tokens
            .get(index)
            .map_or(text.len(), |token| token.range.start);
        text[start..end].to_string()
    };
    for index in 0..=before.len() {
        if separator(original, &before, index) != separator(candidate, &after, index) {
            return false;
        }
    }
    before.iter().zip(&after).all(|(old, new)| {
        let old_text = &original[old.range.clone()];
        let new_text = &candidate[new.range.clone()];
        if old_text == new_text {
            return true;
        }
        let Some(flag) = flags.iter().find(|flag| flag.range == old.core) else {
            return false;
        };
        original[old.range.start..old.core.start] == candidate[new.range.start..new.core.start]
            && original[old.core.end..old.range.end] == candidate[new.core.end..new.range.end]
            && flag
                .options
                .iter()
                .any(|option| normalize(option) == normalize(&candidate[new.core.clone()]))
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EngineError {
    Unavailable,
    Timeout,
    Failed,
}

/// The local language model. Production uses [`FoundationModelsEngine`];
/// tests inject fakes.
pub(crate) trait HomophoneEngine: Send + Sync {
    fn choose(
        &self,
        instructions: &str,
        prompt: &str,
        budget: Duration,
        max_tokens: u32,
    ) -> Result<String, EngineError>;
}

/// Content-free outcome of one pass, for logs and tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PassOutcome {
    /// No confusable token: the engine was not called.
    NotTriggered,
    /// Too many flagged words or too long: the engine was not called.
    BoundedSkip,
    /// The engine kept every recognized spelling.
    Unchanged,
    Accepted,
    Rejected,
    Timeout,
    Unavailable,
    Failed,
}

impl PassOutcome {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::NotTriggered => "not_triggered",
            Self::BoundedSkip => "bounded_skip",
            Self::Unchanged => "unchanged",
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
            Self::Timeout => "timeout",
            Self::Unavailable => "unavailable",
            Self::Failed => "failed",
        }
    }

    /// Whether the deterministic text is delivered because the pass could
    /// not produce a trusted answer.
    pub(crate) fn is_fallback(self) -> bool {
        matches!(
            self,
            Self::Rejected | Self::Timeout | Self::Unavailable | Self::Failed
        )
    }
}

pub(crate) struct PassResult {
    pub text: String,
    pub outcome: PassOutcome,
    pub flagged: usize,
    /// Wall time spent in the engine; zero when it was not called.
    pub engine_us: u64,
    /// Substitutions in original-text byte order. Empty unless accepted.
    pub replacements: Vec<Replacement>,
}

pub(crate) fn run_pass(
    text: &str,
    vocabulary_terms: &[String],
    protected: &[Range<usize>],
    engine: &dyn HomophoneEngine,
) -> PassResult {
    let unchanged = |outcome, flagged, engine_us| PassResult {
        text: text.to_string(),
        outcome,
        flagged,
        engine_us,
        replacements: Vec::new(),
    };
    if text.chars().take(MAX_TEXT_CHARS + 1).count() > MAX_TEXT_CHARS {
        return unchanged(PassOutcome::BoundedSkip, 0, 0);
    }
    let flags = find_confusables(text, vocabulary_terms, protected);
    if flags.is_empty() {
        return unchanged(PassOutcome::NotTriggered, 0, 0);
    }
    if flags.len() > MAX_FLAGGED {
        return unchanged(PassOutcome::BoundedSkip, flags.len(), 0);
    }
    let prompt = build_prompt(text, &flags);
    let max_tokens = ANSWER_TOKENS_PER_WORD * flags.len() as u32;
    let started = Instant::now();
    let answer = engine.choose(INSTRUCTIONS, &prompt, ENGINE_BUDGET, max_tokens);
    let engine_us = started.elapsed().as_micros() as u64;
    let answer = match answer {
        Ok(answer) => answer,
        Err(EngineError::Unavailable) => {
            return unchanged(PassOutcome::Unavailable, flags.len(), engine_us)
        }
        Err(EngineError::Timeout) => {
            return unchanged(PassOutcome::Timeout, flags.len(), engine_us)
        }
        Err(EngineError::Failed) => return unchanged(PassOutcome::Failed, flags.len(), engine_us),
    };
    // A late answer is still a budget miss.
    if Duration::from_micros(engine_us) > ENGINE_BUDGET {
        return unchanged(PassOutcome::Timeout, flags.len(), engine_us);
    }
    let Some(choices) = parse_answer(&answer, &flags) else {
        return unchanged(PassOutcome::Rejected, flags.len(), engine_us);
    };
    let replacements = substitutions(text, &flags, &choices);
    if replacements.is_empty() {
        return unchanged(PassOutcome::Unchanged, flags.len(), engine_us);
    }
    let candidate = apply(text, &replacements);
    if !only_flagged_tokens_changed(text, &candidate, &flags) {
        return unchanged(PassOutcome::Rejected, flags.len(), engine_us);
    }
    PassResult {
        text: candidate,
        outcome: PassOutcome::Accepted,
        flagged: flags.len(),
        engine_us,
        replacements,
    }
}

/// Shift protected byte ranges after substitutions that never overlap them.
pub(crate) fn shift_ranges(
    ranges: &[Range<usize>],
    replacements: &[Replacement],
) -> Vec<Range<usize>> {
    ranges
        .iter()
        .map(|range| {
            let mut delta = 0isize;
            for replacement in replacements {
                if replacement.range.end <= range.start {
                    delta += replacement.text.len() as isize - replacement.range.len() as isize;
                }
            }
            let start = (range.start as isize + delta) as usize;
            start..start + range.len()
        })
        .collect()
}

#[cfg(target_os = "macos")]
mod ffi {
    use std::os::raw::c_char;

    extern "C" {
        pub fn murmur_fm_choose(
            instructions: *const c_char,
            prompt: *const c_char,
            timeout_ms: u32,
            max_tokens: u32,
            out: *mut *mut c_char,
        ) -> i32;
        pub fn murmur_fm_free(pointer: *mut c_char);
    }
}

/// Apple FoundationModels on-device engine (macOS 26+), via the in-process
/// Swift bridge in `macos/FoundationModelsBridge.swift`.
pub(crate) struct FoundationModelsEngine;

#[cfg(target_os = "macos")]
impl HomophoneEngine for FoundationModelsEngine {
    fn choose(
        &self,
        instructions: &str,
        prompt: &str,
        budget: Duration,
        max_tokens: u32,
    ) -> Result<String, EngineError> {
        let instructions = std::ffi::CString::new(instructions).map_err(|_| EngineError::Failed)?;
        let prompt = std::ffi::CString::new(prompt).map_err(|_| EngineError::Failed)?;
        let mut answer: *mut std::os::raw::c_char = std::ptr::null_mut();
        // SAFETY: both strings are valid NUL-terminated buffers for the call;
        // the bridge writes either null or a malloc'd string it owns to `answer`.
        let status = unsafe {
            ffi::murmur_fm_choose(
                instructions.as_ptr(),
                prompt.as_ptr(),
                budget.as_millis().min(u32::MAX as u128) as u32,
                max_tokens,
                &mut answer,
            )
        };
        match status {
            0 if !answer.is_null() => {
                // SAFETY: `answer` is the bridge's NUL-terminated allocation,
                // released exactly once through the bridge.
                let text = unsafe { std::ffi::CStr::from_ptr(answer) }
                    .to_string_lossy()
                    .into_owned();
                unsafe { ffi::murmur_fm_free(answer) };
                Ok(text)
            }
            1 => Err(EngineError::Unavailable),
            2 => Err(EngineError::Timeout),
            _ => Err(EngineError::Failed),
        }
    }
}

pub(crate) fn system_engine() -> Arc<dyn HomophoneEngine> {
    Arc::new(FoundationModelsEngine)
}

#[cfg(not(target_os = "macos"))]
impl HomophoneEngine for FoundationModelsEngine {
    fn choose(&self, _: &str, _: &str, _: Duration, _: u32) -> Result<String, EngineError> {
        Err(EngineError::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    struct FakeEngine {
        answer: Result<String, EngineError>,
        calls: AtomicUsize,
        prompts: Mutex<Vec<String>>,
    }

    impl FakeEngine {
        fn answering(answer: &str) -> Self {
            Self::with(Ok(answer.to_string()))
        }

        fn with(answer: Result<String, EngineError>) -> Self {
            Self {
                answer,
                calls: AtomicUsize::new(0),
                prompts: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl HomophoneEngine for FakeEngine {
        fn choose(
            &self,
            _instructions: &str,
            prompt: &str,
            _budget: Duration,
            _max_tokens: u32,
        ) -> Result<String, EngineError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.prompts.lock().unwrap().push(prompt.to_string());
            self.answer.clone()
        }
    }

    fn run(text: &str, engine: &FakeEngine) -> PassResult {
        run_pass(text, &[], &[], engine)
    }

    #[test]
    fn dictation_without_confusable_tokens_never_calls_the_engine() {
        let engine = FakeEngine::answering("1: queue");
        for text in [
            "",
            "Ship the release notes tomorrow morning.",
            "Please review the pull request before lunch.",
            "queued cueing cloudy clouds",
        ] {
            let result = run(text, &engine);
            assert_eq!(result.outcome, PassOutcome::NotTriggered, "{text}");
            assert_eq!(result.text, text);
            assert_eq!(result.engine_us, 0);
        }
        assert_eq!(engine.calls(), 0);
    }

    #[test]
    fn accepts_a_choice_among_the_flagged_options() {
        let engine = FakeEngine::answering("1: queue");
        let result = run("Add the job to the cue, then deploy.", &engine);
        assert_eq!(result.outcome, PassOutcome::Accepted);
        assert_eq!(result.text, "Add the job to the queue, then deploy.");
        assert_eq!(engine.calls(), 1);
        assert_eq!(
            engine.prompts.lock().unwrap()[0],
            "Sentence: Add the job to the [1], then deploy.\n1: cue|queue"
        );
    }

    #[test]
    fn accepts_multiple_choices_and_preserves_casing_and_punctuation() {
        // Flags in prompt order: cloud, there, There, CPO. An unanswered
        // number keeps its recognized spelling.
        let engine = FakeEngine::answering("1=Claude\n2: their\n[4]: CPU.");
        let result = run(
            "Ask cloud about \"there\" plan. There the CPO spikes.",
            &engine,
        );
        assert_eq!(result.flagged, 4);
        assert_eq!(result.outcome, PassOutcome::Accepted);
        assert_eq!(
            result.text,
            "Ask Claude about \"their\" plan. There the CPU spikes."
        );

        let engine = FakeEngine::answering("1: their");
        let result = run("There engineers shipped it.", &engine);
        assert_eq!(result.text, "Their engineers shipped it.");
    }

    #[test]
    fn keeping_every_original_spelling_is_unchanged() {
        let engine = FakeEngine::answering("1: cloud");
        let result = run("The rain cloud moved on.", &engine);
        assert_eq!(result.outcome, PassOutcome::Unchanged);
        assert_eq!(result.text, "The rain cloud moved on.");
    }

    #[test]
    fn rejects_rephrasing_unknown_options_and_conflicting_lines() {
        let text = "Add the job to the cue.";
        for answer in [
            "Add the job to the queue.",
            "1: line",
            "2: queue",
            "1: queue\n1: cue",
            "1: queue and also",
            "",
            "Sure! 1: queue",
        ] {
            let engine = FakeEngine::answering(answer);
            let result = run(text, &engine);
            assert_eq!(result.outcome, PassOutcome::Rejected, "{answer:?}");
            assert_eq!(result.text, text);
        }
    }

    #[test]
    fn engine_errors_deliver_the_deterministic_text() {
        for (error, outcome) in [
            (EngineError::Timeout, PassOutcome::Timeout),
            (EngineError::Unavailable, PassOutcome::Unavailable),
            (EngineError::Failed, PassOutcome::Failed),
        ] {
            let engine = FakeEngine::with(Err(error));
            let result = run("Add it to the cue.", &engine);
            assert_eq!(result.outcome, outcome);
            assert!(outcome.is_fallback());
            assert_eq!(result.text, "Add it to the cue.");
        }
    }

    #[test]
    fn a_late_answer_is_a_timeout() {
        struct SlowEngine;
        impl HomophoneEngine for SlowEngine {
            fn choose(
                &self,
                _: &str,
                _: &str,
                budget: Duration,
                _: u32,
            ) -> Result<String, EngineError> {
                std::thread::sleep(budget + Duration::from_millis(5));
                Ok("1: queue".into())
            }
        }
        let result = run_pass("Add it to the cue.", &[], &[], &SlowEngine);
        assert_eq!(result.outcome, PassOutcome::Timeout);
        assert_eq!(result.text, "Add it to the cue.");
    }

    #[test]
    fn alignment_check_rejects_anything_but_flagged_option_swaps() {
        let text = "Add the job to the cue, then deploy.";
        let flags = find_confusables(text, &[], &[]);
        assert!(only_flagged_tokens_changed(
            text,
            "Add the job to the queue, then deploy.",
            &flags
        ));
        for candidate in [
            "Add the task to the queue, then deploy.",
            "Add the job to the queue then deploy.",
            "Add the job to the queue, then  deploy.",
            "Add the job to the line, then deploy.",
            "Add the job to the queue, then deploy it.",
            "Add the job to queue, then deploy.",
        ] {
            assert!(
                !only_flagged_tokens_changed(text, candidate, &flags),
                "{candidate}"
            );
        }
    }

    #[test]
    fn protected_spans_and_vocabulary_spellings_are_never_flagged() {
        let text = "Ask cloud to sync";
        // "cloud" is inside an explicit-alias span.
        assert_eq!(
            find_confusables(text, &[], std::slice::from_ref(&(4..9))).len(),
            1
        );
        // "Claude" already spells a user vocabulary term.
        assert!(find_confusables("Ask Claude now", &["Claude".into()], &[]).is_empty());
    }

    #[test]
    fn vocabulary_terms_flag_sounds_like_words_only() {
        let terms = vec!["Kubrick".to_string(), "Claude".to_string()];
        let flags = find_confusables("ask cloud about the tray", &terms, &[]);
        assert_eq!(flags.len(), 1);
        assert_eq!(flags[0].options, vec!["cloud", "Claude"]);
        assert!(find_confusables("the clown laughed", &terms, &[]).is_empty());
    }

    #[test]
    fn vocabulary_only_sounds_like_group_can_trigger_without_curated_words() {
        let engine = FakeEngine::answering("1=Read");
        let result = run_pass("Contact red today.", &["Read".into()], &[], &engine);
        assert_eq!(result.outcome, PassOutcome::Accepted);
        assert_eq!(result.text, "Contact Read today.");
        assert_eq!(engine.calls(), 1);
    }

    /// Explicit hardware evaluation; prints aggregate numbers only. This is
    /// synthetic data, never the user's corpus or dictation history.
    #[test]
    #[ignore = "requires Apple Intelligence; run explicitly for synthetic latency evidence"]
    fn synthetic_system_engine_metrics() {
        let cases = [
            (
                "Send the meeting notes to the team.",
                "Send the meeting notes to the team.",
            ),
            (
                "Please review this pull request today.",
                "Please review this pull request today.",
            ),
            (
                "Push the fix to the release branch.",
                "Push the fix to the release branch.",
            ),
            (
                "Open the settings and enable dark mode.",
                "Open the settings and enable dark mode.",
            ),
            (
                "Thanks for the quick turnaround.",
                "Thanks for the quick turnaround.",
            ),
            ("Add the job to the cue.", "Add the job to the queue."),
            (
                "Ask cloud to explain this Rust function.",
                "Ask Claude to explain this Rust function.",
            ),
            (
                "The CPO usage spiked during compilation.",
                "The CPU usage spiked during compilation.",
            ),
            (
                "The engineers pushed there changes.",
                "The engineers pushed their changes.",
            ),
            (
                "Merge into Maine after review.",
                "Merge into main after review.",
            ),
        ];
        let mut outcomes = std::collections::BTreeMap::new();
        let mut elapsed = Vec::new();
        let (mut triggered, mut improved, mut regressions) = (0, 0, 0);
        for (text, expected) in cases {
            let started = Instant::now();
            let result = run_pass(text, &[], &[], &FoundationModelsEngine);
            elapsed.push(started.elapsed().as_micros() as u64);
            *outcomes.entry(result.outcome.as_str()).or_insert(0) += 1;
            triggered += usize::from(result.flagged > 0);
            improved += usize::from(text != expected && result.text == expected);
            regressions += usize::from(result.text != text && result.text != expected);
            assert!(only_flagged_tokens_changed(
                text,
                &result.text,
                &find_confusables(text, &[], &[])
            ));
        }
        let neutral_max_us = *elapsed[..5].iter().max().unwrap();
        elapsed.sort_unstable();
        println!("homophone_synthetic samples={} triggered={} improved={} regressions={} neutral_max_us={} median_us={} max_us={} outcomes={:?}",
            cases.len(), triggered, improved, regressions, neutral_max_us, elapsed[elapsed.len()/2], elapsed.last().unwrap(), outcomes);
    }

    #[test]
    fn bounds_skip_the_engine() {
        let engine = FakeEngine::answering("1: queue");
        let text = "cue cue cue cue cue cue cue";
        let result = run(text, &engine);
        assert_eq!(result.outcome, PassOutcome::BoundedSkip);
        assert_eq!(engine.calls(), 0);
    }

    #[test]
    fn shift_ranges_follows_length_changes_before_a_span() {
        let replacements = vec![Replacement {
            range: 4..7,
            text: "queue".into(),
        }];
        assert_eq!(
            shift_ranges(&[10..14, 0..3], &replacements),
            vec![12..16, 0..3]
        );
    }

    /// Content-free trigger-rate measurement on synthetic phrases.
    #[test]
    fn synthetic_trigger_rate() {
        const PHRASES: &[&str] = &[
            "Send the meeting notes to the team.",
            "Remind me to call the dentist tomorrow.",
            "The build failed on the second step.",
            "Can you review this pull request today?",
            "Let's grab lunch at noon.",
            "Push the fix to the release branch.",
            "I will be ten minutes late.",
            "Open the settings and turn on dark mode.",
            "The quarterly report is ready for review.",
            "Thanks for the quick turnaround.",
            "Add the job to the cue before the deploy.",
            "Ask cloud to summarize this thread.",
            "The CPO usage spiked during the test.",
            "The engineers pushed there changes last night.",
            "Merge it into Maine after review.",
            "Clear the cash and restart the server.",
            "Did the files sink to the laptop?",
            "We need to decide whether to ship.",
            "Your build is green.",
            "Pick a route through the city.",
        ];
        let triggered = PHRASES
            .iter()
            .filter(|phrase| !find_confusables(phrase, &[], &[]).is_empty())
            .count();
        // Half the corpus deliberately contains confusables.
        assert_eq!(triggered, 10);
        let neutral = &PHRASES[..10];
        assert!(neutral
            .iter()
            .all(|phrase| find_confusables(phrase, &[], &[]).is_empty()));
    }
}
