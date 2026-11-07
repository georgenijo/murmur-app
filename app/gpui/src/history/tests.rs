use super::{History, fixtures::*, host::Authority, inbox::Inbox, model::*};
use gpui::{Entity, TestAppContext, VisualTestContext, prelude::*};
use std::{
    rc::Rc,
    time::{Duration, Instant},
};
fn active() -> Model {
    let mut m = Model::default();
    m.attach();
    let t = m.ticket();
    assert!(m.event(Event::Read {
        ticket: t,
        revision: 1,
        clear_epoch: 0,
        result: Ok(snapshot())
    }));
    m.set_clock(clock());
    m
}
#[test]
fn complete_query_stable_order_combined_filters_and_unicode_highlights() {
    let mut m = active();
    assert_eq!(m.matches.len(), 200);
    assert_eq!(m.entries[m.matches[0]].key, 200);
    let mut q = Query {
        text: "VIOLET lighthouse".into(),
        source: Source::Files,
        ..Default::default()
    };
    m.set_query(q.clone());
    assert_eq!(m.matches.len(), 66);
    q.pinned = true;
    q.date = Date::Week;
    m.set_query(q);
    assert_eq!(m.matches.len(), 1);
    assert!(
        m.query
            .filter_label()
            .contains("Files · Past 7 days · Pinned only")
    );
    for (text, query) in [
        ("therein", "there here"),
        ("İstanbul Straße 🐱 café", "i café 🐱"),
        ("[x] + (a)", "[x] +"),
    ] {
        let ranges = highlights(text, query);
        assert!(!ranges.is_empty());
        for r in ranges {
            assert!(text.is_char_boundary(r.start) && text.is_char_boundary(r.end));
        }
    }
    assert!(highlights(&"x ".repeat(10_000), "x").len() <= 200);
}
#[test]
fn revisions_clear_epochs_notifications_and_attachment_admit_only_current_work() {
    let mut m = active();
    let stale = m.ticket();
    assert!(m.event(Event::Changed {
        attachment: stale.attachment,
        revision: 2,
        clear_epoch: 1,
        snapshot: Snapshot {
            entries: vec![],
            retain_history: false,
            retry_import: false
        }
    }));
    assert!(!m.event(Event::Read {
        ticket: stale,
        revision: 99,
        clear_epoch: 0,
        result: Ok(snapshot())
    }));
    assert!(!m.event(Event::Changed {
        attachment: stale.attachment,
        revision: 3,
        clear_epoch: 0,
        snapshot: snapshot()
    }));
    let t = m.ticket();
    assert!(!m.event(Event::Changed {
        attachment: t.attachment,
        revision: 2,
        clear_epoch: 1,
        snapshot: snapshot()
    }));
    m.detach();
    m.attach();
    assert!(!m.event(Event::Action {
        ticket: t,
        outcome: Outcome::Copied
    }));
    assert!(m.entries.is_empty());
}
#[test]
fn delete_timeout_query_and_data_changes_disarm_exact_scope_and_leases() {
    let mut m = active();
    let now = Instant::now();
    assert!(!m.arm(DeleteScope::Shown, now));
    assert!(m.arm(DeleteScope::Shown, now + Duration::from_secs(3)));
    m.expire(now + Duration::from_secs(4));
    assert!(m.confirmation.is_none());
    m.arm(DeleteScope::All, now);
    m.set_query(Query {
        text: "older".into(),
        ..Default::default()
    });
    assert!(m.confirmation.is_none());
    let a = Authority::default();
    a.sync(&m);
    let lease = a.lease(m.ticket());
    assert!(lease.is_current());
    let t = m.ticket();
    m.event(Event::Changed {
        attachment: t.attachment,
        revision: 2,
        clear_epoch: 0,
        snapshot: snapshot(),
    });
    a.sync(&m);
    assert!(!lease.is_current());
    m.arm(DeleteScope::All, now);
    m.detach();
    assert!(m.confirmation.is_none() && m.entries.is_empty());
}
#[test]
fn export_entire_matching_set_excludes_teaching_scope_and_raw_only_in_json() {
    let m = active();
    let refs = m.matches.iter().map(|&i| &m.entries[i]).collect::<Vec<_>>();
    for format in Format::ALL {
        let text = export(&refs, format, "2026-11-07T00:00:00Z", |_| {
            "2026-11-07 00:00:00".into()
        })
        .unwrap();
        assert!(
            !text.contains("synthetic.private.scope")
                && !text.contains("/synthetic/private")
                && !text.contains("teachingContext")
        );
        assert!(text.contains("Synthetic transcript 1:"));
        assert_eq!(
            text.contains("synthetic raw recognition"),
            format == Format::Json
        );
        if format == Format::Json {
            let v: serde_json::Value = serde_json::from_str(&text).unwrap();
            assert_eq!(v["count"], 200);
            assert_eq!(v["entries"][0]["id"], "synthetic-200");
        }
    }
}
#[test]
fn inbox_is_bounded_and_terminal_with_no_payload_debug() {
    let i = Inbox::default();
    let t = active().ticket();
    for _ in 0..8 {
        assert!(
            i.publish(Event::Action {
                ticket: t,
                outcome: Outcome::Copied
            })
            .is_ok()
        );
    }
    assert!(
        i.publish(Event::Action {
            ticket: t,
            outcome: Outcome::Copied
        })
        .is_err()
    );
    assert_eq!(i.drain().len(), 8);
    i.close();
    assert!(
        i.publish(Event::Read {
            ticket: t,
            revision: 2,
            clear_epoch: 0,
            result: Ok(snapshot())
        })
        .is_err()
    );
    let mut s = snapshot();
    s.entries.push(entry(201));
    assert!(!valid(&s));
}
fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|w, cx| w.draw(cx).clear(cx));
}
fn harness(cx: &mut TestAppContext) -> (Entity<History>, Rc<FixtureHost>, &mut VisualTestContext) {
    cx.update(crate::init);
    let host = Rc::new(FixtureHost::default());
    let mut entity = None;
    let (_, cx) = cx.add_window_view(|w, cx| {
        let d = cx.new(|cx| History::new(host.clone(), w, cx));
        entity = Some(d.clone());
        gpui_base::Root::new(d, w, cx)
    });
    let entity = entity.unwrap();
    entity.update(cx, |s, cx| {
        s.fixture = true;
        s.attach(cx);
        s.drain(cx);
        s.model.set_clock(clock());
        s.prepare(cx);
    });
    draw(cx);
    (entity, host, cx)
}
#[gpui::test]
fn host_subscribes_before_read_keyboard_virtualizes_and_copies_full_text(cx: &mut TestAppContext) {
    let (e, host, cx) = harness(cx);
    assert_eq!(host.calls.borrow()[0], "subscribe");
    let painted = e.read_with(cx, |s, _| s.rendered.len());
    assert!(painted > 0 && painted < 20, "visible rows {painted}");
    cx.update(|w, cx| {
        e.update(cx, |s, cx| w.focus(&s.focus, cx));
    });
    cx.simulate_keystrokes("home space");
    e.update(cx, |s, cx| s.drain(cx));
    draw(cx);
    assert_eq!(*host.clipboard.borrow(), entry(200).text);
    cx.simulate_keystrokes("end");
    draw(cx);
    assert_eq!(e.read_with(cx, |s, _| s.model.selected), Some(1));
    assert!(e.read_with(cx, |s, _| s.rendered.start) > 150);
    cx.simulate_keystrokes("right");
    draw(cx);
    assert_eq!(e.read_with(cx, |s, _| s.model.detail), Some(1));
    cx.simulate_keystrokes("escape");
    draw(cx);
    assert_eq!(e.read_with(cx, |s, _| s.model.detail), None);
    e.update(cx, |s, _| s.detach());
    assert_eq!(host.stops.get(), 1);
    assert!(e.read_with(cx, |s, _| s.inbox.is_closed()));
}
#[gpui::test]
fn search_shortcut_filters_full_set_save_cancel_and_delete_scope(cx: &mut TestAppContext) {
    let (e, host, cx) = harness(cx);
    cx.update(|w, cx| {
        e.update(cx, |s, cx| w.focus(&s.focus, cx));
    });
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("transcript 199:");
    draw(cx);
    assert_eq!(e.read_with(cx, |s, _| s.model.matches.len()), 1);
    cx.simulate_keystrokes("escape");
    draw(cx);
    assert_eq!(e.read_with(cx, |s, _| s.model.matches.len()), 200);
    cx.update(|w, cx| e.update(cx, |s, cx| s.action("save-2", w, cx)));
    e.update(cx, |s, cx| s.drain(cx));
    assert_eq!(host.saved.borrow().len(), 1);
    host.cancel_save.set(true);
    cx.update(|w, cx| e.update(cx, |s, cx| s.action("save-2", w, cx)));
    e.update(cx, |s, cx| s.drain(cx));
    assert_eq!(host.saved.borrow().len(), 1);
    cx.update(|w, cx| {
        e.update(cx, |s, cx| {
            s.delete(DeleteScope::All, w, cx);
            s.delete(DeleteScope::All, w, cx);
        })
    });
    e.update(cx, |s, cx| s.drain(cx));
    assert!(host.rows.borrow().is_empty());
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
    fn named(t: &TreeUpdate, name: &str) -> NodeId {
        t.nodes
            .iter()
            .find(|(_, n)| n.label() == Some(name))
            .unwrap_or_else(|| panic!("missing synthetic control {name}"))
            .0
    }
    #[gpui::test]
    fn native_tree_bounds_rows_exposes_search_filters_detail_and_nested_actions(
        cx: &mut TestAppContext,
    ) {
        let (e, host, cx) = harness(cx);
        let t = tree(cx);
        assert!(t.nodes.len() < 250);
        named(&t, "Search transcripts");
        click(cx, named(&t, "Filter transcripts: Everything"));
        let t = tree(cx);
        named(&t, "Show transcripts");
        named(&t, "From date");
        named(&t, "Pinned only");
        let t = tree(cx);
        click(cx, named(&t, "Show more for transcript 1"));
        let t = tree(cx);
        named(&t, "Transcript detail");
        named(&t, "Transcript text page 1 of 4");
        named(&t, "Correct & teach…");
        assert!(
            host.clipboard.borrow().is_empty(),
            "nested action copied row"
        );
        click(cx, named(&t, "Copy full transcript"));
        e.update(cx, |s, cx| s.drain(cx));
        let t = tree(cx);
        named(&t, "Copied");
        assert_eq!(*host.clipboard.borrow(), entry(200).text);
        click(cx, named(&t, "Delete transcript…"));
        let t = tree(cx);
        named(&t, "Confirm deletion");
        click(cx, named(&t, "Cancel deletion"));
        assert_eq!(host.rows.borrow().len(), 200);
    }
}

