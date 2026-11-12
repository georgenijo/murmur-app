use crate::model_runtime::ModelRuntimeManager;
use crate::MutexExt;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub use crate::transcriber::WHISPER_SAMPLE_RATE;

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DictationStatus {
    #[default]
    Idle,
    Starting,
    Recording,
    Recovering,
    Processing,
}

/// Per-app dictation profile. When the frontmost macOS app's bundle id matches
/// `bundle_id`, each `*_override` (when `Some`) replaces the corresponding global
/// setting in the immutable recording-start snapshot. `None` means "no override
/// — use global".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WritingStyle {
    Inherit,
    Conversational,
    Polished,
    CodeTechnical,
    Verbatim,
    Notes,
}

impl WritingStyle {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Inherit => "inherit",
            Self::Conversational => "conversational",
            Self::Polished => "polished",
            Self::CodeTechnical => "code_technical",
            Self::Verbatim => "verbatim",
            Self::Notes => "notes",
        }
    }

    pub fn code(self) -> u64 {
        match self {
            Self::Inherit => 0,
            Self::Conversational => 1,
            Self::Polished => 2,
            Self::CodeTechnical => 3,
            Self::Verbatim => 4,
            Self::Notes => 5,
        }
    }
}

/// Content-free text register for one recording. It names the register that
/// the frozen writing style resolves to, so pipeline stages can gate
/// context-sensitive rules without receiving profile names, settings values,
/// or transcript text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextRegister {
    Everyday,
    Messages,
    Email,
    Notes,
    Technical,
    Terminal,
    Verbatim,
}

impl TextRegister {
    /// Resolve the register from the effective writing style. Forced command
    /// formatting distinguishes Terminal from Technical.
    pub fn resolve(style: WritingStyle, forced_commands: bool) -> Self {
        match style {
            WritingStyle::Inherit => Self::Everyday,
            WritingStyle::Conversational => Self::Messages,
            WritingStyle::Polished => Self::Email,
            WritingStyle::Notes => Self::Notes,
            WritingStyle::CodeTechnical if forced_commands => Self::Terminal,
            WritingStyle::CodeTechnical => Self::Technical,
            WritingStyle::Verbatim => Self::Verbatim,
        }
    }

