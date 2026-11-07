use super::{Home, fixtures, host::Action, model::*};
use crate::onboarding::model::{Access, Permission};
use fixtures::FixtureHost;
use gpui::{Entity, TestAppContext, VisualTestContext, prelude::*};
use gpui::{Focusable, KeyDownEvent, KeyUpEvent, Keystroke};
use std::rc::Rc;
fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|w, cx| w.draw(cx).clear(cx));
}
fn keys(cx: &mut VisualTestContext, keys: &str) {
    for key in keys.split_whitespace() {
        let keystroke =
            cx.update(|_, cx| Keystroke::parse_with_os(key, cx.platform_info().os).unwrap());
        cx.simulate_event(KeyDownEvent {
            keystroke: keystroke.clone(),
            is_held: false,
            prefer_character_input: false,
        });
        cx.simulate_event(KeyUpEvent { keystroke });
        draw(cx);
    }
}
fn harness(cx: &mut TestAppContext) -> (Entity<Home>, Rc<FixtureHost>, &mut VisualTestContext) {
    cx.update(crate::init);
    let host = Rc::new(FixtureHost::default());
    let mut entity = None;
    let (_, cx) = cx.add_window_view(|w, cx| {
        let e = cx.new(|cx| {
            Home::new(
                host.clone(),
                Rc::new(crate::history::fixtures::FixtureHost::default()),
                w,
                cx,
            )
        });
        entity = Some(e.clone());
        gpui_base::Root::new(e, w, cx)
    });
    let e = entity.unwrap();
    e.update(cx, |s, cx| s.attach(cx));
    draw(cx);
    (e, host, cx)
}
#[test]
fn state_copy_and_permission_denial_parity() {
    let mut s = fixtures::snapshot();
    assert!(crate::workspace::Projection::valid(&s));
    assert_eq!(s.title(), "Click to start talking");
    s.status = Status::Starting;
    assert_eq!(s.title(), "Connecting to your microphone…");
    s.status = Status::Recording;
    s.recording_seconds = 65;
    assert_eq!(s.title(), "Listening · 1:05");
    s.status = Status::Processing;
    assert_eq!(s.title(), "Writing it out…");
    s.status = Status::Recovering;
    assert_eq!(s.title(), "Reconnecting your microphone…");
    s.status = Status::Idle;
    s.permissions[0] = Access::Unknown;
    assert!(s.warning().is_none());
    s.permissions[0] = Access::NotDetermined;
    assert!(s.warning().is_none());
    s.permissions[0] = Access::Denied;
    assert_eq!(s.warning().unwrap().0, "Microphone access was revoked.");
    s.permissions[2] = Access::Denied;
    assert_eq!(s.warning().unwrap().1, Permission::SystemAudio);
}
#[gpui::test]
fn subscribe_before_read_without_capture_and_embedded_history(cx: &mut TestAppContext) {
    let (e, h, cx) = harness(cx);
    assert_eq!(h.calls.borrow()[0], "subscribe");
    assert_eq!(*h.actions.borrow(), vec![Action::Read]);
    assert_eq!(
        e.read_with(cx, |s, cx| s.history.read(cx).model.entries.len()),
        200
    );
}
#[gpui::test]
fn exact_generation_stop_and_no_record_while_meeting_or_processing(cx: &mut TestAppContext) {
    let (e, h, cx) = harness(cx);
    e.update(cx, |s, cx| s.record(cx));
    draw(cx);
    let generation = h.state.borrow().recording_generation.unwrap();
    e.update(cx, |s, cx| s.record(cx));
    draw(cx);
    assert_eq!(
        h.actions.borrow().last(),
        Some(&Action::Stop { generation })
    );
    let count = h.actions.borrow().len();
    e.update(cx, |s, cx| s.record(cx));
    assert_eq!(h.actions.borrow().len(), count);
    let mut state = h.state.borrow().clone();
    state.revision += 1;
    state.status = Status::Idle;
    state.meeting_busy = true;
    e.update(cx, |s, cx| {
        let _ = s.inbox.publish(Event::Changed {
            attachment: s.model.attachment,
            snapshot: state,
        });
        s.drain(cx);
        s.record(cx);
    });
    assert_eq!(h.actions.borrow().len(), count);
}
#[gpui::test]
fn keyboard_shortcuts_and_editor_return_focus(cx: &mut TestAppContext) {
    let (e, h, cx) = harness(cx);
    cx.update(|w, cx| w.focus(&e.read(cx).focus["settings"].clone(), cx));
    keys(cx, "cmd-,");
    assert_eq!(
        h.actions.borrow().last(),
        Some(&Action::Open(Destination::General))
    );
    cx.update(|w, cx| e.update(cx, |s, cx| s.destination_closed(w, cx)));
    assert!(cx.update(|w, cx| e.read(cx).focus["settings"].is_focused(w)));
    keys(cx, "cmd-k");
    assert_eq!(
        h.actions.borrow().last(),
        Some(&Action::Open(Destination::Palette))
    );
    keys(cx, "cmd-f");
    assert!(cx.update(|w, cx| {
        e.read(cx)
            .history
            .read(cx)
            .search
            .read(cx)
            .focus_handle(cx)
            .is_focused(w)
    }));
}
#[gpui::test]
fn separator_arrows_clamp_and_toggle_restores_focus(cx: &mut TestAppContext) {
    let (e, _, cx) = harness(cx);
    cx.update(|w, cx| w.focus(&e.read(cx).focus["sidebar-width"].clone(), cx));
    keys(cx, "end");
    assert_eq!(e.read_with(cx, |s, _| s.expanded_width), 384.);
    keys(cx, "right home left");
    assert_eq!(e.read_with(cx, |s, _| s.expanded_width), 256.);
    cx.update(|w, cx| e.update(cx, |s, cx| s.toggle(w, cx)));
    draw(cx);
    assert!(e.read_with(cx, |s, _| s.collapsed()));
    assert!(cx.update(|w, cx| e.read(cx).focus["sidebar-toggle"].is_focused(w)));
}
#[gpui::test]
fn checklist_click_is_not_meeting_evidence_and_dismissal_keeps_baseline(cx: &mut TestAppContext) {
    let (e, h, cx) = harness(cx);
    let baseline = h.state.borrow().discovery["baseline"].clone();
    e.update(cx, |s, cx| s.request(Action::Checklist("meeting"), cx));
    draw(cx);
    assert!(!h.state.borrow().completed("meeting"));
    e.update(cx, |s, cx| s.request(Action::Checklist("shortcuts"), cx));
    draw(cx);
    assert!(h.state.borrow().completed("shortcuts"));
    e.update(cx, |s, cx| s.request(Action::DismissChecklist, cx));
    draw(cx);
    assert_eq!(h.state.borrow().discovery["baseline"], baseline);
    assert!(!h.state.borrow().checklist());
}
#[gpui::test]
fn hide_revokes_leases_releases_history_and_reopen_subscribes_again(cx: &mut TestAppContext) {
    let (e, h, cx) = harness(cx);
    let lease = e.read_with(cx, |s, _| s.authority.lease(s.model.ticket()));
    cx.update(|w, cx| e.update(cx, |s, cx| s.hide(w, cx)));
    assert!(!lease.is_current());
    assert_eq!(h.stops.get(), 1);
    assert!(e.read_with(cx, |s, cx| s.model.snapshot.is_none()
        && s.inbox.is_closed()
        && s.history.read(cx).model.entries.is_empty()));
    e.update(cx, |s, cx| s.attach(cx));
    draw(cx);
    assert!(e.read_with(cx, |s, _| s.model.snapshot.is_some()));
    assert_eq!(
        h.calls
            .borrow()
            .iter()
            .filter(|c| **c == "subscribe")
            .count(),
        2
    );
}
#[test]
fn discovery_routes_match_react_and_validate_bounds() {
    for (id, _, _, _) in ITEMS {
        assert!(checklist_destination(id).is_some());
    }
    assert_eq!(
        checklist_destination("correction"),
        Some(Destination::TeachLatest)
    );
    let mut s = fixtures::snapshot();
    s.audio_level = f32::NAN;
    assert!(!crate::workspace::Projection::valid(&s));
    s.audio_level = 0.;
    s.status = Status::Recording;
    assert!(!crate::workspace::Projection::valid(&s));
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
            .unwrap_or_else(|| panic!("missing {label}"))
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
    fn ax_record_button_navigation_and_discovery_actions(cx: &mut TestAppContext) {
        let (e, h, cx) = harness(cx);
        let t = tree(cx);
        named(&t, "Murmur navigation");
        named(&t, "Notetaker");
        named(&t, "Insights");
        click(cx, named(&t, "Click to start talking"));
        let t = tree(cx);
        click(cx, named(&t, "Click to stop recording, 0:00"));
        assert!(matches!(
            h.actions.borrow().last(),
            Some(Action::Stop { .. })
        ));
        let t = tree(cx);
        click(cx, named(&t, "Dismiss Get more from Murmur"));
        let t = tree(cx);
        assert!(
            !t.nodes
                .iter()
                .any(|(_, n)| n.label() == Some("Dismiss Get more from Murmur"))
        );
        assert!(!e.read_with(cx, |s, _| s.model.snapshot.as_ref().unwrap().checklist()));
    }
    #[gpui::test]
    fn collapsed_navigation_keeps_full_ax_names(cx: &mut TestAppContext) {
        let (e, _, cx) = harness(cx);
        let t = tree(cx);
        click(cx, named(&t, "Collapse sidebar"));
        let t = tree(cx);
        named(&t, "Home");
        named(&t, "Notetaker");
        named(&t, "Insights");
        assert!(
            !t.nodes
                .iter()
                .any(|(_, n)| n.label() == Some("Sidebar width"))
        );
        assert!(e.read_with(cx, |s, _| s.collapsed()));
    }
}
