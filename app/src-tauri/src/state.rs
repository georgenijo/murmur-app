use crate::model_runtime::ModelRuntimeManager;
use crate::MutexExt;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub use crate::transcriber::WHISPER_SAMPLE_RATE;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DictationStatus {
    Idle,
    Recording,
    Processing,
}

impl Default for DictationStatus {
    fn default() -> Self {
        Self::Idle
    }
}

/// Status of the AX-selection transform pipeline (issue #312). Deliberately a
/// separate field on `AppState`, NOT a `DictationStatus` variant — dictation
/// recording/processing and a transform pass are independent activities that
/// each need to know about (and block) the other, not a shared state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransformStatus {
    Idle,
    /// Reading the AX selection (`selection::capture_selection`).
    Capturing,
    /// Selection captured; waiting for a follow-up spoken instruction.
    Listening,
    /// Running the transform (LLM call or equivalent) on the captured text.
    Thinking,
    /// Transform result is ready and awaiting user confirmation/edit.
    ReviewPending,
    /// Writing the accepted result back (clipboard/injection).
    Applying,
}

impl Default for TransformStatus {
    fn default() -> Self {
        Self::Idle
    }
}

impl TransformStatus {
    /// Human-readable event/telemetry string (see `log_capture_outcome`-style
    /// callers in `selection.rs`). Not called from production code yet — the
    /// transform pipeline that would log these phase transitions lands in a
    /// later PR in the #312 series; exercised directly by tests until then.
    #[allow(dead_code)]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Capturing => "capturing",
            Self::Listening => "listening",
            Self::Thinking => "thinking",
            Self::ReviewPending => "review_pending",
            Self::Applying => "applying",
        }
    }

    /// Whether this status should block starting a new dictation recording.
    /// Only `Idle` allows recording to start — every other transform phase is
    /// actively using the shared pipeline/clipboard/AX surface.
    pub fn blocks_recording(self) -> bool {
        self != Self::Idle
    }
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

/// A user-defined voice command: when `phrase` is spoken (matched
/// case-insensitively on word boundaries), it is replaced by `replacement`.
/// Applied after the built-in command set, so users can extend — not override —
/// the defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceCommand {
    pub phrase: String,
    pub replacement: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VocabularyScope {
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

impl Default for VocabularyScope {
    fn default() -> Self {
        Self::Global
    }
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
    pub auto_paste_delay_ms: u64,
    pub vad_sensitivity: u32,
    pub custom_vocabulary: String,
    #[serde(default)]
    pub vocabulary_entries: Vec<VocabularyEntry>,
    pub smart_punctuation: bool,
    pub save_transcript: bool,
    pub save_audio: bool,
    pub output_dir: String,
    /// Per-app profiles resolved once from the frontmost app at recording start.
    pub app_profiles: Vec<AppProfile>,
    pub voice_commands_enabled: bool,
    /// User-defined voice commands applied after the built-in set.
    #[serde(default)]
    pub voice_command_pairs: Vec<VoiceCommand>,
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
            auto_paste_delay_ms: 50,
            vad_sensitivity: 50,
            custom_vocabulary: String::new(),
            vocabulary_entries: Vec::new(),
            smart_punctuation: true,
            save_transcript: false,
            save_audio: false,
            output_dir: String::new(),
            app_profiles: Vec::new(),
            voice_commands_enabled: false,
            voice_command_pairs: Vec::new(),
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
        }
    }
}

struct ActiveDictationContext {
    recording_id: u64,
    snapshot: Arc<crate::dictation_context::DictationContextSnapshot>,
}

