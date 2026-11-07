//! Post-model text correction (Tiers 1–2): backend-agnostic vocabulary correction.
//!
//! Speech engines (Parakeet/sherpa, Whisper) emit ordinary words: "use effect",
//! "standard error", "red pivot". This layer rewrites those into the intended
//! written forms ("useEffect", "stderr", "rePivot") *after* transcription, so it
//! works on every backend — unlike the old Whisper-only `initial_prompt` vocab
//! bias, which is a silent no-op on the default Parakeet engine.
//!
//! Two tiers, both no-LLM and fully local:
//!   - **Tier 1 — exact map.** A spoken→written phrase table compiled into a
//!     single Aho-Corasick automaton; one pass over the text replaces every
//!     word-boundary match. Catches the forms we can enumerate ("use effect").
//!   - **Tier 2 — sounds-like.** For tokens that Tier 1 didn't fix, a phonetic
//!     key + edit-distance check against the same vocab catches *mishearings* of
//!     those terms ("red pivot" ≈ "re pivot" → "rePivot"). Fires only when a
//!     vocab term is phonetically close, so ordinary English is left alone.
//!
//! The expensive part — compiling the automaton and the phonetic index — happens
//! once in [`CorrectionMatcher::build`] (called on settings-change), not per
//! transcription. [`CorrectionMatcher::apply`] is the hot path and is a couple of
//! linear passes over a short transcript.

use aho_corasick::{AhoCorasick, MatchKind};
use std::collections::HashMap;
use std::ops::Range;
use std::sync::LazyLock;

use crate::state::TextRegister;

/// Common dev abbreviations whose written form can't be derived from how they're
/// spoken (you say "standard error", you write "stderr"). Only loaded when the
/// caller opts into dev-context builtins (the code-vocab signal), since these are
/// literal substitutions that would misfire on prose ("the standard error of the
/// mean"). Kept deliberately small, unambiguous, and dev-specific — no semantic
/// guesses like "kubernetes"→"kubectl" (different things). Spoken form lowercase.
pub const BUILTIN_ABBREVS: &[(&str, &str)] = &[
    ("standard error", "stderr"),
    ("standard out", "stdout"),
    ("standard output", "stdout"),
    ("standard in", "stdin"),
    ("standard input", "stdin"),
];

/// Tier-2 edit-distance ceiling. A token is only rewritten to a vocab term if the
/// (lowercased) edit distance to that term's spoken form is `<=` this *and* their
/// phonetic keys match. Kept tight to avoid "correcting" real English.
const FUZZY_MAX_DIST: usize = 2;

/// Minimum length for Tier-2 fuzzy matching (applies to both the candidate phrase
/// and the vocab term's spoken form). Short words collide far too easily — e.g.
/// "get" and "git" share a phonetic key and are 1 edit apart, so without this floor
/// a "git" vocab entry would rewrite every spoken "get". Short terms still match
/// exactly via Tier 1; Tier 2 only kicks in for longer, lower-collision words.
const MIN_FUZZY_LEN: usize = 5;

/// Content-free recording context consulted by contextual rules. It comes
/// from the frozen stage configuration and is never derived from the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CorrectionContext {
    pub register: TextRegister,
    pub project_context: bool,
}

impl Default for CorrectionContext {
    fn default() -> Self {
        Self {
            register: TextRegister::Everyday,
            project_context: false,
        }
    }
}

/// Lowercase words that must (or must not) occur within a bounded number of
/// words before and after a candidate phrase, inside the same clause.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CueWindow {
    words: Vec<String>,
    before: usize,
    after: usize,
}

impl CueWindow {
    fn new(words: &[&str], before: usize, after: usize) -> Self {
        Self {
            words: words.iter().map(|word| word.to_string()).collect(),
            before,
            after,
        }
    }

    /// The phrase occupies tokens `first..end`. Neither side of the window
    /// crosses a clause boundary, so "Ship the app. Tori is here" gives the
    /// second sentence no cue.
    fn matches(&self, tokens: &[ContextToken], first: usize, end: usize) -> bool {
        let clause = tokens[first].clause;
        let before = tokens[..first]
            .iter()
            .rev()
            .take(self.before)
            .take_while(|token| token.clause == clause);
        let after = tokens[end..]
            .iter()
            .take(self.after)
            .take_while(|token| token.clause == clause);
        before
            .chain(after)
            .any(|token| self.words.contains(&token.lower))
    }
}

/// When a built-in contextual phrase may be rewritten. The frozen register or
/// local project signal enables it outright; otherwise every window of at
/// least one clause must match. Any blocker refuses the rewrite: a cue and a
/// counter-cue together are ambiguous, so the phrase stays as recognized.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ContextRequirement {
    registers: Vec<TextRegister>,
    project_context: bool,
    clauses: Vec<Vec<CueWindow>>,
    blockers: Vec<CueWindow>,
}

impl ContextRequirement {
    fn allows(
        &self,
        context: CorrectionContext,
        tokens: &[ContextToken],
        first: usize,
        end: usize,
    ) -> bool {
        if self
            .blockers
            .iter()
            .any(|cue| cue.matches(tokens, first, end))
        {
            return false;
        }
        self.registers.contains(&context.register)
            || (self.project_context && context.project_context)
            || self
                .clauses
                .iter()
                .any(|clause| clause.iter().all(|cue| cue.matches(tokens, first, end)))
    }
}

/// A Unicode-aware whole word, so a rule for `tori` never matches inside
/// `Torié` or a mixed-script name.
#[derive(Debug, Clone)]
struct ContextToken {
    lower: String,
    start: usize,
    end: usize,
    /// Sentence or clause index; cue windows stop at `.`, `!`, `?`, `;`, and
    /// line breaks.
    clause: usize,
}

fn context_tokens(text: &str) -> Vec<ContextToken> {
    // Marks are part of the word even when the recognizer emits decomposed
    // Unicode: `Tori\u{301}` must not match the builtin phrase `Tori`.
    static WORDS: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"[\p{Alphabetic}\p{N}\p{M}_]+").expect("valid contextual word pattern")
    });
    let mut tokens = Vec::new();
    let mut clause = 0;
    let mut gap_start = 0;
    for word in WORDS.find_iter(text) {
        if is_clause_break(&text[gap_start..word.start()]) {
            clause += 1;
        }
        tokens.push(ContextToken {
            lower: word.as_str().to_lowercase(),
            start: word.start(),
            end: word.end(),
            clause,
        });
        gap_start = word.end();
    }
    tokens
}

/// A gap ends a clause at `!`, `?`, `;`, a line break, or a `.` followed by
/// whitespace. A `.` inside `tauri.conf` or `2.0` does not.
fn is_clause_break(gap: &str) -> bool {
    let mut characters = gap.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '!' | '?' | ';' | '\n' | '\r' => return true,
            '.' if characters.peek().is_some_and(|next| next.is_whitespace()) => return true,
            _ => {}
        }
    }
    false
}

/// A built-in recognizer variant rewritten only in a proven context.
struct ContextRule {
    spoken: Vec<String>,
    written: &'static str,
    requirement: ContextRequirement,
}

const DEVELOPER_REGISTERS: &[TextRegister] = &[TextRegister::Technical, TextRegister::Terminal];

