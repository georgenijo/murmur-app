//! Live streaming transcription **preview** (issue #129).
//!
//! This module powers the optional, default-off "live preview" that shows
//! partial words in the Dynamic Island overlay *while the user is still
//! speaking*. It is strictly preview-only:
//!
//! - It NEVER produces the authoritative injected/clipboard text. The final
//!   one-shot transcription on stop (see `commands/recording.rs`) is unchanged.
//! - It uses its **own** dedicated `WhisperContext` + `WhisperState`, separate
//!   from the cached backend state in `AppState::backend`. This is the same
//!   isolation pattern VAD uses (a fresh context per run) and is the key to
//!   concurrency safety: the preview pass and the final pass can never share or
//!   corrupt each other's `WhisperState`.
//! - Whisper backend only. Parakeet (and any future non-whisper backend) gets a
//!   graceful no-op — the preview engine simply fails to build and the caller
//!   silently skips it.
//!
//! The pure decision logic (trailing-window selection, preview-text gating) is
//! split out into free functions so it can be unit-tested without whisper-rs.

/// Number of trailing seconds of audio to feed each preview pass. A short window
/// keeps each partial transcription cheap (CPU-bound) and responsive, at the
/// cost of not re-deriving the full utterance every tick. The authoritative
/// final pass always sees the complete audio, so this windowing only affects the
/// throwaway preview text.
pub const PREVIEW_WINDOW_SECS: f32 = 12.0;

/// Don't bother running a preview pass until at least this many samples have
/// accumulated. Below this, whisper tends to hallucinate on the tiny clip and
/// the preview would just be noise.
pub const PREVIEW_MIN_SAMPLES: usize = 8_000; // 0.5s at 16kHz

/// Select the trailing window of samples to feed a preview pass.
///
/// Returns `None` when there isn't enough audio yet (caller should skip this
/// tick). Otherwise returns the most-recent `window_secs` of samples (or the
/// whole buffer when it's shorter than the window). Pure + allocation-light: it
/// borrows a sub-slice of the input, it does not copy.
pub fn select_preview_window(
    samples: &[f32],
    sample_rate: u32,
    window_secs: f32,
    min_samples: usize,
) -> Option<&[f32]> {
    if samples.len() < min_samples {
        return None;
    }
    if sample_rate == 0 || window_secs <= 0.0 {
        // Defensive: a bad rate/window means "use everything" rather than panic.
        return Some(samples);
    }
    let window_len = (sample_rate as f32 * window_secs) as usize;
    if window_len == 0 || samples.len() <= window_len {
        Some(samples)
    } else {
        Some(&samples[samples.len() - window_len..])
    }
}

/// Normalize raw whisper output into preview text suitable for the overlay.
///
/// Returns `None` when the result is effectively empty (so the caller can skip
/// emitting and avoid flickering the overlay to blank). Whisper sometimes emits
/// bracketed non-speech markers like `[BLANK_AUDIO]` or `(silence)` on a quiet
/// trailing window; those are dropped so they never reach the UI.
pub fn clean_preview_text(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Drop a lone bracketed marker (e.g. "[BLANK_AUDIO]", "(silence)"). These are
    // whole-string non-speech annotations, not partial words.
    let is_bracket_marker = (trimmed.starts_with('[') && trimmed.ends_with(']'))
        || (trimmed.starts_with('(') && trimmed.ends_with(')'));
    if is_bracket_marker {
        return None;
    }
    Some(trimmed.to_string())
}

/// Whether a live preview pass should run at all for the given backend + setting.
///
/// Centralizes the gate so the on/off behavior is unit-testable and identical
/// everywhere. Preview runs only when the user enabled it AND the active backend
/// is whisper. `backend_name` comes from `TranscriptionBackend::name()`.
pub fn should_run_preview(live_preview_enabled: bool, backend_name: &str) -> bool {
    live_preview_enabled && backend_name == "whisper"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_skips_when_below_min() {
        let samples = vec![0.0f32; 100];
        assert!(select_preview_window(&samples, 16_000, 12.0, PREVIEW_MIN_SAMPLES).is_none());
    }

    #[test]
    fn window_returns_all_when_shorter_than_window() {
        let samples = vec![0.1f32; 16_000]; // 1s, window is 12s
        let win = select_preview_window(&samples, 16_000, 12.0, 8_000).unwrap();
        assert_eq!(win.len(), 16_000);
    }

    #[test]
    fn window_returns_trailing_slice_when_longer() {
        // 20s of audio at 16kHz, 12s window => last 12s = 192_000 samples.
        let samples = vec![0.2f32; 16_000 * 20];
        let win = select_preview_window(&samples, 16_000, 12.0, 8_000).unwrap();
        assert_eq!(win.len(), 16_000 * 12);
        // It must be the *trailing* slice (tail of the buffer).
        let tail = &samples[samples.len() - 16_000 * 12..];
        assert_eq!(win.as_ptr(), tail.as_ptr());
    }

    #[test]
    fn window_exactly_window_len_returns_all() {
        let samples = vec![0.3f32; 16_000 * 12];
        let win = select_preview_window(&samples, 16_000, 12.0, 8_000).unwrap();
        assert_eq!(win.len(), 16_000 * 12);
    }

    #[test]
    fn window_zero_rate_uses_everything() {
        let samples = vec![0.4f32; 10_000];
        let win = select_preview_window(&samples, 0, 12.0, 8_000).unwrap();
        assert_eq!(win.len(), 10_000);
    }

    #[test]
    fn window_zero_window_secs_uses_everything() {
        let samples = vec![0.4f32; 10_000];
        let win = select_preview_window(&samples, 16_000, 0.0, 8_000).unwrap();
        assert_eq!(win.len(), 10_000);
    }

    #[test]
    fn clean_text_trims_and_keeps_words() {
        assert_eq!(clean_preview_text("  hello world  ").as_deref(), Some("hello world"));
    }

    #[test]
    fn clean_text_drops_empty() {
        assert!(clean_preview_text("").is_none());
        assert!(clean_preview_text("    ").is_none());
    }

    #[test]
    fn clean_text_drops_blank_audio_marker() {
        assert!(clean_preview_text("[BLANK_AUDIO]").is_none());
        assert!(clean_preview_text("  (silence) ").is_none());
        assert!(clean_preview_text("[ Silence ]").is_none());
    }

    #[test]
    fn clean_text_keeps_text_with_internal_brackets() {
        // Only a *whole-string* bracket marker is dropped; real words survive.
        assert_eq!(
            clean_preview_text("the array[i] value").as_deref(),
            Some("the array[i] value")
        );
    }

    #[test]
    fn gate_off_when_disabled() {
        assert!(!should_run_preview(false, "whisper"));
    }

    #[test]
    fn gate_off_for_non_whisper_backend() {
        assert!(!should_run_preview(true, "parakeet"));
    }

    #[test]
    fn gate_on_for_whisper_when_enabled() {
        assert!(should_run_preview(true, "whisper"));
    }
}

