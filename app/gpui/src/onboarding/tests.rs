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
    for step in Step::ALL {
        assert!(!step.title().is_empty());
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
fn every_permission_polls_only_its_active_visit_and_observes_a_later_grant(
    cx: &mut TestAppContext,
) {
    let (e, h, cx) = harness(cx);
    for step in [
        Step::Microphone,
        Step::Accessibility,
        Step::SystemAudio,
        Step::Calendar,
    ] {
        let permission = step.permission().unwrap();
        h.state.borrow_mut().permissions[permission.index()] = Access::NotDetermined;
        e.update(cx, |s, cx| s.visit(step, cx));
        draw(cx);
        e.update(cx, |s, _| s.mic_auto_consumed = true);
        let before = h.actions.borrow().len();
        h.state.borrow_mut().permissions[permission.index()] = Access::Granted;
        e.update(cx, |s, cx| s.tick(s.last_poll + Duration::from_secs(2), cx));
        draw(cx);
        assert_eq!(&h.actions.borrow()[before..], &[Action::Probe(permission)]);
        assert_eq!(e.read_with(cx, |s, _| s.access()), Access::Granted);
        assert!(e.read_with(cx, |s, _| s.model.problem.is_none()));
        assert!(e.read_with(cx, |s, _| s.fresh.is_some()));
    }
    e.update(cx, |s, cx| s.visit(Step::Hotkey, cx));
    let before = h.actions.borrow().len();
    e.update(cx, |s, cx| s.tick(s.last_poll + Duration::from_secs(2), cx));
    assert_eq!(h.actions.borrow().len(), before);
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
    fn static_text(t: &TreeUpdate, text: &str) {
        assert_eq!(t.nodes.iter().filter(|(_, n)| n.role() == gpui::accesskit::Role::Label && n.value() == Some(text)).count(), 1, "missing or duplicated static text: {text}");
    }
    #[gpui::test]
    fn prose_and_done_summary_survive_fresh_ax_reads(cx: &mut TestAppContext) {
        let (e, _, cx) = harness(cx);
        static_text(
            &tree(cx),
            "Setup covers permissions, an offline transcription model, and your recording shortcut.",
        );
        for step in Step::ALL {
            e.update(cx, |s, cx| s.visit(step, cx));
            static_text(&tree(cx), step.subtitle());
        }
        for _ in 0..2 {
            let t = tree(cx);
            for text in [
                "Microphone · Not granted — enable in Settings before recording",
                "Accessibility · Not granted — shortcut unavailable outside Murmur",
                "System audio · Optional — enable later in Meetings",
                "Calendar · Optional — name meetings manually",
                "Model · Installed",
                "Try it out",
                "Hold Left Shift and speak, then release. Your words are transcribed and copied to the clipboard.",
            ] {
                static_text(&t, text);
            }
        }
        e.update(cx, |s, cx| s.visit(Step::Accessibility, cx));
        static_text(
            &tree(cx),
            "Enable this copy of Murmur in System Settings → Privacy & Security → Accessibility for the global recording key and auto-paste. Then come back; this screen updates automatically.",
        );
        e.update(cx, |s, cx| s.visit(Step::Model, cx));
        static_text(
            &tree(cx),
            "Fastest on Apple Silicon — multilingual, Apple Neural Engine",
        );
    }
    #[gpui::test]
    fn held_cancel_fixture_retains_disabled_fallbacks_until_receipt(cx: &mut TestAppContext) {
        let (e, h, cx) = harness(cx);
        e.update(cx, |s, cx| s.visit(Step::Model, cx));
        draw(cx);
        h.hold_cancel.set(true);
        // Same failed active download as cancel-error in onboarding-qa.
        h.hold.set(true);
        e.update(cx, |s, cx| {
            s.selected_model = CURATED[0].into();
            s.activate(cx);
            s.model.pending = false;
            s.model.problem = Some(Problem::Unavailable);
        });
        h.hold.set(false);
        let t = tree(cx);
        click(cx, named(&t, "Cancel download"));
        for _ in 0..2 {
            let t = tree(cx);
            for label in [
                "Cancelling…",
                "Back",
                "Use Whisper base",
                "Use CPU Parakeet",
            ] {
                let id = named(&t, label);
                let node = &t.nodes.iter().find(|(i, _)| *i == id).unwrap().1;
                assert!(
                    node.is_disabled(),
                    "{label} enabled before cancellation receipt"
                );
            }
            assert!(e.read_with(cx, |s, _| s.cancelling && s.downloading()));
        }
        let (r, inbox) = h.held.borrow_mut().pop().unwrap();
        assert!(matches!(r.action, Action::CancelDownload(_)));
        let mut snapshot = h.state.borrow().clone();
        snapshot.revision = r.ticket.revision;
        inbox
            .publish(Event::Reply {
                ticket: r.ticket,
                result: Ok(snapshot),
            })
            .unwrap_or_else(|_| panic!("reply"));
        draw(cx);
        assert!(!e.read_with(cx, |s, _| s.cancelling || s.downloading()));
    }

    #[gpui::test]
    fn failed_initial_read_has_keyboard_retry_and_recovers(cx: &mut TestAppContext) {
        let (e, h, cx) = harness(cx);
        e.update(cx, |s, cx| s.hide(cx));
        h.fail.set(true);
        e.update(cx, |s, cx| s.attach(cx));
        draw(cx);
        let t = tree(cx);
        let retry = named(&t, "Retry");
        assert!(
            !t.nodes
                .iter()
                .find(|(id, _)| *id == retry)
                .unwrap()
                .1
                .is_disabled()
        );
        keys(cx, "enter");
        assert_eq!(e.read_with(cx, |s, _| s.step), Step::Welcome);
        // Welcome's first enabled tab stop must offer recovery.
        keys(cx, "tab space");
        draw(cx);
        assert_eq!(h.actions.borrow().last(), Some(&Action::Read));
        assert!(e.read_with(cx, |s, _| s.model.snapshot.is_some()
            && s.model.problem.is_none()));
        assert_eq!(e.read_with(cx, |s, _| s.step), Step::Welcome);
        let t = tree(cx);
        click(cx, named(&t, "Get started"));
        assert_eq!(e.read_with(cx, |s, _| s.step), Step::Microphone);
    }
    #[gpui::test]
    fn failed_cancel_disables_both_fallback_actions_until_terminal(cx: &mut TestAppContext) {
        let (e, h, cx) = harness(cx);
        e.update(cx, |s, cx| s.visit(Step::Model, cx));
        draw(cx);
        h.hold.set(true);
        e.update(cx, |s, cx| {
            s.selected_model = CURATED[0].into();
            s.activate(cx);
            s.cancel_download(cx);
        });
        let (cancel, inbox) = h.held.borrow_mut().pop().unwrap();
        inbox
            .publish(Event::Reply {
                ticket: cancel.ticket,
                result: Err(Problem::Unavailable),
            })
            .unwrap_or_else(|_| panic!("reply"));
        let t = tree(cx);
        for label in ["Use Whisper base", "Use CPU Parakeet"] {
            let id = named(&t, label);
            assert!(
                t.nodes
                    .iter()
                    .find(|(i, _)| *i == id)
                    .unwrap()
                    .1
                    .is_disabled(),
                "{label} enabled while download is active"
            );
            click(cx, id);
        }
        assert_eq!(e.read_with(cx, |s, _| s.selected_model.clone()), CURATED[0]);
        let t = tree(cx);
        click(cx, named(&t, "Cancel download"));
        let (retry, inbox) = h.held.borrow_mut().pop().unwrap();
        let mut state = fixtures::snapshot();
        state.revision = retry.ticket.revision;
        inbox
            .publish(Event::Reply {
                ticket: retry.ticket,
                result: Ok(state),
            })
            .unwrap_or_else(|_| panic!("reply"));
        draw(cx);
        assert!(!e.read_with(cx, |s, _| s.downloading()));
    }
    /// The production adapter's explanation for a Core ML cache that did not
    /// validate (`murmur_services::model_failure`), delivered as the download's
    /// failed reply. Whisper base is installed in this catalog.
    const EXPLAINED: &str = "Core ML setup finished, but the installed cache did not validate. Retry Core ML, or choose Whisper Base or the CPU Parakeet fallback.";
    fn failed_first_download(
        cx: &mut TestAppContext,
    ) -> (Entity<Onboarding>, Rc<FixtureHost>, &mut VisualTestContext) {
        let (e, h, cx) = harness(cx);
        for m in h.state.borrow_mut().catalog.as_mut().unwrap() {
            m.installed |= m.id == "base.en";
        }
        e.update(cx, |s, cx| s.request(Action::Read, cx));
        draw(cx);
        e.update(cx, |s, cx| s.visit(Step::Model, cx));
        draw(cx);
        h.hold.set(true);
        e.update(cx, |s, cx| {
            s.selected_model = CURATED[0].into();
            s.activate(cx);
        });
        h.hold.set(false);
        let (r, inbox) = h.held.borrow_mut().pop().unwrap();
        assert_eq!(r.action, Action::Download(CURATED[0].into()));
        super::inbox::reply(&inbox, r.ticket, Err(Problem::Operation(EXPLAINED)));
        draw(cx);
        (e, h, cx)
    }
    #[gpui::test]
    fn first_launch_download_failure_shows_its_explanation_and_retries_the_download(
        cx: &mut TestAppContext,
    ) {
        let (e, h, cx) = failed_first_download(cx);
        let t = tree(cx);
        named(&t, EXPLAINED);
        named(&t, "Use Whisper base");
        named(&t, "Use CPU Parakeet");
        // The primary action is the retry, not the first-time download.
        let retry = named(&t, "Retry download");
        assert!(
            !t.nodes
                .iter()
                .any(|(_, n)| n.label() == Some("Download model"))
        );
        h.actions.borrow_mut().clear();
        h.hold.set(true);
        click(cx, retry);
        assert_eq!(
            *h.actions.borrow(),
            vec![Action::Download(CURATED[0].into())],
            "Retry download must re-download, not read"
        );
        assert!(e.read_with(cx, |s, _| s.downloading() && s.download_failed.is_none()));
        // The retry fails again with its own explanation: still retryable.
        let (r, inbox) = h.held.borrow_mut().pop().unwrap();
        super::inbox::reply(&inbox, r.ticket, Err(Problem::Operation(EXPLAINED)));
        draw(cx);
        named(&tree(cx), "Retry download");
    }
    #[gpui::test]
    fn first_launch_download_failure_survives_the_catalog_refresh_that_follows_it(
        cx: &mut TestAppContext,
    ) {
        let (e, h, cx) = failed_first_download(cx);
        // The service's terminal model-status event queues a refresh, delivered
        // as a higher-revision `Changed` after the error reply.
        let mut refreshed = h.state.borrow().clone();
        refreshed.revision = e.read_with(cx, |s, _| s.model.revision) + 1;
        h.state.borrow_mut().revision = refreshed.revision;
        e.update(cx, |s, cx| {
            assert!(
                s.inbox
                    .publish(Event::Changed {
                        attachment: s.model.attachment,
                        snapshot: refreshed
                    })
                    .is_ok()
            );
            s.drain(cx);
        });
        draw(cx);
        assert!(
            e.read_with(cx, |s, _| s.model.problem.is_none()),
            "the refresh clears the section problem"
        );
        let t = tree(cx);
        named(&t, EXPLAINED);
        named(&t, "Use Whisper base");
        named(&t, "Use CPU Parakeet");
        let retry = named(&t, "Retry download");
        assert!(
            !t.nodes
                .iter()
                .any(|(_, n)| n.label() == Some("Download model"))
        );
        // Still retryable, and a new attempt dismisses the failure.
        h.actions.borrow_mut().clear();
        h.hold.set(true);
        click(cx, retry);
        assert_eq!(
            *h.actions.borrow(),
            vec![Action::Download(CURATED[0].into())]
        );
        assert!(
            !tree(cx)
                .nodes
                .iter()
                .any(|(_, n)| n.label() == Some(EXPLAINED))
        );
        // Choosing a model also dismisses a failure that survived a refresh.
        let (r, inbox) = h.held.borrow_mut().pop().unwrap();
        super::inbox::reply(&inbox, r.ticket, Err(Problem::Operation(EXPLAINED)));
        draw(cx);
        let mut refreshed = h.state.borrow().clone();
        refreshed.revision = e.read_with(cx, |s, _| s.model.revision) + 1;
        e.update(cx, |s, cx| {
            let _ = s.inbox.publish(Event::Changed {
                attachment: s.model.attachment,
                snapshot: refreshed,
            });
            s.drain(cx);
        });
        click(cx, named(&tree(cx), "Use Whisper base"));
        let t = tree(cx);
        assert!(!t.nodes.iter().any(|(_, n)| n.label() == Some(EXPLAINED)));
        named(&t, "Continue");
    }
    #[gpui::test]
    fn first_launch_download_failure_fallbacks_continue_or_download(cx: &mut TestAppContext) {
        let (e, h, cx) = failed_first_download(cx);
        // Installed fallback: selected, the failure is dismissed, setup continues.
        let t = tree(cx);
        click(cx, named(&t, "Use Whisper base"));
        assert_eq!(e.read_with(cx, |s, _| s.selected_model.clone()), "base.en");
        assert!(e.read_with(cx, |s, _| s.model.problem.is_none()));
        let t = tree(cx);
        let primary = named(&t, "Continue");
        assert!(!t.nodes.iter().any(|(_, n)| n.label() == Some(EXPLAINED)));
        h.actions.borrow_mut().clear();
        click(cx, primary);
        assert!(
            !h.actions
                .borrow()
                .iter()
                .any(|a| matches!(a, Action::Download(_)))
        );
        assert_eq!(e.read_with(cx, |s, _| s.step), Step::Hotkey);
    }
    #[gpui::test]
    fn first_launch_download_failure_missing_fallback_downloads_on_primary(
        cx: &mut TestAppContext,
    ) {
        let (e, h, cx) = failed_first_download(cx);
        // Missing fallback: selected, then the primary action downloads it.
        let t = tree(cx);
        click(cx, named(&t, "Use CPU Parakeet"));
        assert_eq!(
            e.read_with(cx, |s, _| s.selected_model.clone()),
            "parakeet-tdt-0.6b-v2-fp16"
        );
        let t = tree(cx);
        let primary = named(&t, "Download model");
        h.actions.borrow_mut().clear();
        click(cx, primary);
        assert_eq!(
            *h.actions.borrow(),
            vec![Action::Download("parakeet-tdt-0.6b-v2-fp16".into())]
        );
    }
    #[gpui::test]
    fn compact_onboarding_tabs_reveal_model_buttons_and_hotkey_choices(cx: &mut TestAppContext) {
        let (e, _, cx) = harness(cx);
        cx.simulate_resize(gpui::size(gpui::px(480.), gpui::px(360.)));
        for reduce_motion in [false, true] {
            cx.update(|_, cx| {
                crate::preferences::apply(
                    crate::preferences::DisplayPreferences {
                        reduce_motion,
                        ..Default::default()
                    },
                    cx,
                )
            });
            for step in [Step::Model, Step::Hotkey] {
                e.update(cx, |s, cx| s.visit(step, cx));
                draw(cx);
                for key in ["tab", "shift-tab"] {
                    for _ in 0..16 {
                        keys(cx, key);
                        cx.update(|w, cx| w.simulate_next_frame(cx));
                        draw(cx);
                        let t = tree(cx);
                        let node = &t
                            .nodes
                            .iter()
                            .find(|(id, _)| *id == t.focus)
                            .expect("focused control")
                            .1;
                        let bounds = node.bounds().unwrap();
                        let scale = cx.update(|w, _| f64::from(w.scale_factor()));
                        let viewport = e.read_with(cx, |s, _| s.scroll.bounds());
                        // Footer actions sit outside the scrolling body.
                        if node.label().is_some_and(|l| {
                            l == "Back" || l == "Continue" || l == "Download model"
                        }) {
                            continue;
                        }
                        assert!(
                            bounds.y0 >= scale * f64::from(f32::from(viewport.top()))
                                && bounds.y1 <= scale * f64::from(f32::from(viewport.bottom())),
                            "{:?} {:?} outside {:?}",
                            node.label(),
                            bounds,
                            viewport
                        );
                    }
                }
            }
        }
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

#[gpui::test]
fn cancellation_retry_uses_retained_download_identity(cx: &mut TestAppContext) {
    let (e, h, cx) = harness(cx);
    e.update(cx, |s, cx| s.visit(Step::Model, cx));
    draw(cx);
    h.hold.set(true);
    e.update(cx, |s, cx| {
        s.selected_model = CURATED[0].into();
        s.activate(cx);
        s.cancel_download(cx);
    });
    let (cancel, inbox) = h.held.borrow_mut().pop().unwrap();
    inbox
        .publish(Event::Reply {
            ticket: cancel.ticket,
            result: Err(Problem::Unavailable),
        })
        .unwrap_or_else(|_| panic!("reply"));
    draw(cx);
    e.update(cx, |s, cx| {
        // Even an independently changed selection cannot redirect cancellation.
        s.selected_model = "base.en".into();
        s.cancel_download(cx);
    });
    let (retry, _) = h.held.borrow_mut().pop().unwrap();
    assert_eq!(retry.action, Action::CancelDownload(CURATED[0].into()));
    // A reply from the failed attempt cannot release the retry's owner.
    inbox
        .publish(Event::Reply {
            ticket: cancel.ticket,
            result: Ok(fixtures::snapshot()),
        })
        .unwrap_or_else(|_| panic!("reply"));
    draw(cx);
    assert!(e.read_with(cx, |s, _| s.cancelling
        && s.downloading()
        && s.model.pending));
}

#[gpui::test]
fn initial_read_timeout_offers_retry_and_rejects_late_reply(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let host = Rc::new(FixtureHost::default());
    host.hold.set(true);
    let mut entity = None;
    let (_, cx) = cx.add_window_view(|w, cx| {
        let e = cx.new(|cx| Onboarding::new(host.clone(), w, cx));
        entity = Some(e.clone());
        gpui_base::Root::new(e, w, cx)
    });
    let e = entity.unwrap();
    e.update(cx, |s, cx| s.attach(cx));
    let (old, inbox) = host.held.borrow_mut().pop().unwrap();
    e.update(cx, |s, cx| {
        s.tick(s.read_started.unwrap() + Duration::from_secs(10), cx);
        assert!(!s.model.pending);
        assert!(s.model.problem.unwrap().message().contains("10 seconds"));
        assert!(!old.lease.is_current());
    });
    host.hold.set(false);
    e.update(cx, |s, cx| s.request(Action::Read, cx));
    super::inbox::reply(&inbox, old.ticket, Err(Problem::Unavailable));
    draw(cx);
    assert!(e.read_with(cx, |s, _| s.model.snapshot.is_some()
        && s.model.problem.is_none()));
}

#[gpui::test]
fn rejected_initial_projection_delivers_error_instead_of_loading_forever(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let host = Rc::new(FixtureHost::default());
    host.state.borrow_mut().hardware_summary = Some("x".repeat(513));
    let mut entity = None;
    let (_, cx) = cx.add_window_view(|w, cx| {
        let e = cx.new(|cx| Onboarding::new(host.clone(), w, cx));
        entity = Some(e.clone());
        gpui_base::Root::new(e, w, cx)
    });
    let e = entity.unwrap();
    e.update(cx, |s, cx| s.attach(cx));
    draw(cx);
    assert!(e.read_with(cx, |s, _| !s.model.pending
        && s.model.problem == Some(Problem::Invalid)));
    host.state.borrow_mut().hardware_summary = None;
    e.update(cx, |s, cx| s.request(Action::Read, cx));
    draw(cx);
    assert!(e.read_with(cx, |s, _| s.model.snapshot.is_some()
        && s.model.problem.is_none()));
}

#[gpui::test]
fn completion_allows_deferred_microphone_and_keeps_specific_failure(cx: &mut TestAppContext) {
    let (e, h, cx) = harness(cx);
    assert_eq!(h.state.borrow().permissions[0], Access::NotDetermined);
    e.update(cx, |s, cx| s.visit(Step::Done, cx));
    h.hold.set(true);
    e.update(cx, |s, cx| s.activate(cx));
    let (r, inbox) = h.held.borrow_mut().pop().unwrap();
    let message = "Could not save setup completion. Your previous local data is preserved. Retry.";
    super::inbox::reply(&inbox, r.ticket, Err(Problem::Operation(message)));
    draw(cx);
    assert_eq!(
        e.read_with(cx, |s, _| s.model.problem.unwrap().message()),
        message
    );
    h.hold.set(false);
    e.update(cx, |s, cx| s.activate(cx));
    draw(cx);
    assert!(e.read_with(cx, |s, _| s.finished));
    assert_eq!(h.state.borrow().permissions[0], Access::NotDetermined);
}

#[gpui::test]
fn saturated_progress_inbox_keeps_download_and_cancellation_terminal_replies(
    cx: &mut TestAppContext,
) {
    let (e, h, cx) = harness(cx);
    e.update(cx, |s, cx| s.visit(Step::Model, cx));
    draw(cx);
    for outcome in 0..3 {
        h.hold.set(true);
        let cancel = outcome == 2;
        e.update(cx, |s, cx| {
            s.selected_model = CURATED[0].into();
            s.activate(cx);
            if cancel {
                s.cancel_download(cx);
            }
        });
        let (request, inbox) = h.held.borrow_mut().pop().unwrap();
        let mut progress = h.state.borrow().clone();
        progress.revision = request.ticket.revision;
        progress.download_percent = Some(42);
        for _ in 0..8 {
            assert!(
                inbox
                    .publish(Event::Progress {
                        ticket: request.ticket,
                        snapshot: progress.clone()
                    })
                    .is_ok()
            );
        }
        assert!(
            inbox
                .publish(Event::Progress {
                    ticket: request.ticket,
                    snapshot: progress.clone()
                })
                .is_err()
        );
        // Both successful and failed downloads, and successful cancellation,
        // must release their pending owner even when progress cannot enqueue.
        if outcome == 1 {
            progress
                .catalog
                .as_mut()
                .unwrap()
                .iter_mut()
                .find(|model| model.id == CURATED[0])
                .unwrap()
                .installed = true;
        }
        super::inbox::reply(
            &inbox,
            request.ticket,
            if outcome == 0 {
                Err(Problem::Unavailable)
            } else {
                Ok(progress)
            },
        );
        draw(cx);
        assert!(e.read_with(cx, |s, _| !s.model.pending
            && !s.downloading()
            && !s.cancelling));
        if outcome == 1 {
            assert_eq!(e.read_with(cx, |s, _| s.step), Step::Hotkey);
        }
        h.hold.set(false);
        e.update(cx, |s, cx| s.visit(Step::Calendar, cx));
        assert_eq!(e.read_with(cx, |s, _| s.step), Step::Calendar);
        e.update(cx, |s, cx| s.visit(Step::Model, cx));
        draw(cx);
    }
}

#[test]
fn reserved_terminal_slot_rejects_stale_replies_and_recovers_invalid_projection() {
    let inbox = Inbox::default();
    let mut session = Session::default();
    session.attach();
    let old = session.begin().unwrap();
    session.invalidate();
    let ticket = session.begin().unwrap();
    let mut invalid = fixtures::snapshot();
    invalid.hardware_summary = Some("x".repeat(513));
    super::inbox::reply(&inbox, ticket, Ok(invalid));
    super::inbox::reply(&inbox, old, Err(Problem::Unavailable));
    for event in inbox.drain() {
        session.event(event);
    }
    assert!(!session.pending);
    assert_eq!(session.problem, Some(Problem::Invalid));
    let retry = session.begin().unwrap();
    super::inbox::reply(&inbox, retry, Ok(fixtures::snapshot()));
    for event in inbox.drain() {
        session.event(event);
    }
    assert!(!session.pending);
    assert!(session.problem.is_none());
    inbox.close();
    super::inbox::reply(&inbox, retry, Err(Problem::Unavailable));
    assert!(inbox.drain().is_empty());
}
