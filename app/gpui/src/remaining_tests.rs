//! Real GPUI interaction/AX trees over synthetic service owners only.
use crate::remaining::Review;
use gpui::{
    Entity, KeyDownEvent, KeyUpEvent, Keystroke, TestAppContext, VisualTestContext, prelude::*,
};
fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|w, cx| w.draw(cx).clear(cx));
}
fn harness(cx: &mut TestAppContext, index: usize) -> (Entity<Review>, &mut VisualTestContext) {
    harness_options(cx, index, crate::remaining::QaOptions::default())
}
fn harness_options(
    cx: &mut TestAppContext,
    index: usize,
    options: crate::remaining::QaOptions,
) -> (Entity<Review>, &mut VisualTestContext) {
    cx.update(crate::init);
    let mut entity = None;
    let (_, cx) = cx.add_window_view(|w, cx| {
        let e = cx.new(|cx| Review::with_options(options, w, cx));
        entity = Some(e.clone());
        gpui_base::Root::new(e, w, cx)
    });
    let e = entity.unwrap();
    cx.update(|w, cx| e.update(cx, |s, cx| s.select(index, w, cx)));
    draw(cx);
    (e, cx)
}
fn key(cx: &mut VisualTestContext, key: &str) {
    let k = cx.update(|_, cx| Keystroke::parse_with_os(key, cx.platform_info().os).unwrap());
    cx.simulate_event(KeyDownEvent {
        keystroke: k.clone(),
        is_held: false,
        prefer_character_input: false,
    });
    cx.simulate_event(KeyUpEvent { keystroke: k });
    draw(cx);
}
#[gpui::test]
fn palette_input_ranking_wrapping_escape_and_reopen(cx: &mut TestAppContext) {
    let (e, cx) = harness(cx, 0);
    let palette = e.read_with(cx, |s, _| s.palette.clone());
    cx.update(|w, cx| {
        palette.update(cx, |s, cx| {
            s.search
                .update(cx, |f, cx| f.set_value("settings: delivery", w, cx))
        })
    });
    draw(cx);
    key(cx, "down");
    assert_eq!(palette.read_with(cx, |s, _| s.selected), 0);
    key(cx, "escape");
    assert!(!palette.read_with(cx, |s, _| s.open));
    cx.update(|w, cx| palette.update(cx, |s, cx| s.show(w, cx)));
    draw(cx);
    assert_eq!(
        palette.read_with(cx, |s, cx| s.search.read(cx).value().to_string()),
        ""
    );
    key(cx, "up");
    assert!(palette.read_with(cx, |s, _| s.selected) > 0);
    key(cx, "enter");
    assert!(!palette.read_with(cx, |s, _| s.open));
}
#[gpui::test]
fn navigation_revokes_old_inboxes_and_drops_private_projection(cx: &mut TestAppContext) {
    let (e, cx) = harness(cx, 3);
    let old = e.read_with(cx, |s, cx| s.knowledge.read(cx).controller.inbox.clone());
    cx.update(|w, cx| e.update(cx, |s, cx| s.select(4, w, cx)));
    draw(cx);
    assert!(old.is_closed());
    assert!(e.read_with(cx, |s, cx| {
        s.knowledge.read(cx).controller.model.snapshot.is_none()
    }));
}
#[gpui::test]
fn stale_reply_cannot_replace_reopened_section(cx: &mut TestAppContext) {
    let (e, cx) = harness(cx, 2);
    let models = e.read_with(cx, |s, _| s.models.clone());
    let (ticket, inbox) = models.read_with(cx, |s, _| {
        (s.controller.model.ticket(), s.controller.inbox.clone())
    });
    cx.update(|w, cx| {
        e.update(cx, |s, cx| {
            s.select(1, w, cx);
            s.select(2, w, cx);
        })
    });
    draw(cx);
    assert!(
        inbox
            .publish(crate::workspace::Event::Reply {
                ticket,
                result: Ok(crate::models::fixtures::snapshot())
            })
            .is_err()
    );
    let attachment = models.read_with(cx, |s, _| s.controller.model.attachment);
    assert!(attachment > ticket.attachment);
}
#[cfg(feature = "a11y-tests")]
mod ax {
    use super::*;
    use gpui::Focusable;
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
            .unwrap_or_else(|| panic!("missing AX label: {label}"))
            .0
    }
    fn click(cx: &mut VisualTestContext, id: NodeId) {
        let w = cx.update(|w, _| w.window_handle());
        cx.dispatch_accessibility_action(
            w,
            ActionRequest {
                action: gpui::accesskit::Action::Click,
                target_node: id,
                target_tree: TreeId::ROOT,
                data: None,
            },
        );
        draw(cx);
    }
    fn focus(cx: &mut VisualTestContext, label: &str) {
        let t = tree(cx);
        let w = cx.update(|w, _| w.window_handle());
        cx.dispatch_accessibility_action(
            w,
            ActionRequest {
                action: gpui::accesskit::Action::Focus,
                target_node: named(&t, label),
                target_tree: TreeId::ROOT,
                data: None,
            },
        );
        draw(cx);
    }
    fn activate(cx: &mut VisualTestContext, label: &str) {
        focus(cx, label);
        let t = tree(cx);
        click(cx, named(&t, label));
    }
    fn static_text(t: &TreeUpdate, expected: &str) {
        assert_eq!(
            t.nodes
                .iter()
                .filter(|(_, n)| n.role() == gpui::Role::Label && n.value() == Some(expected))
                .count(),
            1,
            "missing or duplicated prose: {expected}"
        );
    }
    #[gpui::test]
    fn remaining_sections_expose_visible_prose_once_in_accesskit(cx: &mut TestAppContext) {
        let (e, cx) = harness(cx, 1);
        let t = tree(cx);
        for (label, value, detail) in crate::insights::fixtures::snapshot().totals() {
            static_text(&t, label);
            let group = t
                .nodes
                .iter()
                .find(|(_, n)| n.role() == gpui::accesskit::Role::Group && n.label() == Some(label))
                .unwrap();
            assert!(
                group.1.children().iter().any(|id| t
                    .nodes
                    .iter()
                    .any(|(i, n)| i == id && n.value() == Some(value.as_str()))),
                "{label} retains its independently readable value"
            );
            assert!(group.1.children().iter().any(|id| {
                t.nodes
                    .iter()
                    .any(|(i, n)| i == id && n.value() == Some(detail.as_str()))
            }));
        }
        let totals = t
            .nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Usage totals"))
            .unwrap();
        assert!(!totals.1.children().is_empty());
        for (index, prose) in [
            (2, "Synthetic Apple Silicon · 16 GB"),
            (3, "Synthetic replacement 0"),
            (
                4,
                "Reads only the current URL from an allowed browser, immediately discards everything except its exact host, and never reads page text or history. A host is saved only when you add a rule.",
            ),
            (5, "Sonic"),
            (0, "↑↓ navigate · Enter run · Esc close"),
        ] {
            cx.update(|w, cx| e.update(cx, |s, cx| s.select(index, w, cx)));
            let t = tree(cx);
            if index == 5 {
                named(&t, "Use Sonic theme");
            } else {
                static_text(&t, prose);
            }
            if index == 2 {
                static_text(&t, "Synthetic size · Local transcription · Mac · Installed");
                static_text(
                    &t,
                    "Synthetic size · Local transcription · Mac · Not installed · Selected",
                );
            }
            if index == 3 {
                assert_eq!(
                    t.nodes
                        .iter()
                        .filter(|(_, n)| n.value() == Some("\"global\" · \"manual\" · updated 0"))
                        .count(),
                    3
                );
            }
        }
        for (page, prose) in [
            (
                crate::customize::model::Page::Spellings,
                "For reliable replacement, enter the words Murmur actually hears on the left. Separate variants with commas.",
            ),
            (crate::customize::model::Page::Commands, "Hello {{date}}"),
            (
                crate::customize::model::Page::Apps,
                "Project symbols stay in memory. No screen, selection, or clipboard context is read.",
            ),
            (
                crate::customize::model::Page::Scan,
                "Built-in dev terms active. Scan this folder to add your own identifiers.",
            ),
        ] {
            cx.update(|w, cx| e.update(cx, |s, cx| s.select_customize(page, w, cx)));
            if page == crate::customize::model::Page::Apps {
                draw(cx);
                e.read_with(cx, |s, _| s.customize.clone())
                    .update(cx, |s, cx| {
                        let mut profile = crate::customize::model::profile(
                            "example.synthetic",
                            "Synthetic editor",
                        );
                        profile["ideContextEnabled"] = serde_json::json!(true);
                        s.controller.model.snapshot.as_mut().unwrap().settings["appProfiles"] =
                            serde_json::json!([profile]);
                        cx.notify();
                    });
            }
            static_text(&tree(cx), prose);
        }
    }
    #[gpui::test]
    fn existing_knowledge_editor_starts_on_enabled_title_and_return_saves(cx: &mut TestAppContext) {
        let (_, cx) = harness(cx, 3);
        activate(cx, "Edit Synthetic phrase 0");
        let t = tree(cx);
        let focused = t.nodes.iter().find(|(id, _)| *id == t.focus).unwrap();
        assert!(!focused.1.is_disabled());
        assert_eq!(
            focused.1.label(),
            Some("Heard phrase / written term / trigger / name")
        );
        key(cx, "enter");
        assert!(
            !tree(cx)
                .nodes
                .iter()
                .any(|(_, n)| n.label() == Some("Edit knowledge"))
        );
    }
    #[gpui::test]
    fn partial_bulk_fixture_presents_recovery_and_failed_ids(cx: &mut TestAppContext) {
        let options = crate::remaining::QaOptions::parse("partial-bulk", "", "", "").unwrap();
        let (_, cx) = harness_options(cx, 3, options);
        static_text(&tree(cx), "Recovered from a local backup.");
        activate(cx, "Disable matches");
        static_text(
            &tree(cx),
            "2 succeeded · 0 unchanged · 1 failed · Failed IDs: synthetic-1. Review these records before retrying.",
        );
    }
    #[gpui::test]
    fn knowledge_import_fixtures_present_conflict_without_success(cx: &mut TestAppContext) {
        for parameter in ["stale-import", "conflicting-import"] {
            let options = crate::remaining::QaOptions::parse(parameter, "", "", "").unwrap();
            let (_, cx) = harness_options(cx, 3, options);
            activate(cx, "Import…");
            let t = tree(cx);
            named(&t, "Import knowledge preview");
            activate(cx, "Confirm");
            static_text(&tree(cx), crate::workspace::Problem::Conflict.message());
            assert!(
                !tree(cx)
                    .nodes
                    .iter()
                    .any(|(_, n)| n.value() == Some("Knowledge imported."))
            );
        }
    }
    #[gpui::test]
    fn spelling_import_fixtures_disable_unsafe_confirmation(cx: &mut TestAppContext) {
        for parameter in ["stale-import", "conflicting-import"] {
            let options = crate::remaining::QaOptions::parse("", parameter, "", "").unwrap();
            let (e, cx) = harness_options(cx, 3, options);
            cx.update(|w, cx| {
                e.update(cx, |s, cx| {
                    s.select_customize(crate::customize::model::Page::Spellings, w, cx)
                })
            });
            activate(cx, "Import saved spellings…");
            let t = tree(cx);
            let confirm = named(&t, "Confirm spelling import");
            assert!(
                t.nodes
                    .iter()
                    .find(|(id, _)| *id == confirm)
                    .unwrap()
                    .1
                    .is_disabled()
            );
        }
    }
    #[gpui::test]
    fn mode_file_fixture_reports_full_import_and_export_completion(cx: &mut TestAppContext) {
        let options = crate::remaining::QaOptions::parse("", "", "1", "").unwrap();
        let (_, cx) = harness_options(cx, 4, options);
        activate(cx, "Import modes…");
        static_text(
            &tree(cx),
            "1 new modes · 0 exact duplicates · 0 missing apps · 0 conflicts",
        );
        activate(cx, "Confirm import");
        static_text(&tree(cx), "Modes imported from the synthetic file.");
        activate(cx, "Export custom modes…");
        static_text(&tree(cx), "Modes exported to synthetic sink · 1 saved");
    }
    #[gpui::test]
    fn regression_palette_active_descendant_tracks_search_selection(cx: &mut TestAppContext) {
        let (e, cx) = harness(cx, 0);
        let assert_selected = |cx: &mut VisualTestContext| {
            let t = tree(cx);
            let (id, _) = t
                .nodes
                .iter()
                .find(|(_, n)| {
                    n.role() == gpui::Role::ListBoxOption && n.is_selected() == Some(true)
                })
                .unwrap();
            assert_eq!(t.focus, *id, "AX focus must follow the selected command");
            let search = e.read_with(cx, |s, cx| {
                s.palette.read(cx).search.read(cx).focus_handle(cx)
            });
            cx.update(|w, _| assert!(search.is_focused(w), "typing stays in search"));
        };
        assert_selected(cx);
        key(cx, "down");
        assert_selected(cx);
        key(cx, "ctrl-n");
        assert_selected(cx);
        // An unfocused palette must never claim another control's AX focus.
        focus(cx, "Review Themes");
        let t = tree(cx);
        assert_eq!(t.focus, named(&t, "Review Themes"));
    }
    #[gpui::test]
    fn regression_knowledge_aliases_survive_unrelated_edit(cx: &mut TestAppContext) {
        use serde_json::json;
        let (e, cx) = harness(cx, 3);
        let knowledge = e.read_with(cx, |s, _| s.knowledge.clone());
        knowledge.update(cx, |s, cx| {
            let record = &mut s.controller.model.snapshot.as_mut().unwrap().records[0];
            record["payload"] = json!({"kind":"vocabulary_term","written":"Tori","aliases":["Tori","Tory","Tor, ee"]});
            cx.notify();
        });
        draw(cx);
        activate(cx, "Edit Tori");
        let editor = knowledge.read_with(cx, |s, _| s.editor.clone());
        cx.update(|w, cx| {
            editor.update(cx, |f, cx| {
                let mut values = f.values.clone();
                values["title"] = json!("Tori updated");
                f.set(values, w, cx);
            })
        });
        draw(cx);
        activate(cx, "Save");
        let aliases = knowledge.read_with(cx, |s, _| {
            s.controller
                .model
                .snapshot
                .as_ref()
                .unwrap()
                .records
                .iter()
                .find(|v| v["id"] == "synthetic-0")
                .unwrap()["payload"]["aliases"]
                .clone()
        });
        assert_eq!(aliases, json!(["Tori", "Tory", "Tor, ee"]));
    }
    #[gpui::test]
    fn regression_theme_editor_rebases_rendered_colors(cx: &mut TestAppContext) {
        use crate::themes::{compile, library};
        use serde_json::json;
        let (e, cx) = harness(cx, 5);
        let themes = e.read_with(cx, |s, _| s.themes.clone());
        for id in ["opencode", "synthetic-orchid"] {
            themes.update(cx, |s, cx| {
                let snapshot = s.controller.model.snapshot.as_mut().unwrap();
                snapshot.appearance =
                    library::selection(&snapshot.appearance, &snapshot.library, id, Some("dark"))
                        .unwrap();
                cx.notify();
            });
            draw(cx);
            let old = themes.read_with(cx, |s, _| {
                compile::value(&compile::resolve(
                    &s.controller.model.snapshot.as_ref().unwrap().appearance["theme"],
                    true,
                ))
            });
            activate(cx, "Create theme from Sonic");
            let editor = themes.read_with(cx, |s, _| s.editor.clone());
            assert_eq!(
                editor.read_with(cx, |f, _| f.value("background").to_string()),
                old["background"].as_str().unwrap()
            );
            cx.update(|w, cx| {
                editor.update(cx, |f, cx| {
                    let mut values = f.values.clone();
                    values["accent"] = json!("#ee7733");
                    f.set(values, w, cx);
                })
            });
            draw(cx);
            activate(cx, "Save theme");
            let saved = themes.read_with(cx, |s, _| {
                s.controller.model.snapshot.as_ref().unwrap().appearance["theme"].clone()
            });
            assert!(saved.get("light").is_none() && saved.get("dark").is_none());
            assert_eq!(saved["accent"], "#ee7733");
            let t = tree(cx);
            assert!(
                t.nodes
                    .iter()
                    .any(|(_, n)| n.label() == Some("Use Custom theme"))
            );
            activate(cx, "Edit colors…");
            assert_eq!(
                editor.read_with(cx, |f, _| f.value("accent").to_string()),
                "#ee7733"
            );
            activate(cx, "Export current…");
            assert!(themes.read_with(cx, |s, _| {
                s.controller
                    .model
                    .snapshot
                    .as_ref()
                    .unwrap()
                    .notice
                    .as_ref()
                    .unwrap()
                    .contains("exported current")
            }));
            // Export changes the section reply revision, not the appearance being edited.
            activate(cx, "Save theme");
            activate(cx, "Edit colors…");
            activate(cx, "Reset to Sonic");
            assert_eq!(
                themes.read_with(cx, |s, _| s
                    .controller
                    .model
                    .snapshot
                    .as_ref()
                    .unwrap()
                    .appearance["selection"]
                    .clone()),
                json!({"light":"sonic","dark":"sonic"})
            );

            assert_ne!(
                compile::value(&compile::resolve(&saved, true))["primary"],
                old["primary"]
            );
        }
    }
    #[gpui::test]
    fn regression_theme_card_actions_match_react(cx: &mut TestAppContext) {
        let (_, cx) = harness(cx, 5);
        let t = tree(cx);
        let labels: Vec<_> = t.nodes.iter().filter_map(|(_, n)| n.label()).collect();
        assert!(labels.contains(&"Create theme from Sonic"));
        assert!(!labels.contains(&"Duplicate Sonic"));
        assert!(!labels.contains(&"Export Sonic…"));
        assert!(!labels.contains(&"Edit colors…"));
        let export = labels
            .iter()
            .position(|n| *n == "Export OpenCode…")
            .unwrap();
        let duplicate = labels
            .iter()
            .position(|n| *n == "Duplicate OpenCode")
            .unwrap();
        assert!(export < duplicate);
        assert!(labels.contains(&"Sonic dark variant"));
        activate(cx, "Create theme from Sonic");
        let t = tree(cx);
        assert!(
            t.nodes
                .iter()
                .any(|(_, n)| n.label() == Some("Export current…"))
        );
        assert!(
            t.nodes
                .iter()
                .any(|(_, n)| n.label() == Some("Reset to Sonic"))
        );
    }
    #[gpui::test]
    fn regression_theme_card_keeps_source_action_for_community_themes(cx: &mut TestAppContext) {
        let (e, cx) = harness(cx, 5);
        let themes = e.read_with(cx, |s, _| s.themes.clone());
        let labels = |cx: &mut gpui::VisualTestContext| {
            tree(cx)
                .nodes
                .iter()
                .filter_map(|(_, n)| n.label().map(str::to_string))
                .collect::<Vec<_>>()
        };
        assert!(!labels(cx).iter().any(|n| n.ends_with(" source…")));
        themes.update(cx, |s, cx| {
            let library = &mut s.controller.model.snapshot.as_mut().unwrap().library;
            let entry = library["themes"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|v| v["id"] == "synthetic-orchid")
                .unwrap();
            entry["source"] = serde_json::json!({
                "kind": "open-vsx",
                "sourceUrl": "https://example.com/sample/orchid"
            });
            cx.notify();
        });
        draw(cx);
        assert!(labels(cx).contains(&"Open Synthetic Orchid source…".to_string()));
    }
    #[gpui::test]
    fn regression_import_theme_focuses_visible_first_action(cx: &mut TestAppContext) {
        let (_, cx) = harness(cx, 5);
        activate(cx, "Import theme…");
        let t = tree(cx);
        assert_eq!(t.focus, named(&t, "Choose local file…"));
    }
    #[gpui::test]
    fn regression_mode_cancel_and_escape_restore_opener(cx: &mut TestAppContext) {
        let (_, cx) = harness(cx, 4);
        activate(cx, "Create mode");
        for cancel in [true, false] {
            activate(cx, "Edit mode…");
            if cancel {
                activate(cx, "Cancel edit");
            } else {
                key(cx, "escape");
            }
            let t = tree(cx);
            assert!(!t.nodes.iter().any(|(_, n)| n.label() == Some("Mode name")));
            assert_eq!(t.focus, named(&t, "Edit mode…"));
        }
    }
    #[gpui::test]
    fn regression_reopen_after_async_save_focuses_editor(cx: &mut TestAppContext) {
        use serde_json::json;
        let (e, cx) = harness(cx, 3);
        activate(cx, "Review App overrides");
        let custom = e.read_with(cx, |s, _| s.customize.clone());
        custom.update(cx, |s, cx| {
            s.controller.model.snapshot.as_mut().unwrap().settings["appProfiles"] =
                json!([crate::customize::model::profile(
                    "example.synthetic",
                    "Synthetic app"
                )]);
            cx.notify();
        });
        draw(cx);
        activate(cx, "Edit app overrides…");
        activate(cx, "Save");
        draw(cx);
        // An asynchronous save closes the editor while the render-synced
        // disabled flag is still set from its pending frame.
        custom.update(cx, |s, cx| {
            s.editor.update(cx, |f, _| f.disabled = true);
            cx.notify();
        });
        draw(cx);
        activate(cx, "Edit app overrides…");
        let t = tree(cx);
        assert_eq!(t.focus, named(&t, "App label"));
    }
    #[gpui::test]
    fn regression_app_profile_bundle_id_is_disabled_and_immutable(cx: &mut TestAppContext) {
        use serde_json::json;
        let (e, cx) = harness(cx, 3);
        activate(cx, "Review App overrides");
        let custom = e.read_with(cx, |s, _| s.customize.clone());
        custom.update(cx, |s, cx| {
            s.controller.model.snapshot.as_mut().unwrap().settings["appProfiles"] =
                json!([crate::customize::model::profile(
                    "example.synthetic",
                    "Synthetic app"
                )]);
            cx.notify();
        });
        draw(cx);
        activate(cx, "Edit app overrides…");
        let t = tree(cx);
        assert_eq!(
            t.focus,
            named(&t, "App label"),
            "existing overrides skip immutable bundle ID"
        );
        let id = named(&t, "App bundle ID");
        assert!(
            t.nodes
                .iter()
                .find(|(i, _)| *i == id)
                .unwrap()
                .1
                .is_disabled()
        );
        focus(cx, "App bundle ID");
        key(cx, "cmd-a");
        cx.simulate_input("example.changed");
        draw(cx);
        assert_eq!(
            custom.read_with(cx, |s, cx| s.editor.read(cx).value("bundleId").to_string()),
            "example.synthetic"
        );
        activate(cx, "Save");
        assert_eq!(
            custom.read_with(cx, |s, _| s
                .controller
                .model
                .snapshot
                .as_ref()
                .unwrap()
                .settings["appProfiles"]
                .as_array()
                .unwrap()
                .len()),
            1
        );
    }
    fn assert_visible_input(cx: &mut VisualTestContext, label: &str, height: f64) {
        let t = tree(cx);
        let node = &t
            .nodes
            .iter()
            .find(|(_, n)| n.label() == Some(label) && n.role() == gpui::Role::TextInput)
            .unwrap()
            .1;
        let bounds = node.bounds().expect("input has AX bounds");
        let top = t
            .nodes
            .iter()
            .find(|(_, n)| n.role() == gpui::Role::Navigation)
            .unwrap()
            .1
            .bounds()
            .unwrap()
            .y1;
        let scale = cx.update(|w, _| f64::from(w.scale_factor()));
        assert!(
            bounds.y0 >= top && bounds.y1 <= height * scale,
            "focused {label} must be visible below navigation ({top}), got {bounds:?}"
        );
    }
    #[gpui::test]
    fn regression_scroll_forms_have_focus_reveal_parents(cx: &mut TestAppContext) {
        let (_, cx) = harness(cx, 3);
        cx.simulate_resize(gpui::size(gpui::px(880.), gpui::px(200.)));
        activate(cx, "Delete all…");
        focus(cx, "Cancel");
        focus(cx, "Type DELETE to confirm");
        assert_visible_input(cx, "Type DELETE to confirm", 200.);
    }
    #[gpui::test]
    fn regression_voice_search_has_focus_reveal_parent(cx: &mut TestAppContext) {
        let (e, cx) = harness(cx, 3);
        cx.simulate_resize(gpui::size(gpui::px(880.), gpui::px(250.)));
        activate(cx, "Review Voice commands");
        focus(cx, "Delete command…");
        let search = e.read_with(cx, |s, cx| s.customize.read(cx).search.clone());
        cx.update(|w, cx| search.update(cx, |f, cx| f.focus_first(w, cx)));
        draw(cx);
        assert_visible_input(cx, "Search commands or terms", 250.);
    }
    #[gpui::test]
    fn regression_knowledge_return_saves_only_edit_dialog_defaults(cx: &mut TestAppContext) {
        use serde_json::json;
        let (e, cx) = harness(cx, 3);
        let knowledge = e.read_with(cx, |s, _| s.knowledge.clone());
        for target in ["Heard phrase / written term / trigger / name", "Save"] {
            activate(cx, "Create knowledge…");
            let editor = knowledge.read_with(cx, |s, _| s.editor.clone());
            cx.update(|w, cx| editor.update(cx, |f, cx| f.set(json!({"kind":"replacement_rule","title":"Synthetic default","detail":"Synthetic replacement","scope":"global","enabled":"true"}), w, cx)));
            draw(cx);
            focus(cx, target);
            key(cx, "enter");
            let t = tree(cx);
            assert!(
                !t.nodes.iter().any(|(_, n)| n.label() == Some("Save")),
                "Return saves valid draft from {target}"
            );
        }
        activate(cx, "Delete all…");
        let phrase = knowledge.read_with(cx, |s, _| s.phrase.clone());
        cx.update(|w, cx| phrase.update(cx, |f, cx| f.set(json!({"phrase":"DELETE"}), w, cx)));
        draw(cx);
        focus(cx, "Confirm");
        key(cx, "enter");
        assert!(knowledge.read_with(
            cx,
            |s, _| s.controller.model.snapshot.as_ref().unwrap().total > 0
        ));
    }
    #[gpui::test]
    fn knowledge_default_preserves_editing_and_ignores_modified_repeated_return(
        cx: &mut TestAppContext,
    ) {
        use serde_json::json;
        let (e, cx) = harness(cx, 3);
        activate(cx, "Create knowledge…");
        let editor = e.read_with(cx, |s, cx| s.knowledge.read(cx).editor.clone());
        cx.update(|w, cx| editor.update(cx, |f, cx| f.set(json!({"kind":"replacement_rule","title":"Synthetic","detail":"Replacement","scope":"global","enabled":"true"}), w, cx)));
        draw(cx);
        for target in [
            "Replacement / aliases (one per line) / body / instruction",
            "Scope",
            "Cancel",
        ] {
            focus(cx, target);
            key(cx, "enter");
            let t = tree(cx);
            named(&t, "Save");
            if target == "Scope" {
                key(cx, "enter");
            }
        }
        focus(cx, "Heard phrase / written term / trigger / name");
        key(cx, "shift-enter");
        key(cx, "cmd-enter");
        for (is_held, prefer_character_input) in [(true, false), (false, true)] {
            let keystroke = cx
                .update(|_, cx| Keystroke::parse_with_os("enter", cx.platform_info().os).unwrap());
            cx.simulate_event(KeyDownEvent {
                keystroke: keystroke.clone(),
                is_held,
                prefer_character_input,
            });
            cx.simulate_event(KeyUpEvent { keystroke });
            draw(cx);
            named(&tree(cx), "Save");
        }
    }
    #[gpui::test]
    fn every_section_has_named_navigation_and_main_surface(cx: &mut TestAppContext) {
        let (e, cx) = harness(cx, 0);
        for (index, label, expected) in [
            (0, "Command palette", "Search commands"),
            (1, "Insights", "Usage totals"),
            (2, "Models", "Download Whisper Tiny"),
            (3, "Knowledge", "Search knowledge"),
            (4, "Modes", "Mode test input"),
            (5, "Themes", "Import theme…"),
        ] {
            let t = tree(cx);
            click(cx, named(&t, &format!("Review {label}")));
            let t = tree(cx);
            named(&t, expected);
            assert_eq!(e.read_with(cx, |s, _| s.selected), index);
        }
    }
    /// A Core ML download failure on the Models section, over the fixture
    /// catalog: Whisper base installed, CPU Parakeet missing.
    fn core_ml_failure(cx: &mut VisualTestContext, e: &Entity<Review>, fallbacks: bool) {
        let models = e.read_with(cx, |s, _| s.models.clone());
        models.update(cx, |s, _| {
            s.controller.model.snapshot.as_mut().unwrap().failure =
                Some(crate::models::model::Failure {
                    model: "parakeet-tdt-0.6b-v3-coreml".into(),
                    operation: crate::models::model::Operation::Download,
                    message: "Core ML setup finished, but the installed cache did not validate.",
                    fallbacks,
                })
        });
        draw(cx);
    }
    fn has(t: &TreeUpdate, label: &str) -> bool {
        t.nodes.iter().any(|(_, n)| n.label() == Some(label))
    }
    #[gpui::test]
    fn core_ml_failure_offers_use_for_an_installed_fallback_and_download_for_a_missing_one(
        cx: &mut TestAppContext,
    ) {
        let (e, cx) = harness(cx, 2);
        core_ml_failure(cx, &e, true);
        let t = tree(cx);
        named(&t, "Retry download");
        // Installed: chosen, not downloaded again. Missing: downloaded.
        named(&t, "Use Whisper base");
        named(&t, "Download CPU Parakeet");
        assert!(!has(&t, "Download Whisper base"));
        assert!(!has(&t, "Use CPU Parakeet"));
        let models = e.read_with(cx, |s, _| s.models.clone());
        let state = |cx: &mut VisualTestContext| {
            models.read_with(cx, |s, _| {
                let snapshot = s.controller.model.snapshot.clone().unwrap();
                (
                    snapshot.selected.clone(),
                    snapshot.model("parakeet-tdt-0.6b-v2-fp16").unwrap().install,
                )
            })
        };
        let (selected, install) = state(cx);
        assert_eq!(selected, "parakeet-tdt-0.6b-v3-coreml");
        assert_eq!(install, crate::models::model::Install::Missing);
        click(cx, named(&t, "Use Whisper base"));
        let (selected, install) = state(cx);
        assert_eq!(selected, "base.en", "Use selects the installed fallback");
        assert_eq!(
            install,
            crate::models::model::Install::Missing,
            "Use must not download anything"
        );
        // The fixture owner clears the failure on its reply; keep it showing to
        // check the selected fallback is no longer offered and the missing one is.
        core_ml_failure(cx, &e, true);
        let t = tree(cx);
        assert!(!has(&t, "Use Whisper base"));
        click(cx, named(&t, "Download CPU Parakeet"));
        let (selected, install) = state(cx);
        assert_eq!(
            selected, "base.en",
            "Download does not change the selection"
        );
        assert_eq!(install, crate::models::model::Install::Installed);
    }
    #[gpui::test]
    fn failures_without_fallbacks_offer_none(cx: &mut TestAppContext) {
        let (e, cx) = harness(cx, 2);
        core_ml_failure(cx, &e, false);
        let t = tree(cx);
        named(&t, "Retry download");
        for label in [
            "Use Whisper base",
            "Download Whisper base",
            "Use CPU Parakeet",
            "Download CPU Parakeet",
        ] {
            assert!(!has(&t, label), "{label}");
        }
    }
    #[gpui::test]
    fn knowledge_editor_delete_confirmation_and_cancel(cx: &mut TestAppContext) {
        let (e, cx) = harness(cx, 3);
        let t = tree(cx);
        click(cx, named(&t, "Create knowledge…"));
        let t = tree(cx);
        named(&t, "Knowledge type");
        named(&t, "Scope");
        named(
            &t,
            "Replacement / aliases (one per line) / body / instruction",
        );
        click(cx, named(&t, "Cancel"));
        let t = tree(cx);
        click(cx, named(&t, "Delete all…"));
        let t = tree(cx);
        named(&t, "Type DELETE to confirm");
        click(cx, named(&t, "Confirm"));
        assert_eq!(
            e.read_with(cx, |s, cx| s
                .knowledge
                .read(cx)
                .controller
                .model
                .snapshot
                .as_ref()
                .unwrap()
                .total),
            3
        );
        let t = tree(cx);
        click(cx, named(&t, "Cancel"));
    }
    #[gpui::test]
    fn mode_name_matching_a_settings_row_keeps_both_accessible_text_nodes(cx: &mut TestAppContext) {
        let (e, cx) = harness(cx, 4);
        activate(cx, "Create mode");
        e.update(cx, |s, cx| {
            s.modes.update(cx, |m, cx| {
                m.controller.model.snapshot.as_mut().unwrap().settings["modes"][0]["name"] =
                    serde_json::json!("Custom mode");
                cx.notify();
            })
        });
        let t = tree(cx);
        assert_eq!(
            t.nodes
                .iter()
                .filter(|(_, n)| n.role() == gpui::Role::Label && n.value() == Some("Custom mode"))
                .count(),
            2
        );
    }
    #[gpui::test]
    fn modes_create_edit_policies_and_bindings(cx: &mut TestAppContext) {
        let (e, cx) = harness(cx, 4);
        let t = tree(cx);
        click(cx, named(&t, "Create mode"));
        let t = tree(cx);
        click(cx, named(&t, "Edit mode…"));
        let t = tree(cx);
        named(&t, "Mode name");
        named(&t, "Mode model");
        named(&t, "Vocabulary policy");
        named(&t, "Auto-paste");
        click(cx, named(&t, "Cancel edit"));
        assert_eq!(
            e.read_with(cx, |s, cx| s
                .modes
                .read(cx)
                .controller
                .model
                .snapshot
                .as_ref()
                .unwrap()
                .settings["modes"]
                .as_array()
                .unwrap()
                .len()),
            1
        );
    }
    #[gpui::test]
    fn themes_import_disclosure_preview_and_cancel(cx: &mut TestAppContext) {
        let (_, cx) = harness(cx, 5);
        let t = tree(cx);
        click(cx, named(&t, "Import theme…"));
        let t = tree(cx);
        click(cx, named(&t, "Browse community…"));
        let t = tree(cx);
        named(&t, "Search community themes");
        named(&t, "Search Open VSX");
        click(cx, named(&t, "Cancel"));
        let t = tree(cx);
        click(cx, named(&t, "Import theme…"));
        let t = tree(cx);
        click(cx, named(&t, "Choose local file…"));
        let t = tree(cx);
        named(&t, "Theme import preview");
        click(cx, named(&t, "Cancel"));
        let t = tree(cx);
        named(&t, "Import theme…");
    }
}
#[cfg(feature = "a11y-tests")]
#[gpui::test]
fn customization_sheets_preserve_explicit_clipboard_and_profile_defaults(cx: &mut TestAppContext) {
    use gpui::accesskit::{ActionRequest, TreeId};
    let (e, cx) = harness(cx, 3);
    let click = |cx: &mut VisualTestContext, label: &str| {
        let w = cx.update(|w, _| w.window_handle());
        cx.activate_accessibility(w);
        draw(cx);
        let t = cx.accessibility_tree(w);
        let id = t
            .nodes
            .iter()
            .find(|(_, n)| n.label() == Some(label))
            .unwrap_or_else(|| panic!("missing {label}"))
            .0;
        cx.dispatch_accessibility_action(
            w,
            ActionRequest {
                action: gpui::accesskit::Action::Click,
                target_node: id,
                target_tree: TreeId::ROOT,
                data: None,
            },
        );
        draw(cx);
    };
    click(cx, "Review Voice commands");
    click(cx, "Duplicate voice command");
    let custom = e.read_with(cx, |s, _| s.customize.clone());
    assert_eq!(
        custom.read_with(cx, |s, cx| s.editor.read(cx).value("phrase").to_string()),
        ""
    );
    assert_eq!(
        custom.read_with(cx, |s, cx| s
            .tester
            .read(cx)
            .value("readClipboard")
            .to_string()),
        "false"
    );
    click(cx, "Cancel");
    click(cx, "Review App overrides");
    click(cx, "Add app manually…");
    assert_eq!(
        custom.read_with(cx, |s, cx| s
            .editor
            .read(cx)
            .value("ideContextEnabled")
            .to_string()),
        "false"
    );
    assert_eq!(
        custom.read_with(cx, |s, cx| s
            .editor
            .read(cx)
            .value("autoPasteOverride")
            .to_string()),
        "global"
    );
    click(cx, "Cancel");
    click(cx, "Review Saved spellings");
    click(cx, "Import saved spellings…");
    click(cx, "Confirm spelling import");
    assert_eq!(
        custom.read_with(cx, |s, _| s
            .controller
            .model
            .snapshot
            .as_ref()
            .unwrap()
            .settings["vocabularyEntries"]
            .as_array()
            .unwrap()
            .len()),
        2
    );
}