/// Words that make `Maine` a place rather than the git branch `main`.
const MAIN_GEOGRAPHY_WORDS: &[&str] = &[
    "augusta",
    "born",
    "city",
    "climate",
    "coast",
    "country",
    "government",
    "legislature",
    "live",
    "lived",
    "office",
    "portland",
    "state",
    "tourism",
    "travel",
    "trip",
    "vacation",
    "visit",
    "visited",
    "visiting",
    "weather",
];
const MAIN_PREPOSITIONS: &[&str] = &["from", "into", "on", "onto", "to"];
const MAIN_VERSION_CONTROL_ACTIONS: &[&str] = &[
    "check", "checkout", "fetch", "latest", "merge", "pull", "rebase", "switch", "sync",
];
const MAIN_CHANGES: &[&str] = &["change", "changes"];

/// Words near `Tori` / `Tory` that establish the Tauri framework.
const TAURI_CUE_WORDS: &[&str] = &[
    "rust", "cargo", "crate", "crates", "plugin", "plugins", "tauri", "webview", "npm", "pnpm",
    "yarn", "ipc", "sidecar", "app", "apps", "build", "builds", "bundle", "desktop", "frontend",
    "backend",
];
/// Words near `Tori` / `Tory` that point to a person, so the name is kept.
const PERSON_CUE_WORDS: &[&str] = &[
    "she",
    "her",
    "hers",
    "herself",
    "he",
    "him",
    "his",
    "friend",
    "sister",
    "brother",
    "mom",
    "dad",
    "mother",
    "father",
    "daughter",
    "son",
    "wife",
    "husband",
    "girlfriend",
    "boyfriend",
    "aunt",
    "uncle",
    "cousin",
    "niece",
    "nephew",
    "named",
    "hey",
    "hi",
    "hello",
    "dear",
    "thanks",
    "thank",
    "met",
    "birthday",
    "ms",
    "mrs",
    "miss",
];
/// Words near `Tory` that point to British politics.
const POLITICS_CUE_WORDS: &[&str] = &[
    "party",
    "government",
    "mp",
    "mps",
    "minister",
    "conservative",
    "conservatives",
    "labour",
    "election",
    "vote",
    "voters",
    "leader",
    "leadership",
    "parliament",
    "westminster",
    "britain",
    "british",
];

/// Built-in contextual variants, evaluated after explicit aliases and before
/// derived vocabulary. The developer entries fix recurring recognizer misses
/// for the default engines and apply only in Technical/Terminal registers or
/// local project context (Tauri also when a nearby cue names the framework).
static BUILTIN_CONTEXT_RULES: LazyLock<Vec<ContextRule>> = LazyLock::new(|| {
    let developer = |spoken: &[&str], written: &'static str| ContextRule {
        spoken: spoken.iter().map(|word| word.to_string()).collect(),
        written,
        requirement: ContextRequirement {
            registers: DEVELOPER_REGISTERS.to_vec(),
            project_context: true,
            ..ContextRequirement::default()
        },
    };
    let main_branch = |spoken: &str, changes_clause: Vec<CueWindow>| ContextRule {
        spoken: vec![spoken.to_string()],
        written: "main",
        requirement: ContextRequirement {
            clauses: vec![vec![CueWindow::new(&["branch"], 1, 1)], changes_clause],
            blockers: vec![CueWindow::new(MAIN_GEOGRAPHY_WORDS, 4, 4)],
            ..ContextRequirement::default()
        },
    };
    let tauri = |spoken: &str, blockers: Vec<CueWindow>| ContextRule {
        spoken: vec![spoken.to_string()],
        written: "Tauri",
        requirement: ContextRequirement {
            registers: DEVELOPER_REGISTERS.to_vec(),
            project_context: true,
            clauses: vec![vec![CueWindow::new(TAURI_CUE_WORDS, 8, 8)]],
            blockers,
        },
    };
    let mut rules = vec![
        // The recognizer hears the git branch `main` as `Maine` or `me`. `me`
        // is far more common in prose, so it needs the exact observed shape
        // "check [out] ... changes on me".
        main_branch(
            "maine",
            vec![
                CueWindow::new(MAIN_PREPOSITIONS, 1, 0),
                CueWindow::new(MAIN_VERSION_CONTROL_ACTIONS, 7, 0),
                CueWindow::new(MAIN_CHANGES, 7, 0),
            ],
        ),
        main_branch(
            "me",
            vec![
                CueWindow::new(&["on"], 1, 0),
                CueWindow::new(MAIN_CHANGES, 7, 0),
                CueWindow::new(&["check"], 7, 0),
            ],
        ),
        tauri("tori", vec![CueWindow::new(PERSON_CUE_WORDS, 4, 4)]),
        tauri(
            "tory",
            vec![
                CueWindow::new(PERSON_CUE_WORDS, 4, 4),
                CueWindow::new(POLITICS_CUE_WORDS, 4, 4),
            ],
        ),
        developer(&["postgres", "sql"], "PostgreSQL"),
        developer(&["postgre", "sql"], "PostgreSQL"),
        developer(&["npxtsc"], "npx tsc"),
        developer(&["coreml"], "Core ML"),
    ];
    rules.sort_by_key(|rule| std::cmp::Reverse(rule.spoken.len()));
    rules
});

/// A single vocab entry: the written form the user wants, and the lowercase
/// spoken form we expect the ASR to emit for it.
#[derive(Debug, Clone)]
struct Term {
    written: String,
    spoken: String,
    /// Whether this term may be matched by Tier-2 fuzzy. Only *structured*
    /// identifiers qualify (see [`is_fuzzy_eligible`]); plain words are exact-only.
    fuzzy_eligible: bool,
}

#[derive(Debug, Clone)]
struct ExplicitAlias {
    spoken: Vec<char>,
    written: String,
    priority: u8,
}

/// Match one normalized lowercase alias at an original UTF-8 byte boundary.
/// Lowercase expansion (for example `İ` -> `i` + combining dot) is consumed per
/// source scalar, so the returned end always remains a valid source byte offset.
fn explicit_alias_match_end(text: &str, start: usize, spoken: &[char]) -> Option<usize> {
    let mut expected = 0usize;
    for (offset, character) in text[start..].char_indices() {
        for lowercase in character.to_lowercase() {
            if spoken.get(expected) != Some(&lowercase) {
                return None;
            }
            expected += 1;
        }
        if expected == spoken.len() {
            return Some(start + offset + character.len_utf8());
        }
    }
    None
}

/// A compiled, reusable correction matcher. Build once on settings-change, then
/// call [`apply`](Self::apply) per transcription.
pub struct CorrectionMatcher {
    /// User-authored exact aliases. Kept separate from generic fuzzy terms so
    /// an explicit alias never broadens correction beyond the entered phrase.
    explicit_aliases: Vec<ExplicitAlias>,
    /// Tier-1 automaton over spoken phrases (case-insensitive, leftmost-longest).
    ac: Option<AhoCorasick>,
    /// Written replacement for each Tier-1 pattern, parallel to the automaton's
    /// pattern ids.
    replacements: Vec<String>,
    /// Whether Tier 2 (fuzzy) is enabled.
    fuzzy: bool,
    /// All terms, for Tier-2 distance checks.
    terms: Vec<Term>,
    /// Lowercased written forms, so Tier 2 never "corrects" an already-correct token.
    written_lower: std::collections::HashSet<String>,
    /// Explicit aliases whose spoken form contains a digit. Correct and Teach
    /// learns from final text, where spoken numbers are already digits, so
    /// these are matched again after the spoken-numbers stage.
    numeric_aliases: Vec<ExplicitAlias>,
    /// Single-word user vocabulary spellings, used only as sounds-like
    /// targets by the optional homophone context pass.
    homophone_terms: Vec<String>,
}

