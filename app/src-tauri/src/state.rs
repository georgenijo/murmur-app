use std::sync::Mutex;
use std::time::Instant;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use serde::{Deserialize, Serialize};
use crate::transcriber::{TranscriptionBackend, WhisperBackend};

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

/// Per-app dictation profile. When the frontmost macOS app's bundle id matches
/// `bundle_id`, each `*_override` (when `Some`) replaces the corresponding global
/// setting at transcription time. `None` means "no override — use global".
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DictationState {
    pub status: DictationStatus,
    pub model_name: String,
    pub language: String,
    pub auto_paste: bool,
    pub auto_paste_delay_ms: u64,
    pub vad_sensitivity: u32,
    pub custom_vocabulary: String,
    pub smart_punctuation: bool,
    pub save_transcript: bool,
    pub save_audio: bool,
    pub output_dir: String,
    /// Per-app profiles that override auto-paste based on the frontmost app.
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

pub struct AppState {
    pub dictation: Mutex<DictationState>,
    /// Serializes recorder start/stop/cancel transitions. Audio startup waits
    /// for the cpal stream to become ready, so a fast key release must not tear
    /// the recorder down until that startup has fully completed.
    pub recording_transition: tokio::sync::Mutex<()>,
    pub backend: Mutex<Box<dyn TranscriptionBackend>>,
    pub last_transcription_at: Mutex<Option<Instant>>,
    pub idle_timeout_minutes: Mutex<u32>,
    /// Monotonically increasing ID assigned to each recording session.
    pub recording_id: AtomicU64,
    /// Monotonically increasing opaque ID assigned to every post-recognition
    /// transformation pass (live recordings and imported files).
    pub transcript_session_id: AtomicU64,
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
    pub correction_matcher: Mutex<Option<std::sync::Arc<crate::correction::CorrectionMatcher>>>,
    /// At most one bounded incremental Whisper worker is attached to the active
    /// recording. The handle owns no audio; it snapshots one fixed-size window
    /// at a time and stores only reconciled text and timing counters.
    pub streaming_session: tokio::sync::Mutex<Option<crate::streaming::StreamingSession>>,
}

impl AppState {
    /// Increment and return the next recording ID.
    pub fn next_recording_id(&self) -> u64 {
        self.recording_id.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn next_transcript_session_id(&self) -> u64 {
        self.transcript_session_id.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Mark a recording as cancelled by storing its ID.
    pub fn cancel_recording(&self, id: u64) {
        self.cancelled_id.fetch_max(id, Ordering::SeqCst);
    }

    /// Check whether a given recording ID has been cancelled.
    pub fn is_cancelled(&self, id: u64) -> bool {
        self.cancelled_id.load(Ordering::SeqCst) >= id
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            dictation: Mutex::new(DictationState::default()),
            recording_transition: tokio::sync::Mutex::new(()),
            backend: Mutex::new(Box::new(WhisperBackend::new())),
            last_transcription_at: Mutex::new(None),
            idle_timeout_minutes: Mutex::new(5),
            recording_id: AtomicU64::new(0),
            transcript_session_id: AtomicU64::new(0),
            cancelled_id: AtomicU64::new(0),
            file_transcribing: AtomicBool::new(false),
            correction_matcher: Mutex::new(None),
            streaming_session: tokio::sync::Mutex::new(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
