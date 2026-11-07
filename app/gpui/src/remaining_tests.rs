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
    cx.update(crate::init);
    let mut entity = None;
    let (_, cx) = cx.add_window_view(|w, cx| {
        let e = cx.new(|cx| Review::new(w, cx));
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
            activate(cx, "Edit colors…");
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
            assert_ne!(
                compile::value(&compile::resolve(&saved, true))["primary"],
                old["primary"]
            );
        }
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
