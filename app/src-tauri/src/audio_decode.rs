//! Audio file decoding for the "transcribe a file" feature.
//!
//! Decodes WAV/MP3/M4A via symphonia, downmixes to mono, and resamples to
//! 16kHz so the result can feed the same Whisper pipeline as live capture.

use crate::state::WHISPER_SAMPLE_RATE;
use std::path::Path;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Decode an audio file to 16kHz mono `f32` samples.
///
/// Supports the formats enabled in the symphonia feature set (WAV, MP3,
/// and MP4/M4A containers carrying AAC or ALAC). Multi-channel audio is
/// downmixed to mono by averaging channels; the result is resampled to
/// [`WHISPER_SAMPLE_RATE`] if the source rate differs.
pub fn decode_to_mono_16k(path: &str) -> Result<Vec<f32>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("Failed to open file: {}", e))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = Path::new(path).extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| format!("Unsupported or corrupt audio file: {}", e))?;
    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| "No decodable audio track found in file".to_string())?;
    let track_id = track.id;

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("No decoder for this audio codec: {}", e))?;

    // Source rate/channels are taken from the decoded buffers (authoritative),
    // falling back to track metadata for the initial values.
    let mut source_rate = track.codec_params.sample_rate.unwrap_or(WHISPER_SAMPLE_RATE);
    let mut channels = track.codec_params.channels.map(|c| c.count()).unwrap_or(1);
    let mut interleaved: Vec<f32> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            // Clean end-of-stream.
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(SymphoniaError::ResetRequired) => break,
            Err(e) => return Err(format!("Error reading audio packet: {}", e)),
        };

        if packet.track_id() != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(decoded) => {
                let spec = *decoded.spec();
                source_rate = spec.rate;
                channels = spec.channels.count();

                // SampleBuffer<f32> handles conversion from any source sample format.
                let mut buf = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
                buf.copy_interleaved_ref(decoded);
                interleaved.extend_from_slice(buf.samples());
            }
            // A single bad packet is non-fatal; skip it.
            Err(SymphoniaError::DecodeError(e)) => {
                tracing::warn!(target: "pipeline", error = %e, "audio_decode: skipping bad packet");
                continue;
            }
            Err(e) => return Err(format!("Audio decode failed: {}", e)),
        }
    }

    if interleaved.is_empty() {
        return Err("File contained no decodable audio".to_string());
    }

    // Downmix to mono by averaging channels.
    let mono: Vec<f32> = if channels <= 1 {
        interleaved
    } else {
        interleaved
            .chunks(channels)
            .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
            .collect()
    };

    // Resample to Whisper's required rate if needed.
    let out = if source_rate != WHISPER_SAMPLE_RATE {
        crate::audio::resample(&mono, source_rate, WHISPER_SAMPLE_RATE)
    } else {
        mono
    };

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Write a minimal 16-bit PCM WAV to a temp path and return the path.
    fn write_wav(samples_per_channel: &[i16], channels: u16, sample_rate: u32) -> std::path::PathBuf {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("murmur_decode_test_{}_{}.wav", std::process::id(), samples_per_channel.len()));
        let spec = hound::WavSpec {
            channels,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        for &s in samples_per_channel {
            writer.write_sample(s).unwrap();
        }
        writer.finalize().unwrap();
        path
    }

    #[test]
    fn decodes_mono_16k_wav_unchanged_length() {
        // 16000 mono samples @ 16kHz -> ~1s, no resample, no downmix.
        let samples = vec![1000i16; 16_000];
        let path = write_wav(&samples, 1, 16_000);
        let out = decode_to_mono_16k(path.to_str().unwrap()).unwrap();
        let _ = std::fs::remove_file(&path);
        // Length preserved (allow tiny rounding slack, though none expected here).
        assert!((out.len() as i64 - 16_000).abs() <= 1, "got {} samples", out.len());
    }

    #[test]
    fn downmixes_stereo_and_resamples_to_16k() {
        // 1s of stereo @ 32kHz -> expect ~16000 mono samples after resample.
        let frames = 32_000;
        let mut interleaved = Vec::with_capacity(frames * 2);
        for _ in 0..frames {
            interleaved.push(2000i16); // L
            interleaved.push(0i16); // R  (avg -> 1000)
        }
        let path = write_wav(&interleaved, 2, 32_000);
        let out = decode_to_mono_16k(path.to_str().unwrap()).unwrap();
        let _ = std::fs::remove_file(&path);
        // 32kHz -> 16kHz halves the sample count.
        assert!((out.len() as i64 - 16_000).abs() <= 2, "got {} samples", out.len());
        // Channel average of 2000 and 0 ≈ 1000/32768 ≈ 0.0305.
        let mid = out[out.len() / 2];
        assert!((mid - 0.0305).abs() < 0.01, "got {mid}");
    }

    #[test]
    fn errors_on_missing_file() {
        let err = decode_to_mono_16k("/nonexistent/murmur/file.wav").unwrap_err();
        assert!(err.contains("Failed to open file"), "got {err}");
    }
}
