use super::{Onboarding, fixtures, host::Action, inbox::Inbox, model::*};
use fixtures::FixtureHost;
use gpui::{Entity, TestAppContext, VisualTestContext, prelude::*};
use gpui::{KeyDownEvent, KeyUpEvent, Keystroke};
use std::{
    rc::Rc,
    time::{Duration, Instant},
};
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
fn harness(
    cx: &mut TestAppContext,
) -> (Entity<Onboarding>, Rc<FixtureHost>, &mut VisualTestContext) {
    cx.update(crate::init);
    let host = Rc::new(FixtureHost::default());
    let mut entity = None;
    let (_, cx) = cx.add_window_view(|w, cx| {
        let e = cx.new(|cx| Onboarding::new(host.clone(), w, cx));
        entity = Some(e.clone());
        gpui_base::Root::new(e, w, cx)
    });
    let e = entity.unwrap();
    e.update(cx, |s, cx| s.attach(cx));
    draw(cx);
    (e, host, cx)
}
#[test]
fn catalog_defaults_command_mapping_and_grandfather_gate() {
    let s = fixtures::snapshot();
    assert!(crate::workspace::Projection::valid(&s));
    assert_eq!(s.preferred(), "tiny.en");
    assert!(!s.grandfathered());
    let mut s = s;
    s.permissions[0] = Access::Granted;
    s.permissions[1] = Access::Granted;
    assert!(s.grandfathered());
    assert_eq!(
        Action::OpenSettings(Permission::Microphone).command(),
        "request_microphone_permission"
    );
    assert_eq!(
        Action::Request(Permission::Microphone).command(),
        "request_microphone_access"
    );
    assert_eq!(Step::ALL.len(), 8);
    let react = include_str!("../../../src/components/onboarding/OnboardingFlow.tsx");
    for step in Step::ALL {
        assert!(react.contains(step.title()));
    }
}
#[gpui::test]
fn subscribe_before_read_no_optional_probes_or_side_effects_on_welcome(cx: &mut TestAppContext) {
    let (e, h, cx) = harness(cx);
    assert_eq!(h.calls.borrow()[0], "subscribe");
    assert_eq!(*h.actions.borrow(), vec![Action::Read]);
    assert_eq!(e.read_with(cx, |s, _| s.step), Step::Welcome);
}
#[gpui::test]
fn default_return_and_back_move_to_step_heading(cx: &mut TestAppContext) {
    let (e, h, cx) = harness(cx);
    cx.update(|w, cx| w.focus(&e.read(cx).heading.clone(), cx));
    keys(cx, "enter");
    assert_eq!(e.read_with(cx, |s, _| s.step), Step::Microphone);
    assert_eq!(
        h.actions.borrow().last(),
        Some(&Action::Probe(Permission::Microphone))
    );
}
#[gpui::test]
fn automatic_mic_request_once_and_bounded_wait(cx: &mut TestAppContext) {
    let (e, h, cx) = harness(cx);
    e.update(cx, |s, cx| s.visit(Step::Microphone, cx));
    draw(cx);
    e.update(cx, |s, cx| {
        s.tick(s.visit_started + Duration::from_millis(601), cx)
    });
    draw(cx);
    assert_eq!(
        h.actions
            .borrow()
            .iter()
            .filter(|a| **a == Action::Request(Permission::Microphone))
            .count(),
        1
    );
    e.update(cx, |s, cx| {
        s.visit(Step::Accessibility, cx);
        s.visit(Step::Microphone, cx);
    });
    draw(cx);
    e.update(cx, |s, cx| {
        s.tick(s.visit_started + Duration::from_secs(2), cx)
    });
    draw(cx);
    assert_eq!(
        h.actions
            .borrow()
            .iter()
            .filter(|a| **a == Action::Request(Permission::Microphone))
            .count(),
        1
    );
    e.update(cx, |s, cx| {
        s.mic_wait = Some(Instant::now());
        s.tick(Instant::now() + Duration::from_secs(16), cx);
    });
    assert!(e.read_with(cx, |s, _| s.mic_wait.is_none()));
}
#[gpui::test]
fn optional_permissions_only_on_own_visits_and_restricted_can_continue(cx: &mut TestAppContext) {
    let (e, h, cx) = harness(cx);
    for step in [Step::Microphone, Step::Accessibility] {
        e.update(cx, |s, cx| s.visit(step, cx));
        draw(cx);
    }
    assert!(!h.actions.borrow().iter().any(|a| matches!(
        a,
        Action::Probe(Permission::SystemAudio | Permission::Calendar)
    )));
    h.state.borrow_mut().permissions[3] = Access::Restricted;
    e.update(cx, |s, cx| s.visit(Step::Calendar, cx));
    draw(cx);
    e.update(cx, |s, cx| s.activate(cx));
    draw(cx);
    assert_eq!(e.read_with(cx, |s, _| s.step), Step::Model);
    assert!(
        !h.actions
            .borrow()
            .contains(&Action::Request(Permission::Calendar))
    );
}
#[gpui::test]
fn stale_permission_reply_cannot_advance_new_visit(cx: &mut TestAppContext) {
    let (e, h, cx) = harness(cx);
    h.hold.set(true);
    e.update(cx, |s, cx| s.visit(Step::Calendar, cx));
    let (r, inbox) = h.held.borrow_mut().pop().unwrap();
    e.update(cx, |s, cx| s.visit(Step::Hotkey, cx));
    assert!(!r.lease.is_current());
    let mut snapshot = h.state.borrow().clone();
    snapshot.permissions[3] = Access::Granted;
    assert!(
        inbox
            .publish(Event::Reply {
                ticket: r.ticket,
                result: Ok(snapshot)
            })
            .is_ok()
    );
    draw(cx);
    assert_eq!(e.read_with(cx, |s, _| s.step), Step::Hotkey);
    assert!(e.read_with(cx, |s, _| s.fresh.is_none()));
}
#[gpui::test]
fn baseline_grant_does_not_advance_but_fresh_grant_does(cx: &mut TestAppContext) {
    let (e, h, cx) = harness(cx);
    h.state.borrow_mut().permissions[3] = Access::Granted;
    e.update(cx, |s, cx| s.visit(Step::Calendar, cx));
    draw(cx);
    assert!(e.read_with(cx, |s, _| s.fresh.is_none()));
    h.state.borrow_mut().permissions[3] = Access::NotDetermined;
    e.update(cx, |s, cx| s.refresh(cx));
    draw(cx);
    e.update(cx, |s, cx| s.activate(cx));
    draw(cx);
    e.update(cx, |s, cx| {
        s.tick(Instant::now() + Duration::from_secs(1), cx)
    });
    draw(cx);
    assert_eq!(e.read_with(cx, |s, _| s.step), Step::Model);
}
#[gpui::test]
fn installed_noncurated_model_continues_without_download(cx: &mut TestAppContext) {
    let (e, h, cx) = harness(cx);
    e.update(cx, |s, cx| s.visit(Step::Model, cx));
    draw(cx);
    e.update(cx, |s, cx| s.activate(cx));
    draw(cx);
    assert_eq!(e.read_with(cx, |s, _| s.step), Step::Hotkey);
    assert!(
        !h.actions
            .borrow()
            .iter()
            .any(|a| matches!(a, Action::Download(_)))
    );
}
#[gpui::test]
fn download_cancel_waits_for_exact_terminal_and_hide_revokes(cx: &mut TestAppContext) {
    let (e, h, cx) = harness(cx);
    e.update(cx, |s, cx| s.visit(Step::Model, cx));
    draw(cx);
    h.hold.set(true);
    e.update(cx, |s, cx| {
        s.selected_model = CURATED[0].into();
        s.activate(cx);
    });
    let (r, inbox) = h.held.borrow_mut().pop().unwrap();
    e.update(cx, |s, cx| {
        s.visit(Step::Calendar, cx);
        s.cancel_download(cx);
    });
    assert_eq!(e.read_with(cx, |s, _| s.step), Step::Model);
    assert!(e.read_with(cx, |s, _| s.cancelling && s.model.pending));
    let (cancel, _) = h.held.borrow_mut().pop().unwrap();
    assert_eq!(cancel.ticket, r.ticket);
    e.update(cx, |s, cx| s.hide(cx));
    assert!(!r.lease.is_current());
    assert!(inbox.is_closed());
    assert!(
        inbox
            .publish(Event::Reply {
                ticket: r.ticket,
                result: Ok(fixtures::snapshot())
            })
            .is_err()
    );
    assert_eq!(h.stops.get(), 1);
}
#[gpui::test]
fn completion_failure_preserves_choices_and_retry_finishes(cx: &mut TestAppContext) {
    let (e, h, cx) = harness(cx);
    e.update(cx, |s, cx| {
        s.visit(Step::Done, cx);
        s.recording_mode = "both".into();
        s.trigger_key = "f12".into();
    });
    h.fail.set(true);
    e.update(cx, |s, cx| s.activate(cx));
    draw(cx);
    assert!(!h.state.borrow().complete);
    assert_eq!(e.read_with(cx, |s, _| s.recording_mode.clone()), "both");
    e.update(cx, |s, cx| s.activate(cx));
    draw(cx);
    assert!(e.read_with(cx, |s, _| s.finished));
    assert!(h.state.borrow().complete);
    assert_eq!(h.state.borrow().settings["doubleTapKey"], "f12");
}
#[test]
fn inbox_bounds_validation_and_attachment_revision_fences() {
    use crate::workspace::{Authority, Projection};
    let inbox = Inbox::default();
    let s = fixtures::snapshot();
    assert!(s.valid());
    for _ in 0..8 {
        assert!(
            inbox
                .publish(Event::Changed {
                    attachment: 1,
                    snapshot: s.clone()
                })
                .is_ok()
        );
    }
    assert!(
        inbox
            .publish(Event::Changed {
                attachment: 1,
                snapshot: s.clone()
            })
            .is_err()
    );
    let mut session = Session::default();
    session.attach();
    let ticket = session.begin().unwrap();
    assert!(session.event(Event::Reply {
        ticket,
        result: Ok(s.clone())
    }));
    let ticket = session.begin().unwrap();
    let a = Authority::default();
    a.sync(&session);
    let lease = a.lease(ticket);
    let mut newer = s.clone();
    newer.revision = 2;
    assert!(session.event(Event::Changed {
        attachment: 1,
        snapshot: newer
    }));
    a.sync(&session);
    assert!(!lease.is_current());
    assert!(!session.event(Event::Reply {
        ticket,
        result: Ok(s.clone())
    }));
    session.detach();
    session.attach();
    assert!(!session.event(Event::Reply {
        ticket,
        result: Ok(s)
    }));
}
#[cfg(feature = "a11y-tests")]
mod ax {
    use super::*;
    use gpui::accesskit::{ActionRequest, NodeId, TreeId, TreeUpdate};
    use gpui::{Focusable, Role};
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
    fn actual_ax_navigation_optional_skip_and_no_hidden_controls(cx: &mut TestAppContext) {
        let (e, h, cx) = harness(cx);
        let t = tree(cx);
        assert!(
            t.nodes
                .iter()
                .any(|(_, n)| n.role() == Role::ProgressIndicator
                    && n.label() == Some("Step 1 of 8"))
        );
        click(cx, named(&t, "Get started"));
        let t = tree(cx);
        assert!(
            !t.nodes
                .iter()
                .any(|(_, n)| n.label() == Some("Get started"))
        );
        click(cx, named(&t, "Skip for now"));
        let t = tree(cx);
        click(cx, named(&t, "Skip for now"));
        let t = tree(cx);
        click(cx, named(&t, "Skip meetings for now"));
        let t = tree(cx);
        click(cx, named(&t, "Skip Calendar for now"));
        assert_eq!(e.read_with(cx, |s, _| s.step), Step::Model);
        assert!(
            !h.actions
                .borrow()
                .iter()
                .any(|a| matches!(a, Action::Request(_)))
        );
    }
    #[gpui::test]
    fn hotkey_picker_keyboard_and_ax_names(cx: &mut TestAppContext) {
        let (e, _, cx) = harness(cx);
        e.update(cx, |s, cx| {
            s.visit(Step::Hotkey, cx);
            s.recording_mode = "hold_down".into();
        });
        draw(cx);
        let t = tree(cx);
        named(&t, "Recording trigger");
        named(&t, "Trigger key");
        cx.update(|w, cx| w.focus(&e.read(cx).modes.read(cx).focus_handle(cx), cx));
        keys(cx, "space down enter");
        assert_eq!(
            e.read_with(cx, |s, _| s.recording_mode.clone()),
            "double_tap"
        );
    }
}

