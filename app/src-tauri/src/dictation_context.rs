//! Immutable, per-recording dictation context resolution.
//!
//! The resolver is the only place where global settings, per-app profiles, and
//! one-session overrides are combined. The resulting snapshot contains only the
//! typed values consumed by the live pipeline; later settings or focus changes
//! cannot alter an in-flight dictation.

use crate::cli_command::CliFormattingMode;
use crate::correction::CorrectionMatcher;
use crate::state::{AppProfile, DictationState, VoiceCommand};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveAppIdentity {
    pub bundle_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchedAppProfile {
    pub bundle_id: String,
    pub label: String,
    pub auto_paste_override: Option<bool>,
    pub cleanup_override: Option<bool>,
    pub cli_formatting_override: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VocabularySource {
    None,
    Custom,
    CodeAware,
    CustomAndCodeAware,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VocabularyIdentity {
    pub source: VocabularySource,
    /// Monotonic configuration revision captured with the vocabulary inputs.
    pub version: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnabledCommandGroups {
    pub built_in_voice_commands: bool,
    pub custom_voice_commands: bool,
}

/// Permissions to read user context as an input to dictation.
///
/// These are deliberately separate from clipboard-first output. A `false`
/// `clipboard` value forbids reading clipboard contents as prompt context; it
/// does not affect writing the final transcript to the clipboard or auto-paste.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ContextCapturePermissions {
    pub selected_text: bool,
    pub surrounding_screen_text: bool,
    pub clipboard: bool,
}

#[derive(Debug, Clone)]
pub struct TranscriptionSettings {
    pub model_name: String,
    pub language: String,
    pub vad_sensitivity: u32,
    pub prompt: Option<String>,
    pub smart_punctuation: bool,
}

#[derive(Clone)]
pub struct TransformationSettings {
    pub cleanup_enabled: bool,
    pub cleanup_remove_filler: bool,
    pub cleanup_capitalize: bool,
    pub voice_command_pairs: Vec<VoiceCommand>,
    pub correction_enabled: bool,
    pub correction_matcher: Option<Arc<CorrectionMatcher>>,
    pub cli_formatting_mode: CliFormattingMode,
}

#[derive(Debug, Clone)]
pub struct DeliverySettings {
    pub auto_paste: bool,
    pub paste_delay_ms: u64,
    pub save_transcript: bool,
    pub save_audio: bool,
    pub output_dir: String,
}

#[derive(Clone)]
pub struct DictationContextSnapshot {
    pub app: ActiveAppIdentity,
    pub matched_profile: Option<MatchedAppProfile>,
    pub transcription: TranscriptionSettings,
    pub transformations: TransformationSettings,
    pub delivery: DeliverySettings,
    pub vocabulary: VocabularyIdentity,
    pub enabled_command_groups: EnabledCommandGroups,
    pub context_capture: ContextCapturePermissions,
}

/// Ephemeral overrides supplied by the recording trigger. No caller supplies
/// them today, but keeping them explicit makes precedence testable and avoids a
/// second resolution path when session-specific behavior is introduced.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SessionOverrides {
    pub auto_paste: Option<bool>,
    pub cleanup_enabled: Option<bool>,
    pub cli_formatting_enabled: Option<bool>,
}

pub struct ResolverInputs<'a> {
    pub bundle_id: Option<&'a str>,
    pub global: &'a DictationState,
    pub prompt: Option<String>,
    pub correction_matcher: Option<Arc<CorrectionMatcher>>,
    pub vocabulary_version: u64,
    pub session_overrides: SessionOverrides,
}

/// Resolve global defaults -> matching app profiles -> one-session overrides.
///
/// Duplicate profiles intentionally preserve the legacy behavior: for each
/// field, the first matching profile that supplies that override wins. A
/// matching entry with `None` falls through to the next duplicate entry.
pub fn resolve(inputs: ResolverInputs<'_>) -> DictationContextSnapshot {
    let global = inputs.global;
    let auto_paste = inputs.session_overrides.auto_paste.unwrap_or_else(|| {
        resolve_profile_override(
            global.auto_paste,
            inputs.bundle_id,
            &global.app_profiles,
            |profile| profile.auto_paste_override,
        )
    });
    let cleanup_enabled = inputs.session_overrides.cleanup_enabled.unwrap_or_else(|| {
        resolve_profile_override(
            global.cleanup_enabled,
            inputs.bundle_id,
            &global.app_profiles,
            |profile| profile.cleanup_override,
        )
    });
    let cli_formatting_mode = match inputs.session_overrides.cli_formatting_enabled.or_else(|| {
        resolve_profile_optional(inputs.bundle_id, &global.app_profiles, |profile| {
            profile.cli_formatting_override
        })
    }) {
        Some(true) => CliFormattingMode::Enabled,
        Some(false) => CliFormattingMode::Disabled,
        None => CliFormattingMode::Auto,
    };
    let matched_profile = inputs.bundle_id.and_then(|bundle_id| {
        global
            .app_profiles
            .iter()
            .find(|profile| profile.bundle_id == bundle_id)
            .map(|profile| MatchedAppProfile {
                bundle_id: profile.bundle_id.clone(),
                label: profile.label.clone(),
                auto_paste_override: profile.auto_paste_override,
                cleanup_override: profile.cleanup_override,
                cli_formatting_override: profile.cli_formatting_override,
            })
    });
    let custom_vocab = !global.custom_vocabulary.trim().is_empty();
    let code_vocab = global.code_vocab_enabled;
    let source = match (custom_vocab, code_vocab) {
        (false, false) => VocabularySource::None,
        (true, false) => VocabularySource::Custom,
        (false, true) => VocabularySource::CodeAware,
        (true, true) => VocabularySource::CustomAndCodeAware,
    };
    let voice_commands = global.voice_commands_enabled;

    DictationContextSnapshot {
        app: ActiveAppIdentity {
            bundle_id: inputs.bundle_id.map(str::to_string),
        },
        matched_profile,
        transcription: TranscriptionSettings {
            model_name: global.model_name.clone(),
            language: global.language.clone(),
            vad_sensitivity: global.vad_sensitivity,
            prompt: inputs.prompt,
            smart_punctuation: global.smart_punctuation,
        },
        transformations: TransformationSettings {
            cleanup_enabled,
            cleanup_remove_filler: global.cleanup_remove_filler,
            cleanup_capitalize: global.cleanup_capitalize,
            voice_command_pairs: global.voice_command_pairs.clone(),
            correction_enabled: global.correction_enabled,
            correction_matcher: inputs.correction_matcher,
            cli_formatting_mode,
        },
        delivery: DeliverySettings {
            auto_paste,
            paste_delay_ms: global.auto_paste_delay_ms,
            save_transcript: global.save_transcript,
            save_audio: global.save_audio,
            output_dir: global.output_dir.clone(),
        },
        vocabulary: VocabularyIdentity {
            source,
            version: inputs.vocabulary_version,
        },
        enabled_command_groups: EnabledCommandGroups {
            built_in_voice_commands: voice_commands,
            custom_voice_commands: voice_commands && !global.voice_command_pairs.is_empty(),
        },
        // No selected-text, screen-text, or clipboard reads exist. Future
        // features must add an explicit setting/profile override here first.
        context_capture: ContextCapturePermissions::default(),
    }
}

fn resolve_profile_optional<T: Copy>(
    bundle_id: Option<&str>,
    profiles: &[AppProfile],
    get_override: impl Fn(&AppProfile) -> Option<T>,
) -> Option<T> {
    let bundle_id = bundle_id?;
    profiles
        .iter()
        .filter(|profile| profile.bundle_id == bundle_id)
        .find_map(get_override)
}

fn resolve_profile_override<T: Copy>(
    global: T,
    bundle_id: Option<&str>,
    profiles: &[AppProfile],
    get_override: impl Fn(&AppProfile) -> Option<T>,
) -> T {
    let Some(bundle_id) = bundle_id else {
        return global;
    };
    profiles
        .iter()
        .filter(|profile| profile.bundle_id == bundle_id)
        .find_map(get_override)
        .unwrap_or(global)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(
        bundle_id: &str,
        auto_paste_override: Option<bool>,
        cleanup_override: Option<bool>,
    ) -> AppProfile {
        AppProfile {
            bundle_id: bundle_id.to_string(),
            label: bundle_id.to_string(),
            auto_paste_override,
            cleanup_override,
            cli_formatting_override: None,
        }
    }

    fn resolve_test(
        global: &DictationState,
        bundle_id: Option<&str>,
        session_overrides: SessionOverrides,
    ) -> DictationContextSnapshot {
        resolve(ResolverInputs {
            bundle_id,
            global,
            prompt: None,
            correction_matcher: None,
            vocabulary_version: 7,
            session_overrides,
        })
    }

    #[test]
    fn matching_profile_resolves_effective_values() {
        let mut global = DictationState {
            auto_paste: true,
            cleanup_enabled: false,
            ..DictationState::default()
        };
        global.app_profiles = vec![profile("com.apple.Terminal", Some(false), Some(true))];

        let snapshot = resolve_test(
            &global,
            Some("com.apple.Terminal"),
            SessionOverrides::default(),
        );

        assert!(!snapshot.delivery.auto_paste);
        assert!(snapshot.transformations.cleanup_enabled);
        assert_eq!(
            snapshot
                .matched_profile
                .as_ref()
                .map(|profile| profile.bundle_id.as_str()),
            Some("com.apple.Terminal")
        );
    }

    #[test]
    fn no_match_or_app_identity_uses_global_values() {
        let mut global = DictationState {
            auto_paste: true,
            cleanup_enabled: false,
            ..DictationState::default()
        };
        global.app_profiles = vec![profile("com.apple.Terminal", Some(false), Some(true))];

        for bundle_id in [None, Some("com.apple.Safari")] {
            let snapshot = resolve_test(&global, bundle_id, SessionOverrides::default());
            assert!(snapshot.delivery.auto_paste);
            assert!(!snapshot.transformations.cleanup_enabled);
            assert!(snapshot.matched_profile.is_none());
        }
    }

    #[test]
    fn session_overrides_have_highest_precedence() {
        let mut global = DictationState {
            auto_paste: false,
            cleanup_enabled: true,
            ..DictationState::default()
        };
        global.app_profiles = vec![profile("com.apple.Terminal", Some(true), Some(false))];

        let snapshot = resolve_test(
            &global,
            Some("com.apple.Terminal"),
            SessionOverrides {
                auto_paste: Some(false),
                cleanup_enabled: Some(true),
                cli_formatting_enabled: Some(false),
            },
        );

        assert!(!snapshot.delivery.auto_paste);
        assert!(snapshot.transformations.cleanup_enabled);
        assert_eq!(
            snapshot.transformations.cli_formatting_mode,
            CliFormattingMode::Disabled
        );
    }

    #[test]
    fn duplicate_profiles_preserve_first_supplied_override_per_field() {
        let mut global = DictationState {
            auto_paste: true,
            cleanup_enabled: true,
            ..DictationState::default()
        };
        global.app_profiles = vec![
            profile("com.apple.Terminal", None, Some(false)),
            profile("com.apple.Terminal", Some(false), Some(true)),
            profile("com.apple.Terminal", Some(true), None),
        ];
        global.app_profiles[0].label = "first match".to_string();
        global.app_profiles[1].label = "second match".to_string();

        let snapshot = resolve_test(
            &global,
            Some("com.apple.Terminal"),
            SessionOverrides::default(),
        );

        assert!(!snapshot.delivery.auto_paste);
        assert!(!snapshot.transformations.cleanup_enabled);
        assert_eq!(snapshot.matched_profile.unwrap().label, "first match");
    }

    #[test]
    fn resolved_snapshot_does_not_follow_later_settings_changes() {
        let mut global = DictationState {
            model_name: "base.en".to_string(),
            auto_paste: false,
            cleanup_enabled: false,
            ..DictationState::default()
        };
        let snapshot = resolve_test(&global, None, SessionOverrides::default());

        global.model_name = "small.en".to_string();
        global.auto_paste = true;
        global.cleanup_enabled = true;

        assert_eq!(snapshot.transcription.model_name, "base.en");
        assert!(!snapshot.delivery.auto_paste);
        assert!(!snapshot.transformations.cleanup_enabled);
    }

    #[test]
    fn cli_profile_override_resolves_auto_enabled_and_disabled_modes() {
        let mut global = DictationState::default();
        assert_eq!(
            resolve_test(&global, None, SessionOverrides::default())
                .transformations
                .cli_formatting_mode,
            CliFormattingMode::Auto
        );

        let mut enabled = profile("com.apple.Terminal", None, None);
        enabled.cli_formatting_override = Some(true);
        let mut disabled = profile("com.apple.mail", None, None);
        disabled.cli_formatting_override = Some(false);
        global.app_profiles = vec![enabled, disabled];

        assert_eq!(
            resolve_test(
                &global,
                Some("com.apple.Terminal"),
                SessionOverrides::default(),
            )
            .transformations
            .cli_formatting_mode,
            CliFormattingMode::Enabled
        );
        assert_eq!(
            resolve_test(&global, Some("com.apple.mail"), SessionOverrides::default(),)
                .transformations
                .cli_formatting_mode,
            CliFormattingMode::Disabled
        );

        assert_eq!(
            resolve_test(
                &global,
                Some("com.apple.Terminal"),
                SessionOverrides {
                    cli_formatting_enabled: Some(false),
                    ..SessionOverrides::default()
                },
            )
            .transformations
            .cli_formatting_mode,
            CliFormattingMode::Disabled
        );
    }

    #[test]
    fn context_capture_is_deny_by_default_without_disabling_clipboard_delivery() {
        let global = DictationState {
            auto_paste: true,
            ..DictationState::default()
        };
        let snapshot = resolve_test(&global, None, SessionOverrides::default());

        assert_eq!(
            snapshot.context_capture,
            ContextCapturePermissions::default()
        );
        assert!(snapshot.delivery.auto_paste);
    }
}