mod fixture_parameters {
    use crate::{
        remaining::{QaOptions, Scenario},
        section::Controller,
        workspace::Problem,
    };
    use std::rc::Rc;

    #[test]
    fn partial_bulk_parameter_keeps_failed_records_and_exposes_recovery() {
        use crate::knowledge::{
            fixtures::FixtureHost,
            host::Action,
            model::{BulkAction, Filter},
        };
        let options = QaOptions::parse("partial-bulk", "", "", "").unwrap();
        let host = Rc::new(FixtureHost::with_scenario(options.knowledge));
        let mut controller = Controller::new(host.clone());
        controller.attach(Action::List {
            filter: Filter::default(),
            offset: 0,
        });
        controller.drain();
        assert_eq!(
            controller.model.snapshot.as_ref().unwrap().availability,
            "recovered"
        );
        controller.request(Action::Collect {
            filter: Filter::default(),
            action: BulkAction::Disable,
        });
        controller.drain();
        let s = host.state.borrow();
        assert_eq!(s.records[0]["enabled"], false);
        assert_eq!(s.records[0]["revision"], 2);
        assert_eq!(s.records[1]["enabled"], true);
        assert_eq!(s.records[1]["revision"], 1);
        assert!(
            s.notice
                .as_ref()
                .unwrap()
                .contains("2 succeeded · 0 unchanged · 1 failed")
        );
        assert!(s.notice.as_ref().unwrap().contains("synthetic-1"));
        drop(s);
        controller.request(Action::Collect {
            filter: Filter::default(),
            action: BulkAction::Delete,
        });
        controller.drain();
        let bulk = controller
            .model
            .snapshot
            .as_ref()
            .unwrap()
            .bulk
            .clone()
            .unwrap();
        controller.request(Action::Apply {
            bulk,
            action: BulkAction::Delete,
        });
        controller.drain();
        let s = host.state.borrow();
        assert_eq!(s.total, 1);
        assert_eq!(s.records[0]["id"], "synthetic-1");
        assert_eq!(s.records[0]["revision"], 1);
    }
    fn knowledge_import(parameter: &str) {
        use crate::knowledge::{fixtures::FixtureHost, host::Action, model::Filter};
        let options = QaOptions::parse(parameter, "", "", "").unwrap();
        let host = Rc::new(FixtureHost::with_scenario(options.knowledge));
        let mut controller = Controller::new(host.clone());
        controller.attach(Action::List {
            filter: Filter::default(),
            offset: 0,
        });
        controller.drain();
        controller.request(Action::InspectImport);
        controller.drain();
        let preview = controller
            .model
            .snapshot
            .as_ref()
            .unwrap()
            .import_preview
            .clone()
            .unwrap();
        assert_eq!(
            preview["conflicts"],
            usize::from(options.knowledge == Scenario::ConflictingImport)
        );
        let before = host.state.borrow().records.clone();
        controller.request(Action::Import { preview });
        controller.drain();
        assert_eq!(controller.model.problem, Some(Problem::Conflict));
        assert_eq!(host.state.borrow().records, before);
        assert!(host.state.borrow().notice.is_none());
    }
    #[test]
    fn knowledge_stale_import_parameter_refuses_write() {
        knowledge_import("stale-import");
    }
    #[test]
    fn knowledge_conflicting_import_parameter_refuses_write() {
        knowledge_import("conflicting-import");
    }