#[gpui::test]
fn completion_uses_real_core_sources_and_preserves_unrelated_defaults(cx: &mut TestAppContext) {
    let (e, h, cx) = harness(cx);
    let root = std::env::temp_dir().join(format!(
        "murmur-onboarding-synthetic-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let durable = Rc::new(FixtureHost {
        root: Some(root.clone()),
        ..Default::default()
    });
    *durable.state.borrow_mut() = h.state.borrow().clone();
    e.update(cx, |s, cx| {
        s.host = durable.clone();
        s.visit(Step::Done, cx);
        s.activate(cx);
    });
    draw(cx);
    assert!(e.read_with(cx, |s, _| s.finished));
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("settings.json")).unwrap()).unwrap();
    assert_eq!(saved["launchOnboardingComplete"], true);
    assert_eq!(saved["model"], "tiny.en");
    assert_eq!(
        saved["autoPaste"],
        murmur_core::state::settings_defaults()["autoPaste"]
    );
    let ui = std::fs::read_to_string(root.join("ui-state.json")).unwrap();
    assert!(ui.contains("murmur_onboarding_complete"));
    assert!(ui.contains("murmur_discovery_v1"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn missing_noncurated_selection_uses_curated_default_and_installed_choice_wins() {
    let mut s = fixtures::snapshot();
    s.settings["model"] = serde_json::json!("small.en");
    for m in s.catalog.as_mut().unwrap() {
        m.installed = false;
    }
    assert_eq!(s.preferred(), CURATED[0]);
    s.catalog
        .as_mut()
        .unwrap()
        .iter_mut()
        .find(|m| m.id == "medium.en")
        .unwrap()
        .installed = true;
    assert_eq!(s.preferred(), "medium.en");
}

#[gpui::test]
fn cancellation_failure_and_source_revision_keep_back_locked_until_termination(
    cx: &mut TestAppContext,
) {
    let (e, h, cx) = harness(cx);
    e.update(cx, |s, cx| s.visit(Step::Model, cx));
    draw(cx);
    h.hold.set(true);
    e.update(cx, |s, cx| {
        s.selected_model = CURATED[0].into();
        s.activate(cx);
    });
    let (original, _) = h.held.borrow_mut().pop().unwrap();
    let mut changed = h.state.borrow().clone();
    changed.revision += 1;
    e.update(cx, |s, cx| {
        assert!(
            s.inbox
                .publish(Event::Changed {
                    attachment: s.model.attachment,
                    snapshot: changed
                })
                .is_ok()
        );
        s.drain(cx);
    });
    let (cancel, inbox) = h.held.borrow_mut().pop().unwrap();
    assert!(matches!(cancel.action, Action::CancelDownload(_)));
    assert!(!original.lease.is_current());
    assert!(
        inbox
            .publish(Event::Reply {
                ticket: cancel.ticket,
                result: Err(Problem::Unavailable)
            })
            .is_ok()
    );
    draw(cx);
    e.update(cx, |s, cx| s.visit(Step::Calendar, cx));
    assert_eq!(e.read_with(cx, |s, _| s.step), Step::Model);
    e.update(cx, |s, cx| s.cancel_download(cx));
    let (retry, inbox) = h.held.borrow_mut().pop().unwrap();
    assert_ne!(cancel.ticket, retry.ticket);
    let mut state = h.state.borrow().clone();
    state.revision = retry.ticket.revision;
    assert!(
        inbox
            .publish(Event::Reply {
                ticket: retry.ticket,
                result: Ok(state)
            })
            .is_ok()
    );
    draw(cx);
    e.update(cx, |s, cx| s.visit(Step::Calendar, cx));
    assert_eq!(e.read_with(cx, |s, _| s.step), Step::Calendar);
}
#[gpui::test]
fn leaving_microphone_clears_its_wait_without_reprompting(cx: &mut TestAppContext) {
    let (e, _, cx) = harness(cx);
    e.update(cx, |s, cx| {
        s.visit(Step::Microphone, cx);
        s.drain(cx);
        s.mic_wait = Some(Instant::now());
        s.visit(Step::Welcome, cx);
    });
    draw(cx);
    assert!(e.read_with(cx, |s, _| s.mic_wait.is_none() && s.mic_auto_consumed));
    keys(cx, "enter");
    assert_eq!(e.read_with(cx, |s, _| s.step), Step::Microphone);
}

#[gpui::test]
fn grandfathered_install_with_a_different_installed_model_keeps_the_model_gate(
    cx: &mut TestAppContext,
) {
    cx.update(crate::init);
    let host = Rc::new(FixtureHost::default());
    host.state.borrow_mut().permissions = [Access::Granted; 4];
    let mut entity = None;
    let (_, cx) = cx.add_window_view(|w, cx| {
        let e = cx.new(|cx| Onboarding::new(host.clone(), w, cx));
        entity = Some(e.clone());
        gpui_base::Root::new(e, w, cx)
    });
    let e = entity.unwrap();
    e.update(cx, |s, cx| s.attach(cx));
    draw(cx);
    assert!(host.state.borrow().complete);
    assert_eq!(e.read_with(cx, |s, _| s.step), Step::Model);
    assert!(!e.read_with(cx, |s, _| s.finished));
    assert_eq!(
        *host.actions.borrow(),
        vec![Action::Read, Action::Grandfather]
    );
    e.update(cx, |s, cx| s.activate(cx));
    draw(cx);
    assert!(e.read_with(cx, |s, _| s.finished));
    assert_eq!(host.state.borrow().settings["model"], "tiny.en");
}
