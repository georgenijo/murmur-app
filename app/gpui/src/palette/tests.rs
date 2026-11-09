use super::{fixtures, model::*};
#[test]
fn tiers_and_tokens_match_react() {
    let s = fixtures::snapshot();
    assert_eq!(
        s.commands[filter(&s.commands, "set del")[0]].id,
        "settings-delivery"
    );
    assert!(filter(&s.commands, "unknown query").is_empty());
    assert!(
        score(
            s.commands
                .iter()
                .find(|c| c.id == "settings-delivery")
                .unwrap(),
            "delivery"
        )
        .unwrap()
            > score(
                s.commands
                    .iter()
                    .find(|c| c.id == "settings-delivery")
                    .unwrap(),
                "dlv"
            )
            .unwrap()
    );
}
#[test]
fn editing_shortcuts_are_preserved() {
    assert_eq!(
        shortcut("K", true, false, false, false, true),
        Some(Shortcut::Palette)
    );
    assert_eq!(shortcut("k", false, true, false, false, true), None);
    assert_eq!(
        shortcut("f", false, true, false, false, false),
        Some(Shortcut::Search)
    );
    assert_eq!(shortcut("k", true, false, false, true, false), None);
    // Every shortcut works with Command anywhere and with Control outside fields.
    for (key, expected) in [
        ("k", Shortcut::Palette),
        ("f", Shortcut::Search),
        (",", Shortcut::Settings),
        ("l", Shortcut::Logs),
    ] {
        assert_eq!(
            shortcut(key, true, false, false, false, true),
            Some(expected)
        );
        assert_eq!(
            shortcut(key, false, true, false, false, false),
            Some(expected)
        );
        assert_eq!(shortcut(key, false, true, false, false, true), None);
        assert_eq!(shortcut(key, true, false, true, false, false), None);
    }
    assert_eq!(shortcut("j", true, false, false, false, false), None);
}

#[test]
fn registry_tracks_runtime_and_has_no_empty_history_actions() {
    use super::registry::*;
    let mut s = State::default();
    let find = |v: Vec<Command>, id: &str| v.into_iter().find(|c| c.id == id);
    assert!(find(commands(&s), "history-copy-last").is_none());
    s.recording = Recording::Starting;
    assert_eq!(
        find(commands(&s), "recording-toggle").unwrap().title,
        "Cancel microphone connection"
    );
    s.meeting = Meeting::Recording;
    assert!(find(commands(&s), "recording-toggle").is_none());
    assert_eq!(
        find(commands(&s), "meeting-toggle").unwrap().title,
        "Stop meeting capture"
    );
    s.disabled = true;
    assert_eq!(
        find(commands(&s), "app-disable-toggle").unwrap().title,
        "Enable Murmur"
    );
    s.enabled_modes = vec![("builtin.email".into(), "Email".into())];
    s.pending_mode = Some("builtin.email".into());
    assert_eq!(
        find(commands(&s), "next-recording-builtin.email")
            .unwrap()
            .hint
            .as_deref(),
        Some("pending")
    );
    assert!(find(commands(&s), "next-recording-clear").is_some());
}
