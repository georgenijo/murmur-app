//! Named event protocol shared by hosts. Payloads retain their native Serialize
//! types and existing wire shape; routing never inspects or logs their contents.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Consumer {
    Main,
    Overlay,
    Diagnostics,
    DictationPreview,
    DictationControl,
    SoundCues,
    SmartAuto,
    MeetingSuggestions,
}
impl Consumer {
    pub fn window_label(self) -> Option<&'static str> {
        match self {
            Self::Main => Some("main"),
            Self::Overlay => Some("overlay"),
            Self::Diagnostics => Some("diagnostics"),
            Self::DictationPreview => Some("dictation-preview"),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WaveformConsumer {
    MainRecording,
    Corpus,
    Overlay,
}
impl WaveformConsumer {
    pub fn consumer(self) -> Consumer {
        match self {
            Self::MainRecording | Self::Corpus => Consumer::Main,
            Self::Overlay => Consumer::Overlay,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EventKind {
    AppDisabledChanged,
    AppEvent,
    AudioInitializationStalled,
    AudioLevel,
    AudioRecoveryStarted,
    AutoPasteFailed,
    BenchmarkProgress,
    CheckForUpdatesRequested,
    CorpusRecordingStatus,
    DeliveryRetryFeedback,
    DiagnosticsTabRequested,
    DictationControlError,
    DictationDeliveryOutcome,
    DictationGenerationStarted,
    DictationLockChanged,
    DictationStopFailed,
    DoubleTapToggle,
    DownloadProgress,
    EscapeCancel,
    FileOutputFailed,
    FileTranscriptionStatusChanged,
    HistoryChanged,
    HoldDownStart,
    HoldDownStop,
    HoldOverRecordingEnd,
    HoldOverRecordingStart,
    HotkeyTapRejected,
    KeyboardListenerError,
    MeetingSegmentFailed,
    MeetingSegmentFinalized,
    MeetingSpeakersUpdated,
    MeetingStatusChanged,
    ModeManualChanged,
    ModeRuntimeChanged,
    ModelRuntimeStatusChanged,
    OverlayGeometryChanged,
    OverlayPositionApplied,
    OverlaySurfaceChanged,
    OverlayVisibleChanged,
    PerformanceDiagnosticsCleared,
    PerformanceResourceSample,
    PerformanceRunCompleted,
    RecordingCancelled,
    RecordingCleanupFinished,
    RecordingInitializationFailed,
    RecordingInterrupted,
    RecordingRecoveryStalled,
    RecordingStatusChanged,
    SmartAutoMicrophoneChanged,
    SmartDestinationSelected,
    StatisticsChanged,
    SystemAudioPermissionChanged,
    TranscriptionComplete,
    UpdaterBackgroundCheckRequested,
    UpdaterStateChanged,
    VocabScanProgress,
    MicrophonePreviewStatus,
    TransformStateChanged,
    QueryStateChanged,
    MicrophoneStartupBenchmarkProgress,
}
impl EventKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::AppDisabledChanged => "app-disabled-changed",
            Self::AppEvent => "app-event",
            Self::AudioInitializationStalled => "audio-initialization-stalled",
            Self::AudioLevel => "audio-level",
            Self::AudioRecoveryStarted => "audio-recovery-started",
            Self::AutoPasteFailed => "auto-paste-failed",
            Self::BenchmarkProgress => "benchmark-progress",
            Self::CheckForUpdatesRequested => "check-for-updates-requested",
            Self::CorpusRecordingStatus => "corpus-recording-status",
            Self::DeliveryRetryFeedback => "delivery-retry-feedback",
            Self::DiagnosticsTabRequested => "diagnostics-tab-requested",
            Self::DictationControlError => "dictation-control-error",
            Self::DictationDeliveryOutcome => "dictation-delivery-outcome",
            Self::DictationGenerationStarted => "dictation-generation-started",
            Self::DictationLockChanged => "dictation-lock-changed",
            Self::DictationStopFailed => "dictation-stop-failed",
            Self::DoubleTapToggle => "double-tap-toggle",
            Self::DownloadProgress => "download-progress",
            Self::EscapeCancel => "escape-cancel",
            Self::FileOutputFailed => "file-output-failed",
            Self::FileTranscriptionStatusChanged => "file-transcription-status-changed",
            Self::HistoryChanged => "history-changed",
            Self::HoldDownStart => "hold-down-start",
            Self::HoldDownStop => "hold-down-stop",
            Self::HoldOverRecordingEnd => "hold-over-recording-end",
            Self::HoldOverRecordingStart => "hold-over-recording-start",
            Self::HotkeyTapRejected => "hotkey-tap-rejected",
            Self::KeyboardListenerError => "keyboard-listener-error",
            Self::MeetingSegmentFailed => "meeting-segment-failed",
            Self::MeetingSegmentFinalized => "meeting-segment-finalized",
            Self::MeetingSpeakersUpdated => "meeting-speakers-updated",
            Self::MeetingStatusChanged => "meeting-status-changed",
            Self::ModeManualChanged => "mode-manual-changed",
            Self::ModeRuntimeChanged => "mode-runtime-changed",
            Self::ModelRuntimeStatusChanged => "model-runtime-status-changed",
            Self::OverlayGeometryChanged => "overlay-geometry-changed",
            Self::OverlayPositionApplied => "overlay-position-applied",
            Self::OverlaySurfaceChanged => "overlay-surface-changed",
            Self::OverlayVisibleChanged => "overlay-visible-changed",
            Self::PerformanceDiagnosticsCleared => "performance-diagnostics-cleared",
            Self::PerformanceResourceSample => "performance-resource-sample",
            Self::PerformanceRunCompleted => "performance-run-completed",
            Self::RecordingCancelled => "recording-cancelled",
            Self::RecordingCleanupFinished => "recording-cleanup-finished",
            Self::RecordingInitializationFailed => "recording-initialization-failed",
            Self::RecordingInterrupted => "recording-interrupted",
            Self::RecordingRecoveryStalled => "recording-recovery-stalled",
            Self::RecordingStatusChanged => "recording-status-changed",
            Self::SmartAutoMicrophoneChanged => "smart-auto-microphone-changed",
            Self::SmartDestinationSelected => "smart-destination-selected",
            Self::StatisticsChanged => "statistics-changed",
            Self::SystemAudioPermissionChanged => "system-audio-permission-changed",
            Self::TranscriptionComplete => "transcription-complete",
            Self::UpdaterBackgroundCheckRequested => "updater-background-check-requested",
            Self::UpdaterStateChanged => "updater-state-changed",
            Self::VocabScanProgress => "vocab-scan-progress",
            Self::MicrophonePreviewStatus => "microphone-preview-status",
            Self::TransformStateChanged => "transform-state-changed",
            Self::QueryStateChanged => "query-state-changed",
            Self::MicrophoneStartupBenchmarkProgress => "microphone-startup-benchmark-progress",
        }
    }
    pub fn consumers(self) -> &'static [Consumer] {
        use Consumer::*;
        match self {
            Self::AppDisabledChanged => &[Main, Overlay, SmartAuto, MeetingSuggestions],
            Self::AppEvent => &[Main, Diagnostics],
            Self::AudioInitializationStalled => &[Overlay],
            Self::AudioLevel => &[Main, Overlay],
            Self::AudioRecoveryStarted => &[],
            Self::AutoPasteFailed => &[Main],
            Self::BenchmarkProgress => &[Main, MeetingSuggestions],
            Self::CheckForUpdatesRequested => &[Main],
            Self::CorpusRecordingStatus => &[Main],
            Self::DeliveryRetryFeedback => &[Main],
            Self::DiagnosticsTabRequested => &[Diagnostics],
            Self::DictationControlError => &[Main],
            Self::DictationDeliveryOutcome => &[Main, Overlay, SoundCues],
            Self::DictationGenerationStarted => &[Main, Overlay, DictationPreview, SoundCues],
            Self::DictationLockChanged => &[Overlay],
            Self::DictationStopFailed => &[Main],
            Self::DoubleTapToggle => &[],
            Self::DownloadProgress => &[Main],
            Self::EscapeCancel => &[],
            Self::FileOutputFailed => &[Main],
            Self::FileTranscriptionStatusChanged => &[Main, SmartAuto, MeetingSuggestions],
            Self::HistoryChanged => &[Main, Diagnostics],
            Self::HoldDownStart => &[],
            Self::HoldDownStop => &[],
            Self::HoldOverRecordingEnd => &[],
            Self::HoldOverRecordingStart => &[],
            Self::HotkeyTapRejected => &[Main, Overlay],
            Self::KeyboardListenerError => &[],
            Self::MeetingSegmentFailed => &[Main],
            Self::MeetingSegmentFinalized => &[Main],
            Self::MeetingSpeakersUpdated => &[Main],
            Self::MeetingStatusChanged => &[Main, Overlay, SmartAuto, MeetingSuggestions],
            Self::ModeManualChanged => &[Main],
            Self::ModeRuntimeChanged => &[Main, Overlay],
            Self::ModelRuntimeStatusChanged => &[Main, MeetingSuggestions],
            Self::OverlayGeometryChanged => &[Overlay],
            Self::OverlayPositionApplied => &[Main],
            Self::OverlaySurfaceChanged => &[Overlay],
            Self::OverlayVisibleChanged => &[Overlay],
            Self::PerformanceDiagnosticsCleared => &[Main, Diagnostics],
            Self::PerformanceResourceSample => &[Main, Diagnostics],
            Self::PerformanceRunCompleted => &[Main, Diagnostics],
            Self::RecordingCancelled => &[Main, Overlay, DictationPreview],
            Self::RecordingCleanupFinished => &[Main],
            Self::RecordingInitializationFailed => &[Main, Overlay, SoundCues],
            Self::RecordingInterrupted => &[Main, Overlay, SoundCues],
            Self::RecordingRecoveryStalled => &[Main],
            Self::RecordingStatusChanged => &[
                Main,
                Overlay,
                DictationPreview,
                DictationControl,
                SoundCues,
                SmartAuto,
                MeetingSuggestions,
            ],
            Self::SmartAutoMicrophoneChanged => &[Main, Overlay],
            Self::SmartDestinationSelected => &[Main, Overlay],
            Self::StatisticsChanged => &[Main, Diagnostics],
            Self::SystemAudioPermissionChanged => &[Main],
            Self::TranscriptionComplete => &[Main],
            Self::UpdaterBackgroundCheckRequested => &[Main],
            Self::UpdaterStateChanged => &[Main],
            Self::VocabScanProgress => &[Main],
            Self::MicrophonePreviewStatus => &[Main, SmartAuto, MeetingSuggestions],
            Self::TransformStateChanged => &[SmartAuto],
            Self::QueryStateChanged => &[Main, SmartAuto],
            Self::MicrophoneStartupBenchmarkProgress => &[Main, MeetingSuggestions],
        }
    }
}

/// Implemented by the host, never by a webview. Keeping the payload generic
/// preserves existing DTOs without coupling the core to app-owned types.
pub trait EventSink {
    fn publish<P: Serialize + Clone>(&self, event: EventKind, payload: P) -> Result<(), String>;
}

#[derive(Default, Clone)]
pub struct Router {
    waveforms: BTreeSet<WaveformConsumer>,
}
impl Router {
    pub fn set_waveform(&mut self, consumer: WaveformConsumer, active: bool) {
        if active {
            self.waveforms.insert(consumer);
        } else {
            self.waveforms.remove(&consumer);
        }
    }
    pub fn clear_window(&mut self, label: &str) {
        self.waveforms
            .retain(|waveform| waveform.consumer().window_label() != Some(label));
    }
    /// The host checks existence and actual native visibility on each delivery,
    /// so showing a subscribed consumer resumes levels on the very next sample.
    pub fn recipients(
        &self,
        event: EventKind,
        mut available: impl FnMut(Consumer, bool) -> bool,
    ) -> Vec<Consumer> {
        event
            .consumers()
            .iter()
            .copied()
            .filter(|consumer| {
                let waveform = event == EventKind::AudioLevel;
                (!waveform || self.waveforms.iter().any(|w| w.consumer() == *consumer))
                    && available(*consumer, waveform)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_routes_keep_native_consumers_without_main_and_skip_absent_windows() {
        let router = Router::default();
        assert_eq!(
            router.recipients(EventKind::RecordingStatusChanged, |c, _| c
                .window_label()
                .is_none()),
            vec![
                Consumer::DictationControl,
                Consumer::SoundCues,
                Consumer::SmartAuto,
                Consumer::MeetingSuggestions
            ]
        );
        assert_eq!(
            router.recipients(EventKind::AppEvent, |c, _| c == Consumer::Diagnostics),
            vec![Consumer::Diagnostics]
        );
        assert!(router
            .recipients(EventKind::HistoryChanged, |_, _| false)
            .is_empty());
        assert_eq!(
            router.recipients(EventKind::HistoryChanged, |_, _| true),
            vec![Consumer::Main, Consumer::Diagnostics]
        );
    }

    #[test]
    fn waveform_requires_interest_and_native_visibility_and_resumes_without_restart() {
        let mut router = Router::default();
        assert!(router
            .recipients(EventKind::AudioLevel, |_, _| true)
            .is_empty());
        router.set_waveform(WaveformConsumer::Overlay, true);
        router.set_waveform(WaveformConsumer::MainRecording, true);
        let visible = |c, waveform: bool| {
            assert!(waveform);
            c == Consumer::Overlay
        };
        assert_eq!(
            router.recipients(EventKind::AudioLevel, visible),
            vec![Consumer::Overlay]
        );
        // Showing main requires no capture reopen or subscription reset.
        assert_eq!(
            router.recipients(EventKind::AudioLevel, |_, _| true),
            vec![Consumer::Main, Consumer::Overlay]
        );
        assert_eq!(
            router.recipients(EventKind::AudioLevel, visible),
            vec![Consumer::Overlay]
        );
        router.set_waveform(WaveformConsumer::Corpus, true);
        router.set_waveform(WaveformConsumer::MainRecording, false);
        assert_eq!(
            router.recipients(EventKind::AudioLevel, |_, _| true),
            vec![Consumer::Main, Consumer::Overlay]
        );
        router.clear_window("main");
        assert_eq!(
            router.recipients(EventKind::AudioLevel, |_, _| true),
            vec![Consumer::Overlay]
        );
        router.clear_window("overlay");
        assert!(router
            .recipients(EventKind::AudioLevel, |_, _| true)
            .is_empty());
        // Only waveform traffic is gated by interest/visibility.
        assert_eq!(
            router.recipients(EventKind::DictationLockChanged, |_, waveform| !waveform),
            vec![Consumer::Overlay]
        );
    }
}
