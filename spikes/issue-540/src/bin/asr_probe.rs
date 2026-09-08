use fluidaudio_rs::FluidAudio;
use serde_json::json;
use std::env;
use std::error::Error;
use std::path::PathBuf;
use std::time::Instant;

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = env::args_os().skip(1);
    let audio_path = PathBuf::from(
        arguments
            .next()
            .ok_or("usage: asr_probe AUDIO.wav [runs]")?,
    );
    let runs = arguments
        .next()
        .map(|value| value.to_string_lossy().parse::<usize>())
        .transpose()?
        .unwrap_or(20);
    if !audio_path.is_file() || runs == 0 {
        return Err("audio must be a file and runs must be positive".into());
    }

    let audio = FluidAudio::new()?;
    let initialized_at = Instant::now();
    audio.init_asr()?;
    println!(
        "{}",
        json!({"kind": "initialization", "elapsedSeconds": initialized_at.elapsed().as_secs_f64()})
    );

    let warmed_at = Instant::now();
    let warm = audio.transcribe_file(&audio_path)?;
    println!(
        "{}",
        json!({
            "kind": "warmup",
            "elapsedSeconds": warmed_at.elapsed().as_secs_f64(),
            "audioSeconds": warm.duration,
        })
    );

    for run in 1..=runs {
        let started_at = Instant::now();
        let result = audio.transcribe_file(&audio_path)?;
        println!(
            "{}",
            json!({
                "kind": "run",
                "run": run,
                "elapsedSeconds": started_at.elapsed().as_secs_f64(),
                "audioSeconds": result.duration,
                "backendProcessingSeconds": result.processing_time,
            })
        );
    }
    Ok(())
}
