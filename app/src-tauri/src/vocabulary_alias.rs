//! Structured vocabulary entries and deterministic explicit spoken aliases.

use crate::correction::{normalize_alias, CorrectionMatcher};
use crate::knowledge_store::{KnowledgeEntry, KnowledgePayload, KnowledgeScope};
use crate::state::{AppProfile, VocabularyEntry, VocabularyScope, VoiceCommand};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

const MAX_ENTRIES: usize = 500;
const MAX_ALIASES_PER_ENTRY: usize = 16;
const MAX_VALUE_CHARS: usize = 256;

pub(crate) struct CorrectionMatcherSet {
    global: Arc<CorrectionMatcher>,
    by_bundle_id: HashMap<String, Arc<CorrectionMatcher>>,
}

impl CorrectionMatcherSet {
    pub(crate) fn build(
        base_terms: &[String],
        entries: &[VocabularyEntry],
        app_profiles: &[AppProfile],
        fuzzy: bool,
        include_builtins: bool,
    ) -> Self {
        Self::build_with_knowledge(
            base_terms,
            entries,
            app_profiles,
            &[],
            fuzzy,
            include_builtins,
        )
    }

    pub(crate) fn build_with_knowledge(
        base_terms: &[String],
        entries: &[VocabularyEntry],
        app_profiles: &[AppProfile],
        knowledge: &[KnowledgeEntry],
        fuzzy: bool,
        include_builtins: bool,
    ) -> Self {
        let global_entries = applicable_entries(entries, None, app_profiles);
        let global_learned = applicable_learned_pairs(knowledge, None, None);
        let global = Arc::new(build_matcher(
            base_terms,
            &global_entries,
            &global_learned,
            fuzzy,
            include_builtins,
        ));

        let mut bundle_ids = app_profiles
            .iter()
            .map(|profile| profile.bundle_id.clone())
            .collect::<HashSet<_>>();
        for entry in entries {
            match &entry.scope {
                VocabularyScope::App { bundle_id } | VocabularyScope::Project { bundle_id, .. } => {
                    bundle_ids.insert(bundle_id.clone());
                }
                VocabularyScope::Global => {}
            }
        }
        for entry in knowledge {
            if let KnowledgeScope::App { bundle_id } | KnowledgeScope::Project { bundle_id, .. } =
                &entry.scope
            {
                bundle_ids.insert(bundle_id.clone());
            }
        }

        let by_bundle_id = bundle_ids
            .into_iter()
            .map(|bundle_id| {
                let applicable = applicable_entries(entries, Some(&bundle_id), app_profiles);
                let project_root = unambiguous_project_root(&bundle_id, app_profiles);
                let learned = applicable_learned_pairs(knowledge, Some(&bundle_id), project_root);
                let matcher = Arc::new(build_matcher(
                    base_terms,
                    &applicable,
                    &learned,
                    fuzzy,
                    include_builtins,
                ));
                (bundle_id, matcher)
            })
            .collect();

        Self {
            global,
            by_bundle_id,
        }
    }

    pub(crate) fn select(&self, bundle_id: Option<&str>) -> Arc<CorrectionMatcher> {
        bundle_id
            .and_then(|bundle_id| self.by_bundle_id.get(bundle_id))
            .cloned()
            .unwrap_or_else(|| self.global.clone())
    }
}

fn build_matcher(
    base_terms: &[String],
    entries: &[&VocabularyEntry],
    learned_pairs: &[(String, String)],
    fuzzy: bool,
    include_builtins: bool,
) -> CorrectionMatcher {
    let mut terms = base_terms.to_vec();
    let mut pairs = Vec::new();
    for entry in entries {
        terms.push(entry.written.trim().to_string());
        for alias in &entry.aliases {
            pairs.push((alias.trim().to_string(), entry.written.trim().to_string()));
        }
    }
    CorrectionMatcher::build_with_learned(&terms, &pairs, learned_pairs, fuzzy, include_builtins)
}

