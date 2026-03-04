use std::sync::Mutex;
use std::time::Instant;
use std::sync::atomic::{AtomicU64, Ordering};
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DictationState {
    pub status: DictationStatus,
    pub model_name: String,
    pub language: String,
    pub auto_paste: bool,
    pub auto_paste_delay_ms: u64,
    pub vad_sensitivity: u32,
    pub custom_vocabulary: String,
}

impl Default for DictationState {
    fn default() -> Self {
        Self {
            status: DictationStatus::Idle,
            model_name: "base.en".to_string(),
            language: "en".to_string(),
            auto_paste: false,
            auto_paste_delay_ms: 50,
            vad_sensitivity: 50,
            custom_vocabulary: String::new(),
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
        }
    }
}
