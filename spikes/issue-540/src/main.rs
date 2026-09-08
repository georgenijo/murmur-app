use fluidaudio_rs::FluidAudio;
use serde_json::json;
use std::env;
use std::error::Error;
use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = env::args_os().skip(1);
    let audio_path = PathBuf::from(
        arguments
            .next()
            .ok_or("usage: murmur-issue-540-diarization-spike AUDIO.wav [threshold] [runs]")?,
    );
    let threshold = arguments
        .next()
        .map(|value| value.to_string_lossy().parse::<f64>())
        .transpose()?
        .unwrap_or(0.7);
    let runs = arguments
        .next()
        .map(|value| value.to_string_lossy().parse::<usize>())
        .transpose()?
        .unwrap_or(2);

    if !audio_path.is_file() {
        return Err(format!("audio file does not exist: {}", audio_path.display()).into());
    }
    if !(0.0..=1.0).contains(&threshold) || runs == 0 {
        return Err("threshold must be in 0..=1 and runs must be positive".into());
    }

    let audio = FluidAudio::new()?;
    let initialized_at = Instant::now();
    audio.init_diarization(threshold)?;
    println!(
        "{}",
        json!({
            "kind": "initialization",
            "threshold": threshold,
            "elapsedSeconds": initialized_at.elapsed().as_secs_f64(),
            "available": audio.is_diarization_available(),
        })
    );
    std::io::stdout().flush()?;

    for run in 1..=runs {
        let started_at = Instant::now();
        let segments = audio.diarize_file(&audio_path)?;
        let elapsed_seconds = started_at.elapsed().as_secs_f64();
        let speakers = segments
            .iter()
            .map(|segment| segment.speaker_id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        println!(
            "{}",
            json!({
                "kind": "run",
                "run": run,
                "elapsedSeconds": elapsed_seconds,
                "speakerCount": speakers.len(),
                "segmentCount": segments.len(),
                "segments": segments.iter().map(|segment| json!({
                    "speakerId": segment.speaker_id,
                    "startSeconds": segment.start_time,
                    "endSeconds": segment.end_time,
                    "qualityScore": segment.quality_score,
                })).collect::<Vec<_>>(),
            })
        );
    }

    Ok(())
}
