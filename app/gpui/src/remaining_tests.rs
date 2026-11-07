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