fn unambiguous_project_root<'a>(
    bundle_id: &str,
    app_profiles: &'a [AppProfile],
) -> Option<&'a str> {
    app_profiles
        .iter()
        .find(|profile| profile.bundle_id == bundle_id && profile.ide_context_enabled)
        .filter(|profile| profile.ide_project_roots.len() == 1)
        .and_then(|profile| profile.ide_project_roots.first())
        .map(String::as_str)
}

fn applicable_learned_pairs(
    entries: &[KnowledgeEntry],
    bundle_id: Option<&str>,
    project_root: Option<&str>,
) -> Vec<(String, String)> {
    let mut seen = HashSet::new();
    entries
        .iter()
        .filter(|entry| entry.enabled)
        .filter(|entry| match &entry.scope {
            KnowledgeScope::Global => true,
            KnowledgeScope::App {
                bundle_id: required,
            } => bundle_id == Some(required.as_str()),
            KnowledgeScope::Project {
                bundle_id: required,
                root,
            } => bundle_id == Some(required.as_str()) && project_root == Some(root.as_str()),
        })
        .filter_map(|entry| match &entry.payload {
            KnowledgePayload::ReplacementRule {
                source,
                replacement,
            } if seen.insert(normalize_alias(source)) => {
                Some((source.clone(), replacement.clone()))
            }
            _ => None,
        })
        .collect()
}

fn applicable_entries<'a>(
    entries: &'a [VocabularyEntry],
    bundle_id: Option<&str>,
    app_profiles: &[AppProfile],
) -> Vec<&'a VocabularyEntry> {
    let active_roots = bundle_id
        .and_then(|bundle_id| {
            app_profiles
                .iter()
                .find(|profile| profile.bundle_id == bundle_id && profile.ide_context_enabled)
        })
        .map(|profile| profile.ide_project_roots.as_slice())
        .unwrap_or(&[]);

    entries
        .iter()
        .filter(|entry| entry.enabled)
        .filter(|entry| match &entry.scope {
            VocabularyScope::Global => true,
            VocabularyScope::App {
                bundle_id: required,
            } => bundle_id == Some(required.as_str()),
            VocabularyScope::Project {
                bundle_id: required,
                root,
            } => {
                bundle_id == Some(required.as_str())
                    && active_roots.iter().any(|active| active == root)
            }
        })
        .collect()
}

