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

/// A compiled, reusable correction matcher. Build once on settings-change, then
/// call [`apply`](Self::apply) per transcription.
pub struct CorrectionMatcher {
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
        // spoken(lowercased) -> written. Later inserts win, so ordering is:
        // builtin abbrevs < derived-from-terms < explicit pairs (most specific).
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
        for (spoken, written) in pairs {
            let spoken = spoken.trim().to_lowercase();
            let written = written.trim();
            if spoken.is_empty() || written.is_empty() {
                continue;
            }
            map.insert(spoken, written.to_string());
        }

        // Tier-1 automaton: patterns are spoken forms that actually rewrite to
        // something different. Identical spoken==written pairs are skipped here
        // (nothing to replace) but kept for Tier-2 below.
        let mut patterns: Vec<String> = Vec::new();
        let mut replacements: Vec<String> = Vec::new();
        for (spoken, written) in &map {
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
            ac,
            replacements,
            fuzzy,
            terms,
            written_lower,
        }
    }

    /// True when this matcher has no patterns and no fuzzy terms — the pipeline can
    /// skip the stage entirely.
    pub fn is_empty(&self) -> bool {
        self.ac.is_none() && (!self.fuzzy || self.terms.is_empty())
    }

    /// Apply Tier 1 then (if enabled) Tier 2 to `text`, returning the corrected
    /// string. Hot path: two linear scans over a short transcript.
    pub fn apply(&self, text: &str) -> String {
        let t1 = self.apply_tier1(text);
        if self.fuzzy {
            self.apply_tier2(&t1)
        } else {
            t1
        }
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
            if phonetic_ok && best.map_or(true, |(d, _)| dist < d) {
                best = Some((dist, term));
            }
        }
        best.map(|(_, t)| t.written.clone())
    }
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
/// *structure*: an internal uppercase (camelCase/PascalCase), an underscore, or a
/// digit. Plain words ("Errorf", "Record", "kubectl", "NOOP") are exact-only — their
/// spoken forms collide with ordinary English a single edit away (say "error", get
/// "Errorf"), so fuzzy-matching them over-corrects. Structured identifiers derive to
/// distinctive multi-word spoken forms and are safe to fuzzy-match.
fn is_fuzzy_eligible(written: &str) -> bool {
    let has_underscore = written.contains('_');
    let has_digit = written.bytes().any(|c| c.is_ascii_digit());
    // Uppercase anywhere after the first char => camelCase/PascalCase/acronym shape.
    let has_internal_upper = written.bytes().skip(1).any(|c| c.is_ascii_uppercase());
    has_underscore || has_digit || has_internal_upper
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
        assert_eq!(m.apply("abuse effective tactics"), "abuse effective tactics");
    }

    #[test]
    fn tier1_longest_match_wins() {
        let m = matcher(&["stdin"]);
        // builtin "standard input" (2 words) beats "standard in" overlap.
        assert_eq!(m.apply("read from standard input now"), "read from stdin now");
    }

    // ---- Tier 2 (sounds-like) ----

    #[test]
    fn tier2_phonetic_mishear() {
        let m = matcher(&["rePivot"]);
        // ASR misheard "re pivot" as "red pivot"; phonetic + edit-distance recovers it.
        assert_eq!(m.apply("then red pivot the layout"), "then rePivot the layout");
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
            true,  // fuzzy on
            false,
        );
        assert_eq!(m.apply("log the error message now"), "log the error_message now");
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
        assert!(is_fuzzy_eligible("rePivot"));        // internal upper
        assert!(is_fuzzy_eligible("variable_name"));  // underscore
        assert!(is_fuzzy_eligible("large_v3"));       // digit
        assert!(is_fuzzy_eligible("XCTAssertEqual")); // internal upper
        assert!(!is_fuzzy_eligible("Errorf"));        // leading cap only
        assert!(!is_fuzzy_eligible("kubectl"));       // plain lowercase
        assert!(!is_fuzzy_eligible("Record"));        // leading cap only
        assert!(!is_fuzzy_eligible("noop"));          // plain
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
    fn empty_matcher_is_noop() {
        // No terms, no pairs, no builtins -> genuinely empty.
        let m = CorrectionMatcher::build(&[], &[], true, false);
        assert!(m.is_empty());
        assert_eq!(m.apply("nothing to do here"), "nothing to do here");
    }

    #[test]
    fn builtins_gated_off_when_not_dev_context() {
        // Without include_builtins, "standard error" stays as prose.
        let m = CorrectionMatcher::build(&[], &[], true, false);
        assert_eq!(m.apply("the standard error of the mean"), "the standard error of the mean");
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