    fn spelling_import(parameter: &str) {
        use crate::customize::{fixtures::FixtureHost, host::Action};
        let options = QaOptions::parse("", parameter, "", "").unwrap();
        let host = Rc::new(FixtureHost::with_scenario(options.spellings));
        let mut controller = Controller::new(host.clone());
        controller.attach(Action::Read);
        controller.drain();
        controller.request(Action::InspectSpellings);
        controller.drain();
        let preview = controller
            .model
            .snapshot
            .as_ref()
            .unwrap()
            .import_preview
            .clone()
            .unwrap();
        assert_eq!(
            preview.conflicts > 0,
            options.spellings == Scenario::ConflictingImport
        );
        let before = host.state.borrow().settings.clone();
        controller.request(Action::ImportSpellings(preview));
        controller.drain();
        assert_eq!(controller.model.problem, Some(Problem::Conflict));
        assert_eq!(host.state.borrow().settings, before);
    }
    #[test]
    fn spellings_stale_import_parameter_refuses_write() {
        spelling_import("stale-import");
    }
    #[test]
    fn spellings_conflicting_import_parameter_refuses_write() {
        spelling_import("conflicting-import");
    }

    #[test]
    fn mode_files_parameter_completes_parse_import_and_memory_export() {
        use crate::modes::{exchange, fixtures::FixtureHost, host::Action};
        let options = QaOptions::parse("", "", "1", "").unwrap();
        let host = Rc::new(FixtureHost::with_files(options.mode_files));
        let mut controller = Controller::new(host.clone());
        controller.attach(Action::Read);
        controller.drain();
        controller.request(Action::InspectImport);
        controller.drain();
        let preview = controller
            .model
            .snapshot
            .as_ref()
            .unwrap()
            .import_preview
            .clone()
            .unwrap();
        assert_eq!(preview.additions, 1);
        assert_eq!(preview.conflicts, 0);
        controller.request(Action::Import(preview));
        controller.drain();
        assert!(controller.model.problem.is_none());
        assert!(host.state.borrow().import_preview.is_none());
        assert_eq!(
            host.state.borrow().settings["modes"][0]["name"],
            "Synthetic file mode"
        );
        controller.request(Action::Export);
        controller.drain();
        let file = exchange::parse(&host.exports.borrow()[0]).unwrap();
        assert_eq!(file["modes"], host.state.borrow().settings["modes"]);
        assert_eq!(file["appBindings"][0]["bundleId"], "example.synthetic");
        assert_eq!(file["appBindings"][0]["modeId"], "mode.fixture-file");
        assert_eq!(file["browserSiteRules"][0]["host"], "example.com");
        assert_eq!(host.state.borrow().settings["siteModeLookupEnabled"], false);
        assert_eq!(host.exports.borrow().len(), 1);
        assert_eq!(
            host.state.borrow().notice.as_deref(),
            Some("Modes exported to synthetic sink · 1 saved")
        );
    }
    #[test]
    fn normal_motion_parameter_changes_only_injected_preferences() {
        let normal = QaOptions::parse("", "", "", "normal")
            .unwrap()
            .preferences(false);
        let reduced = QaOptions::parse("", "", "", "").unwrap().preferences(false);
        assert!(!normal.reduce_motion);
        assert!(reduced.reduce_motion);
        assert_eq!(normal.reduce_transparency, reduced.reduce_transparency);
        assert_eq!(normal.increase_contrast, reduced.increase_contrast);
        assert!(QaOptions::parse("invalid", "", "", "").is_err());
        assert!(QaOptions::parse("", "partial-bulk", "", "").is_err());
        assert!(QaOptions::parse("", "", "2", "").is_err());
        assert!(QaOptions::parse("", "", "", "invalid").is_err());
    }
}
