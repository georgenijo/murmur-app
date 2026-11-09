//! React App.tsx registry labels/IDs. The host supplies the existing callbacks.
use super::model::Command;
#[derive(Clone, Copy, Default)]
pub enum Recording {
    #[default]
    Idle,
    Starting,
    Recording,
}
#[derive(Clone, Copy, Default)]
pub enum Meeting {
    #[default]
    Idle,
    Starting,
    Recording,
    Stopping,
    Processing,
    Failed,
}
#[derive(Default)]
pub struct State {
    pub recording: Recording,
    pub meeting: Meeting,
    pub disabled: bool,
    pub has_history: bool,
    pub paste_shortcut: Option<String>,
    pub enabled_modes: Vec<(String, String)>,
    pub pending_mode: Option<String>,
}
fn command(id: &str, title: &str, section: &str, keywords: &[&str]) -> Command {
    Command {
        id: id.into(),
        title: title.into(),
        section: section.into(),
        keywords: keywords.iter().map(|s| s.to_string()).collect(),
        hint: None,
    }
}
pub fn commands(s: &State) -> Vec<Command> {
    let mut out = vec![];
    if matches!(s.meeting, Meeting::Idle | Meeting::Failed) {
        out.push(command(
            "recording-toggle",
            match s.recording {
                Recording::Starting => "Cancel microphone connection",
                Recording::Recording => "Stop recording",
                Recording::Idle => "Start recording",
            },
            "Recording",
            &["dictate", "microphone", "transcribe"],
        ));
    }
    for (id, label) in &s.enabled_modes {
        let mut c = command(
            &format!("next-recording-{id}"),
            &format!("Next recording: {label}"),
            "Recording",
            &["mode", "once", "override", "dictation"],
        );
        c.hint = Some(
            if s.pending_mode.as_ref() == Some(id) {
                "pending"
            } else {
                "once"
            }
            .into(),
        );
        out.push(c);
    }
    if s.pending_mode.is_some() {
        out.push(command(
            "next-recording-clear",
            "Clear next recording override",
            "Recording",
            &["mode", "cancel", "pending"],
        ));
    }
    // The shipped single toggle id is split by intent. Dispatch re-reads this
    // registry, so a meeting that changed phase after selection refuses the entry
    // instead of turning "show progress" or "stop" into a new recording.
    let (id, title) = match s.meeting {
        Meeting::Stopping | Meeting::Processing => {
            ("meeting-progress", "Show meeting transcript progress")
        }
        Meeting::Starting | Meeting::Recording => ("meeting-stop", "Stop meeting capture"),
        Meeting::Idle | Meeting::Failed => ("meeting-start", "Start meeting capture"),
    };
    out.push(command(
        id,
        title,
        "Meeting",
        &["system audio", "call", "me", "them", "record"],
    ));
    let mut disabled = command(
        "app-disable-toggle",
        if s.disabled {
            "Enable Murmur"
        } else {
            "Disable Murmur"
        },
        "Recording",
        &["pause", "mute", "hotkey"],
    );
    if s.disabled {
        disabled.hint = Some("currently off".into())
    }
    out.push(disabled);
    out.push(command(
        "transcribe-file",
        "Transcribe audio or video file…",
        "Recording",
        &[
            "file", "import", "audio", "video", "wav", "mp3", "m4a", "mp4", "mov",
        ],
    ));
    let mut paste = command(
        "paste-last",
        "Paste last or retry delivery",
        "History",
        &["clipboard", "again", "recover", "retry", "delivery"],
    );
    paste.hint = s.paste_shortcut.clone();
    out.push(paste);
    out.push(command(
        "history-search",
        "Search transcripts",
        "History",
        &["find", "filter", "history"],
    ));
    if s.has_history {
        out.push(command(
            "history-copy-last",
            "Copy last transcript",
            "History",
            &["clipboard", "again"],
        ));
        out.push(command(
            "history-export",
            "Export history to a Markdown file…",
            "History",
            &["save", "download", "markdown"],
        ));
    }
    out.push(command(
        "show-history",
        "Show transcription history",
        "Navigation",
        &["record", "main"],
    ));
    out.push(command(
        "show-meetings",
        "Show meeting transcripts",
        "Navigation",
        &["system audio", "calls", "me", "them"],
    ));
    out.push(command(
        "settings-shortcuts",
        "Settings: Keyboard shortcuts",
        "Settings",
        &["hotkeys", "bindings", "keys"],
    ));
    for page in crate::settings::schema::Page::NAV {
        if matches!(
            page,
            crate::settings::schema::Page::Diagnostics | crate::settings::schema::Page::Performance
        ) {
            out.push(command(
                &format!("settings-{}", page.id()),
                &format!("Open {}", page.label()),
                "Tools",
                if page == crate::settings::schema::Page::Diagnostics {
                    &["events", "debug", "log", "logs", "runs", "performance"]
                } else {
                    &["benchmark", "models", "speed", "accuracy"]
                },
            ));
        } else {
            out.push(command(
                &format!("settings-{}", page.id()),
                &format!("Settings: {}", page.label()),
                "Settings",
                &["preferences", "options"],
            ));
        }
    }
    for (id, title, keywords) in [
        (
            "check-updates",
            "Check for updates",
            vec!["version", "upgrade"],
        ),
        ("about", "About Murmur", vec!["version", "credits"]),
        (
            "rerun-setup",
            "Re-run setup assistant",
            vec!["onboarding", "permissions", "wizard"],
        ),
    ] {
        out.push(command(id, title, "App", &keywords));
    }
    out
}
