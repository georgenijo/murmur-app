use super::{
    Settings, fixtures,
    host::{Action, Authority, Destination, RequestKind},
    model::{Event, Model, Problem},
    schema::{Kind, Page, ROWS},
};
use fixtures::FixtureHost;
use gpui::{Entity, Focusable, prelude::*};
use gpui::{TestAppContext, VisualTestContext};
use serde_json::{Value, json};
use std::{collections::BTreeSet, rc::Rc};
fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|w, cx| w.draw(cx).clear(cx));
}
fn keys(cx: &mut VisualTestContext, source: &str) {
    for key in source.split_whitespace() {
        let keystroke =
            cx.update(|_, cx| gpui::Keystroke::parse_with_os(key, cx.platform_info().os).unwrap());
        cx.simulate_event(gpui::KeyDownEvent {
            keystroke: keystroke.clone(),
            is_held: false,
            prefer_character_input: false,
        });
        cx.simulate_event(gpui::KeyUpEvent { keystroke });
        draw(cx);
    }
}
fn harness(cx: &mut TestAppContext) -> (Entity<Settings>, Rc<FixtureHost>, &mut VisualTestContext) {
    cx.update(crate::init);
    let host = Rc::new(FixtureHost::default());
    let mut entity = None;
    let (_, cx) = cx.add_window_view(|w, cx| {
        let e = cx.new(|cx| Settings::new(host.clone(), w, cx));
        entity = Some(e.clone());
        gpui_base::Root::new(e, w, cx)
    });
    let e = entity.unwrap();
    e.update(cx, |s, cx| s.attach(cx));
    draw(cx);
    (e, host, cx)
}
#[test]
fn catalog_matches_every_preference_owner_and_core_validation() {
    let defaults = murmur_core::state::settings_defaults();
    let mut keys = BTreeSet::new();
    for row in ROWS {
        assert!(keys.insert(row.key), "duplicate {}", row.key);
        assert!(defaults.get(row.key).is_some(), "unknown {}", row.key);
        if let Kind::Choice(options) = row.kind {
            for (v, _) in options {
                let parsed = row.parse(v).unwrap();
                assert!(
                    murmur_core::state::settings_edit(
                        &murmur_core::state::settings(defaults.clone()).unwrap(),
                        row.key,
                        parsed
                    )
                    .is_ok(),
                    "invalid option {} {v}",
                    row.key
                );
            }
        }
    }
    assert_eq!(
        Page::NAV.map(Page::id),
        [
            "general",
            "recording",
            "customize",
            "delivery",
            "meetings",
            "text",
            "ai",
            "modes",
            "appearance",
            "performance",
            "diagnostics"
        ]
    );
    for (alias, page) in [
        ("dictation", Page::Recording),
        ("model", Page::Transcription),
        ("app", Page::General),
        ("unknown", Page::General),
    ] {
        assert_eq!(Page::resolve(alias), page);
    }
    let mut reference = murmur_core::state::settings(defaults.clone()).unwrap();
    assert_eq!(
        super::schema::shortcut_reference(&reference),
        [
            "Recording: Hold Left Shift · On",
            "Paste last or retry delivery: Not assigned · Off"
        ]
    );
    reference["recordingMode"] = json!("both");
    reference["doubleTapKey"] = json!("f13");
    reference["disabled"] = json!(true);
    reference["pasteLastShortcut"] = json!("command_option_v");
    assert_eq!(
        super::schema::shortcut_reference(&reference),
        [
            "Recording: Hold or double-tap F13 · Paused while Murmur is disabled",
            "Paste last or retry delivery: ⌘⌥V · On"
        ]
    );
    let settings = murmur_core::state::settings(defaults).unwrap();
    for row in ROWS {
        assert_eq!(
            row.reveal(&settings),
            if let Some(parent) = row.parent {
                if settings[parent] != true {
                    parent
                } else {
                    row.key
                }
            } else if row.key == "hotkeyMissFeedback" {
                "recordingMode"
            } else {
                row.key
            }
        );
    }
}
#[test]
fn replies_notifications_and_leases_reject_stale_work() {
    let mut m = Model::default();
    m.attach();
    let ticket = m.begin().unwrap();
    assert!(m.event(Event::Reply {
        ticket,
        result: Ok(fixtures::snapshot())
    }));
    let before = m.settings().clone();
    let ticket = m.begin().unwrap();
    let authority = Authority::default();
    authority.sync(&m);
    let lease = authority.lease(ticket);
    assert!(lease.is_current());
    let mut update = fixtures::snapshot();
    update.revision = 2;
    update.settings["autoPaste"] = json!(true);
    assert!(m.event(Event::Changed {
        attachment: m.attachment,
        snapshot: update
    }));
    authority.sync(&m);
    assert!(!lease.is_current());
    assert!(!m.pending);
    assert_eq!(m.problem, Some(Problem::Conflict));
    assert!(!m.event(Event::Reply {
        ticket,
        result: Ok(fixtures::snapshot())
    }));
    assert_ne!(*m.settings(), before);
    let ticket = m.begin().unwrap();
    m.detach();
    m.attach();
    assert!(!m.event(Event::Reply {
        ticket,
        result: Ok(fixtures::snapshot())
    }));
    assert!(m.snapshot.is_none());
}
#[gpui::test]
fn actual_keyboard_navigation_picker_change_and_failure_reversion(cx: &mut TestAppContext) {
    let (e, host, cx) = harness(cx);
    assert_eq!(host.calls.borrow()[0], "subscribe");
    cx.update(|w, cx| {
        e.update(cx, |s, cx| {
            w.focus(&s.navigation.read(cx).focus_handle(cx), cx)
        })
    });
    keys(cx, "home right space");
    draw(cx);
    assert_eq!(e.read_with(cx, |s, _| s.model.page), Page::Recording);
    cx.update(|w, cx| {
        e.update(cx, |s, cx| {
            w.focus(&s.pickers["recordingMode"].read(cx).focus_handle(cx), cx)
        })
    });
    keys(cx, "space down enter");
    draw(cx);
    assert_eq!(host.state.borrow().settings["recordingMode"], "double_tap");
    host.fail.set(Some(Problem::Denied));
    keys(cx, "space down enter");
    draw(cx);
    assert_eq!(host.state.borrow().settings["recordingMode"], "double_tap");
    assert_eq!(
        e.read_with(cx, |s, cx| s.pickers["recordingMode"]
            .read(cx)
            .value()
            .unwrap()
            .to_string()),
        "double_tap"
    );
    assert_eq!(
        e.read_with(cx, |s, _| s.model.problem),
        Some(Problem::Denied)
    );
}
#[gpui::test]
fn number_validation_and_core_durable_round_trip(cx: &mut TestAppContext) {
    let root =
        std::env::temp_dir().join(format!("murmur-settings-synthetic-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    cx.update(crate::init);
    let host = Rc::new(FixtureHost {
        root: Some(root.clone()),
        ..Default::default()
    });
    let mut entity = None;
    let (_, cx) = cx.add_window_view(|w, cx| {
        let e = cx.new(|cx| Settings::new(host.clone(), w, cx));
        entity = Some(e.clone());
        gpui_base::Root::new(e, w, cx)
    });
    let e = entity.unwrap();
    e.update(cx, |s, cx| s.attach(cx));
    cx.update(|w, cx| {
        e.update(cx, |s, cx| {
            s.navigate(Page::Meetings, None, w, cx);
            s.request(
                RequestKind::Disclosure {
                    page: Page::Meetings,
                    open: true,
                },
                cx,
            );
        })
    });
    draw(cx);
    let row = *ROWS.iter().find(|r| r.key == "meetingMaxSessions").unwrap();
    cx.update(|w, cx| {
        e.update(cx, |s, cx| {
            s.numbers[row.key].update(cx, |f, cx| f.set_value("10001", w, cx));
            w.focus(&s.numbers[row.key].read(cx).focus_handle(cx), cx);
        })
    });
    keys(cx, "enter");
    draw(cx);
    assert_eq!(host.state.borrow().settings["meetingMaxSessions"], 100);
    assert_eq!(
        e.read_with(cx, |s, _| s.model.problem),
        Some(Problem::Invalid)
    );
    cx.update(|w, cx| {
        e.update(cx, |s, cx| {
            s.numbers[row.key].update(cx, |f, cx| f.set_value("72", w, cx))
        })
    });
    keys(cx, "enter");
    draw(cx);
    let durable: Value =
        serde_json::from_slice(&std::fs::read(root.join("settings.json")).unwrap()).unwrap();
    assert_eq!(durable["meetingMaxSessions"], 72);
    assert_eq!(
        murmur_core::state::settings(durable.clone()).unwrap(),
        durable
    );
    assert_eq!(
        durable["appProfiles"],
        host.state.borrow().settings["appProfiles"]
    );
    let disclosure: Value =
        serde_json::from_slice(&std::fs::read(root.join("ui-state.json")).unwrap()).unwrap();
    assert_eq!(
        disclosure["values"]["murmur.settings-disclosure.v1"]["panes"]["meetings"],
        true
    );
    std::fs::remove_dir_all(root).unwrap();
}
#[gpui::test]
fn folded_dependencies_search_do_not_enable_features_or_prompt(cx: &mut TestAppContext) {
    let (e, host, cx) = harness(cx);
    cx.update(|w, cx| e.update(cx, |s, cx| w.focus(&s.search.read(cx).focus_handle(cx), cx)));
    cx.simulate_input("filler");
    draw(cx);
    let row = *ROWS
        .iter()
        .find(|r| r.key == "cleanupRemoveFiller")
        .unwrap();
    cx.update(|w, cx| e.update(cx, |s, cx| s.search_result(row, w, cx)));
    draw(cx);
    assert_eq!(e.read_with(cx, |s, _| s.model.page), Page::Text);
    assert_eq!(e.read_with(cx, |s, _| s.reveal), Some("cleanupEnabled"));
    assert_eq!(host.state.borrow().settings["cleanupEnabled"], false);
    assert!(host.actions.borrow().is_empty());
}
#[gpui::test]
fn nested_owner_back_and_editor_close_restore_actual_callers(cx: &mut TestAppContext) {
    let (e, _, cx) = harness(cx);
    cx.update(|w, cx| {
        e.update(cx, |s, cx| {
            s.open_detail(Page::Shortcuts, "shortcuts", w, cx)
        })
    });
    draw(cx);
    cx.update(|w, cx| {
        e.update(cx, |s, cx| {
            s.open_detail(Page::Recording, "shortcut-recording", w, cx)
        })
    });
    draw(cx);
    keys(cx, "escape");
    draw(cx);
    assert_eq!(e.read_with(cx, |s, _| s.model.page), Page::Shortcuts);
    assert!(cx.update(|w, cx| e.read(cx).focus["shortcut-recording"].is_focused(w)));
    keys(cx, "escape");
    draw(cx);
    assert_eq!(e.read_with(cx, |s, _| s.model.page), Page::General);
    assert!(cx.update(|w, cx| e.read(cx).focus["shortcuts"].is_focused(w)));
    cx.update(|w, cx| {
        e.update(cx, |s, cx| {
            s.navigate(Page::Customize, None, w, cx);
            s.open_editor(Destination::VoiceCommands, "customize-commands", cx);
        })
    });
    draw(cx);
    cx.update(|w, cx| e.update(cx, |s, cx| s.editor_closed(w, cx)));
    draw(cx);
    assert!(cx.update(|w, cx| e.read(cx).focus["customize-commands"].is_focused(w)));
}
#[gpui::test]
fn file_delivery_pause_capture_lock_and_explicit_actions(cx: &mut TestAppContext) {
    let (e, host, cx) = harness(cx);
    let mut next = host.state.borrow().clone();
    next.revision += 1;
    next.settings["autoPaste"] = json!(true);
    next.settings["smartDestinationEnabled"] = json!(true);
    next.settings["saveAudio"] = json!(true);
    next.runtime.recording = true;
    *host.state.borrow_mut() = next.clone();
    e.update(cx, |s, cx| {
        s.receive(
            Event::Changed {
                attachment: s.model.attachment,
                snapshot: next,
            },
            cx,
        )
    });
    for key in [
        "autoPaste",
        "autoPasteDelayMs",
        "smartDestinationEnabled",
        "recordingMode",
        "microphone",
        "model",
        "language",
    ] {
        assert!(e.read_with(cx, |s, _| {
            s.model
                .disabled(*ROWS.iter().find(|r| r.key == key).unwrap())
        }));
    }
    assert_eq!(
        e.read_with(cx, |s, _| s.model.settings()["autoPaste"].clone()),
        true
    );
    assert!(host.actions.borrow().is_empty());
    cx.update(|w, cx| e.update(cx, |s, cx| s.navigate(Page::Performance, None, w, cx)));
    draw(cx);
    assert!(
        host.actions.borrow().is_empty(),
        "opening Lab ran a diagnostic"
    );
    e.update(cx, |s, cx| s.action(Action::ResetStatistics, true, cx));
    assert!(host.actions.borrow().is_empty());
    cx.update(|w, cx| {
        e.update(cx, |s, cx| {
            w.focus(&s.navigation.read(cx).focus_handle(cx), cx)
        })
    });
    keys(cx, "escape");
    draw(cx);
    assert!(e.read_with(cx, |s, _| s.confirmation.is_none()));
}
#[gpui::test]
fn hide_drop_revokes_pending_work_and_releases_subscription(cx: &mut TestAppContext) {
    let (e, host, cx) = harness(cx);
    let old = e.read_with(cx, |s, _| s.model.ticket());
    let lease = e.read_with(cx, |s, _| s.authority.lease(old));
    cx.update(|w, cx| e.update(cx, |s, cx| s.hide(w, cx)));
    assert_eq!(host.stops.get(), 1);
    assert_eq!(host.pages.borrow().last(), Some(&None));
    assert!(!lease.is_current());
    assert!(e.read_with(cx, |s, _| s.inbox.is_closed() && s.model.snapshot.is_none()));
    assert!(e.read_with(cx, |s, cx| s.search.read(cx).value().is_empty()));
}
#[cfg(feature = "a11y-tests")]
mod ax {
    use super::*;
    use gpui::AccessibleAction;
    use gpui::accesskit::{ActionRequest, NodeId, TreeId, TreeUpdate};
    fn tree(cx: &mut VisualTestContext) -> TreeUpdate {
        let w = cx.update(|w, _| w.window_handle());
        cx.activate_accessibility(w);
        draw(cx);
        cx.accessibility_tree(w)
    }
    fn named(t: &TreeUpdate, label: &str) -> NodeId {
        t.nodes
            .iter()
            .find(|(_, n)| n.label() == Some(label))
            .unwrap_or_else(|| panic!("missing {label}"))
            .0
    }
    fn click(cx: &mut VisualTestContext, id: NodeId) {
        let w = cx.update(|w, _| w.window_handle());
        cx.dispatch_accessibility_action(
            w,
            ActionRequest {
                action: AccessibleAction::Click,
                target_tree: TreeId::ROOT,
                target_node: id,
                data: None,
            },
        );
        draw(cx);
    }
    #[gpui::test]
    fn shared_pane_prose_is_static_text(cx: &mut TestAppContext) {
        let (_, _, cx) = harness(cx);
        let t = tree(cx);
        click(cx, named(&t, "Meetings"));
        let t = tree(cx);
        assert_eq!(
            t.nodes
                .iter()
                .filter(|(_, n)| n.role() == gpui::accesskit::Role::Label
                    && n.value() == Some("Meetings are stored separately from dictation history."))
                .count(),
            1
        );
    }

    #[gpui::test]
    fn real_accesskit_roles_navigation_switches_disclosure_and_inert_children(
        cx: &mut TestAppContext,
    ) {
        let (e, host, cx) = harness(cx);
        let t = tree(cx);
        assert!(t.nodes.len() < 200);
        let search = named(&t, "Search settings");
        assert_eq!(
            t.nodes
                .iter()
                .find(|(id, _)| *id == search)
                .unwrap()
                .1
                .role(),
            gpui::accesskit::Role::TextInput
        );
        named(&t, "Settings pages");
        click(cx, named(&t, "Recording"));
        let t = tree(cx);
        named(&t, "Microphone");
        named(&t, "Recording trigger");
        assert!(!t.nodes.iter().any(|(_, n)| n.label() == Some("Pre-roll")));
        click(cx, named(&t, "Advanced"));
        let t = tree(cx);
        click(cx, named(&t, "Warm microphone standby"));
        let t = tree(cx);
        named(&t, "Pre-roll");
        let switch = t
            .nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Warm microphone standby"))
            .unwrap();
        assert_eq!(switch.1.role(), gpui::accesskit::Role::Switch);
        click(cx, switch.0);
        let t = tree(cx);
        assert!(!t.nodes.iter().any(|(_, n)| n.label() == Some("Pre-roll")));
        assert_eq!(host.state.borrow().settings["capturePreRollMs"], 450);
        cx.update(|w, cx| e.update(cx, |s, cx| s.navigate(Page::Delivery, None, w, cx)));
        let t = tree(cx);
        click(cx, named(&t, "Save audio to file"));
        let t = tree(cx);
        named(&t, "Choose folder…");
        assert!(
            t.nodes
                .iter()
                .any(|(_, n)| n.label() == Some("Auto-paste") && n.is_disabled())
        );
    }
    fn publish(
        e: &Entity<Settings>,
        host: &FixtureHost,
        cx: &mut VisualTestContext,
        change: impl FnOnce(&mut super::super::model::Snapshot),
    ) {
        let mut snapshot = host.state.borrow().clone();
        snapshot.revision += 1;
        change(&mut snapshot);
        *host.state.borrow_mut() = snapshot.clone();
        e.update(cx, |s, cx| {
            s.receive(
                Event::Changed {
                    attachment: s.model.attachment,
                    snapshot,
                },
                cx,
            )
        });
        draw(cx);
    }
    #[gpui::test]
    fn regression_manual_microphone_disables_auto_even_for_saved_device(cx: &mut TestAppContext) {
        let (e, host, cx) = harness(cx);
        publish(&e, &host, cx, |snapshot| {
            snapshot.settings["smartAutoMicrophoneEnabled"] = json!(true)
        });
        let row = *ROWS.iter().find(|r| r.key == "microphone").unwrap();
        // Direct control callback hook: no native HID input or real microphone.
        cx.update(|w, cx| e.update(cx, |s, cx| s.edit(row, json!("system_default"), w, cx)));
        assert_eq!(
            host.state.borrow().settings["smartAutoMicrophoneEnabled"],
            false
        );
        publish(&e, &host, cx, |snapshot| {
            snapshot.settings["smartAutoMicrophoneEnabled"] = json!(true)
        });
        cx.update(|w, cx| {
            e.update(cx, |s, cx| {
                s.edit(row, json!("synthetic.microphone"), w, cx)
            })
        });
        let settings = &host.state.borrow().settings;
        assert_eq!(settings["microphone"], "synthetic.microphone");
        assert_eq!(settings["smartAutoMicrophoneEnabled"], false);
        assert_eq!(settings["microphoneIdMigrationComplete"], true);
    }
    #[gpui::test]
    fn regression_prefer_moves_device_to_front_and_marks_only_first(cx: &mut TestAppContext) {
        let (e, host, cx) = harness(cx);
        publish(&e, &host, cx, |snapshot| {
            snapshot.settings["smartAutoMicrophoneEnabled"] = json!(true);
            snapshot.settings["smartAutoApprovedDeviceIds"] =
                json!(["synthetic.microphone", "synthetic.phone"]);
            snapshot.settings["smartAutoPreferredDeviceIds"] = json!(["synthetic.microphone"]);
            snapshot.disclosure["panes"]["recording"] = json!(true);
        });
        cx.update(|w, cx| e.update(cx, |s, cx| s.navigate(Page::Recording, None, w, cx)));
        e.update(cx, |s, cx| {
            s.scroll.scroll_to_bottom();
            cx.notify();
        });
        let t = tree(cx);
        click(cx, named(&t, "Prefer Synthetic phone"));
        assert_eq!(
            host.state.borrow().settings["smartAutoPreferredDeviceIds"],
            json!(["synthetic.phone", "synthetic.microphone"])
        );
        let t = tree(cx);
        let preferred = |label| {
            t.nodes
                .iter()
                .find(|(_, n)| n.label() == Some(label))
                .unwrap()
                .1
                .toggled()
        };
        assert_eq!(
            preferred("Prefer Synthetic phone"),
            Some(gpui::accesskit::Toggled::True)
        );
        assert_eq!(
            preferred("Prefer Synthetic microphone"),
            Some(gpui::accesskit::Toggled::False)
        );
        click(cx, named(&t, "Prefer Synthetic microphone"));
        assert_eq!(
            host.state.borrow().settings["smartAutoPreferredDeviceIds"],
            json!(["synthetic.microphone", "synthetic.phone"])
        );
    }
    #[gpui::test]
    fn regression_calibration_preview_failure_then_save_keeps_acknowledged_requested_offset(
        cx: &mut TestAppContext,
    ) {
        let (e, host, cx) = harness(cx);
        publish(&e, &host, cx, |snapshot| {
            snapshot.disclosure["panes"]["general"] = json!(true);
        });
        for acknowledged in [0, 1] {
            let t = tree(cx);
            click(cx, named(&t, "Calibrate overlay position…"));
            if acknowledged == 1 {
                let t = tree(cx);
                click(cx, named(&t, "Move overlay down one point"));
            }
            assert_eq!(
                host.state.borrow().runtime.calibration.unwrap().applied,
                acknowledged
            );

            host.fail.set(Some(Problem::Unavailable));
            let t = tree(cx);
            click(cx, named(&t, "Move overlay down one point"));
            assert_eq!(
                e.read_with(cx, |s, _| s.model.problem),
                Some(Problem::Unavailable)
            );
            assert!(e.read_with(cx, |s, _| !s.model.pending));
            assert_eq!(
                host.state.borrow().runtime.calibration.unwrap().applied,
                acknowledged
            );

            let t = tree(cx);
            click(cx, named(&t, "Save position"));
            assert_eq!(
                host.actions.borrow().last(),
                Some(&Action::Calibration {
                    offset: acknowledged,
                    finish: true,
                })
            );
            assert_eq!(
                host.state.borrow().settings["overlayVerticalOffset"],
                acknowledged
            );
            assert!(e.read_with(cx, |s, _| s.calibration.is_none()));
        }
    }
    #[gpui::test]
    fn regression_calibration_save_preserves_requested_display_clamped_offset(
        cx: &mut TestAppContext,
    ) {
        let (e, host, cx) = harness(cx);
        publish(&e, &host, cx, |snapshot| {
            snapshot.settings["overlayVerticalOffset"] = json!(4);
            snapshot.disclosure["panes"]["general"] = json!(true);
            snapshot.runtime.calibration = Some(super::super::model::Calibration {
                requested: 4,
                applied: 2,
                min: -2,
                max: 2,
            });
        });
        let t = tree(cx);
        click(cx, named(&t, "Calibrate overlay position…"));
        let t = tree(cx);
        click(cx, named(&t, "Save position"));
        assert_eq!(host.state.borrow().settings["overlayVerticalOffset"], 4);
        assert!(e.read_with(cx, |s, _| s.calibration.is_none()));
    }
    #[gpui::test]
    fn regression_calibration_reopen_escape_and_mounted_focus(cx: &mut TestAppContext) {
        let (e, host, cx) = harness(cx);
        publish(&e, &host, cx, |snapshot| {
            snapshot.disclosure["panes"]["general"] = json!(true);
        });
        for exit in ["save", "escape", "cancel", "escape"] {
            let t = tree(cx);
            click(cx, named(&t, "Calibrate overlay position…"));
            let t = tree(cx);
            assert_eq!(
                t.focus,
                named(&t, "Cancel calibration"),
                "opening must focus a mounted, enabled preview control"
            );
            if exit == "save" {
                for _ in 0..3 {
                    let t = tree(cx);
                    click(cx, named(&t, "Move overlay down one point"));
                }
                let t = tree(cx);
                click(cx, named(&t, "Save position"));
                assert_eq!(host.state.borrow().settings["overlayVerticalOffset"], 3);
            } else if exit == "cancel" {
                let t = tree(cx);
                click(cx, named(&t, "Cancel calibration"));
            } else {
                keys(cx, "escape");
            }
            let t = tree(cx);
            assert!(e.read_with(cx, |s, _| s.calibration.is_none()));
            assert!(
                !t.nodes
                    .iter()
                    .any(|(_, n)| n.label() == Some("Save position"))
            );
            assert_eq!(t.focus, named(&t, "Calibrate overlay position…"));
        }
    }
    #[gpui::test]
    fn regression_all_panes_tab_reveals_every_scrolling_control(cx: &mut TestAppContext) {
        let (e, host, cx) = harness(cx);
        publish(&e, &host, cx, |snapshot| {
            for page in Page::NAV
                .into_iter()
                .chain([Page::Transcription, Page::Shortcuts])
            {
                snapshot.disclosure["panes"][page.id()] = json!(true);
            }
            for key in [
                "captureStandbyEnabled",
                "smartAutoMicrophoneEnabled",
                "soundCuesEnabled",
                "saveTranscript",
                "saveAudio",
                "cleanupEnabled",
                "correctionEnabled",
                "codeVocabEnabled",
                "meetingSuggestionsEnabled",
            ] {
                snapshot.settings[key] = json!(true);
            }
            snapshot.settings["recordingMode"] = json!("both");
            snapshot.settings["smartAutoApprovedDeviceIds"] = json!(["synthetic.microphone"]);
            snapshot.settings["vadSensitivity"] = json!(60);
            snapshot.settings["autoStopSilenceMs"] = json!(1500);
            snapshot.settings["outputDir"] = json!("/synthetic/output");
            snapshot.settings["benchmarkOutputDir"] = json!("/synthetic/reports");
        });
        for (width, height) in [(720., 560.), (1120., 820.)] {
            cx.simulate_resize(gpui::size(gpui::px(width), gpui::px(height)));
            for page in Page::NAV
                .into_iter()
                .chain([Page::Transcription, Page::Shortcuts])
            {
                cx.update(|w, cx| e.update(cx, |s, cx| s.navigate(page, None, w, cx)));
                let t = tree(cx);
                let panel = t
                    .nodes
                    .iter()
                    .find(|(_, n)| n.role() == gpui::accesskit::Role::TabPanel)
                    .unwrap()
                    .0;
                let mut descendants = BTreeSet::new();
                let mut pending = vec![panel];
                while let Some(id) = pending.pop() {
                    if descendants.insert(id) {
                        let node = &t.nodes.iter().find(|(i, _)| *i == id).unwrap().1;
                        pending.extend(node.children());
                    }
                }
                let expected: BTreeSet<_> = t
                    .nodes
                    .iter()
                    .filter(|(id, node)| {
                        descendants.contains(id)
                            && node.supports_action(AccessibleAction::Focus)
                            && !node.is_disabled()
                    })
                    .map(|(id, _)| *id)
                    .collect();
                // Enter from the first Tab stop in the navigation, then traverse
                // the real Root order. No scroll or reveal test hook is invoked.
                cx.update(|w, cx| {
                    e.update(cx, |s, cx| {
                        if width < 768. {
                            w.focus(&s.focus["nav-general"], cx);
                        } else {
                            w.focus(&s.navigation.read(cx).focus_handle(cx), cx);
                        }
                    })
                });
                let mut visited = BTreeSet::new();
                let mut entered = false;
                for _ in 0..180 {
                    keys(cx, "tab");
                    cx.update(|w, cx| w.simulate_next_frame(cx));
                    draw(cx);
                    let t = tree(cx);
                    if !descendants.contains(&t.focus) {
                        if entered {
                            break;
                        }
                        continue;
                    }
                    entered = true;
                    assert!(visited.insert(t.focus), "Tab must advance in {:?}", page);
                    let n = &t.nodes.iter().find(|(id, _)| *id == t.focus).unwrap().1;
                    let bounds = n.bounds().expect("focused control bounds");
                    let viewport = e.read_with(cx, |s, _| s.scroll.bounds());
                    let scale = cx.update(|w, _| f64::from(w.scale_factor()));
                    assert!(
                        bounds.y0 >= f64::from(viewport.top()) * scale
                            && bounds.y1 <= f64::from(viewport.bottom()) * scale,
                        "{:?} {} at {width}×{height}: {bounds:?} outside {viewport:?}",
                        page,
                        n.label().unwrap_or("unnamed")
                    );
                    if page == Page::Recording && n.label() == Some("Microphone") {
                        // S11 starts by closing the microphone picker before Tab.
                        keys(cx, "space escape");
                    }
                }
                assert!(entered, "must traverse pane {:?}", page);
                assert!(
                    expected.is_subset(&visited),
                    "must visit every focusable control in {:?}: missing {:?}",
                    page,
                    expected.difference(&visited).collect::<Vec<_>>()
                );
                if page == Page::Recording {
                    let t = tree(cx);
                    for label in [
                        "Microphone standby window",
                        "Pre-roll",
                        "Voice detection",
                        "Preview start cue",
                        "Reset Voice detection to default",
                        "Prefer Synthetic microphone",
                    ] {
                        assert!(visited.contains(&named(&t, label)), "must exercise {label}");
                    }
                }
            }
        }
    }
    #[gpui::test]
    fn regression_folder_reset_restores_empty_default_through_owner(cx: &mut TestAppContext) {
        let (e, host, cx) = harness(cx);
        publish(&e, &host, cx, |snapshot| {
            snapshot.settings["saveTranscript"] = json!(true);
            snapshot.settings["outputDir"] = json!("/synthetic/output");
            snapshot.settings["benchmarkOutputDir"] = json!("/synthetic/reports");
        });
        for (page, label, key) in [
            (Page::Delivery, "Output folder", "outputDir"),
            (Page::Performance, "Report folder", "benchmarkOutputDir"),
        ] {
            cx.update(|w, cx| e.update(cx, |s, cx| s.navigate(page, None, w, cx)));
            let t = tree(cx);
            click(cx, named(&t, &format!("Reset {label} to default")));
            assert_eq!(host.state.borrow().settings[key], "");
            let t = tree(cx);
            assert!(
                !t.nodes
                    .iter()
                    .any(|(_, n)| n.label() == Some(&format!("Reset {label} to default")))
            );
        }
    }
    #[gpui::test]
    fn regression_escape_revokes_delayed_preview_and_keeps_retry_on_cancel_failure(
        cx: &mut TestAppContext,
    ) {
        use super::super::host::Host;
        let (e, host, cx) = harness(cx);
        publish(&e, &host, cx, |snapshot| {
            snapshot.disclosure["panes"]["general"] = json!(true)
        });
        host.delay_preview.set(true);
        let t = tree(cx);
        click(cx, named(&t, "Calibrate overlay position…"));
        assert!(e.read_with(cx, |s, _| s.model.pending));
        assert!(host.delayed.borrow()[0].0.lease.is_current());
        let t = tree(cx);
        assert!(
            !t.nodes
                .iter()
                .find(|(_, n)| n.label() == Some("Cancel calibration"))
                .unwrap()
                .1
                .is_disabled()
        );
        host.fail.set(Some(Problem::Unavailable));
        // Dispatch through the mounted view, just like the reopened flow.
        keys(cx, "escape");
        assert!(!host.delayed.borrow()[0].0.lease.is_current());
        assert!(e.read_with(cx, |s, _| s.calibration.is_some()));
        let t = tree(cx);
        click(cx, named(&t, "Cancel calibration"));
        assert_eq!(
            host.actions.borrow().last(),
            Some(&Action::CancelCalibration)
        );
        assert!(e.read_with(cx, |s, _| s.calibration.is_none()));
        // Resolve the old continuation after cancellation; its lease must reject it.
        host.delay_preview.set(false);
        let (request, inbox) = host.delayed.borrow_mut().pop().unwrap();
        host.request(request, inbox);
        e.update(cx, |s, cx| s.drain(cx));
        assert!(host.state.borrow().runtime.calibration.is_none());
        assert!(e.read_with(cx, |s, _| !s.model.pending && s.calibration.is_none()));
    }
    #[gpui::test]
    fn synthetic_shortcut_capture_cancels_registers_and_retains_binding_on_conflict(
        cx: &mut TestAppContext,
    ) {
        let (e, host, cx) = harness(cx);
        cx.update(|w, cx| {
            e.update(cx, |s, cx| {
                s.fixture_shortcuts = true;
                s.navigate(Page::Shortcuts, None, w, cx);
            })
        });
        let t = tree(cx);
        click(cx, named(&t, "Capture dictation shortcut (synthetic)"));
        keys(cx, "escape");
        assert_eq!(host.state.borrow().settings["doubleTapKey"], "shift_l");
        assert!(e.read_with(cx, |s, _| s.fixture_capture.is_none()));
        let t = tree(cx);
        assert_eq!(t.focus, named(&t, "Capture dictation shortcut (synthetic)"));
        click(cx, named(&t, "Capture dictation shortcut (synthetic)"));
        keys(cx, "f20");
        assert_eq!(host.state.borrow().settings["doubleTapKey"], "f20");
        host.fail_shortcut.set(true);
        let t = tree(cx);
        click(cx, named(&t, "Capture dictation shortcut (synthetic)"));
        keys(cx, "f19");
        assert_eq!(host.state.borrow().settings["doubleTapKey"], "f20");
        let t = tree(cx);
        named(
            &t,
            "Synthetic shortcut registration conflict; previous binding retained",
        );
        host.fail_shortcut.set(false);
        let t = tree(cx);
        click(cx, named(&t, "Capture Paste last shortcut (synthetic)"));
        keys(cx, "cmd-shift-v");
        assert_eq!(
            host.state.borrow().settings["pasteLastShortcut"],
            "command_shift_v"
        );
    }
    #[gpui::test]
    fn all_navigation_rows_fit_the_settings_navigation_surface(cx: &mut TestAppContext) {
        let (_, _, cx) = harness(cx);
        let t = tree(cx);
        let nav = named(&t, "Settings pages");
        let diagnostics = named(&t, "Diagnostics");
        let nav = t
            .nodes
            .iter()
            .find(|(id, _)| *id == nav)
            .unwrap()
            .1
            .bounds()
            .unwrap();
        let diagnostics = t
            .nodes
            .iter()
            .find(|(id, _)| *id == diagnostics)
            .unwrap()
            .1
            .bounds()
            .unwrap();
        assert!(
            diagnostics.y1 <= nav.y1,
            "Tools navigation must not be clipped inside the shared picker height cap"
        );
    }
    #[gpui::test]
    fn assistive_range_increment_changes_the_persisted_value(cx: &mut TestAppContext) {
        let (e, host, cx) = harness(cx);
        cx.update(|w, cx| e.update(cx, |s, cx| s.navigate(Page::Recording, None, w, cx)));
        let t = tree(cx);
        let id = named(&t, "Voice detection");
        let w = cx.update(|w, _| w.window_handle());
        cx.dispatch_accessibility_action(
            w,
            ActionRequest {
                action: AccessibleAction::Increment,
                target_tree: TreeId::ROOT,
                target_node: id,
                data: None,
            },
        );
        draw(cx);
        assert_eq!(host.state.borrow().settings["vadSensitivity"], 55);
    }
    #[gpui::test]
    fn every_pane_preference_has_a_named_native_control(cx: &mut TestAppContext) {
        let (e, host, cx) = harness(cx);
        let mut snapshot = host.state.borrow().clone();
        snapshot.revision += 1;
        for key in [
            "autoPaste",
            "cleanupEnabled",
            "correctionEnabled",
            "captureStandbyEnabled",
            "soundCuesEnabled",
        ] {
            snapshot.settings[key] = json!(true);
        }
        snapshot.settings["recordingMode"] = json!("both");
        snapshot.settings["autoStopSilenceMs"] = json!(1500);
        snapshot.runtime.notchpill = Some(true);
        for page in Page::NAV.into_iter().chain([Page::Transcription]) {
            snapshot.disclosure["panes"][page.id()] = json!(true);
        }
        *host.state.borrow_mut() = snapshot.clone();
        e.update(cx, |s, cx| {
            s.receive(
                Event::Changed {
                    attachment: s.model.attachment,
                    snapshot,
                },
                cx,
            )
        });
        for page in Page::NAV.into_iter().chain([Page::Transcription]) {
            cx.update(|w, cx| e.update(cx, |s, cx| s.navigate(page, None, w, cx)));
            let t = tree(cx);
            for row in ROWS
                .iter()
                .filter(|r| r.page == page && r.key != "outputDir")
            {
                if matches!(row.kind, Kind::Folder) {
                    continue;
                }
                let node = named(&t, row.label);
                let role = t.nodes.iter().find(|(id, _)| *id == node).unwrap().1.role();
                assert_eq!(
                    role,
                    match row.kind {
                        Kind::Toggle => gpui::accesskit::Role::Switch,
                        Kind::Choice(_) => gpui::accesskit::Role::ComboBox,
                        Kind::Number(_, _) => gpui::accesskit::Role::TextInput,
                        Kind::Range(_, _, _, _) => gpui::accesskit::Role::Slider,
                        _ => unreachable!(),
                    },
                    "{}",
                    row.key
                );
            }
        }
    }
    #[gpui::test]
    fn compact_rail_resize_restores_navigation_focus_and_retains_names(cx: &mut TestAppContext) {
        let (e, _, cx) = harness(cx);
        cx.update(|w, cx| {
            e.update(cx, |s, cx| {
                w.focus(&s.navigation.read(cx).focus_handle(cx), cx)
            });
        });
        cx.simulate_resize(gpui::size(gpui::px(720.), gpui::px(560.)));
        let t = tree(cx);
        for page in Page::NAV {
            named(&t, page.label());
        }
        assert!(cx.update(|w, cx| e.read(cx).focus["nav-general"].is_focused(w)));
        cx.simulate_resize(gpui::size(gpui::px(1120.), gpui::px(820.)));
        draw(cx);
        assert!(cx.update(|w, cx| {
            e.read(cx)
                .navigation
                .read(cx)
                .focus_handle(cx)
                .is_focused(w)
        }));
    }
}

#[gpui::test]
fn real_tab_order_and_recording_fields_are_keyboard_reachable(cx: &mut TestAppContext) {
    let (e, _, cx) = harness(cx);
    cx.update(|w, cx| e.update(cx, |s, cx| w.focus(&s.search.read(cx).focus_handle(cx), cx)));
    keys(cx, "tab");
    draw(cx);
    assert!(cx.update(|w, cx| {
        e.read(cx)
            .navigation
            .read(cx)
            .focus_handle(cx)
            .is_focused(w)
    }));
    keys(cx, "tab");
    draw(cx);
    assert!(cx.update(|w, cx| e.read(cx).focus["launchAtLogin"].is_focused(w)));
    keys(cx, "space");
    draw(cx);
    assert_eq!(
        e.read_with(cx, |s, _| s.model.settings()["launchAtLogin"].clone()),
        true
    );
    cx.update(|w, cx| {
        e.update(cx, |s, cx| {
            s.navigate(Page::Recording, None, w, cx);
            w.focus(&s.navigation.read(cx).focus_handle(cx), cx);
        })
    });
    draw(cx);
    keys(cx, "tab");
    draw(cx);
    assert!(cx.update(|w, cx| {
        e.read(cx).pickers["microphone"]
            .read(cx)
            .focus_handle(cx)
            .is_focused(w)
    }));
}
#[gpui::test]
fn calibration_save_cancel_and_destructive_confirmation_use_explicit_actions(
    cx: &mut TestAppContext,
) {
    let (e, host, cx) = harness(cx);
    e.update(cx, |s, cx| {
        s.calibration = Some(4);
        s.action(
            Action::Calibration {
                offset: 4,
                finish: false,
            },
            false,
            cx,
        );
    });
    draw(cx);
    assert_eq!(host.state.borrow().settings["overlayVerticalOffset"], 0);
    e.update(cx, |s, cx| {
        s.action(
            Action::Calibration {
                offset: 4,
                finish: true,
            },
            false,
            cx,
        )
    });
    draw(cx);
    assert_eq!(host.state.borrow().settings["overlayVerticalOffset"], 4);
    assert!(e.read_with(cx, |s, _| s.calibration.is_none()));
    e.update(cx, |s, cx| {
        s.calibration = Some(-2);
        s.action(Action::CancelCalibration, false, cx);
    });
    draw(cx);
    assert_eq!(host.state.borrow().settings["overlayVerticalOffset"], 4);
    e.update(cx, |s, cx| {
        s.action(
            Action::Calibration {
                offset: 0,
                finish: true,
            },
            false,
            cx,
        )
    });
    assert_eq!(host.state.borrow().settings["overlayVerticalOffset"], 0);
    host.state.borrow_mut().runtime.calibration = Some(super::model::Calibration {
        requested: 0,
        applied: 0,
        min: -2,
        max: 2,
    });
    e.update(cx, |s, cx| {
        s.calibration = Some(4);
        s.action(
            Action::Calibration {
                offset: 4,
                finish: false,
            },
            false,
            cx,
        );
    });
    draw(cx);
    assert_eq!(
        e.read_with(cx, |s, _| s.model.runtime().calibration.unwrap().applied),
        2
    );
    e.update(cx, |s, cx| s.action(Action::CancelCalibration, false, cx));
    let before = host.actions.borrow().len();
    e.update(cx, |s, cx| s.action(Action::ResetStatistics, true, cx));
    assert_eq!(host.actions.borrow().len(), before);
    e.update(cx, |s, cx| s.action(Action::ResetStatistics, true, cx));
    assert_eq!(host.actions.borrow().len(), before + 1);
}

#[gpui::test]
fn range_keys_commit_immediately_and_pointer_draft_flushes_once_on_leave(cx: &mut TestAppContext) {
    let (e, host, cx) = harness(cx);
    cx.update(|w, cx| {
        e.update(cx, |s, cx| {
            s.navigate(Page::Recording, None, w, cx);
            w.focus(&s.ranges["vadSensitivity"].read(cx).focus_handle(cx), cx);
        })
    });
    draw(cx);
    keys(cx, "right");
    assert_eq!(host.state.borrow().settings["vadSensitivity"], 55);
    keys(cx, "home");
    assert_eq!(host.state.borrow().settings["vadSensitivity"], 0);
    keys(cx, "end");
    assert_eq!(host.state.borrow().settings["vadSensitivity"], 100);
    cx.update(|w, cx| {
        e.update(cx, |s, cx| {
            let range = s.ranges["vadSensitivity"].clone();
            range.update(cx, |r, cx| {
                r.state.update(cx, |slider, cx| {
                    slider.update_value_by_position(
                        gpui::Axis::Horizontal,
                        gpui::point(gpui::px(100.), gpui::px(100.)),
                        false,
                        w,
                        cx,
                    )
                })
            });
        })
    });
    draw(cx);
    let draft = e.read_with(cx, |s, cx| s.ranges["vadSensitivity"].read(cx).value(cx));
    let before = host.calls.borrow().len();
    cx.update(|w, cx| e.update(cx, |s, cx| s.navigate(Page::General, None, w, cx)));
    draw(cx);
    assert_eq!(host.state.borrow().settings["vadSensitivity"], draft);
    assert!(host.calls.borrow().len() <= before + 1);
}

#[gpui::test]
fn runtime_notifications_preserve_open_picker_and_numeric_drafts(cx: &mut TestAppContext) {
    let (e, host, cx) = harness(cx);
    cx.update(|w, cx| {
        e.update(cx, |s, cx| {
            s.navigate(Page::Recording, None, w, cx);
            w.focus(&s.pickers["recordingMode"].read(cx).focus_handle(cx), cx);
        })
    });
    draw(cx);
    keys(cx, "space down");
    let mut snapshot = host.state.borrow().clone();
    snapshot.revision += 1;
    snapshot.runtime.meter = Some((0.2, 0.3));
    *host.state.borrow_mut() = snapshot.clone();
    e.update(cx, |s, cx| {
        s.receive(
            Event::Changed {
                attachment: s.model.attachment,
                snapshot,
            },
            cx,
        )
    });
    draw(cx);
    keys(cx, "enter");
    assert_eq!(host.state.borrow().settings["recordingMode"], "double_tap");
    cx.update(|w, cx| {
        e.update(cx, |s, cx| {
            s.navigate(Page::Meetings, None, w, cx);
            s.numbers["meetingMaxSessions"].update(cx, |f, cx| f.set_value("72", w, cx));
        })
    });
    let mut snapshot = host.state.borrow().clone();
    snapshot.revision += 1;
    *host.state.borrow_mut() = snapshot.clone();
    e.update(cx, |s, cx| {
        s.receive(
            Event::Changed {
                attachment: s.model.attachment,
                snapshot,
            },
            cx,
        )
    });
    draw(cx);
    assert_eq!(
        e.read_with(cx, |s, cx| s.numbers["meetingMaxSessions"]
            .read(cx)
            .value()
            .to_string()),
        "72"
    );
}

#[test]
fn regression_enabled_smart_stop_can_be_disabled_without_dependencies() {
    use super::model::Install;
    let row = *ROWS
        .iter()
        .find(|r| r.key == "smartAutoStopEnabled")
        .unwrap();
    for install in [
        Install::Missing,
        Install::Unknown,
        Install::Busy,
        Install::Failed,
        Install::Installed,
    ] {
        for silence in [0, 1500] {
            let mut m = Model::default();
            let mut snapshot = fixtures::snapshot();
            snapshot.runtime.smart_turn = install;
            snapshot.settings["autoStopSilenceMs"] = json!(silence);
            snapshot.settings["smartAutoStopEnabled"] = json!(true);
            m.snapshot = Some(snapshot);
            assert!(!m.disabled(row), "must allow disabling enabled preference");
            m.snapshot.as_mut().unwrap().settings["smartAutoStopEnabled"] = json!(false);
            assert_eq!(
                m.disabled(row),
                silence == 0 || install != Install::Installed
            );
            m.snapshot.as_mut().unwrap().runtime.recording = true;
            assert!(m.disabled(row), "capture lock remains authoritative");
        }
    }
}

#[test]
fn live_runtime_updates_keep_an_admitted_preference_ticket_current() {
    let mut model = Model::default();
    model.attach();
    assert!(model.event(Event::Changed {
        attachment: model.attachment,
        snapshot: fixtures::snapshot()
    }));
    let ticket = model.begin().unwrap();
    let mut runtime = model.runtime();
    runtime.meter = Some((0.1, 0.2));
    runtime.login_requires_approval = true;
    assert!(model.event(Event::Runtime {
        attachment: model.attachment,
        runtime: Box::new(runtime)
    }));
    assert!(model.pending);
    assert_eq!(model.ticket(), ticket);
    assert!(model.event(Event::Reply {
        ticket,
        result: Ok(fixtures::snapshot())
    }));
    let mut bad = model.runtime();
    bad.meter = Some((f32::NAN, 0.));
    assert!(!model.event(Event::Runtime {
        attachment: model.attachment,
        runtime: Box::new(bad)
    }));
    model.detach();
    assert!(!model.event(Event::Runtime {
        attachment: ticket.attachment,
        runtime: Box::default()
    }));
}
