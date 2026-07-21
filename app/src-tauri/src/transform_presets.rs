//! Built-in spoken transform presets (issue #312 Phase D1).
//!
//! Pure, unit-tested name/alias → full instruction expansion. Used by
//! `finish_transform_instruction` so a short spoken name expands to a complete
//! rewrite prompt before the sidecar sees it. Never logs the instruction text.

/// One built-in preset: canonical name, aliases, and full instruction string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransformPreset {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub instruction: &'static str,
}

/// Built-in presets shipped with #312 D1.
pub const BUILTIN_PRESETS: &[TransformPreset] = &[
    TransformPreset {
        name: "Shorten",
        aliases: &["make shorter", "shorter", "condense", "brief"],
        instruction: "Rewrite the selected text to be shorter and more concise while preserving the original meaning, tone, and key facts. Do not add new information.",
    },
    TransformPreset {
        name: "Bullets",
        aliases: &["bullet points", "bullet list", "as bullets", "make bullets"],
        instruction: "Rewrite the selected text as a clear bullet list. Keep each bullet one idea, preserve meaning, and do not invent facts.",
    },
    TransformPreset {
        name: "Professional",
        aliases: &["formal", "more professional", "make professional"],
        instruction: "Rewrite the selected text in a clear, professional tone suitable for workplace communication. Preserve meaning; do not add new claims.",
    },
    TransformPreset {
        name: "Fix grammar",
        aliases: &["grammar", "fix spelling", "proofread", "correct grammar"],
        instruction: "Fix grammar, spelling, and punctuation in the selected text. Preserve meaning and voice; do not rewrite for style beyond corrections.",
    },
    TransformPreset {
        name: "Casual",
        aliases: &["informal", "make casual", "friendlier", "more casual"],
        instruction: "Rewrite the selected text in a friendly, casual tone. Preserve meaning and do not add new information.",
    },
];

fn normalize(s: &str) -> String {
    s.trim()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

/// Resolve a spoken (or typed) name to a built-in preset instruction.
/// Case-insensitive match against canonical name and aliases. Returns
/// `None` when no preset matches so the raw transcript is used as-is.
pub fn resolve_preset(spoken: &str) -> Option<&'static str> {
    let key = normalize(spoken);
    if key.is_empty() {
        return None;
    }
    for preset in BUILTIN_PRESETS {
        if normalize(preset.name) == key {
            return Some(preset.instruction);
        }
        for alias in preset.aliases {
            if normalize(alias) == key {
                return Some(preset.instruction);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_canonical_names_case_insensitively() {
        assert_eq!(
            resolve_preset("shorten"),
            Some(BUILTIN_PRESETS[0].instruction)
        );
        assert_eq!(
            resolve_preset("  BULLETS  "),
            Some(BUILTIN_PRESETS[1].instruction)
        );
        assert_eq!(
            resolve_preset("Professional"),
            Some(BUILTIN_PRESETS[2].instruction)
        );
        assert_eq!(
            resolve_preset("fix grammar"),
            Some(BUILTIN_PRESETS[3].instruction)
        );
        assert_eq!(
            resolve_preset("Casual"),
            Some(BUILTIN_PRESETS[4].instruction)
        );
    }

    #[test]
    fn resolves_aliases() {
        assert_eq!(
            resolve_preset("make shorter"),
            Some(BUILTIN_PRESETS[0].instruction)
        );
        assert_eq!(
            resolve_preset("bullet points"),
            Some(BUILTIN_PRESETS[1].instruction)
        );
        assert_eq!(
            resolve_preset("proofread"),
            Some(BUILTIN_PRESETS[3].instruction)
        );
    }

    #[test]
    fn unknown_transcript_returns_none() {
        assert_eq!(resolve_preset("translate to french"), None);
        assert_eq!(resolve_preset(""), None);
        assert_eq!(resolve_preset("   "), None);
    }

    #[test]
    fn every_preset_has_nonempty_instruction() {
        for preset in BUILTIN_PRESETS {
            assert!(!preset.instruction.is_empty(), "{}", preset.name);
            assert!(!preset.name.is_empty());
        }
    }
}