impl CorrectionMatcher {
    /// Build a matcher from the unified vocab list.
    ///
    /// `terms` are written forms (e.g. `useEffect`, `kubectl`, `rePivot`) — the
    /// same identifiers the vocab feature already collects. Their spoken forms are
    /// auto-derived by splitting camelCase / snake_case / digit boundaries. `pairs`
    /// are explicit spoken→written overrides (e.g. from custom vocabulary entries
    /// containing a `=`), layered on top. The built-in abbreviation table is always
    /// included. `fuzzy` toggles Tier 2.
    ///
    /// `include_builtins` loads [`BUILTIN_ABBREVS`] (gate on the dev-context /
    /// code-vocab signal so they don't misfire on prose).
    ///
    /// Returns an empty (no-op) matcher when there is nothing to correct.
    pub fn build(
        terms: &[String],
        pairs: &[(String, String)],
        fuzzy: bool,
        include_builtins: bool,
    ) -> Self {
        Self::build_with_learned(terms, pairs, &[], fuzzy, include_builtins)
    }

    /// Build the same matcher while inserting reviewed learned replacements
    /// between explicit vocabulary aliases and lower-priority derived/fuzzy
    /// vocabulary. Explicit aliases retain priority for identical phrases.
    pub fn build_with_learned(
        terms: &[String],
        pairs: &[(String, String)],
        learned_pairs: &[(String, String)],
        fuzzy: bool,
        include_builtins: bool,
    ) -> Self {
        // spoken(lowercased) -> written for derived/built-in vocabulary.
        let mut map: HashMap<String, String> = HashMap::new();

        if include_builtins {
            for (spoken, written) in BUILTIN_ABBREVS {
                map.insert(spoken.to_string(), written.to_string());
            }
        }
        for written in terms {
            let written = written.trim();
            if written.is_empty() {
                continue;
            }
            let spoken = derive_spoken_form(written);
            // Only useful when the spoken form differs from the written one;
            // "kubectl" -> "kubectl" is a no-op for Tier 1 but still seeds Tier 2.
            if !spoken.is_empty() {
                map.entry(spoken).or_insert_with(|| written.to_string());
            }
        }
        let mut explicit_aliases = pairs
            .iter()
            .map(|pair| (pair, 2_u8))
            .chain(learned_pairs.iter().map(|pair| (pair, 1_u8)))
            .filter_map(|((spoken, written), priority)| {
                let spoken = normalize_alias(spoken);
                let written = written.trim();
                (!spoken.is_empty() && !written.is_empty()).then(|| ExplicitAlias {
                    spoken: spoken.chars().collect(),
                    written: written.to_string(),
                    priority,
                })
            })
            .collect::<Vec<_>>();
        // Canonical written forms share the protected rule set. This makes
        // explicit precedence idempotent: a second application cannot feed a
        // canonical such as "standard error" into the lower-priority builtin
        // that would otherwise rewrite it to "stderr".
        explicit_aliases.extend(
            pairs
                .iter()
                .map(|pair| (pair, 2_u8))
                .chain(learned_pairs.iter().map(|pair| (pair, 1_u8)))
                .filter_map(|((_, written), priority)| {
                    let spoken = normalize_alias(written);
                    let written = written.trim();
                    (!spoken.is_empty() && !written.is_empty()).then(|| ExplicitAlias {
                        spoken: spoken.chars().collect(),
                        written: written.to_string(),
                        priority,
                    })
                }),
        );
        explicit_aliases.sort_by(|left, right| {
            right
                .spoken
                .len()
                .cmp(&left.spoken.len())
                .then_with(|| left.spoken.cmp(&right.spoken))
                .then_with(|| right.priority.cmp(&left.priority))
                .then_with(|| left.written.cmp(&right.written))
        });
        explicit_aliases.dedup_by(|left, right| left.spoken == right.spoken);
        let numeric_aliases = explicit_aliases
            .iter()
            .filter(|alias| alias.spoken.iter().any(char::is_ascii_digit))
            .cloned()
            .collect();

        // Tier-1 automaton: patterns are spoken forms that actually rewrite to
        // something different. Identical spoken==written pairs are skipped here
        // (nothing to replace) but kept for Tier-2 below.
        let mut patterns: Vec<String> = Vec::new();
        let mut replacements: Vec<String> = Vec::new();
        let mut derived_patterns = map.iter().collect::<Vec<_>>();
        derived_patterns.sort_by(
            |(left_spoken, left_written), (right_spoken, right_written)| {
                right_spoken
                    .len()
                    .cmp(&left_spoken.len())
                    .then_with(|| left_spoken.cmp(right_spoken))
                    .then_with(|| left_written.cmp(right_written))
            },
        );
        for (spoken, written) in derived_patterns {
            if !spoken.eq_ignore_ascii_case(written) {
                patterns.push(spoken.clone());
                replacements.push(written.clone());
            }
        }
        let ac = if patterns.is_empty() {
            None
        } else {
            AhoCorasick::builder()
                .match_kind(MatchKind::LeftmostLongest)
                .ascii_case_insensitive(true)
                .build(&patterns)
                .ok()
        };

        // Tier-2 term list. Vocab is small (personal lists / capped code scans), so
        // a direct bounded scan per token is sub-millisecond and avoids the recall
        // gap of an exact-phonetic-key index (a 1-edit mishear changes the key).
        let mut terms: Vec<Term> = Vec::new();
        let mut written_lower = std::collections::HashSet::new();
        for (spoken, written) in &map {
            written_lower.insert(written.to_lowercase());
            terms.push(Term {
                written: written.clone(),
                spoken: spoken.clone(),
                fuzzy_eligible: is_fuzzy_eligible(written),
            });
        }

        CorrectionMatcher {
            explicit_aliases,
            ac,
            replacements,
            fuzzy,
            terms,
            written_lower,
            numeric_aliases,
            homophone_terms: Vec::new(),
        }
    }

    /// Attach the user vocabulary spellings the homophone context pass may
    /// offer as sounds-like targets. Deterministic tiers never read them.
    pub(crate) fn with_homophone_terms(mut self, terms: Vec<String>) -> Self {
        self.homophone_terms = terms;
        self
    }

    pub(crate) fn homophone_terms(&self) -> &[String] {
        &self.homophone_terms
    }

    pub(crate) fn has_only_contextual_rules(&self) -> bool {
        self.explicit_aliases.is_empty() && self.ac.is_none() && self.terms.is_empty()
    }

    /// The built-in contextual rules make every matcher actionable, even when
    /// no user vocabulary or fuzzy terms are configured.
    pub fn is_empty(&self) -> bool {
        // The matcher always carries the bounded built-in contextual rules
        // below, even when the user has no vocabulary entries.
        false
    }

    /// Apply every tier without recording context: only cue-word rules can
    /// establish a contextual meaning.
    pub fn apply(&self, text: &str) -> String {
        self.apply_in(text, CorrectionContext::default())
    }

    /// Apply explicit aliases, contextual rules, Tier 1, then (if enabled)
    /// Tier 2 to `text`. Hot path: a few linear scans over a short transcript.
    pub(crate) fn apply_in(&self, text: &str, context: CorrectionContext) -> String {
        self.apply_traced(text, context).0
    }