pub(crate) fn prompt_terms(
    entries: &[VocabularyEntry],
    bundle_id: Option<&str>,
    app_profiles: &[AppProfile],
) -> String {
    applicable_entries(entries, bundle_id, app_profiles)
        .into_iter()
        .map(|entry| entry.written.trim())
        .filter(|written| !written.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) fn has_applicable_entries(
    entries: &[VocabularyEntry],
    bundle_id: Option<&str>,
    app_profiles: &[AppProfile],
) -> bool {
    !applicable_entries(entries, bundle_id, app_profiles).is_empty()
}

fn scopes_overlap(left: &VocabularyScope, right: &VocabularyScope) -> bool {
    match (left, right) {
        (VocabularyScope::Global, _) | (_, VocabularyScope::Global) => true,
        (VocabularyScope::App { bundle_id: left }, VocabularyScope::App { bundle_id: right }) => {
            left == right
        }
        (
            VocabularyScope::Project {
                bundle_id: left, ..
            },
            VocabularyScope::App { bundle_id: right },
        )
        | (
            VocabularyScope::App { bundle_id: left },
            VocabularyScope::Project {
                bundle_id: right, ..
            },
        ) => left == right,
        (
            VocabularyScope::Project {
                bundle_id: left_bundle,
                root: left_root,
            },
            VocabularyScope::Project {
                bundle_id: right_bundle,
                root: right_root,
            },
        ) => left_bundle == right_bundle && left_root == right_root,
    }
}

pub(crate) fn validate_entries(
    entries: &[VocabularyEntry],
    voice_commands: &[VoiceCommand],
) -> Result<(), String> {
    if entries.len() > MAX_ENTRIES {
        return Err(format!(
            "Vocabulary supports at most {MAX_ENTRIES} entries."
        ));
    }

    let enabled = entries
        .iter()
        .filter(|entry| entry.enabled)
        .collect::<Vec<_>>();
    for entry in &enabled {
        let written = entry.written.trim();
        if written.is_empty() {
            return Err("Every enabled vocabulary entry needs a written form.".to_string());
        }
        if written.chars().count() > MAX_VALUE_CHARS {
            return Err(format!("The written form '{written}' is too long."));
        }
        if entry.aliases.len() > MAX_ALIASES_PER_ENTRY {
            return Err(format!(
                "'{written}' supports at most {MAX_ALIASES_PER_ENTRY} spoken aliases."
            ));
        }
        for alias in &entry.aliases {
            let alias = alias.trim();
            if alias.is_empty() {
                return Err(format!("'{written}' contains an empty spoken alias."));
            }
            if alias.chars().count() > MAX_VALUE_CHARS {
                return Err(format!("The spoken alias for '{written}' is too long."));
            }
        }
    }

    for (index, left) in enabled.iter().enumerate() {
        for right in enabled.iter().skip(index + 1) {
            if !scopes_overlap(&left.scope, &right.scope) {
                continue;
            }
            if normalize_alias(&left.written) == normalize_alias(&right.written) {
                return Err(format!(
                    "'{}' conflicts with the existing written term '{}'.",
                    right.written.trim(),
                    left.written.trim()
                ));
            }
        }
    }

    // Detect indirect cycles before the broader canonical collision message.
    let edges = enabled
        .iter()
        .map(|entry| {
            enabled
                .iter()
                .enumerate()
                .filter(|(_, target)| scopes_overlap(&entry.scope, &target.scope))
                .filter(|(_, target)| {
                    entry
                        .aliases
                        .iter()
                        .any(|alias| normalize_alias(alias) == normalize_alias(&target.written))
                })
                .map(|(index, _)| index)
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    fn visits_cycle(
        node: usize,
        edges: &[Vec<usize>],
        visiting: &mut [bool],
        visited: &mut [bool],
    ) -> bool {
        if visiting[node] {
            return true;
        }
        if visited[node] {
            return false;
        }
        visiting[node] = true;
        if edges[node]
            .iter()
            .any(|next| visits_cycle(*next, edges, visiting, visited))
        {
            return true;
        }
        visiting[node] = false;
        visited[node] = true;
        false
    }
    let mut visiting = vec![false; enabled.len()];
    let mut visited = vec![false; enabled.len()];
    if (0..enabled.len()).any(|node| visits_cycle(node, &edges, &mut visiting, &mut visited)) {
        return Err("Cyclic aliases are not allowed. A spoken alias cannot lead back to its starting written term.".to_string());
    }

    let command_phrases = crate::voice_commands::BUILTIN_COMMAND_PHRASES
        .iter()
        .map(|phrase| normalize_alias(phrase))
        .chain(
            voice_commands
                .iter()
                .map(|command| normalize_alias(&command.phrase)),
        )
        .collect::<HashSet<_>>();

    for (entry_index, entry) in enabled.iter().enumerate() {
        let mut seen = HashSet::new();
        for alias in &entry.aliases {
            let normalized = normalize_alias(alias);
            if !seen.insert(normalized.clone()) {
                return Err(format!(
                    "Spoken alias '{}' is duplicated for '{}'.",
                    alias.trim(),
                    entry.written.trim()
                ));
            }
            if normalized == normalize_alias(&entry.written) {
                return Err(format!(
                    "'{}' is already the written form; remove it from Spoken aliases.",
                    alias.trim()
                ));
            }
            if command_phrases.contains(&normalized) {
                return Err(format!(
                    "'{}' is a Voice Command phrase. Aliases cannot override commands.",
                    alias.trim()
                ));
            }
            for canonical in &enabled {
                if std::ptr::eq(*entry, *canonical)
                    || !scopes_overlap(&entry.scope, &canonical.scope)
                {
                    continue;
                }
                if normalized == normalize_alias(&canonical.written) {
                    return Err(format!(
                        "'{}' is already the written form for '{}'.",
                        alias.trim(),
                        canonical.written.trim()
                    ));
                }
            }
            for other in enabled.iter().skip(entry_index + 1) {
                if !scopes_overlap(&entry.scope, &other.scope) {
                    continue;
                }
                if other
                    .aliases
                    .iter()
                    .any(|candidate| normalize_alias(candidate) == normalized)
                {
                    return Err(format!(
                        "Spoken alias '{}' is ambiguous between '{}' and '{}'.",
                        alias.trim(),
                        entry.written.trim(),
                        other.written.trim()
                    ));
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn learned(
        id: &str,
        source: &str,
        replacement: &str,
        scope: KnowledgeScope,
        provenance: crate::knowledge_store::KnowledgeProvenance,
        updated_at_ms: i64,
    ) -> KnowledgeEntry {
        KnowledgeEntry {
            id: id.to_string(),
            payload: KnowledgePayload::ReplacementRule {
                source: source.to_string(),
                replacement: replacement.to_string(),
            },
            enabled: true,
            scope,
            provenance,
            created_at_ms: updated_at_ms,
            updated_at_ms,
            revision: 1,
        }
    }

    fn entry(written: &str, aliases: &[&str]) -> VocabularyEntry {
        VocabularyEntry {
            id: written.to_string(),
            written: written.to_string(),
            aliases: aliases.iter().map(|value| value.to_string()).collect(),
            enabled: true,
            scope: VocabularyScope::Global,
        }
    }

    #[test]
    fn explicit_aliases_outrank_learned_replacements() {
        let explicit = entry("Tauri", &["Tori"]);
        let learned = learned(
            "learned",
            "Tori",
            "Tory",
            KnowledgeScope::Global,
            crate::knowledge_store::KnowledgeProvenance::LearnedCorrection,
            2,
        );
        let set = CorrectionMatcherSet::build_with_knowledge(
            &[],
            &[explicit],
            &[],
            &[learned],
            false,
            false,
        );
        assert_eq!(set.select(None).apply("Tori works"), "Tauri works");
    }

    #[test]
    fn learned_rules_use_project_app_global_precedence_and_fail_closed_for_multiple_roots() {
        let mut profile = AppProfile {
            bundle_id: "com.editor".to_string(),
            label: "Editor".to_string(),
            auto_paste_override: None,
            cleanup_override: None,
            cli_formatting_override: None,
            smart_formatting_override: None,
            writing_style: None,
            ide_context_enabled: true,
            ide_project_roots: vec!["/project".to_string()],
        };
        let knowledge = vec![
            learned(
                "project",
                "use hook",
                "projectHook",
                KnowledgeScope::Project {
                    bundle_id: "com.editor".to_string(),
                    root: "/project".to_string(),
                },
                crate::knowledge_store::KnowledgeProvenance::LearnedCorrection,
                3,
            ),
            learned(
                "app",
                "use hook",
                "appHook",
                KnowledgeScope::App {
                    bundle_id: "com.editor".to_string(),
                },
                crate::knowledge_store::KnowledgeProvenance::LearnedCorrection,
                2,
            ),
            learned(
                "global",
                "use hook",
                "globalHook",
                KnowledgeScope::Global,
                crate::knowledge_store::KnowledgeProvenance::LearnedCorrection,
                1,
            ),
        ];
        let set = CorrectionMatcherSet::build_with_knowledge(
            &[],
            &[],
            &[profile.clone()],
            &knowledge,
            false,
            false,
        );
        assert_eq!(
            set.select(Some("com.editor")).apply("use hook"),
            "projectHook"
        );
        assert_eq!(
            set.select(Some("com.other")).apply("use hook"),
            "globalHook"
        );

        profile.ide_project_roots.push("/other".to_string());
        let set = CorrectionMatcherSet::build_with_knowledge(
            &[],
            &[],
            &[profile],
            &knowledge,
            false,
            false,
        );
        assert_eq!(set.select(Some("com.editor")).apply("use hook"), "appHook");
    }

    #[test]
    fn rejects_ambiguous_and_cyclic_aliases() {
        let ambiguous = vec![entry("Tauri", &["Tori"]), entry("Tory CLI", &["Tori"])];
        assert!(validate_entries(&ambiguous, &[])
            .unwrap_err()
            .contains("ambiguous"));

        let cyclic = vec![entry("Tauri", &["Tory"]), entry("Tory", &["Tauri"])];
        assert!(validate_entries(&cyclic, &[])
            .unwrap_err()
            .contains("Cyclic"));

        let indirect = vec![
            entry("Alpha", &["Beta"]),
            entry("Beta", &["Gamma"]),
            entry("Gamma", &["Alpha"]),
        ];
        assert!(validate_entries(&indirect, &[])
            .unwrap_err()
            .contains("Cyclic"));
    }

    #[test]
    fn rejects_voice_command_collisions() {
        let error = validate_entries(&[entry("LineBreak", &["new line"])], &[]).unwrap_err();
        assert!(error.contains("Voice Command"));

        let duplicate = validate_entries(&[entry("Tauri", &["Tori", "tori"])], &[]).unwrap_err();
        assert!(duplicate.contains("duplicated"));
    }

    #[test]
    fn disabled_and_unmatched_scope_entries_do_not_apply() {
        let mut disabled = entry("Tauri", &["Tori"]);
        disabled.enabled = false;
        let scoped = VocabularyEntry {
            id: "scoped".to_string(),
            written: "Murmur".to_string(),
            aliases: vec!["mer mer".to_string()],
            enabled: true,
            scope: VocabularyScope::App {
                bundle_id: "com.example.Editor".to_string(),
            },
        };
        let set = CorrectionMatcherSet::build(&[], &[disabled, scoped], &[], false, false);
        assert_eq!(
            set.select(None).apply("Tori and mer mer"),
            "Tori and mer mer"
        );
        assert_eq!(
            set.select(Some("com.example.Editor")).apply("mer mer"),
            "Murmur"
        );
    }

    #[test]
    fn project_scope_requires_matching_bundle_enabled_profile_and_root() {
        let scoped = VocabularyEntry {
            id: "project".to_string(),
            written: "Tauri".to_string(),
            aliases: vec!["Tori".to_string()],
            enabled: true,
            scope: VocabularyScope::Project {
                bundle_id: "com.example.Editor".to_string(),
                root: "/project/one".to_string(),
            },
        };
        let disabled_profile = AppProfile {
            bundle_id: "com.example.Editor".to_string(),
            label: "Editor".to_string(),
            auto_paste_override: None,
            cleanup_override: None,
            cli_formatting_override: None,
            smart_formatting_override: None,
            writing_style: None,
            ide_context_enabled: false,
            ide_project_roots: vec!["/project/one".to_string()],
        };
        let disabled = CorrectionMatcherSet::build(
            &[],
            std::slice::from_ref(&scoped),
            std::slice::from_ref(&disabled_profile),
            false,
            false,
        );
        assert_eq!(
            disabled.select(Some("com.example.Editor")).apply("Tori"),
            "Tori"
        );

        let mut enabled_profile = disabled_profile;
        enabled_profile.ide_context_enabled = true;
        let enabled = CorrectionMatcherSet::build(&[], &[scoped], &[enabled_profile], false, false);
        assert_eq!(enabled.select(Some("com.other.App")).apply("Tori"), "Tori");
        assert_eq!(
            enabled.select(Some("com.example.Editor")).apply("Tori"),
            "Tauri"
        );
    }
}
