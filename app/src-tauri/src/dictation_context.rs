//! Immutable, per-recording dictation context resolution.
//!
//! The resolver is the only place where global settings, per-app profiles, and
//! one-session overrides are combined. The resulting snapshot contains only the
//! typed values consumed by the live pipeline; later settings or focus changes
//! cannot alter an in-flight dictation.

use crate::cli_command::CliFormattingMode;
use crate::correction::CorrectionMatcher;
use crate::ide_context::IdeContextIndex;
use crate::state::{AppProfile, DictationState, WritingStyle};
use crate::voice_commands::ResolvedVoiceCommand;
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
    pub smart_formatting_override: Option<bool>,
    pub writing_style: Option<WritingStyle>,
    pub ide_context_enabled: bool,
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
    /// Permission to use the configured local project roots. This never grants
    /// screen, selection, or clipboard reads.
    pub local_project_index: bool,
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
    pub voice_commands: Vec<ResolvedVoiceCommand>,
    pub correction_enabled: bool,
    pub correction_matcher: Option<Arc<CorrectionMatcher>>,
    pub cli_formatting_mode: CliFormattingMode,
    pub cli_formatting_enabled: bool,
    pub smart_formatting_enabled: bool,
    pub ide_context_enabled: bool,
    pub ide_context_index: Option<Arc<IdeContextIndex>>,
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
    pub writing_style: WritingStyle,
}

/// Ephemeral overrides supplied by the recording trigger. No caller supplies
/// them today, but keeping them explicit makes precedence testable and avoids a
/// second resolution path when session-specific behavior is introduced.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SessionOverrides {
    pub auto_paste: Option<bool>,
    pub cleanup_enabled: Option<bool>,
    pub cli_formatting_enabled: Option<bool>,
    pub smart_formatting_enabled: Option<bool>,
}

pub struct ResolverInputs<'a> {
    pub bundle_id: Option<&'a str>,
    pub global: &'a DictationState,
    pub prompt: Option<String>,
    pub correction_matcher: Option<Arc<CorrectionMatcher>>,
    pub ide_context_index: Option<Arc<IdeContextIndex>>,
    pub vocabulary_version: u64,
    /// Repository-backed commands already filtered for the active app. `None`
    /// preserves legacy in-memory pairs when the local store is unavailable.
    pub voice_commands: Option<Vec<ResolvedVoiceCommand>>,
    pub session_overrides: SessionOverrides,
}