    /// [`apply_in`](Self::apply_in), also returning the output byte ranges
    /// written by explicit aliases and learned rules (canonical forms
    /// included). Later stages treat those spans as literal.
    pub(crate) fn apply_traced(
        &self,
        text: &str,
        context: CorrectionContext,
    ) -> (String, Vec<Range<usize>>) {
        let tokens = context_tokens(text);
        let mut output = String::with_capacity(text.len());
        let mut explicit = Vec::new();
        for (range, protected) in explicit_segments(&self.explicit_aliases, text, |_| true) {
            match protected {
                Some(written) => {
                    let start = output.len();
                    output.push_str(&written);
                    explicit.push(start..output.len());
                }
                None => {
                    let contextual = apply_context_rules(text, range, &tokens, context);
                    let exact = self.apply_tier1(&contextual);
                    if self.fuzzy {
                        output.push_str(&self.apply_tier2(&exact));
                    } else {
                        output.push_str(&exact);
                    }
                }
            }
        }
        (output, explicit)
    }

    /// Re-apply explicit aliases whose spoken form contains a digit after the
    /// spoken-numbers stage, so a rule learned from final text such as
    /// `GPT 5` matches `GPT five`. A match must overlap a span that stage just
    /// `rendered` and must not touch a `literal` span an earlier tier wrote;
    /// text Smart Correction already saw is never matched twice.
    pub(crate) fn apply_numeric_aliases(
        &self,
        text: &str,
        rendered: &[Range<usize>],
        literal: &[Range<usize>],
    ) -> String {
        if self.numeric_aliases.is_empty() || rendered.is_empty() {
            return text.to_string();
        }
        let overlaps = |spans: &[Range<usize>], range: &Range<usize>| {
            spans
                .iter()
                .any(|span| range.start < span.end && span.start < range.end)
        };
        explicit_segments(&self.numeric_aliases, text, |range| {
            overlaps(rendered, &range) && !overlaps(literal, &range)
        })
        .into_iter()
        .map(|(range, protected)| protected.unwrap_or_else(|| text[range].to_string()))
        .collect()
    }

    /// Tier 1: single Aho-Corasick pass, replacing only word-boundary matches.
    fn apply_tier1(&self, text: &str) -> String {
        let ac = match &self.ac {
            Some(ac) => ac,
            None => return text.to_string(),
        };
        let bytes = text.as_bytes();
        let mut out = String::with_capacity(text.len());
        let mut last = 0usize;
        for m in ac.find_iter(text) {
            let (s, e) = (m.start(), m.end());
            // Word-boundary guard: char before/after the match must not be an
            // ASCII alphanumeric, so "use effect" doesn't fire inside "abuse
            // effective". Spoken forms can contain internal spaces, which is fine.
            let before_ok = s == 0 || !bytes[s - 1].is_ascii_alphanumeric();
            let after_ok = e == bytes.len() || !bytes[e].is_ascii_alphanumeric();
            if before_ok && after_ok {
                out.push_str(&text[last..s]);
                out.push_str(&self.replacements[m.pattern().as_usize()]);
                last = e;
            }
        }
        out.push_str(&text[last..]);
        out
    }

    /// Tier 2: for each 1- and 2-word window, if it phonetically matches a vocab
    /// term within the edit-distance cutoff (and isn't already a written form),
    /// rewrite it. Single left-to-right pass; longer (2-word) windows win.
    fn apply_tier2(&self, text: &str) -> String {
        // Tokenize into (word, byte_start, byte_end), splitting on non-alphanumeric
        // so punctuation is preserved in the gaps.
        let tokens = tokenize(text);
        if tokens.is_empty() {
            return text.to_string();
        }

        let mut out = String::with_capacity(text.len());
        let mut last = 0usize; // byte cursor into `text`
        let mut i = 0usize;
        while i < tokens.len() {
            // Try a 2-word window first, then a 1-word window.
            let mut applied = false;
            for span in [2usize, 1] {
                if i + span > tokens.len() {
                    continue;
                }
                let start = tokens[i].1;
                let end = tokens[i + span - 1].2;
                let phrase = &text[start..end];
                if let Some(written) = self.fuzzy_lookup(phrase) {
                    out.push_str(&text[last..start]);
                    out.push_str(&written);
                    last = end;
                    i += span;
                    applied = true;
                    break;
                }
            }
            if !applied {
                i += 1;
            }
        }
        out.push_str(&text[last..]);
        out
    }

    /// Return the written form if `phrase` is a sounds-like match for a vocab term.
    ///
    /// A direct bounded scan: skip terms whose length already exceeds the edit
    /// cutoff, then accept a term only when it is within the cutoff AND either the
    /// edit is tiny (≤1) or the phonetic keys agree — the phonetic gate keeps real
    /// English (which won't sound like the vocab term) from being rewritten.
    fn fuzzy_lookup(&self, phrase: &str) -> Option<String> {
        let lower = phrase.to_lowercase();
        // Never touch a token that is already a written form (case-insensitive).
        if self.written_lower.contains(&lower) {
            return None;
        }
        // Short phrases collide too easily — skip them (Tier 1 still does exact).
        if lower.len() < MIN_FUZZY_LEN {
            return None;
        }
        let phrase_key = phonetic_key_phrase(&lower);
        let mut best: Option<(usize, &Term)> = None; // (distance, term)
        for term in &self.terms {
            // Exact spoken match is Tier 1's job; skip here.
            if term.spoken == lower {
                continue;
            }
            // Only structured identifiers are fuzzy-eligible. Plain words (e.g.
            // "Errorf", "kubectl") are exact-only: their spoken forms sit one edit
            // from ordinary English ("error", "cube cuddle"), so fuzzy-matching them
            // over-corrects. Structured terms (camelCase/snake_case/digit) derive to
            // multi-word spoken forms that don't collide with single English words.
            if !term.fuzzy_eligible {
                continue;
            }
            // Don't fuzzy-match against short vocab terms (same collision risk).
            if term.spoken.len() < MIN_FUZZY_LEN {
                continue;
            }
            // Cutoff scales with the term length so short words need a near-exact
            // match while longer phrases tolerate up to FUZZY_MAX_DIST edits.
            let cutoff = FUZZY_MAX_DIST.min(1 + term.spoken.len() / 4);
            if lower.len().abs_diff(term.spoken.len()) > cutoff {
                continue;
            }
            let dist = levenshtein(&lower, &term.spoken);
            if dist > cutoff {
                continue;
            }
            let phonetic_ok = dist <= 1 || phrase_key == phonetic_key_phrase(&term.spoken);
            if phonetic_ok && best.is_none_or(|(d, _)| dist < d) {
                best = Some((dist, term));
            }
        }
        best.map(|(_, t)| t.written.clone())
    }
}

