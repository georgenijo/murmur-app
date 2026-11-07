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
    let react = include_str!("../../../src/components/settings/SettingsPanel.tsx");
    for page in Page::NAV {
        assert!(react.contains(&format!("id: '{}'", page.id())));
    }
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