pub struct AppState {
    pub dictation: Mutex<DictationState>,
    /// Serializes recorder start/stop/cancel transitions. Audio startup waits
    /// for the cpal stream to become ready, so a fast key release must not tear
    /// the recorder down until that startup has fully completed.
    pub recording_transition: tokio::sync::Mutex<()>,
    pub model_runtime: ModelRuntimeManager,
    pub last_transcription_at: Mutex<Option<Instant>>,
    pub idle_timeout_minutes: Mutex<u32>,
    /// Monotonically increasing ID assigned to each recording session.
    pub recording_id: AtomicU64,
    /// Monotonically increasing opaque ID assigned to every post-recognition
    /// transformation pass (live recordings and imported files).
    pub transcript_session_id: AtomicU64,
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
    /// Compiled post-model correction matcher, rebuilt on settings-change in
    /// `configure_dictation`. Lives outside `DictationState` because the compiled
    /// Aho-Corasick automaton isn't serializable.
    pub correction_matcher:
        Mutex<Option<std::sync::Arc<crate::vocabulary_alias::CorrectionMatcherSet>>>,
    /// Enabled replacement rules from the local knowledge repository, ordered
    /// by its deterministic precedence. Refreshed only after repository writes;
    /// recording snapshots never query SQLite in the transform hot path.
    pub knowledge_replacements: Mutex<Arc<Vec<crate::knowledge_store::KnowledgeEntry>>>,
    /// Short-lived local project indexes for explicitly opted-in app profiles.
    /// Contents (symbols and root-relative filenames) are never serialized.
    pub ide_context: Mutex<crate::ide_context::IdeContextStore>,
    /// Current phase of the AX-selection transform pipeline (issue #312).
    /// Independent of `dictation.status` — see `TransformStatus`'s doc comment.
    pub transform_status: Mutex<TransformStatus>,
    /// Process-local monotonic ID allocator for physical transform-key passes.
    pub transform_pass_sequence: AtomicU64,
    /// The pass that currently owns the transform lifecycle. Zero means none.
    pub active_transform_pass_id: AtomicU64,
    /// One-based spoken-instruction attempt within the active pass. Retries
    /// keep the pass ID and advance only this counter.
    pub transform_instruction_attempt: AtomicU64,
    /// The single active transform session (captured selection + proposed
    /// replacement + applied flag), if any (issue #312, PR-B2). See
    /// `transform_apply::TransformSession` and its setter/getter helpers —
    /// accessed through those, not locked directly, everywhere except
    /// `transform_apply.rs` itself.
    pub transform_session: Mutex<Option<crate::transform_apply::TransformSession>>,
    /// Monotonic generation counter stamped onto every new `TransformSession`
    /// (issue #312 PR-B2). See `next_transform_session_generation`.
    pub transform_session_generation: AtomicU64,
    /// Monotonic epoch bumped at the start of every apply/undo AND on cancel
    /// (issue #312 PR-C2, B2 review nit N1). A `run_apply` paste fallback
    /// schedules a ~300ms-delayed clipboard restore off the main thread; if a
    /// newer apply/undo (or a cancel) begins inside that window, that stale
    /// restore would clobber the newer op's house-rule clipboard write. The
    /// delayed restore captures the epoch it was scheduled under and no-ops if
    /// this counter has since advanced.
    pub transform_apply_epoch: AtomicU64,
    /// Abort handle for the in-flight sidecar transform task (issue #312
    /// PR-C2). `finish_transform_instruction` spawns the `sidecar.transform`
    /// future as a task and stores its abort handle here so `cancel_transform`
    /// can abort the outer `tokio::spawn` wrapper. **Dropping that future does
    /// not clear the sidecar's busy flag** — `BusyGuard` only drops when the
    /// inner `spawn_blocking` work finishes. Callers must also invoke
    /// `LlmSidecar::cancel_inflight_request` so the blocking loop sends a
    /// protocol Cancel and settles promptly (see `cancel_transform`).
    pub transform_inflight:
        Mutex<Option<(tokio::task::AbortHandle, crate::llm_sidecar::CancelToken)>>,
}

