//! Persist dictation output to disk.
//!
//! When the user enables "save transcript" and/or "save audio", a completed
//! live dictation is written to a file in addition to the usual clipboard copy.
//! Audio is written as 16-bit PCM WAV at the pipeline's 16kHz mono sample rate;
//! the transcript is written as UTF-8 `.txt`. Both share a short sequential
//! base name so a paired recording lines up (`murmur-0001.wav` + `.txt`).
//!
//! Privacy: this module never logs the resolved directory or file path (which
//! would carry the user's home dir/username), only counts and booleans.

use crate::state::WHISPER_SAMPLE_RATE;
use std::path::{Path, PathBuf};

/// Resolve the output directory: the user-chosen `output_dir` if non-empty,
/// otherwise `<Documents>/Murmur` (falling back to `<home>/Murmur`).
pub(crate) fn resolve_output_dir(output_dir: &str) -> Result<PathBuf, String> {
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

/// Parse the sequence number from a `murmur-NNNN` file stem. Returns `None`
/// for anything that isn't exactly `murmur-<digits>` (e.g. older timestamped
/// names, which carry extra `-` separators).
fn sequence_of(stem: &str) -> Option<u32> {
    let digits = stem.strip_prefix("murmur-")?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// Build the next sequential base name (`murmur-0001`, `murmur-0002`, …) by
/// scanning `dir` for the highest existing `murmur-NNNN` and adding one. The
/// returned base is guaranteed free for both `.wav` and `.txt`.
fn next_base_name(dir: &Path) -> String {
    let mut highest = 0u32;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if let Some(stem) = Path::new(&entry.file_name()).file_stem().and_then(|s| s.to_str()) {
                if let Some(n) = sequence_of(stem) {
                    highest = highest.max(n);
                }
            }
        }
    }
    let taken = |name: &str| dir.join(format!("{}.wav", name)).exists()
        || dir.join(format!("{}.txt", name)).exists();
    let mut n = highest + 1;
    loop {
        let candidate = format!("murmur-{:04}", n);
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
) -> Result<usize, String> {
    if !save_audio && !save_transcript {
        return Ok(0);
    }

    let dir = resolve_output_dir(output_dir)?;
    let base = next_base_name(&dir);
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

/// Write a pre-serialized benchmark report as JSON into the resolved output
/// directory (see [`resolve_output_dir`]) under `file_name`. The caller builds
/// the descriptive name (`benchmark-<version>-<machine>-<createdAt>.json`); this
/// function sanitizes it to a bare file component so a crafted name cannot escape
/// the directory, then writes the JSON verbatim. Returns the absolute path.
///
/// Privacy: like the rest of this module, it logs only a boolean, never the path.
pub fn write_benchmark_report(
    output_dir: &str,
    file_name: &str,
    json: &str,
) -> Result<PathBuf, String> {
    let dir = resolve_output_dir(output_dir)?;

    // Reduce the requested name to a single path component so `../` or absolute
    // paths cannot redirect the write outside `dir`, and force a `.json` suffix.
    let mut safe = Path::new(file_name)
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "Invalid benchmark report file name".to_string())?;
    if !safe.to_ascii_lowercase().ends_with(".json") {
        safe.push_str(".json");
    }

    let path = dir.join(&safe);
    std::fs::write(&path, json)
        .map_err(|e| format!("Failed to write benchmark report: {}", e))?;

    tracing::info!(
        target: "pipeline",
        bytes = json.len(),
        "benchmark report written to file"
    );

    Ok(path)
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
        )
        .unwrap();
        assert_eq!(written, 2);

        let wav = dir.join("murmur-0001.wav");
        let txt = dir.join("murmur-0001.txt");
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
        )
        .unwrap();
        assert_eq!(written, 1);
        assert!(dir.join("murmur-0001.wav").exists());
        assert!(!dir.join("murmur-0001.txt").exists());
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
        )
        .unwrap();
        assert_eq!(written, 1);
        assert!(!dir.join("murmur-0001.wav").exists());
        assert!(dir.join("murmur-0001.txt").exists());
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
        )
        .unwrap();
        assert_eq!(written, 0);
    }

    #[test]
    fn sequence_increments_per_recording() {
        let dir = temp_dir("sequence");
        let samples = vec![0.0f32];
        write_dictation_outputs(&samples, "first", true, true, dir.to_str().unwrap()).unwrap();
        write_dictation_outputs(&samples, "second", true, true, dir.to_str().unwrap()).unwrap();

        assert!(dir.join("murmur-0001.txt").exists());
        assert!(dir.join("murmur-0002.txt").exists());
        assert_eq!(
            std::fs::read_to_string(dir.join("murmur-0002.txt")).unwrap(),
            "second"
        );
    }

    #[test]
    fn ignores_non_sequential_names_when_numbering() {
        let dir = temp_dir("mixed_names");
        // An older timestamped name should not inflate the next sequence number.
        std::fs::write(dir.join("murmur-260528-210426.wav"), b"x").unwrap();
        write_dictation_outputs(&[0.0f32], "fresh", true, true, dir.to_str().unwrap()).unwrap();
        assert!(dir.join("murmur-0001.wav").exists());
        assert!(dir.join("murmur-0001.txt").exists());
    }

    #[test]
    fn writes_benchmark_report_json() {
        let dir = temp_dir("bench_report");
        let json = r#"{"reportVersion":2}"#;
        let path = write_benchmark_report(
            dir.to_str().unwrap(),
            "benchmark-0.20.0-Apple-M4-2026-07-20T14-30-00-000Z.json",
            json,
        )
        .unwrap();
        assert_eq!(path.parent().unwrap(), dir);
        assert_eq!(
            path.file_name().unwrap().to_str().unwrap(),
            "benchmark-0.20.0-Apple-M4-2026-07-20T14-30-00-000Z.json"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), json);
    }

    #[test]
    fn benchmark_report_name_cannot_escape_directory() {
        let dir = temp_dir("bench_traversal");
        // A crafted traversal name is reduced to its bare file component.
        let path = write_benchmark_report(dir.to_str().unwrap(), "../evil.json", "{}").unwrap();
        assert_eq!(path, dir.join("evil.json"));
        assert!(dir.join("evil.json").exists());
    }

    #[test]
    fn benchmark_report_name_gets_json_suffix() {
        let dir = temp_dir("bench_suffix");
        let path = write_benchmark_report(dir.to_str().unwrap(), "report", "{}").unwrap();
        assert_eq!(path, dir.join("report.json"));
    }

    #[test]
    fn sequence_of_parses_only_pure_digits() {
        assert_eq!(sequence_of("murmur-0007"), Some(7));
        assert_eq!(sequence_of("murmur-42"), Some(42));
        assert_eq!(sequence_of("murmur-260528-210426"), None);
        assert_eq!(sequence_of("murmur-"), None);
        assert_eq!(sequence_of("other-0001"), None);
    }
}