#[gpui::test]
fn subscription_notifications_precede_hydration_and_hide_revokes_deferred_effects(
    cx: &mut TestAppContext,
) {
    use super::host::{Host, Request, RequestKind, Subscription};
    use std::cell::RefCell;
    #[derive(Default)]
    struct Probe {
        requests: RefCell<Vec<(Request, Inbox)>>,
    }
    struct Stop;
    impl Subscription for Stop {
        fn stop(&mut self) {}
    }
    impl Host for Probe {
        fn subscribe(
            &self,
            _: murmur_core::events::Consumer,
            attachment: u64,
            i: Inbox,
        ) -> Box<dyn Subscription> {
            assert!(
                i.publish(Event::Changed {
                    attachment,
                    revision: 5,
                    clear_epoch: 2,
                    snapshot: snapshot()
                })
                .is_ok()
            );
            Box::new(Stop)
        }
        fn request(&self, r: Request, i: Inbox) {
            self.requests.borrow_mut().push((r, i));
        }
    }
    cx.update(crate::init);
    let host = Rc::new(Probe::default());
    let mut entity = None;
    let (_, cx) = cx.add_window_view(|w, cx| {
        let e = cx.new(|cx| History::new(host.clone(), w, cx));
        entity = Some(e.clone());
        gpui_base::Root::new(e, w, cx)
    });
    let e = entity.unwrap();
    e.update(cx, |s, cx| s.attach(cx));
    assert_eq!(host.requests.borrow()[0].0.ticket.revision, 5);
    assert_eq!(host.requests.borrow()[0].0.ticket.clear_epoch, 2);
    assert!(host.requests.borrow()[0].0.lease.is_current());
    cx.update(|w, cx| {
        e.update(cx, |s, cx| {
            s.request(RequestKind::TranscribeFile);
            s.hide(w, cx);
        })
    });
    assert!(
        host.requests
            .borrow()
            .iter()
            .all(|(r, i)| !r.lease.is_current() && i.is_closed())
    );
    assert!(e.read_with(cx, |s, _| s.model.entries.is_empty()));
    e.update(cx, |s, cx| s.attach(cx));
    assert_eq!(host.requests.borrow().len(), 3);
    assert!(host.requests.borrow()[2].0.lease.is_current());
}

#[test]
fn a_new_clear_epoch_releases_old_text_even_if_refresh_fails_or_is_invalid() {
    for invalid in [false, true] {
        let mut m = active();
        let t = m.ticket();
        let result = if invalid {
            let mut s = snapshot();
            s.entries.push(entry(1));
            Ok(s)
        } else {
            Err(Problem::Unavailable)
        };
        assert!(m.event(Event::Read {
            ticket: t,
            revision: 2,
            clear_epoch: 1,
            result
        }));
        assert!(m.entries.is_empty() && m.detail.is_none());
        assert!(!m.event(Event::Changed {
            attachment: t.attachment,
            revision: 3,
            clear_epoch: 0,
            snapshot: snapshot()
        }));
    }
}
