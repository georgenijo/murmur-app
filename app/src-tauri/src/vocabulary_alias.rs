//! Structured vocabulary entries and deterministic explicit spoken aliases.

use crate::correction::{normalize_alias, CorrectionMatcher};
use crate::state::{AppProfile, VocabularyEntry, VocabularyScope};
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
        Self::build_with_policy(
            base_terms,
            entries,
            app_profiles,
            fuzzy,
            include_builtins,
            false,
        )
    }

    /// Build developer-derived terms only into matchers for profiles carrying
    /// an explicit technical-context signal. Global/app/project preferred
    /// spellings keep their existing scopes.
    pub(crate) fn build_with_contextual_code_terms(
        base_terms: &[String],
        entries: &[VocabularyEntry],
        app_profiles: &[AppProfile],
        fuzzy: bool,
        include_builtins: bool,
    ) -> Self {
        Self::build_with_policy(
            base_terms,
            entries,
            app_profiles,
            fuzzy,
            include_builtins,
            true,
        )
    }

    fn build_with_policy(
        base_terms: &[String],
        entries: &[VocabularyEntry],
        app_profiles: &[AppProfile],
        fuzzy: bool,
        include_builtins: bool,
        contextual_code_terms: bool,
    ) -> Self {
        let global_entries = applicable_entries(entries, None, app_profiles);
        let global_base_terms = if contextual_code_terms {
            &[][..]
        } else {
            base_terms
        };
        let global = Arc::new(build_matcher(
            global_base_terms,
            &global_entries,
            fuzzy,
            include_builtins && !contextual_code_terms,
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

        let by_bundle_id = bundle_ids
            .into_iter()
            .map(|bundle_id| {
                let applicable = applicable_entries(entries, Some(&bundle_id), app_profiles);
                let code_terms_enabled = !contextual_code_terms
                    || crate::state::app_profiles_enable_code_vocabulary(app_profiles, &bundle_id);
                let matcher = Arc::new(build_matcher(
                    if code_terms_enabled { base_terms } else { &[] },
                    &applicable,
                    fuzzy,
                    include_builtins && code_terms_enabled,
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
    let homophone_terms = entries
        .iter()
        .filter_map(|entry| crate::homophone_context::vocabulary_word(&entry.written))
        .collect();
    CorrectionMatcher::build(&terms, &pairs, fuzzy, include_builtins)
        .with_homophone_terms(homophone_terms)
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
                .find(|profile| profile.bundle_id == bundle_id)
                .filter(|profile| profile.ide_context_enabled)
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

pub(crate) fn validate_entries(entries: &[VocabularyEntry]) -> Result<(), String> {
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

    let command_phrases = crate::spoken_structure::BASIC_COMMAND_PHRASES
        .iter()
        .map(|phrase| normalize_alias(phrase))
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
                    "'{}' is a built-in spoken formatting phrase. Aliases cannot override it.",
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
    fn homophone_vocabulary_respects_enablement_and_app_scope() {
        let global = entry("Read", &[]);
        let mut disabled = entry("Claude", &[]);
        disabled.enabled = false;
        let mut scoped = entry("Cache", &["cash"]);
        scoped.scope = VocabularyScope::App {
            bundle_id: "test.editor".into(),
        };
        let set = CorrectionMatcherSet::build(&[], &[global, disabled, scoped], &[], false, false);
        assert_eq!(set.select(None).homophone_terms(), &["Read"]);
        let app = set.select(Some("test.editor"));
        assert!(app.homophone_terms().contains(&"Cache".into()));
        assert!(!app.homophone_terms().contains(&"Claude".into()));
    }

    #[test]
    fn vocabulary_entries_carry_no_user_context_restrictions() {
        // Review B3: a context field the frontend drops, or an older build
        // ignores, would let a restricted alias fire everywhere. Context
        // requirements stay internal to the built-in pack until Settings can
        // author and preserve them.
        let value = serde_json::to_value(entry("Tauri", &["Tori"])).unwrap();
        let mut keys = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        keys.sort_unstable();
        assert_eq!(keys, ["aliases", "enabled", "id", "scope", "written"]);
    }

    #[test]
    fn rejects_ambiguous_and_cyclic_aliases() {
        let ambiguous = vec![entry("Tauri", &["Tori"]), entry("Tory CLI", &["Tori"])];
        assert!(validate_entries(&ambiguous)
            .unwrap_err()
            .contains("ambiguous"));

        let cyclic = vec![entry("Tauri", &["Tory"]), entry("Tory", &["Tauri"])];
        assert!(validate_entries(&cyclic).unwrap_err().contains("Cyclic"));

        let indirect = vec![
            entry("Alpha", &["Beta"]),
            entry("Beta", &["Gamma"]),
            entry("Gamma", &["Alpha"]),
        ];
        assert!(validate_entries(&indirect).unwrap_err().contains("Cyclic"));
    }

    #[test]
    fn rejects_built_in_spoken_command_collisions() {
        for phrase in crate::spoken_structure::BASIC_COMMAND_PHRASES {
            let error = validate_entries(&[entry("Collision", &[phrase])]).unwrap_err();
            assert!(
                error.contains("built-in spoken formatting phrase"),
                "{phrase}"
            );
        }
        let error = validate_entries(&[entry("LineBreak", &[" New  LINE "])]).unwrap_err();
        assert!(error.contains("built-in spoken formatting phrase"));

        let duplicate = validate_entries(&[entry("Tauri", &["Tori", "tori"])]).unwrap_err();
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
    fn contextual_code_terms_do_not_leak_into_ordinary_apps() {
        let ordinary = AppProfile {
            bundle_id: "com.example.Chat".to_string(),
            label: "Chat".to_string(),
            auto_paste_override: None,
            cleanup_override: None,
            cli_formatting_override: None,
            smart_formatting_override: None,
            writing_style: None,
            ide_context_enabled: false,
            ide_project_roots: Vec::new(),
        };
        let mut technical = ordinary.clone();
        technical.bundle_id = "com.example.Editor".to_string();
        technical.label = "Editor".to_string();
        technical.writing_style = Some(crate::state::WritingStyle::CodeTechnical);
        let mut indexed = ordinary.clone();
        indexed.bundle_id = "com.example.IDE".to_string();
        indexed.label = "IDE".to_string();
        indexed.ide_context_enabled = true;
        indexed.ide_project_roots = vec!["/project".to_string()];

        let preferred = entry("Tauri", &["Tori"]);
        let terms = vec!["all__".to_string(), "toBe".to_string()];
        let set = CorrectionMatcherSet::build_with_contextual_code_terms(
            &terms,
            &[preferred],
            &[ordinary, technical, indexed],
            false,
            false,
        );
        let prose = "Tori will deploy this on all the machines before going to be offline";
        let ordinary_expected =
            "Tauri will deploy this on all the machines before going to be offline";
        let technical_expected =
            "Tauri will deploy this on all__ the machines before going toBe offline";

        assert_eq!(set.select(None).apply(prose), ordinary_expected);
        assert_eq!(
            set.select(Some("com.example.Chat")).apply(prose),
            ordinary_expected
        );
        assert_eq!(
            set.select(Some("com.example.Unconfigured")).apply(prose),
            ordinary_expected
        );
        assert_eq!(
            set.select(Some("com.example.Editor")).apply(prose),
            technical_expected
        );
        assert_eq!(
            set.select(Some("com.example.IDE")).apply(prose),
            technical_expected
        );
    }

    #[test]
    fn contextual_code_terms_follow_duplicate_profile_resolution() {
        let mut first = AppProfile {
            bundle_id: "com.example.Editor".to_string(),
            label: "first".to_string(),
            auto_paste_override: None,
            cleanup_override: None,
            cli_formatting_override: None,
            smart_formatting_override: None,
            writing_style: None,
            ide_context_enabled: false,
            ide_project_roots: Vec::new(),
        };
        let mut second = first.clone();
        second.label = "second".to_string();
        second.writing_style = Some(crate::state::WritingStyle::CodeTechnical);

        let terms = vec!["toBe".to_string()];
        let set = CorrectionMatcherSet::build_with_contextual_code_terms(
            &terms,
            &[],
            &[first.clone(), second],
            false,
            false,
        );
        assert_eq!(
            set.select(Some("com.example.Editor"))
                .apply("going to be ready"),
            "going toBe ready"
        );

        first.ide_context_enabled = true;
        first.writing_style = Some(crate::state::WritingStyle::Conversational);
        let set = CorrectionMatcherSet::build_with_contextual_code_terms(
            &terms,
            &[],
            &[first],
            false,
            false,
        );
        assert_eq!(
            set.select(Some("com.example.Editor"))
                .apply("going to be ready"),
            "going toBe ready"
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
