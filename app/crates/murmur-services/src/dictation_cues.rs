//! Output-only synthetic cues. Same sine tones/envelopes as the React player;
//! no microphone samples, transcripts or clipboard contents enter this path.
use crate::host::ServiceAccess;
use crate::MutexExt;
use murmur_core::events::{Consumer, EventKind};
/// What the cue subscriptions heard.
pub(crate) enum Heard<'a> {
    /// A dictation event name and its raw payload, for `cue_for`.
    Dictation(&'a str, &'a str),
    /// A meeting cue already decided from an accepted status transition.
    Meeting(&'static str),
}

pub(crate) fn initialize_event_subscriptions<R: crate::host::Runtime>(
    app: &crate::host::Context<R>,
    callback: impl Fn(Heard<'_>) + Send + Sync + 'static,
) {
    let callback = std::sync::Arc::new(callback);
    // The shipped hook keeps the last status it saw whether or not cues are
    // enabled, so turning the setting on mid-meeting cannot replay a transition.
    let meeting = std::sync::Arc::new(std::sync::Mutex::new(MeetingCues::default()));
    for (event, cue) in [
        (EventKind::DictationGenerationStarted, "start"),
        (EventKind::RecordingStatusChanged, "stop"),
        (EventKind::DictationDeliveryOutcome, "delivery"),
        (EventKind::RecordingInitializationFailed, "failure"),
        (EventKind::RecordingInterrupted, "failure"),
        (EventKind::MeetingStatusChanged, MEETING),
    ] {
        let callback = callback.clone();
        let meeting = meeting.clone();
        crate::events::subscribe(app, Consumer::SoundCues, event, move |payload| {
            if cue == MEETING {
                let heard = meeting.lock_or_recover().hear(payload);
                if let Some(cue) = heard {
                    callback(Heard::Meeting(cue));
                }
            } else {
                callback(Heard::Dictation(cue, payload));
            }
        });
    }
}
/// Event name routed to the meeting status memory; every other name goes to `cue_for`.
const MEETING: &str = "meeting";

pub(crate) fn initialize(app: &crate::host::Context) {
    let handle = app.clone();
    initialize_event_subscriptions(app, move |heard| {
        let (cue, meeting) = match heard {
            Heard::Meeting(cue) => (Some(cue), true),
            Heard::Dictation(event, payload) => (cue_for(event, payload), false),
        };
        let Some(cue) = cue else {
            return;
        };
        let control = handle.control();
        let settings = control.settings.lock_or_recover();
        let meeting_active = !meeting && handle.services().meetings.is_active();
        let Some(volume) = cue_volume(&settings, meeting, meeting_active) else {
            return;
        };
        drop(settings);
        // A blocking worker owns only this short-lived output player and
        // removes its synthetic WAV after playback. It cannot capture input.
        crate::host::runtime::spawn_blocking(move || play(cue, volume));
    });
}

/// The playback volume when settings allow this cue, otherwise `None`. Meeting
/// cues have their own opt-in and are never suppressed by the meeting itself;
/// dictation cues stay silent while a meeting is active.
fn cue_volume(settings: &serde_json::Value, meeting: bool, meeting_active: bool) -> Option<f64> {
    if settings["soundCuesEnabled"] != true {
        return None;
    }
    if meeting && settings["meetingSoundCuesEnabled"] != true {
        return None;
    }
    if !meeting && meeting_active {
        return None;
    }
    let volume = settings["soundCueVolume"]
        .as_f64()
        .unwrap_or(45.0)
        .round()
        .clamp(0.0, 100.0);
    (volume > 0.0).then_some(volume)
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

/// The generation and runtime phase carried by a `meeting-status-changed` payload.
fn meeting_status(payload: &str) -> Option<(u64, String)> {
    let value: serde_json::Value = serde_json::from_str(payload).ok()?;
    Some((
        value["generation"].as_u64()?,
        value["phase"].as_str()?.to_owned(),
    ))
}

/// The last accepted meeting status. Like shipped `useMeetings`, a status from an
/// older generation is dropped before it can replace the newer one or sound a
/// cue: the coordinator publishes a finished meeting's terminal status after
/// releasing ownership, so it can arrive after the next meeting has started.
struct MeetingCues {
    generation: u64,
    phase: String,
}
impl Default for MeetingCues {
    fn default() -> Self {
        Self {
            generation: 0,
            phase: "idle".into(),
        }
    }
}
impl MeetingCues {
    fn hear(&mut self, payload: &str) -> Option<&'static str> {
        let (generation, phase) = meeting_status(payload)?;
        if generation < self.generation {
            return None;
        }
        let cue = meeting_cue(&self.phase, &phase);
        self.generation = generation;
        self.phase = phase;
        cue
    }
}

/// Meeting transitions that sound a cue, matching the shipped `useSoundCues`.
fn meeting_cue(previous: &str, phase: &str) -> Option<&'static str> {
    if previous == phase {
        return None;
    }
    match (previous, phase) {
        ("idle", "starting") => Some("start"),
        (_, "stopping") => Some("stop"),
        ("processing", "idle") => Some("success"),
        (_, "failed") => Some("failure"),
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

pub(crate) fn preview(cue: &str, volume: f64) {
    play(cue, volume)
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
    fn meeting_transitions_sound_the_shipped_cues() {
        assert_eq!(meeting_cue("idle", "starting"), Some("start"));
        assert_eq!(meeting_cue("recording", "stopping"), Some("stop"));
        assert_eq!(meeting_cue("starting", "stopping"), Some("stop"));
        assert_eq!(meeting_cue("processing", "idle"), Some("success"));
        assert_eq!(meeting_cue("recording", "failed"), Some("failure"));
        assert_eq!(meeting_cue("starting", "failed"), Some("failure"));
        // Only the transitions the shipped hook names make a sound.
        assert_eq!(meeting_cue("stopping", "processing"), None);
        assert_eq!(meeting_cue("starting", "recording"), None);
        assert_eq!(meeting_cue("stopping", "idle"), None);
        assert_eq!(meeting_cue("failed", "idle"), None);
        assert_eq!(meeting_cue("failed", "starting"), None);
        // A repeat of the same phase (elapsed-time or channel updates) is silent.
        for phase in [
            "idle",
            "starting",
            "recording",
            "stopping",
            "processing",
            "failed",
        ] {
            assert_eq!(meeting_cue(phase, phase), None);
        }
    }
    fn status(generation: u64, phase: &str) -> String {
        serde_json::json!({"generation":generation,"sessionId":null,"phase":phase}).to_string()
    }
    #[test]
    fn a_full_meeting_sounds_start_stop_and_success_once_each() {
        let mut cues = MeetingCues::default();
        let heard: Vec<_> = [
            "starting",
            "recording",
            "recording",
            "stopping",
            "processing",
            "processing",
            "idle",
            "idle",
        ]
        .into_iter()
        .filter_map(|phase| cues.hear(&status(4, phase)))
        .collect();
        assert_eq!(heard, ["start", "stop", "success"]);
    }
    #[test]
    fn an_older_generation_neither_sounds_nor_rewinds_the_state() {
        let mut cues = MeetingCues::default();
        assert_eq!(cues.hear(&status(2, "starting")), Some("start"));
        assert_eq!(cues.hear(&status(2, "recording")), None);
        // The previous meeting's terminal status arrives late.
        assert_eq!(cues.hear(&status(1, "failed")), None);
        assert_eq!(cues.hear(&status(1, "stopping")), None);
        assert_eq!(cues.hear(&status(1, "idle")), None);
        // It did not move the remembered phase: the stop is still a stop.
        assert_eq!(cues.hear(&status(2, "stopping")), Some("stop"));
        // The same generation is accepted, and so is a newer one.
        assert_eq!(cues.hear(&status(2, "processing")), None);
        assert_eq!(cues.hear(&status(2, "idle")), Some("success"));
        assert_eq!(cues.hear(&status(3, "starting")), Some("start"));
    }
    #[test]
    fn meeting_status_comes_from_the_payload_and_bad_payloads_are_ignored() {
        assert_eq!(
            meeting_status(&status(3, "stopping")),
            Some((3, "stopping".to_owned()))
        );
        assert_eq!(meeting_status("{}"), None);
        assert_eq!(meeting_status("{\"phase\":\"idle\"}"), None);
        assert_eq!(meeting_status("{\"generation\":1}"), None);
        assert_eq!(meeting_status("\"idle\""), None);
        assert_eq!(meeting_status("not json"), None);
        let mut cues = MeetingCues::default();
        assert_eq!(cues.hear("not json"), None);
        assert_eq!(cues.hear(&status(1, "starting")), Some("start"));
    }
    #[test]
    fn meeting_cues_need_both_toggles_and_ignore_the_active_meeting() {
        let settings = |cues: bool, meeting: bool, volume: i64| {
            serde_json::json!({
                "soundCuesEnabled": cues,
                "meetingSoundCuesEnabled": meeting,
                "soundCueVolume": volume,
            })
        };
        // The shipped default has meeting cues off, so nothing sounds.
        assert_eq!(cue_volume(&settings(true, false, 45), true, true), None);
        assert_eq!(cue_volume(&settings(false, true, 45), true, true), None);
        assert_eq!(cue_volume(&settings(true, true, 0), true, true), None);
        // While a meeting is active its own cues still play at the set volume.
        assert_eq!(
            cue_volume(&settings(true, true, 45), true, true),
            Some(45.0)
        );
        assert_eq!(
            cue_volume(&settings(true, true, 250), true, false),
            Some(100.0)
        );
        // Missing settings fall back to the default volume only when both are on.
        let sparse = serde_json::json!({"soundCuesEnabled": true, "meetingSoundCuesEnabled": true});
        assert_eq!(cue_volume(&sparse, true, false), Some(45.0));
    }
    #[test]
    fn dictation_cues_stay_suppressed_during_a_meeting_whatever_the_meeting_toggle() {
        for meeting_toggle in [false, true] {
            let settings = serde_json::json!({
                "soundCuesEnabled": true,
                "meetingSoundCuesEnabled": meeting_toggle,
                "soundCueVolume": 45,
            });
            assert_eq!(cue_volume(&settings, false, true), None);
            assert_eq!(cue_volume(&settings, false, false), Some(45.0));
        }
        let off = serde_json::json!({"soundCuesEnabled": false, "soundCueVolume": 45});
        assert_eq!(cue_volume(&off, false, false), None);
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
