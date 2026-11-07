use super::{Confirm, Diagnostics, fixtures, host::Authority, inbox::Inbox, model::*, reports};
use gpui::{Entity, Focusable, TestAppContext, VisualTestContext, prelude::*};
use reports::*;
use std::rc::Rc;
fn active() -> Model {
    let mut m = Model::default();
    m.attach();
    m
}
fn notify(m: &mut Model, lane: Lane, revision: u64, store_epoch: u64, payload: Payload) -> bool {
    m.event(Event::Notification {
        attachment: m.ticket(lane).attachment,
        lane,
        revision,
        store_epoch,
        payload,
    })
}
#[test]
fn admission_rejects_duplicate_revisions_old_hydrations_and_detached_generations() {
    let mut m = active();
    let t = m.begin_read(Lane::Events);
    assert!(notify(
        &mut m,
        Lane::Events,
        2,
        0,
        Payload::Event(fixtures::event(2))
    ));
    assert!(!m.event(Event::Read {
        ticket: t,
        revision: 3,
        payload: Payload::Events(vec![fixtures::event(1)])
    }));
    assert!(!notify(
        &mut m,
        Lane::Events,
        2,
        0,
        Payload::Event(fixtures::event(3))
    ));
    let t = m.begin_read(Lane::Events);
    assert!(m.event(Event::Read {
        ticket: t,
        revision: 3,
        payload: Payload::Events(vec![fixtures::event(1), fixtures::event(2)])
    }));
    m.detach();
    m.attach();
    assert!(!m.event(Event::Read {
        ticket: t,
        revision: 99,
        payload: Payload::Events(vec![])
    }));
    assert_eq!(m.events.len(), 2);
}
#[test]
fn clear_epoch_cannot_resurrect_pending_runs_and_domains_stay_independent() {
    let mut m = active();
    notify(
        &mut m,
        Lane::Events,
        1,
        0,
        Payload::Event(fixtures::event(1)),
    );
    notify(
        &mut m,
        Lane::Performance,
        1,
        0,
        fixtures::payload(Lane::Performance),
    );
    let old = m.begin_read(Lane::Performance);
    let attachment = old.attachment;
    assert!(m.event(Event::Cleared {
        attachment,
        lane: Lane::Performance,
        revision: 2,
        store_epoch: 1
    }));
    assert!(m.runs.is_empty() && m.samples.is_empty());
    assert_eq!(m.events.len(), 1);
    assert!(!m.event(Event::Read {
        ticket: old,
        revision: 2,
        payload: fixtures::payload(Lane::Performance)
    }));
    assert!(!notify(
        &mut m,
        Lane::Performance,
        3,
        0,
        Payload::Run(fixtures::run())
    ));
    assert!(notify(
        &mut m,
        Lane::Performance,
        4,
        1,
        Payload::Run(fixtures::run())
    ));
    assert!(m.event(Event::Cleared {
        attachment,
        lane: Lane::Events,
        revision: 2,
        store_epoch: 1
    }));
    assert!(m.events.is_empty());
    assert_eq!(m.runs.len(), 1);
}
#[test]
fn histories_and_render_inputs_are_bounded_and_filters_use_strict_correlation() {
    let mut m = active();
    for i in 1..=700 {
        notify(
            &mut m,
            Lane::Events,
            i,
            0,
            Payload::Event(fixtures::event(i)),
        );
    }
    assert_eq!(m.events.len(), 500);
    assert!(m.copy_events().contains("recording_id"));
    m.filters.correlation = "7".into();
    assert_eq!(m.filtered_events().len(), 500);
    for q in ["07", "7.0", "+7", "0", "9007199254740992"] {
        m.filters.correlation = q.into();
        assert!(m.filtered_events().is_empty());
    }
    m.filters.field = CorrelationField::Run;
    m.filters.correlation = "synthetic-run".into();
    assert_eq!(m.filtered_events().len(), 500);
    m.filters.streams.clear();
    assert!(m.copy_events().is_empty());
    assert!(Measurement::Unavailable("no samples").value().is_none());
    assert_eq!(Measurement::NotApplicable.display("ms"), "Not applicable");
}
#[test]
fn inbox_is_bounded_ordered_and_terminal_after_detach() {
    let inbox = Inbox::default();
    for i in 1..=64 {
        assert!(
            inbox
                .publish(Event::Notification {
                    attachment: 1,
                    lane: Lane::Events,
                    revision: i,
                    store_epoch: 0,
                    payload: Payload::Event(fixtures::event(i))
                })
                .is_ok()
        );
    }
    assert!(
        inbox
            .publish(Event::Action {
                ticket: Ticket {
                    attachment: 1,
                    lane: Lane::Events,
                    epoch: 0,
                    store_epoch: 0
                },
                success: true
            })
            .is_err()
    );
    assert_eq!(inbox.drain().len(), 64);
    inbox.close();
    assert!(inbox.is_closed());
    assert!(
        inbox
            .publish(Event::Action {
                ticket: Ticket {
                    attachment: 1,
                    lane: Lane::Events,
                    epoch: 0,
                    store_epoch: 0
                },
                success: true
            })
            .is_err()
    );
}
fn benchmark() -> serde_json::Value {
    serde_json::from_slice(include_bytes!(
        "../../../../tests/fixtures/diagnostic-reports/benchmark-v2.json"
    ))
    .unwrap()
}
fn evaluation() -> serde_json::Value {
    serde_json::from_slice(include_bytes!(
        "../../../../tests/fixtures/diagnostic-reports/evaluation-v1-deterministic.json"
    ))
    .unwrap()
}
fn parse(v: &serde_json::Value) -> Result<Report, ImportError> {
    Report::parse(&serde_json::to_vec(v).unwrap(), 0, Source::Imported)
}
#[test]
fn imports_all_retained_fixtures_and_strips_every_private_surface() {
    for bytes in [
        include_bytes!("../../../../tests/fixtures/diagnostic-reports/benchmark-v2.json")
            .as_slice(),
        include_bytes!("../../../../tests/fixtures/diagnostic-reports/benchmark-legacy.json")
            .as_slice(),
        include_bytes!(
            "../../../../tests/fixtures/diagnostic-reports/evaluation-v1-deterministic.json"
        )
        .as_slice(),
        include_bytes!("../../../../tests/fixtures/diagnostic-reports/evaluation-v1-hardware.json")
            .as_slice(),
    ] {
        let r = Report::parse(bytes, bytes.len(), Source::Imported)
            .ok()
            .unwrap();
        assert!(!r.normalized_for_test().contains("PRIVATE_"));
    }
}
#[test]
fn import_errors_are_stable_and_limits_precede_parsing() {
    assert_eq!(
        Report::parse(b"", 0, Source::Imported).err(),
        Some(ImportError::Empty)
    );
    assert_eq!(
        Report::parse(b"private malformed", 0, Source::Imported).err(),
        Some(ImportError::MalformedJson)
    );
    assert_eq!(
        Report::parse(b"{}", MAX_BYTES + 1, Source::Imported).err(),
        Some(ImportError::Oversized)
    );
    assert_eq!(
        Report::parse(&vec![b' '; MAX_BYTES + 1], 0, Source::Imported).err(),
        Some(ImportError::Oversized)
    );
    let mut b = benchmark();
    b["reportVersion"] = serde_json::json!(3);
    assert_eq!(parse(&b).err(), Some(ImportError::UnsupportedVersion));
    let mut e = evaluation();
    e["fixtureVersion"] = serde_json::json!(2);
    assert_eq!(parse(&e).err(), Some(ImportError::UnsupportedVersion));
    let mut b = benchmark();
    b["unknown_private"] = serde_json::json!("Synthetic secret");
    assert_eq!(parse(&b).err(), Some(ImportError::SchemaMismatch));
    let mut b = benchmark();
    b["results"][0]["warmMedianMs"] = serde_json::Value::Null;
    assert_eq!(parse(&b).err(), Some(ImportError::SchemaMismatch));
    let mut e = evaluation();
    e["cases"][0]["recognition"]["actualRaw"] = serde_json::Value::Null;
    assert_eq!(parse(&e).err(), Some(ImportError::SchemaMismatch));
    let mut b = benchmark();
    b["results"] = serde_json::json!(vec![b["results"][0].clone(); 65]);
    assert_eq!(parse(&b).err(), Some(ImportError::CollectionLimit));
    let mut b = benchmark();
    b["results"][0]["fixtures"] =
        serde_json::json!(vec![b["results"][0]["fixtures"][0].clone(); 513]);
    assert_eq!(parse(&b).err(), Some(ImportError::CollectionLimit));
    let mut e = evaluation();
    e["cases"][0]["transformation"]["stages"] = serde_json::json!(vec![
            e["cases"][0]["transformation"]["stages"][0].clone();
            129
        ]);
    assert_eq!(parse(&e).err(), Some(ImportError::CollectionLimit));
    let mut e = evaluation();
    e["cases"] = serde_json::json!(vec![serde_json::Value::Null; 10001]);
    assert_eq!(parse(&e).err(), Some(ImportError::CollectionLimit));
}
#[test]
fn parser_rejects_duplicate_ids_summary_inconsistency_and_invalid_tier_privacy() {
    let mut e = evaluation();
    e["cases"] = serde_json::json!([e["cases"][0], e["cases"][0]]);
    assert_eq!(parse(&e).err(), Some(ImportError::SchemaMismatch));
    let mut e = evaluation();
    e["summary"]["total"] = serde_json::json!(2);
    assert_eq!(parse(&e).err(), Some(ImportError::SchemaMismatch));
    let mut e = evaluation();
    e["privacy"]["containsRealUserData"] = serde_json::json!(false);
    assert_eq!(parse(&e).err(), Some(ImportError::SchemaMismatch));
    let mut e = evaluation();
    e["tier"] = serde_json::json!("hardware");
    assert_eq!(parse(&e).err(), Some(ImportError::SchemaMismatch));
}
#[test]
fn compatibility_precedes_deltas_and_warnings_disable_recommendations() {
    let b = benchmark();
    let baseline = parse(&b).ok().unwrap();
    let mut c = b.clone();
    c["results"][0]["warmMedianMs"] = serde_json::json!(99);
    let comparison = compare(&baseline, &parse(&c).ok().unwrap());
    assert!(!comparison.blocked());
    assert!(comparison.recommendations.is_some());
    assert!(
        comparison
            .metrics
            .iter()
            .any(|m| m.label == "warmMedianMs" && m.delta == -11.)
    );
    c["appVersion"] = serde_json::json!("synthetic-other");
    let comparison = compare(&baseline, &parse(&c).ok().unwrap());
    assert!(!comparison.blocked() && comparison.recommendations.is_none());
    for path in [
        "/preset",
        "/iterations",
        "/corpus/language",
        "/configuration/executionPath",
        "/results/0/accelerator",
    ] {
        let mut c = b.clone();
        *c.pointer_mut(path).unwrap() = if path == "/iterations" {
            serde_json::json!(4)
        } else {
            serde_json::json!(if path == "/preset" {
                "standard"
            } else {
                "synthetic-other"
            })
        };
        let comparison = compare(&baseline, &parse(&c).ok().unwrap());
        assert!(comparison.blocked() && comparison.metrics.is_empty());
    }
    assert!(compare(&baseline, &parse(&evaluation()).ok().unwrap()).blocked());
    let mut zero = b.clone();
    zero["sharedInitMs"] = serde_json::json!(0);
    let comparison = compare(&parse(&zero).ok().unwrap(), &baseline);
    assert!(
        comparison
            .metrics
            .iter()
            .find(|m| m.label == "sharedInitMs")
            .unwrap()
            .percent
            .is_none()
    );
}
#[test]
fn evaluation_comparison_covers_execution_failures_stages_and_signed_memory() {
    let a = evaluation();
    let mut b = a.clone();
    b["cases"][0]["latency"]["totalMs"] = serde_json::json!(9);
    b["cases"][0]["runtime"]["memoryDeltaMb"] = serde_json::json!(-1);
    let c = compare(&parse(&a).ok().unwrap(), &parse(&b).ok().unwrap());
    assert!(!c.blocked());
    assert!(c.metrics.iter().any(|m| m.label == "totalMs"));
    assert!(c.metrics.iter().any(|m| m.label == "durationUs"));
    b["cases"][0]["delivery"]["finalOnly"] = serde_json::json!(false);
    assert!(compare(&parse(&a).ok().unwrap(), &parse(&b).ok().unwrap()).blocked());
    let mut b = a.clone();
    b["cases"][0]["status"] = serde_json::json!("failed");
    b["cases"][0]["failures"] = serde_json::json!(["PRIVATE_FAILURE_SENTINEL"]);
    let r = parse(&b).ok().unwrap();
    assert!(!r.normalized_for_test().contains("PRIVATE_"));
    assert!(compare(&parse(&a).ok().unwrap(), &r).blocked());
}
#[test]
fn session_clear_and_recovery_invalidate_pending_imports_and_action_leases() {
    let mut m = active();
    let authority = Authority::default();
    let ticket = m.begin_read(Lane::Reports);
    authority.sync(&m);
    let lease = authority.lease(ticket);
    assert!(lease.is_current());
    m.clear_imports();
    authority.sync(&m);
    assert!(!lease.is_current());
    assert!(!m.event(Event::Import {
        ticket,
        result: parse(&benchmark()).map(Some)
    }));
    let t = m.begin_read(Lane::Reports);
    assert!(m.event(Event::Import {
        ticket: t,
        result: parse(&benchmark()).map(Some)
    }));
    assert_eq!(m.reports.len(), 1);
    m.detach();
    authority.sync(&m);
    assert!(!authority.lease(t).is_current());
    assert!(
        !Health {
            available: false,
            recommended: Recovery::Restart,
            problem: Some(StoreProblem::SchemaMigration),
            skipped: 0,
            recent_write_failed: false,
            recovered: false
        }
        .can_recover()
    );
}
fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|w, cx| w.draw(cx).clear(cx));
}
fn harness(cx: &mut TestAppContext) -> (Entity<Diagnostics>, &mut VisualTestContext) {
    cx.update(crate::init);
    let mut entity = None;
    let (_, cx) = cx.add_window_view(|w, cx| {
        let d = cx.new(|cx| {
            let mut d = Diagnostics::new(Rc::new(fixtures::FixtureHost::default()), w, cx);
            d.fixture = true;
            d
        });
        entity = Some(d.clone());
        gpui_base::Root::new(d, w, cx)
    });
    let entity = entity.unwrap();
    entity.update(cx, |s, cx| {
        s.attach(cx);
        s.drain(cx);
        s.sync(cx);
    });
    draw(cx);
    (entity, cx)
}
#[gpui::test]
fn tabs_lists_charts_and_data_pickers_support_keyboard_without_native_windows(
    cx: &mut TestAppContext,
) {
    let (entity, cx) = harness(cx);
    let tabs = entity.read_with(cx, |s, _| s.tabs.clone());
    cx.update(|w, cx| w.focus(&tabs.read(cx).focus_handle(cx), cx));
    cx.simulate_keystrokes("right");
    draw(cx);
    assert_eq!(entity.read_with(cx, |s, _| s.model.tab), Tab::Runs);
    cx.simulate_keystrokes("end");
    draw(cx);
    assert_eq!(entity.read_with(cx, |s, _| s.model.tab), Tab::Dictation);
    cx.simulate_keystrokes("home");
    draw(cx);
    let list = entity.read_with(cx, |s, _| s.events.clone());
    assert_eq!(list.read_with(cx, |s, _| s.rendered_len()), 100);
    cx.update(|w, cx| w.focus(&list.read(cx).focus_handle(cx), cx));
    cx.simulate_keystrokes("home space");
    draw(cx);
    assert!(list.read_with(cx, |s, _| s.is_expanded(61)));
    assert!(!list.read_with(cx, |s, _| s.following));
    cx.simulate_keystrokes("escape");
    draw(cx);
    assert!(!list.read_with(cx, |s, _| s.is_expanded(61)));
    list.update(cx, |s, cx| s.resume(cx));
    assert!(list.read_with(cx, |s, _| s.following));
    entity.update(cx, |s, cx| {
        s.model.tab = Tab::Performance;
        s.sync(cx);
    });
    draw(cx);
    let chart = entity.read_with(cx, |s, _| s.charts[0].clone());
    cx.update(|w, cx| w.focus(&chart.read(cx).focus_handle(cx), cx));
    cx.simulate_keystrokes("end home right");
    draw(cx);
}
#[gpui::test]
fn destructive_confirmation_cancel_and_hide_revoke_the_current_request(cx: &mut TestAppContext) {
    let (entity, cx) = harness(cx);
    cx.update(|w, cx| {
        entity.update(cx, |s, cx| {
            s.open_confirm(Confirm::Performance, Lane::Performance, w, cx)
        })
    });
    draw(cx);
    cx.simulate_keystrokes("escape");
    draw(cx);
    assert!(entity.read_with(cx, |s, _| s.confirm.is_none()));
    cx.update(|w, cx| {
        entity.update(cx, |s, cx| {
            s.open_confirm(Confirm::Events, Lane::Events, w, cx)
        })
    });
    draw(cx);
    cx.update(|w, cx| {
        entity.update(cx, |s, cx| {
            let t = s.model.begin_read(Lane::Events);
            s.receive(
                Event::Read {
                    ticket: t,
                    revision: 2,
                    payload: Payload::Events(vec![fixtures::event(300)]),
                },
                cx,
            );
            s.confirm(w, cx);
        })
    });
    draw(cx);
    assert_eq!(entity.read_with(cx, |s, _| s.model.events.len()), 1);
    assert!(entity.read_with(cx, |s, _| s.model.notice.unwrap().contains("changed")));
    entity.update(cx, |s, _| s.detach());
    assert!(entity.read_with(cx, |s, _| s.inbox.is_closed()));
    entity.update(cx, |s, cx| {
        s.attach(cx);
        s.drain(cx);
        s.drain(cx);
        s.sync(cx);
    });
    assert_eq!(entity.read_with(cx, |s, _| s.model.events.len()), 160);
}
#[cfg(feature = "a11y-tests")]
mod ax {
    use super::*;
    use gpui::accesskit::{ActionRequest, Node, NodeId, TreeId, TreeUpdate};
    use gpui::{AccessibleAction, Role};
    fn tree(cx: &mut VisualTestContext) -> TreeUpdate {
        let w = cx.update(|w, _| w.window_handle());
        cx.activate_accessibility(w);
        draw(cx);
        cx.accessibility_tree(w)
    }
    fn named<'a>(tree: &'a TreeUpdate, label: &str) -> (NodeId, &'a Node) {
        let (id, n) = tree
            .nodes
            .iter()
            .find(|(_, n)| n.label() == Some(label))
            .unwrap_or_else(|| panic!("missing synthetic AX label: {label}"));
        (*id, n)
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
    fn emitted_tree_exposes_tabs_bounded_disclosures_tables_progress_and_confirmation(
        cx: &mut TestAppContext,
    ) {
        let (entity, cx) = harness(cx);
        let t = tree(cx);
        assert_eq!(named(&t, "Diagnostics views").1.role(), Role::TabList);
        let events = entity.read_with(cx, |s, _| s.model.events[60].label());
        let (row, node) = named(&t, &events);
        assert_eq!(node.role(), Role::Button);
        assert_eq!(node.is_expanded(), Some(false));
        click(cx, row);
        let t = tree(cx);
        assert_eq!(named(&t, &events).1.is_expanded(), Some(true));
        assert!(t.nodes.iter().any(|(_, n)| n.label() == Some("Event data")));
        click(cx, named(&t, "Runs").0);
        let t = tree(cx);
        assert_eq!(named(&t, "Performance runs").1.role(), Role::Table);
        assert_eq!(named(&t, "Performance runs").1.column_count(), Some(7));
        click(
            cx,
            named(&t, "View details for Dictation · synthetic-run").0,
        );
        let t = tree(cx);
        assert_eq!(named(&t, "Run stage timings").1.role(), Role::Table);
        click(cx, named(&t, "Show correlated events").0);
        let t = tree(cx);
        assert_eq!(
            entity.read_with(cx, |s, _| s.model.filters.correlation.clone()),
            "7"
        );
        click(cx, named(&t, "Clear events…").0);
        let t = tree(cx);
        assert_eq!(named(&t, "Clear events?").1.role(), Role::AlertDialog);
        assert!(
            !t.nodes
                .iter()
                .any(|(_, n)| n.label() == Some("Copy filtered events"))
        );
        click(cx, named(&t, "Cancel").0);
        entity.update(cx, |s, cx| {
            s.model.tab = Tab::Events;
            s.model.loading.insert(Lane::Events);
            cx.notify();
        });
        let t = tree(cx);
        assert_eq!(
            named(&t, "Loading events").1.role(),
            Role::ProgressIndicator
        );
    }
    #[gpui::test]
    fn compare_gate_and_private_review_have_no_arming_or_automatic_upload_action(
        cx: &mut TestAppContext,
    ) {
        let (entity, cx) = harness(cx);
        entity.update(cx, |s, cx| {
            s.model.tab = Tab::Compare;
            s.sync(cx);
        });
        let t = tree(cx);
        assert!(named(&t, "Comparison deltas").1.row_count().unwrap() > 1);
        entity.update(cx, |s, cx| {
            s.model.tab = Tab::Dictation;
            s.sync(cx);
        });
        let t = tree(cx);
        assert!(
            !t.nodes
                .iter()
                .any(|(_, n)| n.label().is_some_and(|s| s.starts_with("Arm")
                    || s.starts_with("Disarm")
                    || s.starts_with("Upload")))
        );
        let id = t
            .nodes
            .iter()
            .find(|(_, n)| n.label().is_some_and(|s| s.starts_with("Review capture")))
            .unwrap()
            .0;
        click(cx, id);
        entity.update(cx, |s, cx| {
            s.drain(cx);
            s.sync(cx);
        });
        let t = tree(cx);
        click(cx, named(&t, "Upload reviewed private capture…").0);
        let t = tree(cx);
        assert_eq!(
            named(&t, "Upload this reviewed private capture?").1.role(),
            Role::AlertDialog
        );
    }
}
#[test]
fn private_summaries_do_not_hydrate_text_and_stale_detail_does_not_return_after_back() {
    let mut m = active();
    notify(
        &mut m,
        Lane::Captures,
        1,
        0,
        fixtures::payload(Lane::Captures),
    );
    assert!(m.captures.iter().all(|c| c.sections.is_empty()));
    m.selected_capture = Some("synthetic-capture".into());
    let ticket = m.begin_read(Lane::Captures);
    m.selected_capture = None;
    assert!(m.event(Event::Read {
        ticket,
        revision: 2,
        payload: Payload::CaptureDetail(fixtures::capture("synthetic-capture".into()))
    }));
    assert!(m.capture_detail.is_none());
    m.selected_capture = Some("synthetic-capture".into());
    let ticket = m.begin_read(Lane::Captures);
    m.detach();
    assert!(!m.event(Event::Read {
        ticket,
        revision: 3,
        payload: Payload::CaptureDetail(fixtures::capture("synthetic-capture".into()))
    }));
}
#[test]
fn imports_enforce_session_limit_and_cannot_masquerade_as_saved_reports() {
    let mut m = active();
    let report = parse(&benchmark()).ok().unwrap();
    for _ in 0..=MAX_IMPORTS {
        let ticket = m.begin_read(Lane::Reports);
        m.event(Event::Import {
            ticket,
            result: Ok(Some(report.clone())),
        });
    }
    assert_eq!(m.reports.len(), 20);
    assert_eq!(m.import_error, Some(ImportError::SessionLimit));
    m.clear_imports();
    assert!(m.reports.is_empty());
    let local = Report::parse(&serde_json::to_vec(&benchmark()).unwrap(), 0, Source::Local)
        .ok()
        .unwrap();
    let ticket = m.begin_read(Lane::Reports);
    m.event(Event::Import {
        ticket,
        result: Ok(Some(local)),
    });
    assert!(m.reports.is_empty());
}
#[gpui::test]
fn host_subscribes_before_reads_and_stop_revokes_every_retained_producer(cx: &mut TestAppContext) {
    use super::host::{Host, Request, Subscription};
    use std::cell::{Cell, RefCell};
    #[derive(Default)]
    struct Probe {
        calls: RefCell<Vec<&'static str>>,
        stops: Rc<Cell<usize>>,
        requests: RefCell<Vec<(Request, Inbox)>>,
    }
    struct Stop(Rc<Cell<usize>>);
    impl Subscription for Stop {
        fn stop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }
    impl Host for Probe {
        fn subscribe(
            &self,
            c: murmur_core::events::Consumer,
            _: u64,
            _: Inbox,
        ) -> Box<dyn Subscription> {
            assert_eq!(c, murmur_core::events::Consumer::Diagnostics);
            self.calls.borrow_mut().push("subscribe");
            Box::new(Stop(self.stops.clone()))
        }
        fn request(&self, r: Request, i: Inbox) {
            self.calls.borrow_mut().push("read");
            self.requests.borrow_mut().push((r, i));
        }
    }
    cx.update(crate::init);
    let host = Rc::new(Probe::default());
    let mut entity = None;
    let (_, cx) = cx.add_window_view(|w, cx| {
        let d = cx.new(|cx| Diagnostics::new(host.clone(), w, cx));
        entity = Some(d.clone());
        gpui_base::Root::new(d, w, cx)
    });
    let entity = entity.unwrap();
    entity.update(cx, |s, cx| s.attach(cx));
    assert_eq!(host.calls.borrow()[0], "subscribe");
    assert_eq!(host.requests.borrow().len(), 7);
    assert!(
        host.requests
            .borrow()
            .iter()
            .all(|(r, _)| r.lease.is_current())
    );
    entity.update(cx, |s, _| {
        s.detach();
        s.detach();
    });
    assert_eq!(host.stops.get(), 1);
    assert!(
        host.requests
            .borrow()
            .iter()
            .all(|(r, i)| !r.lease.is_current() && i.is_closed())
    );
    entity.update(cx, |s, cx| s.attach(cx));
    assert_eq!(host.calls.borrow()[8], "subscribe");
    assert_eq!(host.stops.get(), 1);
}
#[test]
fn corrupt_invalid_and_newer_schema_health_have_distinct_guidance_and_permissions() {
    let mut health = Health {
        available: false,
        recommended: Recovery::Reinitialize,
        problem: Some(StoreProblem::CorruptIntegrity),
        skipped: 0,
        recent_write_failed: false,
        recovered: false,
    };
    let corrupt = health.guidance();
    assert!(health.can_recover());
    health.problem = Some(StoreProblem::InvalidRecord);
    assert_ne!(corrupt, health.guidance());
    health.problem = Some(StoreProblem::SchemaMigration);
    health.recommended = Recovery::Restart;
    assert!(!health.can_recover());
    assert!(health.guidance().contains("cannot be reinitialized"));
}
#[gpui::test]
fn diagnostics_follows_the_hosts_live_palette_without_overwriting_other_windows(
    cx: &mut TestAppContext,
) {
    let (entity, cx) = harness(cx);
    entity.update(cx, |s, _| s.fixture = false);
    cx.update(|_, cx| crate::tokens::apply_appearance(true, cx));
    draw(cx);
    cx.update(|_, cx| assert!(crate::tokens::Palette::get(cx).surface.l < 0.5));
    cx.update(|_, cx| crate::tokens::apply_appearance(false, cx));
    draw(cx);
    cx.update(|_, cx| assert!(crate::tokens::Palette::get(cx).surface.l > 0.5));
}