// The whisper-rs–backed preview engine. Kept below the pure logic and behind the
// same crate so the unit tests above (run via `rustc --test`) don't need to link
// whisper-rs. This is the only part that touches the GPU/model.
pub use engine::PreviewTranscriber;

mod engine {
    use super::clean_preview_text;
    use std::path::PathBuf;
    use std::sync::Once;
    use whisper_rs::{
        FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
    };

    static INIT_LOGGING: Once = Once::new();

    fn suppress_whisper_logs() {
        INIT_LOGGING.call_once(|| {
            whisper_rs::install_logging_hooks();
        });
    }

    /// A self-contained whisper engine used ONLY for live preview. Owns its own
    /// `WhisperState` (which internally holds an Arc-cloned context) so it can
    /// never race or corrupt the authoritative backend's cached `WhisperState`.
    pub struct PreviewTranscriber {
        state: WhisperState,
    }

    impl PreviewTranscriber {
        /// Build a preview engine for `model_path`. Returns `Err` if the model
        /// can't be loaded — the caller treats that as "skip preview this
        /// session" and never surfaces it to the user.
        pub fn new(model_path: &str) -> Result<Self, String> {
            suppress_whisper_logs();
            let params = WhisperContextParameters::default();
            let context = WhisperContext::new_with_params(model_path, params)
                .map_err(|e| format!("preview: failed to load whisper model: {}", e))?;
            // `WhisperState` keeps its own Arc clone of the context, so the local
            // `context` binding can drop here without invalidating the state.
            let state = context
                .create_state()
                .map_err(|e| format!("preview: failed to create whisper state: {}", e))?;
            Ok(Self { state })
        }

        /// Run a single preview pass over `samples` (16kHz mono). Returns the
        /// cleaned partial text, or `None` when there's nothing worth showing.
        /// Best-effort: any whisper error is mapped to `Err` and the caller logs
        /// + skips it.
        pub fn transcribe_partial(
            &mut self,
            samples: &[f32],
            language: Option<&str>,
        ) -> Result<Option<String>, String> {
            let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
            if let Some(lang) = language {
                params.set_language(Some(lang));
            }
            params.set_print_special(false);
            params.set_print_progress(false);
            params.set_print_realtime(false);
            params.set_print_timestamps(false);
            params.set_suppress_blank(true);
            params.set_single_segment(true);
            params.set_debug_mode(false);
            // Speed over accuracy for the throwaway preview pass.
            params.set_no_context(true);

            self.state
                .full(params, samples)
                .map_err(|e| format!("preview transcription failed: {}", e))?;

            let num_segments = self.state.full_n_segments();
            let mut text = String::new();
            for i in 0..num_segments {
                if let Some(segment) = self.state.get_segment(i) {
                    if let Ok(s) = segment.to_str() {
                        text.push_str(s);
                    }
                }
            }
            Ok(clean_preview_text(&text))
        }
    }

    /// Resolve the on-disk path for a whisper model name, mirroring the search
    /// paths used by the authoritative backend. Kept here (rather than reaching
    /// into `whisper.rs` private fns) so the preview engine is self-contained.
    pub fn preview_model_path(model_name: &str) -> Option<PathBuf> {
        let filename = format!("ggml-{}.bin", model_name);
        let mut dirs_to_search: Vec<PathBuf> = Vec::new();
        if let Ok(custom) = std::env::var("WHISPER_MODEL_DIR") {
            dirs_to_search.push(PathBuf::from(custom));
        }
        if let Some(data_dir) = dirs::data_dir() {
            dirs_to_search.push(
                ["local-dictation", "models"]
                    .iter()
                    .fold(data_dir.clone(), |p, s| p.join(s)),
            );
            dirs_to_search.push(data_dir.join("pywhispercpp").join("models"));
        }
        if let Some(home) = dirs::home_dir() {
            dirs_to_search.push(home.join(".cache").join("whisper.cpp"));
            dirs_to_search.push(home.join(".cache").join("whisper"));
            dirs_to_search.push(home.join(".whisper").join("models"));
        }
        dirs_to_search
            .into_iter()
            .map(|d| d.join(&filename))
            .find(|p| p.exists())
    }
}

pub use engine::preview_model_path;