/// Resolve global defaults -> matching app profiles -> one-session overrides.
///
/// Duplicate profiles intentionally preserve the legacy behavior: for each
/// field, the first matching profile that supplies that override wins. A
/// matching entry with `None` falls through to the next duplicate entry.
pub fn resolve(inputs: ResolverInputs<'_>) -> DictationContextSnapshot {
    let global = inputs.global;
    let explicit_profile = inputs.bundle_id.and_then(|bundle_id| {
        global
            .app_profiles
            .iter()
            .find(|profile| profile.bundle_id == bundle_id)
    });
    let ide_context_enabled = explicit_profile.is_some_and(|profile| profile.ide_context_enabled);
    let writing_style =
        resolve_profile_optional(inputs.bundle_id, &global.app_profiles, |profile| {
            profile
                .writing_style
                .filter(|style| *style != WritingStyle::Inherit)
        })
        .unwrap_or(WritingStyle::Inherit);
    let style = StylePolicy::for_style(writing_style);
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
            style.cleanup_enabled.unwrap_or(global.cleanup_enabled),
            inputs.bundle_id,
            &global.app_profiles,
            |profile| profile.cleanup_override,
        )
    });
    let cli_override = inputs.session_overrides.cli_formatting_enabled.or_else(|| {
        resolve_profile_optional(inputs.bundle_id, &global.app_profiles, |profile| {
            profile.cli_formatting_override
        })
    });
    let cli_formatting_mode = match cli_override {
        Some(true) => CliFormattingMode::Enabled,
        Some(false) => CliFormattingMode::Disabled,
        None => style.cli_formatting_mode.unwrap_or(CliFormattingMode::Auto),
    };
    let cli_formatting_enabled = cli_override.is_some() || style.cli_formatting_enabled;
    let resolved_smart_formatting = inputs
        .session_overrides
        .smart_formatting_enabled
        .unwrap_or_else(|| {
            resolve_profile_override(
                style
                    .smart_formatting_enabled
                    .unwrap_or(global.smart_formatting_enabled),
                inputs.bundle_id,
                &global.app_profiles,
                |profile| profile.smart_formatting_override,
            )
        });
    // The explicit local-project opt-in defines a code context. Deterministic
    // prose rewriting is always bypassed there, even if another style or
    // fine-tuning override would otherwise enable it.
    let smart_formatting_enabled = !ide_context_enabled && resolved_smart_formatting;
    let matched_profile = explicit_profile.map(|profile| MatchedAppProfile {
        bundle_id: profile.bundle_id.clone(),
        label: profile.label.clone(),
        auto_paste_override: profile.auto_paste_override,
        cleanup_override: profile.cleanup_override,
        cli_formatting_override: profile.cli_formatting_override,
        smart_formatting_override: profile.smart_formatting_override,
        writing_style: profile.writing_style,
        ide_context_enabled: profile.ide_context_enabled,
    });
    let custom_vocab = crate::vocabulary_alias::has_applicable_entries(
        &global.vocabulary_entries,
        inputs.bundle_id,
        &global.app_profiles,
    );
    let code_vocab = global.code_vocab_enabled;
    let source = match (custom_vocab, code_vocab) {
        (false, false) => VocabularySource::None,
        (true, false) => VocabularySource::Custom,
        (false, true) => VocabularySource::CodeAware,
        (true, true) => VocabularySource::CustomAndCodeAware,
    };
    let voice_commands = style
        .voice_commands_enabled
        .unwrap_or(global.voice_commands_enabled);
    let resolved_voice_commands = inputs.voice_commands.unwrap_or_else(|| {
        global
            .voice_command_pairs
            .iter()
            .enumerate()
            .map(|(index, command)| ResolvedVoiceCommand {
                id: format!("legacy-runtime-{index:08}"),
                phrase: command.phrase.clone(),
                command_type: crate::knowledge_store::VoiceCommandKind::TextReplacement,
                content: command.replacement.clone(),
                allow_clipboard_read: false,
                app_scoped: false,
            })
            .collect::<Vec<_>>()
    });
    let clipboard_read_allowed = voice_commands
        && resolved_voice_commands
            .iter()
            .any(|command| command.allow_clipboard_read);
    let custom_voice_commands = voice_commands && !resolved_voice_commands.is_empty();

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
            cleanup_remove_filler: style
                .cleanup_remove_filler
                .unwrap_or(global.cleanup_remove_filler),
            cleanup_capitalize: style
                .cleanup_capitalize
                .unwrap_or(global.cleanup_capitalize),
            voice_commands: resolved_voice_commands,
            correction_enabled: style
                .correction_enabled
                .unwrap_or(global.correction_enabled),
            correction_matcher: inputs.correction_matcher,
            cli_formatting_mode,
            cli_formatting_enabled,
            smart_formatting_enabled,
            ide_context_enabled,
            ide_context_index: if ide_context_enabled {
                inputs.ide_context_index
            } else {
                None
            },
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
            custom_voice_commands,
        },
        // Clipboard input is granted only when an applicable command explicitly
        // opts in; selected/screen text remain denied. Project indexing is separate.
        context_capture: ContextCapturePermissions {
            clipboard: clipboard_read_allowed,
            local_project_index: ide_context_enabled,
            ..ContextCapturePermissions::default()
        },
        writing_style,
    }
}

#[derive(Debug, Clone, Copy)]
struct StylePolicy {
    cleanup_enabled: Option<bool>,
    cleanup_remove_filler: Option<bool>,
    cleanup_capitalize: Option<bool>,
    voice_commands_enabled: Option<bool>,
    correction_enabled: Option<bool>,
    smart_formatting_enabled: Option<bool>,
    cli_formatting_mode: Option<CliFormattingMode>,
    cli_formatting_enabled: bool,
}

