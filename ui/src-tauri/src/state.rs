use std::sync::Mutex;
use whisper_rs::WhisperContext;
use serde::{Deserialize, Serialize};

/// Sample rate required by Whisper models (16kHz)
pub const WHISPER_SAMPLE_RATE: u32 = 16000;

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
}

impl Default for DictationState {
    fn default() -> Self {
        Self {
            status: DictationStatus::Idle,
            model_name: "base.en".to_string(),
            language: "en".to_string(),
        }
    }
}

pub struct AppState {
    pub dictation: Mutex<DictationState>,
    pub whisper_context: Mutex<Option<WhisperContext>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            dictation: Mutex::new(DictationState::default()),
            whisper_context: Mutex::new(None),
        }
    }
}
