//! Explicit, bounded learning from a user-edited final transcript.
//!
//! Proposal generation is side-effect free. A replacement rule can only be
//! persisted by the separate confirmation command after the UI has shown the
//! exact source, replacement, scope, and examples.

use crate::knowledge_store::KnowledgeScope;
use crate::state::DictationState;
use crate::MutexExt;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

const MAX_TRANSCRIPT_CHARS: usize = 8_192;
const MAX_DIFF_TOKENS: usize = 512;
const MAX_RULE_CHARS: usize = 256;
const MAX_RULE_TOKENS: usize = 8;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeachingContext {
    pub app_bundle_id: Option<String>,
    pub app_label: Option<String>,
    pub project_root: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CorrectionProposalRequest {
    pub original_text: String,
    pub corrected_text: String,
    pub teaching_context: Option<TeachingContext>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CorrectionScopeOption {
    pub scope: KnowledgeScope,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum CorrectionProposalOutcome {
    Proposal {
        proposal_id: u64,
        source: String,
        replacement: String,
        occurrence_count: u32,
        original_text: String,
        corrected_text: String,
        scope_options: Vec<CorrectionScopeOption>,
    },
    Unsafe {
        reason: String,
    },
}

#[derive(Debug, Clone)]
pub struct PendingCorrection {
    pub proposal_id: u64,
    pub source: String,
    pub replacement: String,
    pub scopes: Vec<KnowledgeScope>,
}

#[derive(Default)]
pub struct CorrectAndTeachState {
    next_id: AtomicU64,
    pending: Mutex<Option<PendingCorrection>>,
}

impl CorrectAndTeachState {
    pub fn propose(
        &self,
        request: CorrectionProposalRequest,
        dictation: &DictationState,
        knowledge_voice_command_phrases: &[String],
    ) -> CorrectionProposalOutcome {
        // Starting any new review invalidates the previous capability, even if
        // this request turns out to be unsafe. Only the proposal currently on
        // screen may be confirmed.
        *self.pending.lock_or_recover() = None;
        let candidate = match propose_rule(&request.original_text, &request.corrected_text) {
            Ok(candidate) => candidate,
            Err(reason) => return CorrectionProposalOutcome::Unsafe { reason },
        };

        if conflicts_with_voice_command(
            &candidate.source,
            dictation,
            knowledge_voice_command_phrases,
        ) {
            return CorrectionProposalOutcome::Unsafe {
                reason: "That phrase is reserved by Voice Commands, so Murmur will not learn a competing correction.".to_string(),
            };
        }

        let scope_options = available_scopes(request.teaching_context.as_ref(), dictation);
        let proposal_id = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        *self.pending.lock_or_recover() = Some(PendingCorrection {
            proposal_id,
            source: candidate.source.clone(),
            replacement: candidate.replacement.clone(),
            scopes: scope_options
                .iter()
                .map(|option| option.scope.clone())
                .collect(),
        });

        CorrectionProposalOutcome::Proposal {
            proposal_id,
            source: candidate.source,
            replacement: candidate.replacement,
            occurrence_count: candidate.occurrence_count,
            original_text: request.original_text,
            corrected_text: request.corrected_text,
            scope_options,
        }
    }

    pub fn confirmed(
        &self,
        proposal_id: u64,
        scope: &KnowledgeScope,
    ) -> Result<PendingCorrection, String> {
        let pending = self.pending.lock_or_recover().clone().ok_or_else(|| {
            "This correction proposal is no longer available. Review it again.".to_string()
        })?;
        if pending.proposal_id != proposal_id || !pending.scopes.contains(scope) {
            return Err(
                "The correction or its scope changed. Review it again before saving.".to_string(),
            );
        }
        Ok(pending)
    }

    pub fn discard(&self, proposal_id: u64) {
        let mut pending = self.pending.lock_or_recover();
        if pending
            .as_ref()
            .is_some_and(|proposal| proposal.proposal_id == proposal_id)
        {
            *pending = None;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RuleCandidate {
    source: String,
    replacement: String,
    occurrence_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Token<'a> {
    text: &'a str,
    start: usize,
    end: usize,
}

fn tokenize(text: &str) -> Vec<Token<'_>> {
    let mut tokens = Vec::new();
    let mut start = None;
    let mut word = false;
    for (index, character) in text.char_indices() {
        if character.is_whitespace() {
            if let Some(token_start) = start.take() {
                tokens.push(Token {
                    text: &text[token_start..index],
                    start: token_start,
                    end: index,
                });
            }
            continue;
        }
        let current_word = character.is_alphanumeric() || character == '_';
        match start {
            None => {
                start = Some(index);
                word = current_word;
            }
            Some(token_start) if word != current_word || !current_word => {
                tokens.push(Token {
                    text: &text[token_start..index],
                    start: token_start,
                    end: index,
                });
                start = Some(index);
                word = current_word;
            }
            Some(_) => {}
        }
    }
    if let Some(token_start) = start {
        tokens.push(Token {
            text: &text[token_start..],
            start: token_start,
            end: text.len(),
        });
    }
    tokens
}

fn propose_rule(original: &str, corrected: &str) -> Result<RuleCandidate, String> {
    if original == corrected {
        return Err("Make a correction before asking Murmur to learn it.".to_string());
    }
    if original.chars().count() > MAX_TRANSCRIPT_CHARS
        || corrected.chars().count() > MAX_TRANSCRIPT_CHARS
    {
        return Err("This transcript is too long for safe automatic rule extraction.".to_string());
    }
    let before = tokenize(original);
    let after = tokenize(corrected);
    if before.is_empty()
        || after.is_empty()
        || before.len() > MAX_DIFF_TOKENS
        || after.len() > MAX_DIFF_TOKENS
    {
        return Err("Murmur could not find one bounded spoken-to-written replacement.".to_string());
    }

    let matches = unique_normalized_lcs_matches(&before, &after)?;
    let mut hunks = Vec::new();
    let mut before_cursor = 0usize;
    let mut after_cursor = 0usize;
    for (before_match, after_match) in matches
        .iter()
        .copied()
        .chain(std::iter::once((before.len(), after.len())))
    {
        if before_cursor != before_match || after_cursor != after_match {
            hunks.push((before_cursor..before_match, after_cursor..after_match));
        }
        before_cursor = before_match.saturating_add(1);
        after_cursor = after_match.saturating_add(1);
    }
    if hunks.is_empty() {
        let case_only_changes = matches
            .iter()
            .filter(|(before_match, after_match)| {
                before[*before_match].text != after[*after_match].text
            })
            .copied()
            .collect::<Vec<_>>();
        if case_only_changes.len() == 1 {
            let (before_match, after_match) = case_only_changes[0];
            hunks.push((
                before_match..before_match.saturating_add(1),
                after_match..after_match.saturating_add(1),
            ));
        } else if case_only_changes.is_empty() {
            return Err("Whitespace-only edits are not learned automatically.".to_string());
        }
    }
    if hunks.len() != 1 {
        return Err("This edit changes more than one distinct span, so Murmur will not guess a reusable rule.".to_string());
    }
    let (before_range, after_range) = hunks.pop().expect("one hunk was checked");
    if before_range.is_empty() || after_range.is_empty() {
        return Err(
            "Insertions or deletions alone are not safe spoken-to-written replacement rules."
                .to_string(),
        );
    }
    if before_range.len() > MAX_RULE_TOKENS || after_range.len() > MAX_RULE_TOKENS {
        return Err("The changed phrase is too broad for an automatic learned rule.".to_string());
    }
    let source = original[before[before_range.start].start..before[before_range.end - 1].end]
        .trim()
        .to_string();
    let replacement = corrected[after[after_range.start].start..after[after_range.end - 1].end]
        .trim()
        .to_string();
    if source.chars().count() > MAX_RULE_CHARS || replacement.chars().count() > MAX_RULE_CHARS {
        return Err("The changed phrase is too long for an automatic learned rule.".to_string());
    }
    if !source.chars().any(char::is_alphanumeric) || !replacement.chars().any(char::is_alphanumeric)
    {
        return Err(
            "Punctuation or whitespace-only edits are not learned automatically.".to_string(),
        );
    }
    if collapse_whitespace(&source) == collapse_whitespace(&replacement) {
        return Err("Whitespace-only edits are not learned automatically.".to_string());
    }

    Ok(RuleCandidate {
        occurrence_count: count_occurrences(&before, &tokenize(&source)).max(1) as u32,
        source,
        replacement,
    })
}

fn unique_normalized_lcs_matches(
    before: &[Token<'_>],
    after: &[Token<'_>],
) -> Result<Vec<(usize, usize)>, String> {
    let before_normalized = before
        .iter()
        .map(|token| normalize(token.text))
        .collect::<Vec<_>>();
    let after_normalized = after
        .iter()
        .map(|token| normalize(token.text))
        .collect::<Vec<_>>();
    let columns = after.len() + 1;
    let cells = (before.len() + 1) * columns;
    let mut suffix_lengths = vec![0u16; cells];
    for left in (0..before.len()).rev() {
        for right in (0..after.len()).rev() {
            let value = if before_normalized[left] == after_normalized[right] {
                suffix_lengths[(left + 1) * columns + right + 1] + 1
            } else {
                suffix_lengths[(left + 1) * columns + right]
                    .max(suffix_lengths[left * columns + right + 1])
            };
            suffix_lengths[left * columns + right] = value;
        }
    }

    // A matching pair belongs to some optimal alignment exactly when an
    // optimal prefix plus that pair plus an optimal suffix reaches the global
    // LCS length. If the union of all such pairs contains more entries than
    // one alignment, there is more than one optimal alignment and we fail
    // closed instead of applying a tie-break.
    let mut prefix_lengths = vec![0u16; cells];
    for left in 0..before.len() {
        for right in 0..after.len() {
            prefix_lengths[(left + 1) * columns + right + 1] =
                if before_normalized[left] == after_normalized[right] {
                    prefix_lengths[left * columns + right] + 1
                } else {
                    prefix_lengths[left * columns + right + 1]
                        .max(prefix_lengths[(left + 1) * columns + right])
                };
        }
    }

    let optimal_length = usize::from(suffix_lengths[0]);
    let mut viable_matches = Vec::with_capacity(optimal_length);
    for left in 0..before.len() {
        for right in 0..after.len() {
            if before_normalized[left] != after_normalized[right] {
                continue;
            }
            let through_pair = usize::from(prefix_lengths[left * columns + right])
                + 1
                + usize::from(suffix_lengths[(left + 1) * columns + right + 1]);
            if through_pair == optimal_length {
                viable_matches.push((left, right));
            }
        }
    }

    let uniquely_ordered = viable_matches
        .windows(2)
        .all(|matches| matches[0].0 < matches[1].0 && matches[0].1 < matches[1].1);
    if viable_matches.len() != optimal_length || !uniquely_ordered {
        return Err(
            "Repeated or reordered words make this correction ambiguous, so Murmur will not guess a reusable rule."
                .to_string(),
        );
    }
    Ok(viable_matches)
}

fn normalize(value: &str) -> String {
    value
        .split_whitespace()
        .flat_map(|part| {
            part.chars()
                .flat_map(char::to_lowercase)
                .chain(std::iter::once(' '))
        })
        .collect::<String>()
        .trim()
        .to_string()
}

fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn count_occurrences(haystack: &[Token<'_>], needle: &[Token<'_>]) -> usize {
    if needle.is_empty() || needle.len() > haystack.len() {
        return 0;
    }
    haystack
        .windows(needle.len())
        .filter(|window| {
            window
                .iter()
                .zip(needle)
                .all(|(left, right)| normalize(left.text) == normalize(right.text))
        })
        .count()
}

fn conflicts_with_voice_command(
    source: &str,
    dictation: &DictationState,
    knowledge_voice_command_phrases: &[String],
) -> bool {
    let normalized = normalize(source);
    crate::voice_commands::BUILTIN_COMMAND_PHRASES
        .iter()
        .any(|phrase| normalize(phrase) == normalized)
        || dictation
            .voice_command_pairs
            .iter()
            .any(|command| normalize(&command.phrase) == normalized)
        || knowledge_voice_command_phrases
            .iter()
            .any(|phrase| normalize(phrase) == normalized)
}

fn available_scopes(
    context: Option<&TeachingContext>,
    dictation: &DictationState,
) -> Vec<CorrectionScopeOption> {
    let mut options = vec![CorrectionScopeOption {
        scope: KnowledgeScope::Global,
        label: "All apps".to_string(),
    }];
    let Some(context) = context else {
        return options;
    };
    let Some(bundle_id) = context
        .app_bundle_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return options;
    };
    options.push(CorrectionScopeOption {
        scope: KnowledgeScope::App {
            bundle_id: bundle_id.to_string(),
        },
        label: context
            .app_label
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(bundle_id)
            .to_string(),
    });

    if let Some(root) = context
        .project_root
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let valid = dictation
            .app_profiles
            .iter()
            .find(|profile| profile.bundle_id == bundle_id)
            .is_some_and(|profile| {
                profile.ide_context_enabled
                    && profile.ide_project_roots.len() == 1
                    && profile.ide_project_roots[0] == root
            });
        if valid {
            options.push(CorrectionScopeOption {
                scope: KnowledgeScope::Project {
                    bundle_id: bundle_id.to_string(),
                    root: root.to_string(),
                },
                label: root.to_string(),
            });
        }
    }
    options
}

pub fn teaching_context(
    bundle_id: Option<&str>,
    app_label: Option<&str>,
    project_root: Option<&str>,
) -> Option<TeachingContext> {
    bundle_id.map(|bundle_id| TeachingContext {
        app_bundle_id: Some(bundle_id.to_string()),
        app_label: app_label.map(str::to_string),
        project_root: project_root.map(str::to_string),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppProfile;

    #[test]
    fn extracts_casing_names_identifiers_and_phrases() {
        for (before, after, source, replacement) in [
            ("murmur is ready", "Murmur is ready", "murmur", "Murmur"),
            (
                "Ask George Neo today",
                "Ask George Nijo today",
                "Neo",
                "Nijo",
            ),
            (
                "use recording state",
                "useRecordingState",
                "use recording state",
                "useRecordingState",
            ),
            (
                "open the alpha beta panel",
                "open the release notes panel",
                "alpha beta",
                "release notes",
            ),
        ] {
            let proposal = propose_rule(before, after).unwrap();
            assert_eq!(
                (proposal.source.as_str(), proposal.replacement.as_str()),
                (source, replacement)
            );
        }
    }

    #[test]
    fn normalized_alignment_uses_unique_case_insensitive_anchors() {
        let before = tokenize("ASK RED RED NEO TODAY");
        let after = tokenize("Ask red red Nijo today");
        assert_eq!(
            unique_normalized_lcs_matches(&before, &after).unwrap(),
            vec![(0, 0), (1, 1), (2, 2), (4, 4)]
        );

        let proposal = propose_rule("ASK GEORGE NEO TODAY", "Ask George Nijo today").unwrap();
        assert_eq!(proposal.source, "NEO");
        assert_eq!(proposal.replacement, "Nijo");
        assert_eq!(proposal.occurrence_count, 1);
    }

    #[test]
    fn rejects_multiple_optimal_normalized_alignments() {
        let before = tokenize("ONE alpha one");
        let after = tokenize("one beta");
        assert!(unique_normalized_lcs_matches(&before, &after)
            .unwrap_err()
            .contains("ambiguous"));
        assert!(propose_rule("ONE alpha one", "one beta")
            .unwrap_err()
            .contains("ambiguous"));
    }

    #[test]
    fn preserves_exact_spelling_with_punctuation_and_unicode_anchors() {
        for (before, after) in [
            ("ASK GEORGE NEO, TODAY", "Ask George Nijo, today"),
            ("ASK JOSÉ NEO TODAY", "Ask José Nijo today"),
        ] {
            let proposal = propose_rule(before, after).unwrap();
            assert_eq!(proposal.source, "NEO");
            assert_eq!(proposal.replacement, "Nijo");
        }
    }

    #[test]
    fn rejects_ambiguous_multiple_edits_and_unbounded_changes() {
        assert!(propose_rule("alpha middle omega", "beta middle delta")
            .unwrap_err()
            .contains("more than one"));
        assert!(propose_rule("alpha", "alpha beta")
            .unwrap_err()
            .contains("Insertions"));
        assert!(propose_rule("alpha beta", "alpha")
            .unwrap_err()
            .contains("deletions"));
        assert!(propose_rule("hello!", "hello?")
            .unwrap_err()
            .contains("Punctuation"));
        assert!(propose_rule("alpha  beta", "alpha beta")
            .unwrap_err()
            .contains("Whitespace"));
        assert!(propose_rule("alpha beta", "beta alpha").is_err());
        assert!(propose_rule(
            "one two three four five six seven eight nine",
            "a b c d e f g h i"
        )
        .unwrap_err()
        .contains("too broad"));
        assert!(propose_rule("murmur app is ready", "Murmur App is ready")
            .unwrap_err()
            .contains("more than one"));
        assert!(propose_rule(&"a".repeat(MAX_TRANSCRIPT_CHARS + 1), "b").is_err());
    }

    #[test]
    fn project_scope_requires_one_exact_opted_in_root() {
        let state = CorrectAndTeachState::default();
        let mut dictation = DictationState::default();
        dictation.app_profiles.push(AppProfile {
            bundle_id: "com.editor".to_string(),
            label: "Editor".to_string(),
            auto_paste_override: None,
            cleanup_override: None,
            cli_formatting_override: None,
            smart_formatting_override: None,
            writing_style: None,
            ide_context_enabled: true,
            ide_project_roots: vec!["/project".to_string()],
        });
        let outcome = state.propose(
            CorrectionProposalRequest {
                original_text: "use recording state".to_string(),
                corrected_text: "useRecordingState".to_string(),
                teaching_context: Some(TeachingContext {
                    app_bundle_id: Some("com.editor".to_string()),
                    app_label: Some("Editor".to_string()),
                    project_root: Some("/project".to_string()),
                }),
            },
            &dictation,
            &[],
        );
        let CorrectionProposalOutcome::Proposal { scope_options, .. } = outcome else {
            panic!("expected proposal")
        };
        assert_eq!(scope_options.len(), 3);

        dictation.app_profiles[0]
            .ide_project_roots
            .push("/other".to_string());
        let outcome = state.propose(
            CorrectionProposalRequest {
                original_text: "use recording state".to_string(),
                corrected_text: "useRecordingState".to_string(),
                teaching_context: Some(TeachingContext {
                    app_bundle_id: Some("com.editor".to_string()),
                    app_label: None,
                    project_root: Some("/project".to_string()),
                }),
            },
            &dictation,
            &[],
        );
        let CorrectionProposalOutcome::Proposal { scope_options, .. } = outcome else {
            panic!("expected proposal")
        };
        assert_eq!(scope_options.len(), 2);
    }

    #[test]
    fn voice_command_phrases_never_become_competing_rules() {
        let state = CorrectAndTeachState::default();
        let outcome = state.propose(
            CorrectionProposalRequest {
                original_text: "new line".to_string(),
                corrected_text: "newline".to_string(),
                teaching_context: None,
            },
            &DictationState::default(),
            &[],
        );
        assert!(
            matches!(outcome, CorrectionProposalOutcome::Unsafe { reason } if reason.contains("Voice Commands"))
        );

        let outcome = state.propose(
            CorrectionProposalRequest {
                original_text: "my signature".to_string(),
                corrected_text: "Regards, George".to_string(),
                teaching_context: None,
            },
            &DictationState::default(),
            &["my signature".to_string()],
        );
        assert!(
            matches!(outcome, CorrectionProposalOutcome::Unsafe { reason } if reason.contains("Voice Commands"))
        );
    }

    #[test]
    fn confirmation_is_bound_to_reviewed_scope_and_can_be_discarded() {
        let state = CorrectAndTeachState::default();
        let outcome = state.propose(
            CorrectionProposalRequest {
                original_text: "George Neo".to_string(),
                corrected_text: "George Nijo".to_string(),
                teaching_context: None,
            },
            &DictationState::default(),
            &[],
        );
        let CorrectionProposalOutcome::Proposal { proposal_id, .. } = outcome else {
            panic!("expected proposal")
        };
        assert!(state
            .confirmed(proposal_id, &KnowledgeScope::Global)
            .is_ok());
        assert!(state
            .confirmed(
                proposal_id,
                &KnowledgeScope::App {
                    bundle_id: "invented".to_string()
                }
            )
            .is_err());
        state.discard(proposal_id);
        assert!(state
            .confirmed(proposal_id, &KnowledgeScope::Global)
            .is_err());
    }

    #[test]
    fn pending_review_retains_only_the_bounded_rule_not_full_examples() {
        let state = CorrectAndTeachState::default();
        let original = "private prefix use recording state private suffix";
        let corrected = "private prefix useRecordingState private suffix";
        let outcome = state.propose(
            CorrectionProposalRequest {
                original_text: original.to_string(),
                corrected_text: corrected.to_string(),
                teaching_context: None,
            },
            &DictationState::default(),
            &[],
        );
        assert!(matches!(
            outcome,
            CorrectionProposalOutcome::Proposal { .. }
        ));

        let pending = state
            .pending
            .lock_or_recover()
            .clone()
            .expect("proposal should be pending");
        assert_eq!(pending.source, "use recording state");
        assert_eq!(pending.replacement, "useRecordingState");
        let retained = format!("{pending:?}");
        assert!(!retained.contains("private prefix"));
        assert!(!retained.contains("private suffix"));

        let CorrectionProposalOutcome::Proposal { proposal_id, .. } = outcome else {
            unreachable!()
        };
        let unsafe_outcome = state.propose(
            CorrectionProposalRequest {
                original_text: "alpha middle omega".to_string(),
                corrected_text: "beta middle delta".to_string(),
                teaching_context: None,
            },
            &DictationState::default(),
            &[],
        );
        assert!(matches!(
            unsafe_outcome,
            CorrectionProposalOutcome::Unsafe { .. }
        ));
        assert!(state
            .confirmed(proposal_id, &KnowledgeScope::Global)
            .is_err());
    }
}
