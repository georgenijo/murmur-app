//! Output-only synthetic cues. Same sine tones/envelopes as the React player;
//! no microphone samples, transcripts or clipboard contents enter this path.
use crate::host::ServiceAccess;
use crate::MutexExt;
use murmur_core::events::{Consumer, EventKind};
pub(crate) fn initialize_event_subscriptions<R: crate::host::Runtime>(
    app: &crate::host::Context<R>,
    callback: impl Fn(&str, &str) + Send + Sync + 'static,
) {
    let callback = std::sync::Arc::new(callback);
    for (event, cue) in [
        (EventKind::DictationGenerationStarted, "start"),
        (EventKind::RecordingStatusChanged, "stop"),
        (EventKind::DictationDeliveryOutcome, "delivery"),
        (EventKind::RecordingInitializationFailed, "failure"),
        (EventKind::RecordingInterrupted, "failure"),
    ] {
        let callback = callback.clone();
        crate::events::subscribe(app, Consumer::SoundCues, event, move |payload| {
            callback(cue, payload)
        });
    }
}
pub(crate) fn initialize(app: &crate::host::Context) {
    let handle = app.clone();
    initialize_event_subscriptions(app, move |cue, payload| {
        let Some(cue) = cue_for(cue, payload) else {
            return;
        };
        let control = handle.control();
        let settings = control.settings.lock_or_recover();
        if settings["soundCuesEnabled"] != true || handle.services().meetings.is_active() {
            return;
        }
        let volume = settings["soundCueVolume"]
            .as_f64()
            .unwrap_or(45.0)
            .round()
            .clamp(0.0, 100.0);
        if volume == 0.0 {
            return;
        }
        drop(settings);
        // A blocking worker owns only this short-lived output player and
        // removes its synthetic WAV after playback. It cannot capture input.
        crate::host::runtime::spawn_blocking(move || play(cue, volume));
    });
}

fn cue_for(event: &str, payload: &str) -> Option<&'static str> {
    match event {
        "start" => Some("start"),
        "stop" => (payload == "\"processing\"").then_some("stop"),
        "failure" => Some("failure"),
        "delivery" => {
            let value: serde_json::Value = serde_json::from_str(payload).ok()?;
            match value["outcome"].as_str() {
                Some("autoPastePosted" | "clipboardOnly") => Some("success"),
                Some("unconfirmed" | "clipboardWriteFailed") => Some("failure"),
                _ => None,
            }
        }
        _ => None,
    }
}

fn play(cue: &str, volume: f64) {
    let tones: &[(f64, usize, usize)] = match cue {
        "start" => &[(660.0, 0, 70)],
        "stop" => &[(440.0, 0, 70)],
        "success" => &[(587.0, 0, 65), (784.0, 55, 90)],
        _ => &[(330.0, 0, 80), (220.0, 65, 120)],
    };
    let sample_rate = 44100;
    let duration = tones
        .iter()
        .map(|(_, offset, duration)| offset + duration)
        .max()
        .unwrap_or(0);
    let _ = std::fs::create_dir_all(crate::identity::temp_dir());
    let path = crate::identity::temp_dir().join(format!("murmur-cue-{}.wav", uuid::Uuid::new_v4()));
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    if let Ok(mut writer) = hound::WavWriter::create(&path, spec) {
        for i in 0..duration * sample_rate as usize / 1000 {
            let ms = i as f64 * 1000.0 / sample_rate as f64;
            let mut sample = 0.0;
            for &(frequency, offset, duration) in tones {
                let t = ms - offset as f64;
                if t < 0.0 || t >= duration as f64 {
                    continue;
                }
                let peak = (volume / 100.0 * 0.18).max(0.0001);
                let gain = if t < 8.0 {
                    0.0001 * (peak / 0.0001).powf(t / 8.0)
                } else {
                    peak * (0.0001 / peak).powf((t - 8.0) / (duration as f64 - 8.0))
                };
                sample += (std::f64::consts::TAU * frequency * t / 1000.0).sin() * gain;
            }
            let _ = writer.write_sample((sample.clamp(-1.0, 1.0) * i16::MAX as f64) as i16);
        }
        if writer.finalize().is_ok() {
            let _ = std::process::Command::new("/usr/bin/afplay")
                .arg(&path)
                .status();
        }
    }
    let _ = std::fs::remove_file(path);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_cues_preserve_start_stop_and_failure_event_mapping() {
        assert_eq!(cue_for("start", "{}"), Some("start"));
        assert_eq!(cue_for("stop", "\"processing\""), Some("stop"));
        for status in ["idle", "starting", "recording", "recovering"] {
            assert_eq!(cue_for("stop", &format!("\"{status}\"")), None);
        }
        assert_eq!(cue_for("failure", "{}"), Some("failure"));
    }
    #[test]
    fn native_cues_map_every_camel_case_delivery_outcome_and_ignore_unknown_payloads() {
        for (outcome, expected) in [
            ("autoPastePosted", Some("success")),
            ("clipboardOnly", Some("success")),
            ("clipboardWriteFailed", Some("failure")),
            ("unconfirmed", Some("failure")),
            ("bogus", None),
            ("auto_pasted", None),
        ] {
            assert_eq!(
                cue_for(
                    "delivery",
                    &serde_json::json!({"outcome":outcome}).to_string()
                ),
                expected
            );
        }
        assert_eq!(cue_for("delivery", "null"), None);
        assert_eq!(cue_for("delivery", "invalid"), None);
    }
}