/// Split `text` into explicit-alias spans (`Some(written)`, protected from
/// lower tiers) and mutable gaps (`None`). `accept` can refuse a match span.
fn explicit_segments(
    aliases: &[ExplicitAlias],
    text: &str,
    accept: impl Fn(Range<usize>) -> bool,
) -> Vec<(Range<usize>, Option<String>)> {
    if aliases.is_empty() {
        return vec![(0..text.len(), None)];
    }

    let mut segments = Vec::new();
    let mut mutable_start = 0usize;
    let mut scan = 0usize;
    while scan < text.len() {
        let before_ok = text[..scan]
            .chars()
            .next_back()
            .is_none_or(|character| !character.is_alphanumeric());
        let matched = before_ok
            .then(|| {
                aliases.iter().find_map(|alias| {
                    let end = explicit_alias_match_end(text, scan, &alias.spoken)?;
                    let after_ok = text[end..]
                        .chars()
                        .next()
                        .is_none_or(|character| !character.is_alphanumeric());
                    (after_ok && accept(scan..end)).then(|| (end, alias.written.clone()))
                })
            })
            .flatten();
        if let Some((end, written)) = matched {
            if mutable_start < scan {
                segments.push((mutable_start..scan, None));
            }
            segments.push((scan..end, Some(written)));
            scan = end;
            mutable_start = end;
        } else {
            scan += text[scan..]
                .chars()
                .next()
                .expect("scan always points at a character boundary")
                .len_utf8();
        }
    }
    if mutable_start < text.len() {
        segments.push((mutable_start..text.len(), None));
    }
    segments
}

/// Rewrite built-in contextual variants inside `range`. Cue windows read the
/// whole clause, including explicitly corrected spans, so a nearby cue is
/// visible even across an alias.
fn apply_context_rules(
    text: &str,
    range: Range<usize>,
    tokens: &[ContextToken],
    context: CorrectionContext,
) -> String {
    let first = tokens.partition_point(|token| token.start < range.start);
    let end = tokens.partition_point(|token| token.end <= range.end);
    let mut replacements = Vec::new();
    let mut index = first;
    while index < end {
        let mut matched: Option<(usize, &str)> = None;
        let mut ambiguous = false;
        for rule in BUILTIN_CONTEXT_RULES.iter() {
            let length = rule.spoken.len();
            if matched.is_some_and(|(matched_length, _)| length < matched_length) {
                break;
            }
            if index + length > end
                || !phrase_at(text, tokens, index, &rule.spoken)
                || !rule
                    .requirement
                    .allows(context, tokens, index, index + length)
            {
                continue;
            }
            match matched {
                None => matched = Some((length, rule.written)),
                Some((_, written)) if written != rule.written => ambiguous = true,
                Some(_) => {}
            }
        }
        match matched {
            Some((length, written)) if !ambiguous => {
                replacements.push((tokens[index].start, tokens[index + length - 1].end, written));
                index += length;
            }
            _ => index += 1,
        }
    }

    if replacements.is_empty() {
        return text[range].to_string();
    }
    let mut output = String::with_capacity(range.len());
    let mut copied = range.start;
    for (start, end, written) in replacements {
        output.push_str(&text[copied..start]);
        output.push_str(written);
        copied = end;
    }
    output.push_str(&text[copied..range.end]);
    output
}

/// Whether the space-separated lowercase phrase `spoken` starts at token `index`.
fn phrase_at(text: &str, tokens: &[ContextToken], index: usize, spoken: &[String]) -> bool {
    spoken.iter().enumerate().all(|(offset, word)| {
        tokens.get(index + offset).is_some_and(|token| {
            token.lower == *word
                && (offset == 0
                    || text[tokens[index + offset - 1].end..token.start]
                        .chars()
                        .all(|character| matches!(character, ' ' | '\t')))
        })
    })
}

