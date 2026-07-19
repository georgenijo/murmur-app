use tauri::Emitter;

pub(crate) const CONTRACT_VERSION: u8 = 1;

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecordingSessionStarted {
    contract_version: u8,
    recording_id: u64,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PartialTranscriptUpdate {
    contract_version: u8,
    recording_id: u64,
    text: String,
    chunk_index: u32,
    processed_audio_ms: u64,
}

#[derive(Clone, Copy, Debug, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PartialTranscriptClearReason {
    Cancelled,
    Error,
    Fallback,
    Finalized,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PartialTranscriptCleared {
    contract_version: u8,
    recording_id: u64,
    reason: PartialTranscriptClearReason,
}

pub(crate) fn emit_session_started(app: &tauri::AppHandle, recording_id: u64) {
    let _ = app.emit(
        "recording-session-started",
        RecordingSessionStarted {
            contract_version: CONTRACT_VERSION,
            recording_id,
        },
    );
}

pub(crate) fn emit_update(
    app: &tauri::AppHandle,
    recording_id: u64,
    text: String,
    chunk_index: u32,
    processed_audio_ms: u64,
) {
    let _ = app.emit(
        "partial-transcript",
        PartialTranscriptUpdate {
            contract_version: CONTRACT_VERSION,
            recording_id,
            text,
            chunk_index,
            processed_audio_ms,
        },
    );
}

pub(crate) fn emit_clear(
    app: &tauri::AppHandle,
    recording_id: u64,
    reason: PartialTranscriptClearReason,
) {
    let _ = app.emit(
        "partial-transcript-cleared",
        PartialTranscriptCleared {
            contract_version: CONTRACT_VERSION,
            recording_id,
            reason,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_contract_is_versioned_and_camel_case() {
        let value = serde_json::to_value(PartialTranscriptUpdate {
            contract_version: CONTRACT_VERSION,
            recording_id: 42,
            text: "provisional words".to_string(),
            chunk_index: 3,
            processed_audio_ms: 26_000,
        })
        .unwrap();

        assert_eq!(value["contractVersion"], CONTRACT_VERSION);
        assert_eq!(value["recordingId"], 42);
        assert_eq!(value["chunkIndex"], 3);
        assert_eq!(value["processedAudioMs"], 26_000);
        assert!(value.get("recording_id").is_none());
    }

    #[test]
    fn clear_contract_has_privacy_safe_reason_only() {
        let value = serde_json::to_value(PartialTranscriptCleared {
            contract_version: CONTRACT_VERSION,
            recording_id: 9,
            reason: PartialTranscriptClearReason::Fallback,
        })
        .unwrap();

        assert_eq!(value["reason"], "fallback");
        assert!(value.get("text").is_none());
    }
}