impl AppState {
    /// Increment and return the next recording ID.
    pub fn next_recording_id(&self) -> u64 {
        self.recording_id.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn next_transcript_session_id(&self) -> u64 {
        self.transcript_session_id.fetch_add(1, Ordering::SeqCst) + 1
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

    /// Current transform pipeline phase. Independent of `dictation.status`.
    pub fn transform_status(&self) -> TransformStatus {
        *self.transform_status.lock_or_recover()
    }

    pub fn next_transform_pass_id(&self) -> u64 {
        self.transform_pass_sequence.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn active_transform_pass_id(&self) -> Option<u64> {
        match self.active_transform_pass_id.load(Ordering::SeqCst) {
            0 => None,
            id => Some(id),
        }
    }

    pub fn activate_transform_pass(&self, pass_id: u64) {
        debug_assert_ne!(pass_id, 0);
        self.transform_instruction_attempt
            .store(1, Ordering::SeqCst);
        self.active_transform_pass_id
            .store(pass_id, Ordering::SeqCst);
    }

    pub fn clear_transform_pass(&self, pass_id: u64) -> bool {
        self.active_transform_pass_id
            .compare_exchange(pass_id, 0, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    pub fn current_instruction_attempt(&self) -> u64 {
        self.transform_instruction_attempt
            .load(Ordering::SeqCst)
            .max(1)
    }

    pub fn next_instruction_attempt(&self) -> u64 {
        self.transform_instruction_attempt
            .fetch_add(1, Ordering::SeqCst)
            + 1
    }

    /// Set the transform pipeline phase. Independent of `dictation.status`.
    pub fn set_transform_status(&self, status: TransformStatus) {
        let mut current = self.transform_status.lock_or_recover();
        let from = *current;
        *current = status;
        if let Some(pass_id) = self.active_transform_pass_id() {
            crate::transform_trace::transition(pass_id, from.as_str(), status.as_str(), true);
        }
    }

    /// Atomically check-and-set the transform pipeline phase: transitions to
    /// `to` only if the current phase is exactly `from`, returning whether it
    /// did. Both the read and the write happen under a single lock
    /// acquisition, closing the TOCTOU window a separate
    /// `transform_status() == X` check followed by `set_transform_status(Y)`
    /// would leave open between two concurrent `apply_transform_result`/
    /// `undo_transform` command invocations (issue #312 PR-B2 review).
    pub fn try_transition_transform_status(
        &self,
        from: TransformStatus,
        to: TransformStatus,
    ) -> bool {
        let mut status = self.transform_status.lock_or_recover();
        let actual = *status;
        let won = actual == from;
        if won {
            *status = to;
        }
        if let Some(pass_id) = self.active_transform_pass_id() {
            crate::transform_trace::transition(pass_id, actual.as_str(), to.as_str(), won);
        }
        won
    }

    /// Next generation id for a new `TransformSession` (issue #312 PR-B2).
    /// Monotonically increasing so a `set_applied` call that was dispatched
    /// against an OLDER session (e.g. a slow main-thread apply still
    /// in-flight when the user starts a new transform pass) can detect it no
    /// longer applies and no-op instead of mutating the session that replaced
    /// it.
    pub fn next_transform_session_generation(&self) -> u64 {
        self.transform_session_generation
            .fetch_add(1, Ordering::SeqCst)
            + 1
    }

    /// Bump and return the next apply epoch (issue #312 PR-C2, nit N1). Called
    /// at the start of every apply/undo and by `cancel_transform`, so any
    /// clipboard restore still pending from an earlier op sees a changed epoch
    /// and declines to run.
    pub fn next_transform_apply_epoch(&self) -> u64 {
        self.transform_apply_epoch.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Current apply epoch, read by a pending clipboard restore to decide
    /// whether it is still the most recent op (see `next_transform_apply_epoch`).
    pub fn transform_apply_epoch(&self) -> u64 {
        self.transform_apply_epoch.load(Ordering::SeqCst)
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            dictation: Mutex::new(DictationState::default()),
            recording_transition: tokio::sync::Mutex::new(()),
            model_runtime: ModelRuntimeManager::default(),
            last_transcription_at: Mutex::new(None),
            idle_timeout_minutes: Mutex::new(5),
            recording_id: AtomicU64::new(0),
            transcript_session_id: AtomicU64::new(0),
            settings_revision: AtomicU64::new(0),
            active_context: Mutex::new(None),
            cancelled_id: AtomicU64::new(0),
            file_transcribing: AtomicBool::new(false),
            correction_matcher: Mutex::new(None),
            knowledge_replacements: Mutex::new(Arc::new(Vec::new())),
            ide_context: Mutex::new(crate::ide_context::IdeContextStore::default()),
            transform_status: Mutex::new(TransformStatus::default()),
            transform_pass_sequence: AtomicU64::new(0),
            active_transform_pass_id: AtomicU64::new(0),
            transform_instruction_attempt: AtomicU64::new(1),
            transform_session: Mutex::new(None),
            transform_session_generation: AtomicU64::new(0),
            transform_apply_epoch: AtomicU64::new(0),
            transform_inflight: Mutex::new(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dictation_context::{resolve, ResolverInputs, SessionOverrides};

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
            voice_commands: None,
            session_overrides: SessionOverrides::default(),
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
    fn transform_status_defaults_to_idle_and_does_not_block_recording() {
        let state = AppState::default();
        assert_eq!(state.transform_status(), TransformStatus::Idle);
        assert!(!state.transform_status().blocks_recording());
    }

    #[test]
    fn transform_status_setter_getter_roundtrip() {
        let state = AppState::default();
        for status in [
            TransformStatus::Capturing,
            TransformStatus::Listening,
            TransformStatus::Thinking,
            TransformStatus::ReviewPending,
            TransformStatus::Applying,
            TransformStatus::Idle,
        ] {
            state.set_transform_status(status);
            assert_eq!(state.transform_status(), status);
        }
    }

    #[test]
    fn try_transition_transform_status_only_succeeds_when_current_matches_from() {
        let state = AppState::default();
        assert_eq!(state.transform_status(), TransformStatus::Idle);

        // Wrong `from` -- no transition, status untouched.
        assert!(!state.try_transition_transform_status(
            TransformStatus::ReviewPending,
            TransformStatus::Applying
        ));
        assert_eq!(state.transform_status(), TransformStatus::Idle);

        // Correct `from` -- transitions and reports success.
        assert!(state
            .try_transition_transform_status(TransformStatus::Idle, TransformStatus::Capturing));
        assert_eq!(state.transform_status(), TransformStatus::Capturing);

        // A second call with the same `from` now fails -- status already moved on,
        // modeling two concurrent callers racing for the same transition.
        assert!(!state
            .try_transition_transform_status(TransformStatus::Idle, TransformStatus::Capturing));
        assert_eq!(state.transform_status(), TransformStatus::Capturing);
    }

    #[test]
    fn transform_session_generation_is_monotonic() {
        let state = AppState::default();
        assert_eq!(state.next_transform_session_generation(), 1);
        assert_eq!(state.next_transform_session_generation(), 2);
        assert_eq!(state.next_transform_session_generation(), 3);
    }

    #[test]
    fn transform_pass_ids_are_monotonic_and_stale_clears_cannot_remove_new_pass() {
        let state = AppState::default();
        let first = state.next_transform_pass_id();
        let second = state.next_transform_pass_id();
        assert_eq!((first, second), (1, 2));

        state.activate_transform_pass(first);
        assert_eq!(state.active_transform_pass_id(), Some(first));
        state.activate_transform_pass(second);
        assert!(!state.clear_transform_pass(first));
        assert_eq!(state.active_transform_pass_id(), Some(second));
        assert!(state.clear_transform_pass(second));
        assert_eq!(state.active_transform_pass_id(), None);
    }

    #[test]
    fn retries_keep_pass_id_and_advance_instruction_attempt() {
        let state = AppState::default();
        state.activate_transform_pass(7);
        assert_eq!(state.current_instruction_attempt(), 1);
        assert_eq!(state.next_instruction_attempt(), 2);
        assert_eq!(state.active_transform_pass_id(), Some(7));
    }

    #[test]
    fn every_non_idle_transform_status_blocks_recording_start() {
        for status in [
            TransformStatus::Capturing,
            TransformStatus::Listening,
            TransformStatus::Thinking,
            TransformStatus::ReviewPending,
            TransformStatus::Applying,
        ] {
            assert!(
                status.blocks_recording(),
                "{status:?} should block a new recording from starting"
            );
        }
        assert!(!TransformStatus::Idle.blocks_recording());
    }

    #[test]
    fn dictation_status_and_transform_status_are_independent_fields() {
        // "vice versa": neither status field's mutation affects the other —
        // they are two independent activities tracked on the same AppState,
        // not variants of a single shared state machine.
        let state = AppState::default();

        state.dictation.lock_or_recover().status = DictationStatus::Recording;
        assert_eq!(state.transform_status(), TransformStatus::Idle);

        state.set_transform_status(TransformStatus::Thinking);
        assert_eq!(
            state.dictation.lock_or_recover().status,
            DictationStatus::Recording
        );

        state.dictation.lock_or_recover().status = DictationStatus::Idle;
        assert_eq!(state.transform_status(), TransformStatus::Thinking);
    }

    #[test]
    fn transform_status_serde_form_matches_as_str() {
        // Regression guard: `#[serde(rename_all = "snake_case")]` must produce
        // the same wire string as `as_str()` for every variant. Before this
        // fix, `rename_all = "lowercase"` serialized `ReviewPending` as
        // "reviewpending", diverging from `as_str()`'s "review_pending".
        for status in [
            TransformStatus::Idle,
            TransformStatus::Capturing,
            TransformStatus::Listening,
            TransformStatus::Thinking,
            TransformStatus::ReviewPending,
            TransformStatus::Applying,
        ] {
            let serialized = serde_json::to_string(&status).unwrap();
            let expected = format!("\"{}\"", status.as_str());
            assert_eq!(
                serialized, expected,
                "serde form of {status:?} must match as_str()"
            );
        }
    }

    #[test]
    fn transform_status_telemetry_strings_are_stable_and_content_free() {
        let cases = [
            (TransformStatus::Idle, "idle"),
            (TransformStatus::Capturing, "capturing"),
            (TransformStatus::Listening, "listening"),
            (TransformStatus::Thinking, "thinking"),
            (TransformStatus::ReviewPending, "review_pending"),
            (TransformStatus::Applying, "applying"),
        ];
        for (status, expected) in cases {
            assert_eq!(status.as_str(), expected);
        }
    }
}
