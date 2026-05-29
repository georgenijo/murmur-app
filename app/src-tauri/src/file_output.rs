//! Persist dictation output to disk.
//!
//! When the user enables "save transcript" and/or "save audio", a completed
//! live dictation is written to a file in addition to the usual clipboard copy.
//! Audio is written as 16-bit PCM WAV at the pipeline's 16kHz mono sample rate;
//! the transcript is written as UTF-8 `.txt`. Both share a timestamped base name
//! so a paired recording lines up (`murmur-2026-05-28_14-30-01.wav` + `.txt`).
//!
//! Privacy: this module never logs the resolved directory or file path (which
//! would carry the user's home dir/username), only counts and booleans.

use crate::state::WHISPER_SAMPLE_RATE;
use std::path::{Path, PathBuf};

/// Resolve the output directory: the user-chosen `output_dir` if non-empty,
/// otherwise `<Documents>/Murmur` (falling back to `<home>/Murmur`).
fn resolve_output_dir(output_dir: &str) -> Result<PathBuf, String> {
    let dir = if !output_dir.trim().is_empty() {
        PathBuf::from(output_dir)
    } else {
        let base = dirs::document_dir()
            .or_else(dirs::home_dir)
            .ok_or_else(|| "Could not determine a default output directory".to_string())?;
        base.join("Murmur")
    };
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create output directory: {}", e))?;
    Ok(dir)
}

/// Build a base name like `murmur-2026-05-28_14-30-01`, appending `-1`, `-2`, …
/// if a `.wav` or `.txt` with that base already exists in `dir`.
fn unique_base_name(dir: &Path, timestamp: &str) -> String {
    let base = format!("murmur-{}", timestamp);
    let taken = |name: &str| dir.join(format!("{}.wav", name)).exists()
        || dir.join(format!("{}.txt", name)).exists();
    if !taken(&base) {
        return base;
    }
    let mut n = 1;
    loop {
        let candidate = format!("{}-{}", base, n);
        if !taken(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// Write a 16-bit PCM mono WAV at the pipeline sample rate from f32 samples.
fn write_wav(path: &Path, samples: &[f32]) -> Result<(), String> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: WHISPER_SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec)
        .map_err(|e| format!("Failed to create WAV file: {}", e))?;
    for &s in samples {
        let clamped = s.clamp(-1.0, 1.0);
        let value = (clamped * i16::MAX as f32) as i16;
        writer
            .write_sample(value)
            .map_err(|e| format!("Failed to write WAV sample: {}", e))?;
    }
    writer
        .finalize()
        .map_err(|e| format!("Failed to finalize WAV file: {}", e))?;
    Ok(())
}

/// Persist the requested dictation outputs. The transcript is only written when
/// `save_transcript` is set and the text is non-empty; the audio is only written
/// when `save_audio` is set. Returns the number of files written on success.
pub fn write_dictation_outputs(
    samples: &[f32],
    text: &str,
    save_audio: bool,
    save_transcript: bool,
    output_dir: &str,
    timestamp: &str,
) -> Result<usize, String> {
    if !save_audio && !save_transcript {
        return Ok(0);
    }

    let dir = resolve_output_dir(output_dir)?;
    let base = unique_base_name(&dir, timestamp);
    let mut written = 0;

    if save_audio {
        write_wav(&dir.join(format!("{}.wav", base)), samples)?;
        written += 1;
    }

    if save_transcript && !text.trim().is_empty() {
        std::fs::write(dir.join(format!("{}.txt", base)), text)
            .map_err(|e| format!("Failed to write transcript file: {}", e))?;
        written += 1;
    }

    tracing::info!(
        target: "pipeline",
        files_written = written,
        save_audio = save_audio,
        save_transcript = save_transcript,
        "dictation output written to file"
    );

    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "murmur_file_output_test_{}_{}",
            std::process::id(),
            tag
        ));
        // Start clean so collision/uniqueness assertions are deterministic.
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn writes_wav_and_txt() {
        let dir = temp_dir("both");
        let samples = vec![0.0f32, 0.5, -0.5, 1.0, -1.0];
        let written = write_dictation_outputs(
            &samples,
            "hello world",
            true,
            true,
            dir.to_str().unwrap(),
            "2026-05-28_14-30-01",
        )
        .unwrap();
        assert_eq!(written, 2);

        let wav = dir.join("murmur-2026-05-28_14-30-01.wav");
        let txt = dir.join("murmur-2026-05-28_14-30-01.txt");
        assert!(wav.exists());
        assert!(txt.exists());
        assert_eq!(std::fs::read_to_string(&txt).unwrap(), "hello world");

        let reader = hound::WavReader::open(&wav).unwrap();
        assert_eq!(reader.spec().channels, 1);
        assert_eq!(reader.spec().sample_rate, WHISPER_SAMPLE_RATE);
        assert_eq!(reader.len() as usize, samples.len());
    }

    #[test]
    fn empty_text_skips_transcript() {
        let dir = temp_dir("empty_text");
        let written = write_dictation_outputs(
            &[0.1f32, 0.2],
            "   ",
            true,
            true,
            dir.to_str().unwrap(),
            "2026-05-28_14-30-02",
        )
        .unwrap();
        assert_eq!(written, 1);
        assert!(dir.join("murmur-2026-05-28_14-30-02.wav").exists());
        assert!(!dir.join("murmur-2026-05-28_14-30-02.txt").exists());
    }

    #[test]
    fn transcript_only_skips_audio() {
        let dir = temp_dir("txt_only");
        let written = write_dictation_outputs(
            &[0.1f32],
            "text only",
            false,
            true,
            dir.to_str().unwrap(),
            "2026-05-28_14-30-03",
        )
        .unwrap();
        assert_eq!(written, 1);
        assert!(!dir.join("murmur-2026-05-28_14-30-03.wav").exists());
        assert!(dir.join("murmur-2026-05-28_14-30-03.txt").exists());
    }

    #[test]
    fn neither_toggle_writes_nothing() {
        let dir = temp_dir("none");
        let written = write_dictation_outputs(
            &[0.1f32],
            "ignored",
            false,
            false,
            dir.to_str().unwrap(),
            "2026-05-28_14-30-04",
        )
        .unwrap();
        assert_eq!(written, 0);
    }

    #[test]
    fn collision_appends_suffix() {
        let dir = temp_dir("collision");
        let ts = "2026-05-28_14-30-05";
        let samples = vec![0.0f32];
        write_dictation_outputs(&samples, "first", true, true, dir.to_str().unwrap(), ts).unwrap();
        write_dictation_outputs(&samples, "second", true, true, dir.to_str().unwrap(), ts).unwrap();

        assert!(dir.join(format!("murmur-{}.txt", ts)).exists());
        assert!(dir.join(format!("murmur-{}-1.txt", ts)).exists());
        assert_eq!(
            std::fs::read_to_string(dir.join(format!("murmur-{}-1.txt", ts))).unwrap(),
            "second"
        );
    }
}