    pub fn is_technical(self) -> bool {
        matches!(self, Self::Technical | Self::Terminal)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppProfile {
    pub bundle_id: String,
    pub label: String,
    /// Override the global auto-paste setting for this app.
    pub auto_paste_override: Option<bool>,
    /// Override the global transcript-cleanup setting for this app (e.g. force
    /// verbatim output in a code editor, or force cleanup in an email client).
    #[serde(default)]
    pub cleanup_override: Option<bool>,
    /// Override spoken CLI canonicalization for this app. `None` keeps bounded
    /// automatic detection; `Some(true)` enables profile-mode detection and
    /// `Some(false)` disables implicit formatting (explicit "command" still works).
    #[serde(default)]
    pub cli_formatting_override: Option<bool>,
    /// Override deterministic prose smart formatting for this app. `None`
    /// inherits the global setting; code/verbatim profiles can force it off.
    #[serde(default)]
    pub smart_formatting_override: Option<bool>,
    /// Explicit local writing style. `None` is Inherit and preserves the
    /// pre-style resolver path byte-for-byte.
    #[serde(default)]
    pub writing_style: Option<WritingStyle>,
    /// Explicit opt-in to the memory-only local project index for this profile.
    /// Murmur never infers this from the app label or bundle identifier.
    #[serde(default)]
    pub ide_context_enabled: bool,
    /// User-selected project roots. Only this configuration persists; index
    /// contents remain memory-only and are rebuilt locally.
    #[serde(default)]
    pub ide_project_roots: Vec<String>,
}

/// Resolve the effective technical-context signal for an app profile. A
/// technical writing style or local IDE context is an explicit user signal;
/// bundle IDs and app labels are never guessed.
///
/// Duplicate profiles retain the resolver's legacy field semantics: IDE
/// context is owned by the first matching profile, while an absent or
/// `Inherit` writing style falls through to the next matching profile.
pub(crate) fn app_profiles_enable_code_vocabulary(
    app_profiles: &[AppProfile],
    bundle_id: &str,
) -> bool {
    let mut matching = app_profiles
        .iter()
        .filter(|profile| profile.bundle_id == bundle_id);
    let ide_context_enabled = matching
        .clone()
        .next()
        .is_some_and(|profile| profile.ide_context_enabled);
    let writing_style = matching.find_map(|profile| {
        profile
            .writing_style
            .filter(|style| *style != WritingStyle::Inherit)
    });

    ide_context_enabled || writing_style == Some(WritingStyle::CodeTechnical)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VocabularyScope {
    #[default]
    Global,
    App {
        #[serde(rename = "bundleId")]
        bundle_id: String,
    },
    Project {
        #[serde(rename = "bundleId")]
        bundle_id: String,
        root: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VocabularyEntry {
    pub id: String,
    pub written: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub scope: VocabularyScope,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DictationState {
    pub status: DictationStatus,
    pub model_name: String,
    pub language: String,
    pub auto_paste: bool,
    #[serde(default)]
    pub smart_destination_enabled: bool,
    pub auto_paste_delay_ms: u64,
    pub vad_sensitivity: u32,
    pub custom_vocabulary: String,
    #[serde(default)]
    pub vocabulary_entries: Vec<VocabularyEntry>,
    pub smart_punctuation: bool,
    /// Experimental: score speech with an incremental VAD while recording so
    /// Stop has no whole-clip VAD pass (workstream E1). Off by default; the
    /// batch VAD remains the fallback. Resolved once at recording start.
    #[serde(default)]
    pub streaming_pipeline_enabled: bool,
    pub save_transcript: bool,
    pub save_audio: bool,
    /// Opt-in: mirror each final transcript to a local file NotchPill can tail.
    pub mirror_to_notchpill: bool,
    pub output_dir: String,
    /// Per-app profiles resolved once from the frontmost app at recording start.
    pub app_profiles: Vec<AppProfile>,
    /// Built-in spoken formatting commands ("new line", "period", "scratch
    /// that"). Shown in Settings as "Spoken formatting".
    pub voice_commands_enabled: bool,
    /// Rule-based transcript cleanup (filler removal, spacing/capitalization)
    /// applied before injection. Off by default.
    pub cleanup_enabled: bool,
    /// When cleanup is enabled, remove standalone filler tokens ("um", "uh").
    pub cleanup_remove_filler: bool,
    /// When cleanup is enabled, capitalize sentence starts.
    pub cleanup_capitalize: bool,
    /// Deterministic lists, explicit symbols, and bounded same-utterance
    /// backtracking. Off by default and independently configurable.
    pub smart_formatting_enabled: bool,
    /// Code-aware vocabulary: when enabled, identifiers scanned from
    /// `code_vocab_folder` are fed to Whisper as an initial prompt to bias
    /// transcription toward the user's code terms. Whisper backend only.
    pub code_vocab_enabled: bool,
    /// Absolute path to the project folder scanned for code identifiers.
    pub code_vocab_folder: String,
    /// Cached prompt built from the last scan of `code_vocab_folder`. Rebuilt
    /// when the folder/enabled flag changes so we don't rescan every utterance.
    /// `None` means "not yet scanned" (build lazily on first use).
    pub code_vocab_prompt: Option<String>,
    /// Correlates the currently running explicit folder scan. A newer scan or a
    /// settings change supersedes the previous id so its result cannot be
    /// adopted after the user's intent has moved on.
    pub code_vocab_scan_id: Option<String>,
    /// Post-model correction (Tier 1 exact map + Tier 2 sounds-like). Applies the
    /// vocabulary to the text *output* of every backend (not just Whisper's
    /// prompt), so the default Parakeet engine benefits too.
    pub correction_enabled: bool,
    /// Tier 2 phonetic / edit-distance "sounds-like" matching. Gated under
    /// `correction_enabled`.
    pub correction_fuzzy: bool,
    /// Optional on-device homophone context tier. Off by default.
    pub homophone_context_enabled: bool,
}

impl DictationState {
    pub(crate) fn code_vocabulary_enabled_for(&self, bundle_id: Option<&str>) -> bool {
        self.code_vocab_enabled
            && bundle_id.is_some_and(|bundle_id| {
                app_profiles_enable_code_vocabulary(&self.app_profiles, bundle_id)
            })
    }
}

impl Default for DictationState {
    fn default() -> Self {
        Self {
            status: DictationStatus::Idle,
            model_name: "base.en".to_string(),
            // "auto" => whisper auto-detect (None lang param). Non-Whisper
            // backends ignore this. The frontend persists/overrides it via
            // configure_dictation; this is only the pre-configure fallback.
            language: "auto".to_string(),
            auto_paste: false,
            smart_destination_enabled: false,
            auto_paste_delay_ms: 0,
            vad_sensitivity: 50,
            custom_vocabulary: String::new(),
            vocabulary_entries: Vec::new(),
            smart_punctuation: true,
            streaming_pipeline_enabled: false,
            save_transcript: false,
            save_audio: false,
            mirror_to_notchpill: false,
            output_dir: String::new(),
            app_profiles: Vec::new(),
            voice_commands_enabled: false,
            cleanup_enabled: false,
            cleanup_remove_filler: true,
            cleanup_capitalize: true,
            smart_formatting_enabled: false,
            code_vocab_enabled: false,
            code_vocab_folder: String::new(),
            code_vocab_prompt: None,
            code_vocab_scan_id: None,
            // Post-model correction on by default: it's the fix that makes vocab
            // actually work on the default Parakeet engine. No-op without vocab.
            correction_enabled: true,
            correction_fuzzy: true,
            homophone_context_enabled: false,
        }
    }
}

struct ActiveDictationContext {
    recording_id: u64,
    snapshot: Arc<crate::dictation_context::DictationContextSnapshot>,
}

/// When the heartbeat may release an idle dictation model (issue #831).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelIdlePolicy {
    /// Release after this many idle minutes.
    AfterMinutes(u32),
    /// Stay loaded on external power; on battery, release after the default
    /// idle window so laptops keep the old memory behavior.
    NeverOnExternalPower,
    /// Never release on idle.
    Never,
}

impl ModelIdlePolicy {
    pub const DEFAULT_MINUTES: u32 = 5;
    /// Settings wire value for `NeverOnExternalPower`; minutes are otherwise
    /// non-negative and `0` already means `Never`.
    pub const NEVER_ON_EXTERNAL_POWER_SETTING: i64 = -1;

    /// Map the persisted `idleTimeoutMinutes` value. Unknown values fall back
    /// to the default window rather than keeping memory resident.
    pub fn from_setting(value: i64) -> Self {
        match value {
            0 => Self::Never,
            5 | 15 => Self::AfterMinutes(value as u32),
            Self::NEVER_ON_EXTERNAL_POWER_SETTING => Self::NeverOnExternalPower,
            _ => Self::default(),
        }
    }

    /// Idle window that applies now, or `None` when the model stays loaded.
    /// `on_external_power` is only consulted for `NeverOnExternalPower`.
    pub fn release_after(
        self,
        on_external_power: impl FnOnce() -> bool,
    ) -> Option<std::time::Duration> {
        let minutes = match self {
            Self::AfterMinutes(0) | Self::Never => return None,
            Self::AfterMinutes(minutes) => minutes,
            Self::NeverOnExternalPower if on_external_power() => return None,
            Self::NeverOnExternalPower => Self::DEFAULT_MINUTES,
        };
        Some(std::time::Duration::from_secs(u64::from(minutes) * 60))
    }
}

impl Default for ModelIdlePolicy {
    fn default() -> Self {
        Self::AfterMinutes(Self::DEFAULT_MINUTES)
    }
}

pub struct AppState {
    pub dictation: Mutex<DictationState>,
    /// Serializes recorder start/stop/cancel command transitions. Microphone
    /// initialization itself is asynchronous and owned by `audio_lifecycle`;
    /// this lock is never held while waiting for Core Audio or a channel.
    pub recording_transition: tokio::sync::Mutex<()>,
    pub model_runtime: ModelRuntimeManager,
    pub last_transcription_at: Mutex<Option<Instant>>,
    pub model_idle_policy: Mutex<ModelIdlePolicy>,
    /// Monotonically increasing ID assigned to each recording session.
    pub recording_id: AtomicU64,
    /// Exact recording generation currently owning the single live partial decode.
    pub dictation_partial_in_flight_id: AtomicU64,
    pub preview_holds_backend: AtomicBool,
    /// Decode-while-speaking session for the current recording (streaming
    /// setting, workstream E2). Inactive unless the recording asked for it.
    pub stream_reuse: Mutex<crate::stream_reuse::StreamReuse>,
    /// Incremental Silero VAD for the current recording (streaming setting).
    pub vad_stream: crate::vad_stream::StreamSlot,
    /// Last generation with a nonempty display-only decode; contains no text.
    pub dictation_nonempty_preview_id: AtomicU64,
    /// Exactly-once producer guard for accepted live-dictation terminal events.
    pub dictation_telemetry: crate::dictation_telemetry::DictationTelemetry,
    /// Monotonically increasing opaque ID assigned to every post-recognition
    /// transformation pass (live recordings and imported files).
    pub transcript_session_id: AtomicU64,
    /// Monotonically increasing ID assigned to each imported-file run. This is
    /// separate from `recording_id` so diagnostics and structured events can
    /// correlate file work without pretending it is a microphone recording.
    pub file_run_id: AtomicU64,
    /// Monotonic owner for long-running dual-channel meeting sessions.
    pub meeting_generation: AtomicU64,
    /// True from accepted meeting start until the capture helper, segmenter,
    /// and inference queue have all settled.
    pub meeting_active: AtomicBool,
    /// Covers both a live meeting's serialized ASR queue and startup recovery
    /// of crash-durable pending chunks. Other ASR entry points fail fast while
    /// this is set instead of waiting behind an invisible model lock.
    pub meeting_inference_active: AtomicBool,
    /// Monotonic revision for settings and vocabulary inputs captured by each
    /// immutable dictation context snapshot.
    pub settings_revision: AtomicU64,
    /// The immutable context owned by the active recording generation.
    active_context: Mutex<Option<ActiveDictationContext>>,
    /// Set to the recording_id of a cancelled recording. Pipeline checks
    /// `cancelled_id >= my_id` at checkpoints to discard cancelled work.
    pub cancelled_id: AtomicU64,
    /// True while a file transcription is running. Live recording and file
    /// transcription share one Whisper backend, so they must be mutually
    /// exclusive — this flag lets each path refuse to start over the other.
    pub file_transcribing: AtomicBool,
    /// Session-only microphone input-test owner. The exact preview generation
    /// remains claimed through capture recovery so no competing pipeline can
    /// start until the signed worker has exited and been joined.
    pub microphone_preview: crate::microphone_preview::MicrophonePreviewState,
    /// Compiled post-model correction matcher, rebuilt on settings-change in
    /// `configure_dictation`. Lives outside `DictationState` because the compiled
    /// Aho-Corasick automaton isn't serializable.
    pub correction_matcher:
        Mutex<Option<std::sync::Arc<crate::vocabulary_alias::CorrectionMatcherSet>>>,
    /// Short-lived local project indexes for explicitly opted-in app profiles.
    /// Contents (symbols and root-relative filenames) are never serialized.
    pub ide_context: Mutex<crate::ide_context::IdeContextStore>,
}

impl AppState {
    /// Whether meeting capture or crash recovery currently owns the shared ASR
    /// runtime. Callers still serialize ownership claims with
    /// `recording_transition`; this helper keeps the fail-fast policy uniform.
    pub fn meeting_blocks_asr(&self) -> bool {
        self.meeting_active.load(Ordering::SeqCst)
            || self.meeting_inference_active.load(Ordering::SeqCst)
    }

    /// Increment and return the next recording ID.
    pub fn next_recording_id(&self) -> u64 {
        self.recording_id.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn next_transcript_session_id(&self) -> u64 {
        self.transcript_session_id.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn next_file_run_id(&self) -> u64 {
        self.file_run_id.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn next_meeting_generation(&self) -> u64 {
        self.meeting_generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn bump_settings_revision(&self) -> u64 {
        self.settings_revision.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn set_active_context(
        &self,
        recording_id: u64,
        snapshot: Arc<crate::dictation_context::DictationContextSnapshot>,
    ) {
        *self.active_context.lock_or_recover() = Some(ActiveDictationContext {
            recording_id,
            snapshot,
        });
    }

    pub fn active_context(
        &self,
        recording_id: u64,
    ) -> Option<Arc<crate::dictation_context::DictationContextSnapshot>> {
        self.active_context
            .lock_or_recover()
            .as_ref()
            .filter(|active| active.recording_id == recording_id)
            .map(|active| Arc::clone(&active.snapshot))
    }

    /// Clear only the snapshot owned by `recording_id`. A stale guard must not
    /// erase the context installed by a newer recording generation.
    pub fn clear_active_context(&self, recording_id: u64) -> bool {
        let mut active = self.active_context.lock_or_recover();
        if active
            .as_ref()
            .is_some_and(|context| context.recording_id == recording_id)
        {
            *active = None;
            self.vad_stream.clear(recording_id);
            self.stream_reuse.lock_or_recover().clear(recording_id);
            true
        } else {
            false
        }
    }

    /// Mark a recording as cancelled by storing its ID.
    pub fn cancel_recording(&self, id: u64) {
        self.cancelled_id.fetch_max(id, Ordering::SeqCst);
    }

    /// Check whether a given recording ID has been cancelled.
    pub fn is_cancelled(&self, id: u64) -> bool {
        self.cancelled_id.load(Ordering::SeqCst) >= id
    }

    /// Non-blocking dictation phase read for hang diagnostics. `None` means
    /// the lock was contended, which is itself a reportable fact — a probe
    /// investigating a stall must never join the queue behind it.
    pub fn dictation_status_if_uncontended(&self) -> Option<DictationStatus> {
        self.dictation
            .try_lock_or_recover()
            .map(|dictation| dictation.status)
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            dictation: Mutex::new(DictationState::default()),
            recording_transition: tokio::sync::Mutex::new(()),
            model_runtime: ModelRuntimeManager::default(),
            last_transcription_at: Mutex::new(None),
            model_idle_policy: Mutex::new(ModelIdlePolicy::default()),
            recording_id: AtomicU64::new(0),
            dictation_partial_in_flight_id: AtomicU64::new(0),
            preview_holds_backend: AtomicBool::new(false),
            stream_reuse: Mutex::new(crate::stream_reuse::StreamReuse::default()),
            vad_stream: crate::vad_stream::StreamSlot::default(),
            dictation_nonempty_preview_id: AtomicU64::new(0),
            dictation_telemetry: crate::dictation_telemetry::DictationTelemetry::default(),
            transcript_session_id: AtomicU64::new(0),
            file_run_id: AtomicU64::new(0),
            meeting_generation: AtomicU64::new(0),
            meeting_active: AtomicBool::new(false),
            meeting_inference_active: AtomicBool::new(false),
            settings_revision: AtomicU64::new(0),
            active_context: Mutex::new(None),
            cancelled_id: AtomicU64::new(0),
            file_transcribing: AtomicBool::new(false),
            microphone_preview: crate::microphone_preview::MicrophonePreviewState::default(),
            correction_matcher: Mutex::new(None),
            ide_context: Mutex::new(crate::ide_context::IdeContextStore::default()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dictation_context::{resolve, ResolverInputs};

    #[test]
    fn model_idle_policy_maps_settings_and_power_source() {
        use std::time::Duration;
        let five = Some(Duration::from_secs(5 * 60));
        assert_eq!(ModelIdlePolicy::default().release_after(|| true), five);
        assert_eq!(
            ModelIdlePolicy::from_setting(0).release_after(|| false),
            None
        );
        assert_eq!(
            ModelIdlePolicy::from_setting(15).release_after(|| true),
            Some(Duration::from_secs(15 * 60))
        );
        for unknown in [-2, 1, 7, 60, i64::MAX] {
            assert_eq!(
                ModelIdlePolicy::from_setting(unknown),
                ModelIdlePolicy::default()
            );
        }
        let on_power = ModelIdlePolicy::from_setting(-1);
        assert_eq!(on_power, ModelIdlePolicy::NeverOnExternalPower);
        assert_eq!(on_power.release_after(|| true), None);
        assert_eq!(on_power.release_after(|| false), five);
        // Fixed windows never probe the power source.
        assert_eq!(
            ModelIdlePolicy::AfterMinutes(5).release_after(|| panic!("power source read")),
            five
        );
    }

    #[test]
    fn dictation_defaults_to_immediate_paste() {
        assert_eq!(DictationState::default().auto_paste_delay_ms, 0);
    }

    #[test]
    fn smart_destination_is_opt_in_across_persisted_state_migration() {
        let mut stored = serde_json::to_value(DictationState::default()).expect("serialize state");
        stored
            .as_object_mut()
            .expect("dictation state object")
            .remove("smart_destination_enabled");
        let migrated: DictationState =
            serde_json::from_value(stored).expect("deserialize pre-feature state");
        assert!(!migrated.smart_destination_enabled);

        let enabled = DictationState {
            smart_destination_enabled: true,
            ..DictationState::default()
        };
        let round_trip: DictationState =
            serde_json::from_value(serde_json::to_value(enabled).expect("serialize enabled state"))
                .expect("deserialize enabled state");
        assert!(round_trip.smart_destination_enabled);
    }

    fn snapshot(model_name: &str) -> Arc<crate::dictation_context::DictationContextSnapshot> {
        let settings = DictationState {
            model_name: model_name.to_string(),
            ..DictationState::default()
        };
        Arc::new(resolve(ResolverInputs {
            bundle_id: None,
            global: &settings,
            prompt: None,
            correction_matcher: None,
            ide_context_index: None,
            vocabulary_version: 0,
        }))
    }

    #[test]
    fn recording_transition_allows_only_one_audio_operation() {
        let state = AppState::default();
        let first = state.recording_transition.try_lock().unwrap();
        assert!(state.recording_transition.try_lock().is_err());
        drop(first);
        assert!(state.recording_transition.try_lock().is_ok());
    }

    #[test]
    fn transcript_session_ids_are_monotonic() {
        let state = AppState::default();
        assert_eq!(state.next_transcript_session_id(), 1);
        assert_eq!(state.next_transcript_session_id(), 2);
    }

    #[test]
    fn stale_generation_cannot_read_or_clear_newer_context() {
        let state = AppState::default();
        state.set_active_context(2, snapshot("small.en"));

        assert!(state.active_context(1).is_none());
        assert!(!state.clear_active_context(1));
        assert_eq!(
            state.active_context(2).unwrap().transcription.model_name,
            "small.en"
        );
        assert!(state.clear_active_context(2));
        assert!(state.active_context(2).is_none());
    }

    #[test]
    fn active_context_ignores_settings_changes_during_recording() {
        let state = AppState::default();
        state.set_active_context(1, snapshot("base.en"));
        state.dictation.lock_or_recover().model_name = "small.en".to_string();

        assert_eq!(
            state.active_context(1).unwrap().transcription.model_name,
            "base.en"
        );
    }

    #[test]
    fn active_context_keeps_accepted_smart_destination_after_settings_change() {
        use crate::frontmost::{
            AppTransitionSnapshot, DeliveryTargetIdentity, DeliveryTargetSnapshot,
            ProcessInstanceToken,
        };

        let target = DeliveryTargetSnapshot::Complete(DeliveryTargetIdentity {
            bundle_id: "com.example.Editor".into(),
            process_id: 42,
            process_instance: ProcessInstanceToken::for_test(7),
            window_token: None,
            transitions: AppTransitionSnapshot {
                activation_generation: 0,
                space_generation: 0,
            },
        });
        let mut accepted = snapshot("base.en");
        Arc::make_mut(&mut accepted).app.smart_destination =
            crate::smart_destination::Snapshot::Target {
                token: 11,
                revision: 3,
                identity: target,
                current_app: true,
            };

        let state = AppState::default();
        state.dictation.lock_or_recover().smart_destination_enabled = true;
        state.set_active_context(7, accepted);
        state.dictation.lock_or_recover().smart_destination_enabled = false;

        let active = state.active_context(7).expect("accepted context");
        assert!(matches!(
            &active.app.smart_destination,
            crate::smart_destination::Snapshot::Target {
                token: 11,
                revision: 3,
                ..
            }
        ));
    }
}