/// Phonetic key for a (possibly multi-word) phrase: per-token keys joined by
/// spaces, so "re pivot" and "red pivot" stay comparable token-by-token.
fn phonetic_key_phrase(phrase: &str) -> String {
    phrase
        .split_whitespace()
        .map(phonetic_key)
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn normalize_alias(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Apply one reviewed learned replacement with the same case-insensitive,
/// Unicode-aware whole-term contract used by the immutable correction matcher.
/// The count is returned for Correct and Teach review; no matcher state is
/// mutated and no content leaves the caller.
pub(crate) fn preview_exact_replacement(
    text: &str,
    source: &str,
    replacement: &str,
) -> (String, u32) {
    let spoken = normalize_alias(source).chars().collect::<Vec<_>>();
    if spoken.is_empty() {
        return (text.to_string(), 0);
    }

    let mut output = String::with_capacity(text.len());
    let mut copy_start = 0usize;
    let mut scan = 0usize;
    let mut count = 0u32;
    while scan < text.len() {
        let before_ok = text[..scan]
            .chars()
            .next_back()
            .is_none_or(|character| !character.is_alphanumeric());
        let matched_end = before_ok
            .then(|| explicit_alias_match_end(text, scan, &spoken))
            .flatten()
            .filter(|end| {
                text[*end..]
                    .chars()
                    .next()
                    .is_none_or(|character| !character.is_alphanumeric())
            });
        if let Some(end) = matched_end {
            output.push_str(&text[copy_start..scan]);
            output.push_str(replacement);
            count = count.saturating_add(1);
            scan = end;
            copy_start = end;
        } else {
            scan += text[scan..]
                .chars()
                .next()
                .expect("scan always points at a character boundary")
                .len_utf8();
        }
    }
    output.push_str(&text[copy_start..]);
    (output, count)
}

/// Split a written identifier into its likely spoken form: lowercase words joined
/// by spaces. Handles camelCase, PascalCase, snake_case, kebab-case, and
/// letter↔digit boundaries. `useEffect` → "use effect", `rePivot` → "re pivot",
/// `parse_wav` → "parse wav", `large_v3` → "large v 3".
pub fn derive_spoken_form(written: &str) -> String {
    let mut words: Vec<String> = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = written.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if c == '_' || c == '-' || c == ' ' || c == '.' || c == '/' {
            if !cur.is_empty() {
                words.push(std::mem::take(&mut cur));
            }
            continue;
        }
        if i > 0 && !cur.is_empty() {
            let prev = chars[i - 1];
            // Boundary on lower→Upper (camelCase) or letter↔digit transitions.
            let upper_after_lower = c.is_ascii_uppercase() && prev.is_ascii_lowercase();
            let digit_boundary = c.is_ascii_digit() != prev.is_ascii_digit();
            // Boundary on Upper→Upper→lower run (e.g. "JSONParser" -> "JSON Parser").
            let acronym_end = c.is_ascii_lowercase()
                && prev.is_ascii_uppercase()
                && i >= 2
                && chars[i - 2].is_ascii_uppercase();
            if upper_after_lower || digit_boundary || acronym_end {
                if acronym_end {
                    // Move the last char (start of new word) out of `cur`.
                    let last = cur.pop().unwrap();
                    words.push(std::mem::take(&mut cur));
                    cur.push(last);
                } else {
                    words.push(std::mem::take(&mut cur));
                }
            }
        }
        cur.push(c.to_ascii_lowercase());
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    words.retain(|w| !w.is_empty());
    words.join(" ")
}

/// A written form is eligible for Tier-2 fuzzy matching only when it carries code
/// *structure*: a camelCase boundary (a lowercase immediately followed by an
/// uppercase), an underscore, or a digit. Plain words ("Errorf", "Record",
/// "kubectl") AND all-caps acronyms ("NOOP", "HTTP") are exact-only — they derive to
/// single-word spoken forms that sit one edit from ordinary English (say "error",
/// get "Errorf"), so fuzzy-matching them over-corrects. Structured identifiers derive
/// to distinctive multi-word spoken forms and are safe to fuzzy-match.
///
/// Note: "uppercase anywhere after the first char" is NOT enough — an all-caps
/// acronym satisfies it but has no real word boundary, so we require lower→upper.
fn is_fuzzy_eligible(written: &str) -> bool {
    let has_underscore = written.contains('_');
    let has_digit = written.bytes().any(|c| c.is_ascii_digit());
    let b = written.as_bytes();
    let has_camel_boundary =
        (1..b.len()).any(|i| b[i].is_ascii_uppercase() && b[i - 1].is_ascii_lowercase());
    has_underscore || has_digit || has_camel_boundary
}

/// Split text into word tokens with their byte spans. Punctuation and whitespace
/// become the gaps between tokens. An underscore counts as a word char, so a
/// snake_case identifier stays a single token — this keeps Tier 2 from
/// re-fragmenting a written form that Tier 1 already produced (e.g. `error_message`)
/// and fuzzy-rewriting one of its bare sub-tokens (`error` -> `Errorf`). Whole
/// snake_case tokens fall through to the `written_lower` guard in [`fuzzy_lookup`].
fn tokenize(text: &str) -> Vec<(&str, usize, usize)> {
    let is_word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if is_word(bytes[i]) {
            let start = i;
            while i < bytes.len() && is_word(bytes[i]) {
                i += 1;
            }
            out.push((&text[start..i], start, i));
        } else {
            i += 1;
        }
    }
    out
}

/// A compact phonetic key for sounds-like matching. Not full Metaphone — a
/// pragmatic encoding that collapses common homophones: lowercased, vowels after
/// the first letter dropped, voiced/unvoiced and near-equivalent consonant groups
/// merged, doubles collapsed. "red"→"RT", "re"→"R", "pivot"→"PFT", "fone"→"FN",
/// "phone"→"FN". Empty for non-alphabetic input.
pub fn phonetic_key(word: &str) -> String {
    let w: Vec<char> = word
        .chars()
        .filter(|c| c.is_ascii_alphabetic())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    if w.is_empty() {
        return String::new();
    }
    let code = |c: char| -> char {
        match c {
            'b' | 'p' | 'f' | 'v' => 'F',
            'c' | 'g' | 'j' | 'k' | 'q' | 'x' | 'z' | 's' => 'S',
            'd' | 't' => 'T',
            'l' => 'L',
            'm' | 'n' => 'N',
            'r' => 'R',
            // h/w/y are near-silent for homophone purposes ("phone"=="fone",
            // "rite"=="write") — treat them like vowels and drop them.
            _ => '_', // vowels + h/w/y
        }
    };
    let mut out = String::new();
    // Keep the first letter's class (or the vowel itself, uppercased) as an anchor.
    let first = w[0];
    out.push(if "aeiou".contains(first) {
        first.to_ascii_uppercase()
    } else {
        code(first)
    });
    let mut prev = out.chars().next().unwrap();
    for &c in &w[1..] {
        let k = code(c);
        if k == '_' {
            // drop interior vowels
            prev = '_';
            continue;
        }
        if k != prev {
            out.push(k);
        }
        prev = k;
    }
    out
}

/// Classic Levenshtein edit distance (two-row DP). Used on short tokens only.
pub fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, &ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matcher(terms: &[&str]) -> CorrectionMatcher {
        let terms: Vec<String> = terms.iter().map(|s| s.to_string()).collect();
        // include_builtins = true so the abbrev tests (stderr/stdin) exercise them.
        CorrectionMatcher::build(&terms, &[], true, true)
    }

    #[test]
    fn reviewed_replacement_preview_uses_production_boundaries() {
        let (preview, count) =
            preview_exact_replacement("Neo, NEO and neolithic José", "neo", "Nijo");
        assert_eq!(preview, "Nijo, Nijo and neolithic José");
        assert_eq!(count, 2);

        let (preview, count) =
            preview_exact_replacement("use   recording state", "use recording state", "hook");
        assert_eq!(preview, "use   recording state");
        assert_eq!(count, 0);

        let (preview, count) = preview_exact_replacement("JOSÉ josé", "José", "George");
        assert_eq!(preview, "George George");
        assert_eq!(count, 2);
    }

    // ---- derive_spoken_form ----

    #[test]
    fn derive_camel_case() {
        assert_eq!(derive_spoken_form("useEffect"), "use effect");
        assert_eq!(derive_spoken_form("rePivot"), "re pivot");
        assert_eq!(derive_spoken_form("getElementById"), "get element by id");
    }

    #[test]
    fn derive_snake_and_kebab() {
        assert_eq!(derive_spoken_form("parse_wav"), "parse wav");
        assert_eq!(derive_spoken_form("code-aware"), "code aware");
    }

    #[test]
    fn derive_digit_boundary() {
        assert_eq!(derive_spoken_form("large_v3"), "large v 3");
        assert_eq!(derive_spoken_form("utf8"), "utf 8");
    }

    #[test]
    fn derive_acronym_run() {
        assert_eq!(derive_spoken_form("JSONParser"), "json parser");
    }

    // ---- Tier 1 (exact) ----

    #[test]
    fn tier1_camel_case_fix() {
        let m = matcher(&["useEffect"]);
        assert_eq!(m.apply("call use effect here"), "call useEffect here");
    }

    #[test]
    fn tier1_builtin_abbrev() {
        let m = matcher(&[]);
        assert_eq!(m.apply("write to standard error"), "write to stderr");
    }

    #[test]
    fn tier1_respects_word_boundary() {
        let m = matcher(&["useEffect"]);
        // "use effect" inside a larger word must not fire.
        assert_eq!(
            m.apply("abuse effective tactics"),
            "abuse effective tactics"
        );
    }

    #[test]
    fn tier1_longest_match_wins() {
        let m = matcher(&["stdin"]);
        // builtin "standard input" (2 words) beats "standard in" overlap.
        assert_eq!(
            m.apply("read from standard input now"),
            "read from stdin now"
        );
    }

    // ---- Tier 2 (sounds-like) ----

    #[test]
    fn tier2_phonetic_mishear() {
        let m = matcher(&["rePivot"]);
        // ASR misheard "re pivot" as "red pivot"; phonetic + edit-distance recovers it.
        assert_eq!(
            m.apply("then red pivot the layout"),
            "then rePivot the layout"
        );
    }

    #[test]
    fn tier2_short_words_not_over_corrected() {
        // "git" in vocab must NOT rewrite the common word "get" (both 3 chars,
        // 1 edit apart, same phonetic key) — the length floor protects this.
        let m = matcher(&["git"]);
        assert_eq!(m.apply("please get the file"), "please get the file");
    }

    #[test]
    fn tier2_leaves_plain_english_alone() {
        let m = matcher(&["rePivot"]);
        // No vocab term near "the red car" -> untouched.
        assert_eq!(m.apply("the red car drove"), "the red car drove");
    }

    #[test]
    fn tier2_does_not_refragment_tier1_snake_case() {
        // Tier 1 turns "error message" -> error_message. Tier 2 must NOT then split
        // on the underscore and fuzzy-rewrite the bare "error" into the near vocab
        // term "Errorf" (1 edit away) — which previously produced "Errorf_message".
        // The whole snake_case token now hits the written_lower guard and is left alone.
        let m = CorrectionMatcher::build(
            &["error_message".to_string(), "Errorf".to_string()],
            &[],
            true, // fuzzy on
            false,
        );
        assert_eq!(
            m.apply("log the error message now"),
            "log the error_message now"
        );
    }

    #[test]
    fn tier2_unstructured_term_is_exact_only() {
        // Plain lowercase identifiers (no camelCase / underscore / digit) are NOT
        // fuzzy-eligible. "kubectl" still exact-matches via Tier 1, but a mishear
        // "kubecto" is left alone — the precision trade that kills "error"->"Errorf".
        let m = matcher(&["kubectl"]);
        assert_eq!(m.apply("run kubecto apply"), "run kubecto apply");
        // Exact spoken form still corrects (Tier 1 path is unaffected).
        assert_eq!(m.apply("run kubectl apply"), "run kubectl apply");
    }

    #[test]
    fn tier2_unstructured_term_does_not_overcorrect_english() {
        // The real-world bug: vocab term "Errorf" (Go's Errorf, no structure) sits one
        // edit from the common word "error". Unstructured terms are exact-only now, so
        // dictating "error" is left as English instead of flipping to "Errorf".
        let m = CorrectionMatcher::build(&["Errorf".to_string()], &[], true, false);
        assert_eq!(m.apply("log the error now"), "log the error now");
    }

    #[test]
    fn tier2_structured_term_still_fuzzes() {
        // Structured identifiers remain fuzzy-eligible: a multi-word mishear of a
        // camelCase term is still recovered.
        let m = matcher(&["rePivot"]);
        assert_eq!(m.apply("then red pivot now"), "then rePivot now");
    }

    #[test]
    fn is_fuzzy_eligible_classifies_structure() {
        assert!(is_fuzzy_eligible("rePivot")); // camel boundary
        assert!(is_fuzzy_eligible("variable_name")); // underscore
        assert!(is_fuzzy_eligible("large_v3")); // digit
        assert!(is_fuzzy_eligible("XCTAssertEqual")); // camel boundary (t->E)
        assert!(!is_fuzzy_eligible("Errorf")); // leading cap only
        assert!(!is_fuzzy_eligible("kubectl")); // plain lowercase
        assert!(!is_fuzzy_eligible("Record")); // leading cap only
        assert!(!is_fuzzy_eligible("noop")); // plain
        assert!(!is_fuzzy_eligible("NOOP")); // all-caps acronym, no lower->upper
        assert!(!is_fuzzy_eligible("HTTP")); // all-caps acronym
    }

    #[test]
    fn fuzzy_disabled_skips_tier2() {
        let terms = vec!["rePivot".to_string()];
        let m = CorrectionMatcher::build(&terms, &[], false, true);
        assert_eq!(m.apply("then red pivot now"), "then red pivot now");
    }

    // ---- explicit pairs + empties ----

    #[test]
    fn explicit_pair_overrides() {
        let pairs = vec![("the thing".to_string(), "TheThing".to_string())];
        let m = CorrectionMatcher::build(&[], &pairs, true, false);
        assert_eq!(m.apply("use the thing today"), "use TheThing today");
    }

    #[test]
    fn explicit_aliases_cover_casing_punctuation_unicode_and_multiword() {
        let pairs = vec![
            ("Tori".to_string(), "Tauri".to_string()),
            ("Tory command".to_string(), "Tauri CLI".to_string()),
            ("munchen".to_string(), "München".to_string()),
        ];
        let m = CorrectionMatcher::build(&[], &pairs, true, false);
        assert_eq!(m.apply("TORI, tori!"), "Tauri, Tauri!");
        assert_eq!(m.apply("use tory command now"), "use Tauri CLI now");
        assert_eq!(m.apply("visit MUNCHEN."), "visit München.");
        assert_eq!(m.apply("İstanbul and Tori"), "İstanbul and Tauri");
        assert_eq!(
            m.apply("editorial and territory"),
            "editorial and territory"
        );
        assert_eq!(m.apply("Tauri"), "Tauri");
        assert_eq!(m.apply(&m.apply("Tori")), "Tauri");
    }

    #[test]
    fn explicit_alias_output_is_protected_from_lower_precedence_tiers() {
        let pairs = vec![
            ("std spoken".to_string(), "standard error".to_string()),
            ("hook spoken".to_string(), "use effect".to_string()),
        ];
        let m = CorrectionMatcher::build(&["useEffect".to_string()], &pairs, true, true);
        assert_eq!(
            m.apply("std spoken then standard out"),
            "standard error then stdout"
        );
        assert_eq!(
            m.apply("hook spoken then use effect"),
            "use effect then use effect"
        );
        assert_eq!(m.apply(&m.apply("std spoken")), "standard error");
    }

    #[test]
    fn explicit_alias_longest_match_is_stable() {
        let pairs = vec![
            ("tory".to_string(), "Tauri".to_string()),
            ("tory command".to_string(), "Tauri CLI".to_string()),
        ];
        let m = CorrectionMatcher::build(&[], &pairs, false, false);
        assert_eq!(m.apply("tory command"), "Tauri CLI");
    }

    #[test]
    fn matcher_without_vocabulary_keeps_unrelated_prose_unchanged() {
        let m = CorrectionMatcher::build(&[], &[], true, false);
        assert!(!m.is_empty());
        assert_eq!(m.apply("nothing to do here"), "nothing to do here");
    }

    #[test]
    fn contextual_main_branch_homophones_recover_live_repro_phrases() {
        let m = CorrectionMatcher::build(&[], &[], true, false);
        assert_eq!(
            m.apply("Make sure you get the latest change from Maine."),
            "Make sure you get the latest change from main."
        );
        assert_eq!(
            m.apply("Make sure you get the latest changes from Maine."),
            "Make sure you get the latest changes from main."
        );
        assert_eq!(
            m.apply("Can you go check out the changes on me?"),
            "Can you go check out the changes on main?"
        );
        assert_eq!(
            m.apply("Can you go and check out the changes on Maine?"),
            "Can you go and check out the changes on main?"
        );
        assert_eq!(m.apply("Use the Maine branch."), "Use the main branch.");
    }

    #[test]
    fn contextual_main_branch_homophones_preserve_geography_and_ambiguity() {
        let m = CorrectionMatcher::build(&[], &[], true, false);
        for text in [
            "Have you ever been to Maine?",
            "Do you want to go to Maine?",
            "What about visiting the state of Maine?",
            "The latest climate changes in Maine are concerning.",
            "Check out the changes from Maine's legislature.",
            "Please get the latest changes from me.",
            "Switch to me when you are finished.",
            "Switch the destination to Maine.",
            "Visit the Maine branch office.",
        ] {
            assert_eq!(m.apply(text), text);
        }
    }

    #[test]
    fn explicit_aliases_override_contextual_main_branch_homophones() {
        let aliases = vec![("Maine".to_string(), "Maine".to_string())];
        let m = CorrectionMatcher::build(&[], &aliases, true, false);
        assert_eq!(
            m.apply("Check out the latest changes from Maine."),
            "Check out the latest changes from Maine."
        );
    }

    fn in_register(register: TextRegister) -> CorrectionContext {
        CorrectionContext {
            register,
            project_context: false,
        }
    }

    #[test]
    fn builtin_tauri_variant_needs_a_technical_register_project_or_cue() {
        let m = CorrectionMatcher::build(&[], &[], true, false);
        let everyday = CorrectionContext::default();
        // A person named Tori is untouched in ordinary prose.
        for text in [
            "Tori is coming over for dinner.",
            "Ask Tori about the weekend.",
            "My friend Tori is building an app.",
            "Hey Tori, the build is green.",
            "The Tory party lost the vote.",
        ] {
            assert_eq!(m.apply_in(text, everyday), text);
        }
        assert_eq!(
            m.apply_in("The Tori plugin needs a rust update.", everyday),
            "The Tauri plugin needs a rust update."
        );
        assert_eq!(
            m.apply_in("Ship the Tory app build", everyday),
            "Ship the Tauri app build"
        );
        for register in [TextRegister::Technical, TextRegister::Terminal] {
            assert_eq!(m.apply_in("open Tori", in_register(register)), "open Tauri");
        }
        for register in [
            TextRegister::Messages,
            TextRegister::Email,
            TextRegister::Notes,
        ] {
            assert_eq!(m.apply_in("open Tori", in_register(register)), "open Tori");
        }
        let project = CorrectionContext {
            register: TextRegister::Everyday,
            project_context: true,
        };
        assert_eq!(m.apply_in("open Tori", project), "open Tauri");
        // A cue and a counter-cue are ambiguous, even in Technical mode.
        assert_eq!(
            m.apply_in(
                "Tori said she will review the rust plugin",
                in_register(TextRegister::Technical)
            ),
            "Tori said she will review the rust plugin"
        );
        assert_eq!(
            m.apply_in(&m.apply_in("open Tori", project), project),
            "open Tauri"
        );
    }

    #[test]
    fn builtin_cue_windows_stop_at_clause_boundaries() {
        // Review B2: a cue in the previous sentence no longer reaches Tori.
        let m = CorrectionMatcher::build(&[], &[], true, false);
        let everyday = CorrectionContext::default();
        for text in [
            "Ship the app. Tori is coming over for dinner.",
            "Ship the app! Tori is coming over.",
            "Did the build pass? Tori is here.",
            "Update the plugin; Tori called.",
            "Rust is fun\nTori is coming over.",
        ] {
            assert_eq!(m.apply_in(text, everyday), text);
        }
        // A dot inside a file name or version is not a clause boundary.
        assert_eq!(
            m.apply_in("The Tori config lives in app.json", everyday),
            "The Tauri config lives in app.json"
        );
        assert_eq!(
            m.apply_in("Ship the app. The Tori plugin is ready.", everyday),
            "Ship the app. The Tauri plugin is ready."
        );
        // Blockers are bounded the same way.
        assert_eq!(
            m.apply_in("Thank you. Open the Tori plugin.", everyday),
            "Thank you. Open the Tauri plugin."
        );
    }

    #[test]
    fn builtin_variants_match_only_whole_unicode_words() {
        // Review B4: `Torié` and mixed-script names are not `Tori`.
        let m = CorrectionMatcher::build(&[], &[], true, false);
        for context in [
            CorrectionContext::default(),
            in_register(TextRegister::Technical),
        ] {
            for text in [
                "editorial territory Torié app",
                "Toriд plugin build",
                "Tori日本 app",
                "Maineé branch",
                "ÉTori app",
                "Torí app",
                "Tori\u{301} app",
                "Torí",
                "Tori\u{301}",
                "\u{301}Tori app",
                "Tori\u{93e} app",
                "Tori\u{20dd} app",
                "Tori² app",
            ] {
                assert_eq!(m.apply_in(text, context), text);
            }
        }
        assert_eq!(
            m.apply_in("Tori, app", in_register(TextRegister::Technical)),
            "Tauri, app"
        );
    }

    #[test]
    fn builtin_developer_pack_is_gated_to_technical_context() {
        let m = CorrectionMatcher::build(&[], &[], true, false);
        let technical = in_register(TextRegister::Technical);
        let project = CorrectionContext {
            register: TextRegister::Notes,
            project_context: true,
        };
        for (heard, written) in [
            (
                "migrate to Postgres SQL today",
                "migrate to PostgreSQL today",
            ),
            ("run npxtsc first", "run npx tsc first"),
            ("convert it with CoreML", "convert it with Core ML"),
        ] {
            assert_eq!(m.apply_in(heard, technical), written);
            assert_eq!(m.apply_in(heard, project), written);
            assert_eq!(m.apply_in(heard, CorrectionContext::default()), heard);
            assert_eq!(m.apply_in(written, technical), written);
        }
    }

    #[test]
    fn explicit_aliases_outrank_builtin_context_rules() {
        let pairs = vec![("Tori".to_string(), "Tori".to_string())];
        let m = CorrectionMatcher::build(&[], &pairs, true, false);
        assert_eq!(
            m.apply_in("open Tori", in_register(TextRegister::Technical)),
            "open Tori"
        );
    }

    #[test]
    fn numeric_aliases_match_only_freshly_rendered_digits() {
        let learned = vec![
            ("GPT 5".to_string(), "GPT-5".to_string()),
            ("20 pages".to_string(), "twenty pages".to_string()),
            ("Tori".to_string(), "Tauri".to_string()),
        ];
        let m = CorrectionMatcher::build_with_learned(&[], &[], &learned, false, false);
        let text = "GPT 5 wrote 20 pages for Tori";
        let gpt = 4..5;
        let pages = 12..14;
        assert_eq!(
            m.apply_numeric_aliases(text, &[gpt.clone(), pages.clone()], &[]),
            "GPT-5 wrote twenty pages for Tori"
        );
        // Digits the number stage did not render were already seen by Smart
        // Correction and are never matched twice.
        assert_eq!(m.apply_numeric_aliases(text, &[], &[]), text);
        assert_eq!(
            m.apply_numeric_aliases(text, std::slice::from_ref(&gpt), &[]),
            "GPT-5 wrote 20 pages for Tori"
        );
        // A match touching text an earlier tier wrote is refused.
        assert_eq!(
            m.apply_numeric_aliases(text, &[gpt, pages], std::slice::from_ref(&(0..3))),
            "GPT 5 wrote twenty pages for Tori"
        );
        let plain = CorrectionMatcher::build_with_learned(&[], &[], &learned[2..], false, false);
        assert_eq!(
            plain.apply_numeric_aliases("GPT 5", std::slice::from_ref(&(4..5)), &[]),
            "GPT 5"
        );
    }

    #[test]
    fn traced_application_reports_explicit_output_spans() {
        let pairs = vec![("page count".to_string(), "twenty pages".to_string())];
        let m = CorrectionMatcher::build(&["useEffect".to_string()], &pairs, false, false);
        let (text, explicit) =
            m.apply_traced("use effect for page count", CorrectionContext::default());
        assert_eq!(text, "useEffect for twenty pages");
        assert_eq!(
            explicit
                .iter()
                .map(|range| &text[range.clone()])
                .collect::<Vec<_>>(),
            ["twenty pages"]
        );
    }

    #[test]
    fn builtins_gated_off_when_not_dev_context() {
        // Without include_builtins, "standard error" stays as prose.
        let m = CorrectionMatcher::build(&[], &[], true, false);
        assert_eq!(
            m.apply("the standard error of the mean"),
            "the standard error of the mean"
        );
    }

    #[test]
    fn preserves_surrounding_punctuation() {
        let m = matcher(&["useEffect"]);
        assert_eq!(m.apply("(use effect)"), "(useEffect)");
    }

    // ---- phonetic + levenshtein primitives ----

    #[test]
    fn phonetic_collapses_homophones() {
        assert_eq!(phonetic_key("phone"), phonetic_key("fone"));
        assert_eq!(phonetic_key("red"), phonetic_key("read").to_string());
    }

    #[test]
    fn levenshtein_basic() {
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("same", "same"), 0);
        assert_eq!(levenshtein("", "abc"), 3);
    }
}