impl StylePolicy {
    fn for_style(style: WritingStyle) -> Self {
        let inherit = Self {
            cleanup_enabled: None,
            cleanup_remove_filler: None,
            cleanup_capitalize: None,
            voice_commands_enabled: None,
            correction_enabled: None,
            smart_formatting_enabled: None,
            cli_formatting_mode: None,
            cli_formatting_enabled: true,
        };
        match style {
            WritingStyle::Inherit => inherit,
            WritingStyle::Conversational => Self {
                cleanup_enabled: Some(true),
                cleanup_remove_filler: Some(true),
                cleanup_capitalize: Some(true),
                smart_formatting_enabled: Some(false),
                cli_formatting_mode: Some(CliFormattingMode::Disabled),
                ..inherit
            },
            WritingStyle::Polished => Self {
                cleanup_enabled: Some(true),
                cleanup_remove_filler: Some(true),
                cleanup_capitalize: Some(true),
                correction_enabled: Some(true),
                smart_formatting_enabled: Some(true),
                cli_formatting_mode: Some(CliFormattingMode::Disabled),
                ..inherit
            },
            WritingStyle::CodeTechnical => Self {
                cleanup_enabled: Some(false),
                voice_commands_enabled: Some(false),
                correction_enabled: Some(true),
                smart_formatting_enabled: Some(false),
                cli_formatting_mode: Some(CliFormattingMode::Enabled),
                ..inherit
            },
            WritingStyle::Verbatim => Self {
                cleanup_enabled: Some(false),
                voice_commands_enabled: Some(false),
                correction_enabled: Some(false),
                smart_formatting_enabled: Some(false),
                cli_formatting_mode: Some(CliFormattingMode::Disabled),
                cli_formatting_enabled: false,
                ..inherit
            },
            WritingStyle::Notes => Self {
                cleanup_enabled: Some(true),
                cleanup_remove_filler: Some(true),
                cleanup_capitalize: Some(false),
                correction_enabled: Some(true),
                smart_formatting_enabled: Some(true),
                cli_formatting_mode: Some(CliFormattingMode::Disabled),
                ..inherit
            },
        }
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

    fn transform_with_snapshot(raw: &str, snapshot: &DictationContextSnapshot) -> String {
        let context = crate::transcript_transform::TranscriptContext {
            session_id: 1,
            source: crate::transcript_transform::TranscriptSource::Live,
            context_handle: Some("test-context".to_string()),
            cli_formatting_mode: snapshot.transformations.cli_formatting_mode,
            stages: crate::transcript_transform::TranscriptStageConfig {
                cleanup_enabled: snapshot.transformations.cleanup_enabled,
                cleanup_remove_filler: snapshot.transformations.cleanup_remove_filler,
                cleanup_capitalize: snapshot.transformations.cleanup_capitalize,
                voice_commands_enabled: snapshot.enabled_command_groups.built_in_voice_commands,
                smart_correction_enabled: snapshot.transformations.correction_enabled,
                smart_formatting_enabled: snapshot.transformations.smart_formatting_enabled,
                ide_context_enabled: snapshot.transformations.ide_context_enabled,
                cli_command_enabled: snapshot.transformations.cli_formatting_enabled,
            },
        };
        crate::transcript_transform::transform_transcript(
            raw.to_string(),
            &context,
            crate::transcript_transform::TranscriptTransformResources::empty(),
        )
        .unwrap()
        .text
    }

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
            smart_formatting_override: None,
            writing_style: None,
            ide_context_enabled: false,
            ide_project_roots: Vec::new(),
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
            ide_context_index: None,
            vocabulary_version: 7,
            voice_commands: None,
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
                ..SessionOverrides::default()
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
    fn smart_formatting_resolves_global_profile_and_session_precedence() {
        let mut global = DictationState {
            smart_formatting_enabled: false,
            ..DictationState::default()
        };
        let mut terminal = profile("com.apple.Terminal", None, None);
        terminal.smart_formatting_override = Some(true);
        global.app_profiles = vec![terminal];

        let profile_snapshot = resolve_test(
            &global,
            Some("com.apple.Terminal"),
            SessionOverrides::default(),
        );
        assert!(profile_snapshot.transformations.smart_formatting_enabled);

        let session_snapshot = resolve_test(
            &global,
            Some("com.apple.Terminal"),
            SessionOverrides {
                smart_formatting_enabled: Some(false),
                ..SessionOverrides::default()
            },
        );
        assert!(!session_snapshot.transformations.smart_formatting_enabled);

        global.smart_formatting_enabled = true;
        assert!(profile_snapshot.transformations.smart_formatting_enabled);
        assert!(!session_snapshot.transformations.smart_formatting_enabled);
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

    #[test]
    fn ide_context_requires_explicit_matching_profile_and_bypasses_prose() {
        let mut global = DictationState {
            smart_formatting_enabled: true,
            ..DictationState::default()
        };
        let mut editor = profile("com.example.Editor", None, None);
        editor.ide_context_enabled = true;
        editor.ide_project_roots = vec!["/explicit/project".to_string()];
        global.app_profiles = vec![editor];

        let opted_in = resolve_test(
            &global,
            Some("com.example.Editor"),
            SessionOverrides::default(),
        );
        assert!(opted_in.transformations.ide_context_enabled);
        assert!(!opted_in.transformations.smart_formatting_enabled);
        assert!(opted_in.context_capture.local_project_index);
        assert!(!opted_in.context_capture.surrounding_screen_text);
        assert!(!opted_in.context_capture.selected_text);
        assert!(!opted_in.context_capture.clipboard);

        let ide_named_but_unconfigured = resolve_test(
            &global,
            Some("com.apple.dt.Xcode"),
            SessionOverrides::default(),
        );
        assert!(
            !ide_named_but_unconfigured
                .transformations
                .ide_context_enabled
        );
        assert!(
            ide_named_but_unconfigured
                .transformations
                .smart_formatting_enabled
        );
        assert_eq!(
            ide_named_but_unconfigured.context_capture,
            ContextCapturePermissions::default()
        );
    }

    #[test]
    fn writing_styles_resolve_only_typed_transformation_policy() {
        let mut global = DictationState {
            model_name: "small.en".to_string(),
            language: "es".to_string(),
            auto_paste: true,
            save_transcript: true,
            output_dir: "/tmp/murmur-style-test".to_string(),
            cleanup_enabled: false,
            voice_commands_enabled: true,
            smart_formatting_enabled: false,
            correction_enabled: false,
            ..DictationState::default()
        };
        let mut profile = profile("com.example.Editor", None, None);
        profile.writing_style = Some(WritingStyle::CodeTechnical);
        global.app_profiles = vec![profile];

        let snapshot = resolve_test(
            &global,
            Some("com.example.Editor"),
            SessionOverrides::default(),
        );

        assert_eq!(snapshot.writing_style, WritingStyle::CodeTechnical);
        assert!(!snapshot.transformations.cleanup_enabled);
        assert!(!snapshot.enabled_command_groups.built_in_voice_commands);
        assert!(!snapshot.transformations.smart_formatting_enabled);
        assert!(snapshot.transformations.correction_enabled);
        assert_eq!(
            snapshot.transformations.cli_formatting_mode,
            CliFormattingMode::Enabled
        );
        assert!(snapshot.transformations.cli_formatting_enabled);
        assert_eq!(snapshot.transcription.model_name, "small.en");
        assert_eq!(snapshot.transcription.language, "es");
        assert!(snapshot.delivery.auto_paste);
        assert!(snapshot.delivery.save_transcript);
        assert_eq!(snapshot.delivery.output_dir, "/tmp/murmur-style-test");
        assert_eq!(
            snapshot.context_capture,
            ContextCapturePermissions::default()
        );
    }

    #[test]
    fn named_styles_have_transparent_deterministic_effects() {
        let mut global = DictationState {
            cleanup_enabled: false,
            cleanup_remove_filler: false,
            cleanup_capitalize: false,
            voice_commands_enabled: true,
            correction_enabled: false,
            smart_formatting_enabled: false,
            ..DictationState::default()
        };

        let cases = [
            (WritingStyle::Conversational, true, true, false, false, true),
            (WritingStyle::Polished, true, true, true, true, true),
            (WritingStyle::Notes, true, true, true, true, true),
            (WritingStyle::CodeTechnical, false, false, false, true, true),
            (WritingStyle::Verbatim, false, false, false, false, false),
        ];

        for (style, cleanup, commands, prose, correction, cli_stage) in cases {
            let mut app = profile("com.example.App", None, None);
            app.writing_style = Some(style);
            global.app_profiles = vec![app];
            let snapshot = resolve_test(
                &global,
                Some("com.example.App"),
                SessionOverrides::default(),
            );
            assert_eq!(snapshot.writing_style, style);
            assert_eq!(snapshot.transformations.cleanup_enabled, cleanup);
            assert_eq!(
                snapshot.enabled_command_groups.built_in_voice_commands,
                commands
            );
            assert_eq!(snapshot.transformations.smart_formatting_enabled, prose);
            assert_eq!(snapshot.transformations.correction_enabled, correction);
            assert_eq!(snapshot.transformations.cli_formatting_enabled, cli_stage);
        }
    }

    #[test]
    fn inherit_and_unclassified_apps_preserve_current_behavior() {
        let mut global = DictationState {
            cleanup_enabled: true,
            cleanup_remove_filler: false,
            cleanup_capitalize: false,
            voice_commands_enabled: true,
            correction_enabled: false,
            smart_formatting_enabled: true,
            ..DictationState::default()
        };
        let mut terminal = profile("com.apple.Terminal", None, None);
        terminal.writing_style = Some(WritingStyle::Inherit);
        global.app_profiles = vec![terminal];

        for bundle_id in [Some("com.apple.Terminal"), Some("com.apple.dt.Xcode"), None] {
            let snapshot = resolve_test(&global, bundle_id, SessionOverrides::default());
            assert_eq!(snapshot.writing_style, WritingStyle::Inherit);
            assert!(snapshot.transformations.cleanup_enabled);
            assert!(!snapshot.transformations.cleanup_remove_filler);
            assert!(!snapshot.transformations.cleanup_capitalize);
            assert!(snapshot.enabled_command_groups.built_in_voice_commands);
            assert!(!snapshot.transformations.correction_enabled);
            assert!(snapshot.transformations.smart_formatting_enabled);
            assert_eq!(
                snapshot.transformations.cli_formatting_mode,
                CliFormattingMode::Auto
            );
            assert!(snapshot.transformations.cli_formatting_enabled);
        }
    }

    #[test]
    fn duplicate_profiles_preserve_per_field_fallthrough_for_style() {
        let mut first = profile("com.example.Editor", None, Some(false));
        first.label = "first identity".to_string();
        let mut second = profile("com.example.Editor", Some(false), None);
        second.writing_style = Some(WritingStyle::Polished);
        let mut global = DictationState {
            auto_paste: true,
            cleanup_enabled: false,
            ..DictationState::default()
        };
        global.app_profiles = vec![first, second];

        let snapshot = resolve_test(
            &global,
            Some("com.example.Editor"),
            SessionOverrides::default(),
        );

        assert_eq!(snapshot.writing_style, WritingStyle::Polished);
        assert_eq!(snapshot.matched_profile.unwrap().label, "first identity");
        assert!(!snapshot.delivery.auto_paste);
        // First profile's explicit field still wins over the later style policy.
        assert!(!snapshot.transformations.cleanup_enabled);
        assert!(snapshot.transformations.smart_formatting_enabled);
    }

    #[test]
    fn explicit_profile_and_session_overrides_fine_tune_style() {
        let mut app = profile("com.example.Editor", None, Some(true));
        app.writing_style = Some(WritingStyle::Verbatim);
        app.smart_formatting_override = Some(true);
        app.cli_formatting_override = Some(true);
        let mut global = DictationState::default();
        global.app_profiles = vec![app];

        let snapshot = resolve_test(
            &global,
            Some("com.example.Editor"),
            SessionOverrides {
                cleanup_enabled: Some(false),
                smart_formatting_enabled: Some(false),
                cli_formatting_enabled: Some(false),
                ..SessionOverrides::default()
            },
        );

        assert!(!snapshot.transformations.cleanup_enabled);
        assert!(!snapshot.transformations.smart_formatting_enabled);
        assert_eq!(
            snapshot.transformations.cli_formatting_mode,
            CliFormattingMode::Disabled
        );
        assert!(snapshot.transformations.cli_formatting_enabled);
    }

    #[test]
    fn resolved_style_snapshot_is_immutable() {
        let mut app = profile("com.example.Editor", None, None);
        app.writing_style = Some(WritingStyle::Polished);
        let mut global = DictationState::default();
        global.app_profiles = vec![app];
        let snapshot = resolve_test(
            &global,
            Some("com.example.Editor"),
            SessionOverrides::default(),
        );

        global.app_profiles[0].writing_style = Some(WritingStyle::Verbatim);

        assert_eq!(snapshot.writing_style, WritingStyle::Polished);
        assert!(snapshot.transformations.cleanup_enabled);
        assert!(snapshot.transformations.smart_formatting_enabled);
    }

    #[test]
    fn style_transform_outputs_use_only_reviewed_deterministic_stages() {
        let mut global = DictationState {
            cleanup_enabled: false,
            voice_commands_enabled: false,
            correction_enabled: false,
            smart_formatting_enabled: false,
            ..DictationState::default()
        };

        let cases = [
            (
                "com.example.Chat",
                WritingStyle::Conversational,
                "um the tasks are first review second ship",
                "The tasks are first review second ship",
            ),
            (
                "com.apple.mail",
                WritingStyle::Polished,
                "um the tasks are first review second ship",
                "The tasks are:\n1. Review\n2. Ship",
            ),
            (
                "com.microsoft.VSCode",
                WritingStyle::CodeTechnical,
                "NPM run Tauri dev",
                "npm run tauri dev",
            ),
            (
                "com.apple.Notes",
                WritingStyle::Notes,
                "um the notes are first review second ship",
                "the notes are:\n1. Review\n2. Ship",
            ),
            (
                "com.apple.Terminal",
                WritingStyle::Verbatim,
                "  um command NPM new line  ",
                "  um command NPM new line  ",
            ),
        ];

        for (bundle_id, style, raw, expected) in cases {
            let mut app = profile(bundle_id, None, None);
            app.writing_style = Some(style);
            global.app_profiles = vec![app];
            let snapshot = resolve_test(&global, Some(bundle_id), SessionOverrides::default());
            assert_eq!(transform_with_snapshot(raw, &snapshot), expected);
        }
    }

    #[test]
    fn writing_style_telemetry_values_are_stable_and_content_free() {
        let styles = [
            (WritingStyle::Inherit, "inherit", 0),
            (WritingStyle::Conversational, "conversational", 1),
            (WritingStyle::Polished, "polished", 2),
            (WritingStyle::CodeTechnical, "code_technical", 3),
            (WritingStyle::Verbatim, "verbatim", 4),
            (WritingStyle::Notes, "notes", 5),
        ];
        for (style, name, code) in styles {
            assert_eq!(style.as_str(), name);
            assert_eq!(style.code(), code);
        }
    }

    #[test]
    fn verbatim_and_inherit_preserve_false_positive_inputs_byte_for_byte() {
        let raw = "  um NPM new line command cargo test first second  ";
        let mut app = profile("com.example.Verbatim", None, None);
        app.writing_style = Some(WritingStyle::Verbatim);
        let mut global = DictationState {
            cleanup_enabled: false,
            voice_commands_enabled: false,
            correction_enabled: false,
            smart_formatting_enabled: false,
            ..DictationState::default()
        };
        global.app_profiles = vec![app];

        let verbatim = resolve_test(
            &global,
            Some("com.example.Verbatim"),
            SessionOverrides::default(),
        );
        assert_eq!(
            transform_with_snapshot(raw, &verbatim).as_bytes(),
            raw.as_bytes()
        );

        // A terminal/editor-looking bundle id does not imply a style.
        let inherit = resolve_test(
            &global,
            Some("com.apple.Terminal"),
            SessionOverrides::default(),
        );
        assert_eq!(inherit.writing_style, WritingStyle::Inherit);
        assert_eq!(
            transform_with_snapshot(raw, &inherit).as_bytes(),
            raw.as_bytes()
        );
    }
}
