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
/// `bundle_id`, `auto_paste_override` (when `Some`) replaces the global
/// auto-paste setting at inject time. `None` means "no override — use global".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppProfile {
    pub bundle_id: String,
    pub label: String,
    pub auto_paste_override: Option<bool>,
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
    /// Rule-based transcript cleanup (filler removal, spacing/capitalization)
    /// applied before injection. Off by default.
    pub cleanup_enabled: bool,
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
            cleanup_enabled: false,
            code_vocab_enabled: false,
            code_vocab_folder: String::new(),
            code_vocab_prompt: None,
        }
    }
}

pub struct AppState {
    pub dictation: Mutex<DictationState>,
    pub backend: Mutex<Box<dyn TranscriptionBackend>>,
    pub last_transcription_at: Mutex<Option<Instant>>,
    pub idle_timeout_minutes: Mutex<u32>,
    /// Monotonically increasing ID assigned to each recording session.
    pub recording_id: AtomicU64,
    /// Set to the recording_id of a cancelled recording. Pipeline checks
    /// `cancelled_id >= my_id` at checkpoints to discard cancelled work.
    pub cancelled_id: AtomicU64,
    /// True while a file transcription is running. Live recording and file
    /// transcription share one Whisper backend, so they must be mutually
    /// exclusive — this flag lets each path refuse to start over the other.
    pub file_transcribing: AtomicBool,
}

impl AppState {
    /// Increment and return the next recording ID.
    pub fn next_recording_id(&self) -> u64 {
        self.recording_id.fetch_add(1, Ordering::SeqCst) + 1
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
            backend: Mutex::new(Box::new(WhisperBackend::new())),
            last_transcription_at: Mutex::new(None),
            idle_timeout_minutes: Mutex::new(5),
            recording_id: AtomicU64::new(0),
            cancelled_id: AtomicU64::new(0),
            file_transcribing: AtomicBool::new(false),
        }
    }
}
