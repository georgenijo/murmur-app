use std::sync::Mutex;
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
        }
    }
}

pub struct AppState {
    pub dictation: Mutex<DictationState>,
    pub backend: Mutex<Box<dyn TranscriptionBackend>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            dictation: Mutex::new(DictationState::default()),
            backend: Mutex::new(Box::new(WhisperBackend::new())),
        }
    }
}
