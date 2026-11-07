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
    assert_eq!(entity.read_with(cx, |s, _| s.model.tab()), Tab::Runs);
    cx.simulate_keystrokes("end");
    draw(cx);
    assert_eq!(entity.read_with(cx, |s, _| s.model.tab()), Tab::Dictation);
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
        s.model.navigate(Tab::Performance);
        s.sync(cx);
    });
    draw(cx);
    let chart = entity.read_with(cx, |s, _| s.charts[0].clone());
    cx.update(|w, cx| w.focus(&chart.read(cx).focus_handle(cx), cx));
    cx.simulate_keystrokes("end home right");
    draw(cx);
}

fn host_notification(
    entity: &Entity<Diagnostics>,
    cx: &mut VisualTestContext,
    lane: Lane,
    revision: u64,
) {
    entity.update(cx, |s, cx| {
        s.receive(
            Event::Notification {
                attachment: s.model.ticket(lane).attachment,
                lane,
                revision,
                store_epoch: 0,
                payload: fixtures::payload(lane),
            },
            cx,
        );
    });
    draw(cx);
}

#[gpui::test]
fn report_picker_keyboard_commit_survives_unrelated_host_notifications(cx: &mut TestAppContext) {
    let (entity, cx) = harness(cx);
    entity.update(cx, |s, cx| {
        s.model.navigate(Tab::Compare);
        s.sync(cx);
    });
    draw(cx);
    let mut revision = 100;
    for baseline in [true, false] {
        let picker = entity.read_with(cx, |s, _| {
            if baseline {
                s.baseline.clone()
            } else {
                s.candidate.clone()
            }
        });
        // Also cover an explicitly cleared committed selection: None must not
        // reset the uncommitted highlight to the first report on every sync.
        for cleared in [false, true] {
            if cleared {
                entity.update(cx, |s, cx| {
                    if baseline {
                        s.model.baseline = None;
                    } else {
                        s.model.candidate = None;
                    }
                    s.sync(cx);
                });
            }
            let (old, expected) = entity.read_with(cx, |s, _| {
                let old = if baseline {
                    &s.model.baseline
                } else {
                    &s.model.candidate
                };
                let index = old
                    .as_ref()
                    .and_then(|id| s.model.reports.iter().position(|r| r.id() == id))
                    .unwrap_or(0);
                (
                    old.clone(),
                    s.model.reports[(index + 1) % s.model.reports.len()]
                        .id()
                        .to_owned(),
                )
            });
            cx.update(|w, cx| picker.update(cx, |s, cx| s.open(w, cx)));
            draw(cx);
            cx.simulate_keystrokes("down");
            draw(cx);
            for lane in [Lane::Events, Lane::Performance, Lane::Reports] {
                host_notification(&entity, cx, lane, revision);
                revision += 1;
                assert!(picker.read_with(cx, |s, _| s.is_open()));
                assert_eq!(
                    picker.read_with(cx, |s, _| s.value().map(ToString::to_string)),
                    old
                );
            }
            cx.simulate_keystrokes("enter");
            draw(cx);
            assert!(!picker.read_with(cx, |s, _| s.is_open()));
            assert_eq!(
                picker.read_with(cx, |s, _| s.value().map(ToString::to_string)),
                Some(expected.clone())
            );
            entity.read_with(cx, |s, _| {
                assert_eq!(
                    if baseline {
                        &s.model.baseline
                    } else {
                        &s.model.candidate
                    },
                    &Some(expected)
                );
            });
        }
    }
}

#[gpui::test]
fn report_removal_closes_open_picker_and_clears_missing_committed_identity(
    cx: &mut TestAppContext,
) {
    let (entity, cx) = harness(cx);
    for baseline in [true, false] {
        entity.update(cx, |s, cx| {
            s.model.navigate(Tab::Compare);
            s.receive(
                Event::Notification {
                    attachment: s.model.ticket(Lane::Reports).attachment,
                    lane: Lane::Reports,
                    revision: if baseline { 100 } else { 102 },
                    store_epoch: 0,
                    payload: fixtures::payload(Lane::Reports),
                },
                cx,
            );
            let id = s.model.reports[0].id().to_owned();
            if baseline {
                s.model.baseline = Some(id);
            } else {
                s.model.candidate = Some(id);
            }
            s.sync(cx);
        });
        let picker = entity.read_with(cx, |s, _| {
            if baseline {
                s.baseline.clone()
            } else {
                s.candidate.clone()
            }
        });
        cx.update(|w, cx| picker.update(cx, |s, cx| s.open(w, cx)));
        draw(cx);
        cx.simulate_keystrokes("down");
        draw(cx);
        let expected = entity.update(cx, |s, cx| {
            let remaining = s.model.reports[1].clone();
            let expected = remaining.id().to_owned();
            s.receive(
                Event::Notification {
                    attachment: s.model.ticket(Lane::Reports).attachment,
                    lane: Lane::Reports,
                    revision: if baseline { 101 } else { 103 },
                    store_epoch: 0,
                    payload: Payload::Reports(vec![remaining]),
                },
                cx,
            );
            assert!(
                if baseline {
                    &s.model.baseline
                } else {
                    &s.model.candidate
                }
                .is_none()
            );
            assert!(s.model.notice.unwrap().contains("no longer available"));
            expected
        });
        draw(cx);
        assert!(!picker.read_with(cx, |s, _| s.is_open()));
        assert!(picker.read_with(cx, |s, _| s.value().is_none()));
        assert!(cx.update(|w, cx| picker.read(cx).focus_handle(cx).is_focused(w)));
        // A pending Enter cannot commit a disappeared report. It reopens the
        // reconciled list; a second Enter deliberately chooses the valid report.
        cx.simulate_keystrokes("enter");
        draw(cx);
        assert!(picker.read_with(cx, |s, _| s.is_open() && s.value().is_none()));
        cx.simulate_keystrokes("enter");
        draw(cx);
        entity.read_with(cx, |s, _| {
            assert_eq!(
                if baseline {
                    &s.model.baseline
                } else {
                    &s.model.candidate
                },
                &Some(expected)
            )
        });
    }
}

#[gpui::test]
fn unrelated_sync_preserves_tabs_filters_disclosures_and_text_caret(cx: &mut TestAppContext) {
    let (entity, cx) = harness(cx);
    let list = entity.read_with(cx, |s, _| s.events.clone());
    cx.update(|w, cx| w.focus(&list.read(cx).focus_handle(cx), cx));
    cx.simulate_keystrokes("home space");
    draw(cx);
    host_notification(&entity, cx, Lane::Performance, 100);
    assert!(list.read_with(cx, |s, _| s.is_expanded(61) && !s.following));
    cx.simulate_keystrokes("space");
    draw(cx);
    assert!(!list.read_with(cx, |s, _| s.is_expanded(61)));
    let field = entity.read_with(cx, |s, _| s.correlation.clone());
    cx.update(|w, cx| w.focus(&field.read(cx).focus_handle(cx), cx));
    cx.simulate_input("synthetic");
    cx.simulate_keystrokes("left");
    host_notification(&entity, cx, Lane::Performance, 101);
    cx.simulate_input("X");
    draw(cx);
    assert_eq!(
        field.read_with(cx, |s, _| s.value().to_string()),
        "synthetiXc"
    );
    entity.read_with(cx, |s, _| {
        assert_eq!(s.model.filters.correlation, "synthetiXc")
    });
    for (index, (tab, filter)) in entity
        .read_with(cx, |s, _| {
            vec![
                (Tab::Events, s.field.clone()),
                (Tab::Runs, s.kind.clone()),
                (Tab::Runs, s.outcome.clone()),
                (Tab::Latency, s.build.clone()),
            ]
        })
        .into_iter()
        .enumerate()
    {
        entity.update(cx, |s, cx| {
            s.model.navigate(tab);
            s.sync(cx);
        });
        cx.update(|w, cx| filter.update(cx, |s, cx| s.open(w, cx)));
        draw(cx);
        cx.simulate_keystrokes("end");
        host_notification(&entity, cx, Lane::Events, 200 + index as u64);
        assert!(filter.read_with(cx, |s, _| s.is_open()));
        cx.simulate_keystrokes("enter");
        draw(cx);
        assert!(filter.read_with(cx, |s, _| s.value().is_some()));
        entity.read_with(cx, |s, cx| {
            assert_eq!(
                s.tabs.read(cx).selected(),
                Tab::ALL.iter().position(|t| *t == tab).unwrap()
            )
        });
    }
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
    fn focused_label(t: &TreeUpdate) -> &str {
        t.nodes
            .iter()
            .find(|(id, _)| *id == t.focus)
            .unwrap()
            .1
            .label()
            .unwrap_or("")
    }

    #[gpui::test]
    fn tab_order_reaches_tabs_event_list_and_every_chart(cx: &mut TestAppContext) {
        let (entity, cx) = harness(cx);
        cx.update(|w, cx| {
            w.blur(cx);
            w.focus_next(cx);
        });
        assert_eq!(focused_label(&tree(cx)), "Events");
        for label in ["Copy filtered events", "Clear events…"] {
            cx.simulate_keystrokes("tab");
            assert_eq!(focused_label(&tree(cx)), label);
        }
        // Filters and text entry precede the single bounded-list stop.
        for _ in 0..Stream::ALL.len() + Level::ALL.len() + 2 {
            cx.simulate_keystrokes("tab");
            draw(cx);
        }
        let field = entity.read_with(cx, |s, cx| s.correlation.read(cx).focus_handle(cx));
        cx.update(|w, _| assert!(field.is_focused(w)));
        cx.simulate_keystrokes("tab");
        let list = entity.read_with(cx, |s, cx| s.events.read(cx).focus_handle(cx));
        cx.update(|w, _| assert!(list.is_focused(w)));
        cx.simulate_keystrokes("end space");
        assert!(entity.read_with(cx, |s, cx| s.events.read(cx).is_expanded(160)));
        cx.simulate_keystrokes("shift-tab");
        // Manual keyboard navigation reveals Resume before the list.
        assert_eq!(focused_label(&tree(cx)), "Resume auto-scroll");
        entity.update(cx, |s, cx| {
            s.model.navigate(Tab::Performance);
            s.sync(cx);
            cx.notify();
        });
        draw(cx);
        cx.update(|w, cx| {
            w.blur(cx);
            w.focus_next(cx);
        });
        cx.simulate_keystrokes("tab");
        assert_eq!(focused_label(&tree(cx)), "Refresh performance");
        let charts = entity.read_with(cx, |s, _| s.charts.clone());
        for chart in &charts {
            cx.simulate_keystrokes("tab");
            draw(cx);
            cx.update(|w, cx| assert!(chart.read(cx).focus_handle(cx).is_focused(w)));
            cx.simulate_keystrokes("right end home");
            draw(cx);
        }
        for chart in charts.iter().rev().skip(1) {
            cx.simulate_keystrokes("shift-tab");
            draw(cx);
            cx.update(|w, cx| assert!(chart.read(cx).focus_handle(cx).is_focused(w)));
        }
    }

    #[gpui::test]
    fn table_adapter_exposes_rows_columns_headers_and_keyboard_cell_indices(
        cx: &mut TestAppContext,
    ) {
        let (entity, cx) = harness(cx);
        entity.update(cx, |s, cx| {
            s.model.navigate(Tab::Latency);
            s.sync(cx);
            cx.notify();
        });
        draw(cx);
        cx.update(|w, cx| {
            w.blur(cx);
            w.focus_next(cx);
        });
        draw(cx);
        // Tab group, build selector, refresh/copy/clear, then the table.
        let mut reached = false;
        for _ in 0..12 {
            cx.simulate_keystrokes("tab");
            let t = tree(cx);
            if t.nodes
                .iter()
                .any(|(id, n)| *id == t.focus && n.role() == Role::ColumnHeader)
            {
                reached = true;
                break;
            }
        }
        assert!(reached, "latency table must have an ordinary Tab stop");
        let t = tree(cx);
        let table_id = named(&t, "UI latency edges").0;
        let projection = accesskit_macos::test_table(t, table_id);
        assert!(projection.0 > 1);
        assert!(projection.1 > 1);
        assert_eq!(projection.2, projection.1);
        assert_eq!(projection.4.len(), projection.0 * projection.1);
        cx.simulate_keystrokes("down right");
        let t = tree(cx);
        let cell = &t.nodes.iter().find(|(id, _)| *id == t.focus).unwrap().1;
        assert_eq!(cell.role(), Role::Cell);
        assert_eq!((cell.row_index(), cell.column_index()), (Some(1), Some(1)));
        cx.simulate_keystrokes("home");
        assert_eq!(focused_label(&tree(cx)), "Build / route");
        cx.simulate_keystrokes("end");
        let t = tree(cx);
        let cell = &t.nodes.iter().find(|(id, _)| *id == t.focus).unwrap().1;
        assert_eq!(
            (cell.row_index(), cell.column_index()),
            (Some(projection.0 - 1), Some(projection.1 - 1))
        );
    }

    #[gpui::test]
    fn confirmation_restores_keyboard_and_ax_origins_after_remount(cx: &mut TestAppContext) {
        let (entity, cx) = harness(cx);
        cx.update(|w, cx| {
            w.blur(cx);
            w.focus_next(cx);
        });
        draw(cx);
        cx.simulate_keystrokes("tab");
        draw(cx);
        cx.simulate_keystrokes("tab");
        draw(cx);
        assert_eq!(focused_label(&tree(cx)), "Clear events…");
        let keystroke = cx.update(|_, cx| {
            gpui::Keystroke::parse_with_os("space", cx.platform_info().os).unwrap()
        });
        cx.simulate_event(gpui::KeyDownEvent {
            keystroke: keystroke.clone(),
            is_held: false,
            prefer_character_input: false,
        });
        cx.simulate_event(gpui::KeyUpEvent { keystroke });
        draw(cx);
        assert!(
            entity.read_with(cx, |s, _| s.confirm.is_some()),
            "Space must open confirmation"
        );
        assert_eq!(focused_label(&tree(cx)), "Cancel");
        cx.simulate_keystrokes("tab");
        assert_eq!(focused_label(&tree(cx)), "Confirm");
        cx.simulate_keystrokes("shift-tab escape");
        draw(cx);
        cx.update(|w, cx| {
            w.simulate_next_frame(cx);
        });
        draw(cx);
        assert_eq!(focused_label(&tree(cx)), "Clear events…");
        for choice in ["Cancel", "Confirm"] {
            let t = tree(cx);
            click(cx, named(&t, "Clear events…").0);
            let t = tree(cx);
            click(cx, named(&t, choice).0);
            draw(cx);
            cx.update(|w, cx| {
                w.simulate_next_frame(cx);
            });
            draw(cx);
            assert_eq!(focused_label(&tree(cx)), "Clear events…");
        }
    }

    #[gpui::test]
    fn confirmation_falls_back_when_live_update_removes_the_origin(cx: &mut TestAppContext) {
        let (entity, cx) = harness(cx);
        entity.update(cx, |s, cx| {
            s.model.navigate(Tab::Dictation);
            s.sync(cx);
            cx.notify();
        });
        let t = tree(cx);
        click(cx, named(&t, "Delete local capture for recording 7…").0);
        entity.update(cx, |s, cx| {
            s.model.captures.clear();
            cx.notify();
        });
        draw(cx);
        cx.simulate_keystrokes("escape");
        draw(cx);
        cx.update(|w, cx| {
            w.simulate_next_frame(cx);
        });
        draw(cx);
        assert_eq!(focused_label(&tree(cx)), "Dictation");
    }

    #[gpui::test]
    fn minimum_window_scroll_reaches_events_and_resume_with_bounded_inner_list(
        cx: &mut TestAppContext,
    ) {
        let (entity, cx) = harness(cx);
        cx.simulate_resize(gpui::size(gpui::px(480.), gpui::px(440.)));
        entity.update(cx, |s, cx| {
            s.events.update(cx, |list, cx| {
                list.following = false;
                cx.notify();
            });
        });
        draw(cx);
        entity.update(cx, |s, cx| {
            assert!(s.panel_scroll.max_offset().y > gpui::px(0.));
            s.panel_scroll.scroll_to_bottom();
            cx.notify();
        });
        draw(cx);
        let t = tree(cx);
        for label in ["Resume auto-scroll", "Diagnostic events"] {
            let bounds = named(&t, label)
                .1
                .bounds()
                .expect("visible synthetic control bounds");
            let scale = cx.update(|w, _| w.scale_factor()) as f64;
            assert!(
                bounds.y0 >= 0. && bounds.y1 <= 440. * scale,
                "{label}: {bounds:?}"
            );
        }
        entity.read_with(cx, |s, cx| {
            let list = s.events.read(cx);
            assert_eq!(list.rendered_len(), 100);
            assert!(list.scroll.max_offset().y > gpui::px(0.));
        });
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
            s.model.navigate(Tab::Events);
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
            s.model.navigate(Tab::Compare);
            s.sync(cx);
        });
        let t = tree(cx);
        assert!(named(&t, "Comparison deltas").1.row_count().unwrap() > 1);
        entity.update(cx, |s, cx| {
            s.model.navigate(Tab::Dictation);
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

#[test]
fn every_report_string_is_bounded_before_schema_or_identity_work() {
    fn reject_strings(v: &mut serde_json::Value) {
        match v {
            serde_json::Value::String(_) => {
                *v = serde_json::json!("s".repeat(MAX_STRING_BYTES + 1));
            }
            _ => unreachable!(),
        }
    }
    fn visit(root: &serde_json::Value, path: &mut Vec<String>, paths: &mut Vec<Vec<String>>) {
        match root {
            serde_json::Value::String(_) => paths.push(path.clone()),
            serde_json::Value::Array(a) => {
                for (i, v) in a.iter().enumerate() {
                    path.push(i.to_string());
                    visit(v, path, paths);
                    path.pop();
                }
            }
            serde_json::Value::Object(o) => {
                for (k, v) in o {
                    path.push(k.clone());
                    visit(v, path, paths);
                    path.pop();
                }
            }
            _ => {}
        }
    }
    let hardware = serde_json::from_slice(include_bytes!(
        "../../../../tests/fixtures/diagnostic-reports/evaluation-v1-hardware.json"
    ))
    .unwrap();
    for original in [benchmark(), evaluation(), hardware] {
        let mut paths = vec![];
        visit(&original, &mut vec![], &mut paths);
        for path in paths {
            let mut v = original.clone();
            reject_strings(v.pointer_mut(&format!("/{}", path.join("/"))).unwrap());
            assert_eq!(parse(&v).err(), Some(ImportError::StringLimit));
        }
    }
    for path in [
        "/results/0/error",
        "/environment/hardwareModel",
        "/recommendations/fastest",
    ] {
        let mut v = benchmark();
        *v.pointer_mut(path).unwrap() = serde_json::json!("x".repeat(MAX_STRING_BYTES + 1));
        assert_eq!(parse(&v).err(), Some(ImportError::StringLimit));
    }
    let mut exploit = benchmark();
    exploit["results"][0]["modelName"] = serde_json::json!("m".repeat(1024 * 1024));
    assert_eq!(parse(&exploit).err(), Some(ImportError::StringLimit));
}
fn max_fixture_report(models: usize, identity_bytes: usize) -> serde_json::Value {
    let mut v = benchmark();
    let fixture = v["results"][0]["fixtures"][0].clone();
    let fixtures: Vec<_> = (0..512)
        .map(|i| {
            let mut f = fixture.clone();
            f["fixtureId"] = serde_json::json!(format!("fixture-{i}"));
            f
        })
        .collect();
    v["corpus"]["fixtureIds"] =
        serde_json::json!((0..512).map(|i| format!("fixture-{i}")).collect::<Vec<_>>());
    v["corpus"]["fixtureCount"] = serde_json::json!(512);
    let mut model = v["results"][0].clone();
    model["fixtures"] = serde_json::json!(fixtures);
    v["results"] = serde_json::json!(
        (0..models)
            .map(|i| {
                let mut m = model.clone();
                m["modelName"] = serde_json::json!(format!(
                    "{i}{}",
                    "m".repeat(identity_bytes.saturating_sub(1))
                ));
                m
            })
            .collect::<Vec<_>>()
    );
    v
}
#[test]
fn maximum_fixture_comparisons_share_scopes_and_enforce_preflight_budgets() {
    use std::sync::Arc;
    let r = parse(&max_fixture_report(1, MAX_STRING_BYTES))
        .ok()
        .unwrap();
    let c = compare(&r, &r);
    assert!(!c.blocked());
    assert_eq!(c.metrics.len(), 1 + 10 + 512 * 7);
    assert!(c.metrics.len() <= MAX_COMPARISON_METRICS);
    for group in c.metrics[11..].as_chunks::<7>().0 {
        assert!(group.iter().all(|m| Arc::ptr_eq(&group[0].scope, &m.scope)));
    }
    let mut seen = std::collections::BTreeSet::new();
    let scope_bytes: usize = c
        .metrics
        .iter()
        .filter(|m| seen.insert(Arc::as_ptr(&m.scope) as *const () as usize))
        .map(|m| m.scope.len())
        .sum();
    assert!(scope_bytes < 3 * 1024 * 1024);
    // String lengths remain legal, but shared scope amplification is too large.
    let mut large_scope = max_fixture_report(1, MAX_STRING_BYTES);
    large_scope["results"][0]["backend"] = serde_json::json!("b".repeat(MAX_STRING_BYTES));
    large_scope["results"][0]["accelerator"] = serde_json::json!("a".repeat(MAX_STRING_BYTES));
    for v in [max_fixture_report(3, 128), large_scope] {
        let r = parse(&v).ok().unwrap();
        let c = compare(&r, &r);
        assert!(c.blocked() && c.metrics.is_empty());
        assert_eq!(c.findings[0].code, "comparison_limit");
    }
    // Evaluation matching is also bounded before its case-by-case scan.
    let mut v = evaluation();
    let case = v["cases"][0].clone();
    v["cases"] = serde_json::json!(
        (0..1001)
            .map(|i| {
                let mut c = case.clone();
                c["id"] = serde_json::json!(format!("case-{i}"));
                c
            })
            .collect::<Vec<_>>()
    );
    v["summary"]["total"] = serde_json::json!(1001);
    v["summary"]["passed"] = serde_json::json!(1001);
    let r = parse(&v).ok().unwrap();
    assert!(compare(&r, &r).metrics.is_empty());
    assert_eq!(compare(&r, &r).findings[0].code, "comparison_limit");
}
#[test]
fn report_selections_survive_refresh_reorder_and_imports_without_substitution() {
    fn local(n: u64) -> Report {
        let mut v = benchmark();
        v["sharedInitMs"] = serde_json::json!(n);
        Report::parse(&serde_json::to_vec(&v).unwrap(), 0, Source::Local)
            .ok()
            .unwrap()
    }
    let mut m = active();
    notify(
        &mut m,
        Lane::Reports,
        1,
        0,
        Payload::Reports(vec![local(1), local(2)]),
    );
    let baseline = m.baseline.clone();
    let candidate = m.candidate.clone();
    notify(
        &mut m,
        Lane::Reports,
        2,
        0,
        Payload::Reports(vec![local(3), local(2), local(1)]),
    );
    assert_eq!(m.baseline, baseline);
    assert_eq!(m.candidate, candidate);
    let ticket = m.begin_read(Lane::Reports);
    m.event(Event::Import {
        ticket,
        result: Ok(Some(parse(&benchmark()).ok().unwrap())),
    });
    let imported = m.candidate.clone();
    notify(
        &mut m,
        Lane::Reports,
        3,
        0,
        Payload::Reports(vec![local(2), local(3), local(1)]),
    );
    assert_eq!(m.candidate, imported);
    assert_eq!(m.baseline, baseline);
    notify(
        &mut m,
        Lane::Reports,
        4,
        0,
        Payload::Reports(vec![local(2), local(3)]),
    );
    assert!(m.baseline.is_none());
    assert_eq!(m.candidate, imported);
    assert!(m.notice.unwrap().contains("no longer available"));
    m.clear_imports();
    assert!(m.baseline.is_none() && m.candidate.is_none());
    let mut early = active();
    let ticket = early.begin_read(Lane::Reports);
    early.event(Event::Import {
        ticket,
        result: Ok(Some(parse(&benchmark()).ok().unwrap())),
    });
    let imported = early.candidate.clone();
    notify(
        &mut early,
        Lane::Reports,
        1,
        0,
        Payload::Reports(vec![local(1), local(2)]),
    );
    assert_eq!(early.candidate, imported);
    // An identical source read has the same key; timestamp alone is not identity.
    assert_eq!(local(1).id(), local(1).id());
    assert_ne!(local(1).id(), local(2).id());
}
#[gpui::test]
fn routes_and_tab_navigation_clear_review_confirmations_and_revoke_detail(cx: &mut TestAppContext) {
    let (entity, cx) = harness(cx);
    entity.update(cx, |s, cx| {
        s.model.navigate(Tab::Dictation);
        s.sync(cx);
        s.model.selected_capture = Some("synthetic-capture".into());
        s.model.capture_detail = Some(fixtures::capture("synthetic-capture".into()));
        s.confirm = Some(Confirm::Upload("synthetic-capture".into()));
        s.confirm_ticket = Some(s.model.ticket(Lane::Captures));
        s.restore_focus = Some(s.confirm_focus.clone());
        let ticket = s.model.ticket(Lane::Captures);
        let lease = s.authority.lease(ticket);
        for (revision, tab) in [(100, Tab::Events), (101, Tab::Dictation)] {
            s.receive(
                Event::Notification {
                    attachment: s.model.ticket(Lane::Route).attachment,
                    lane: Lane::Route,
                    revision,
                    store_epoch: 0,
                    payload: Payload::Route(tab),
                },
                cx,
            );
            assert!(s.model.capture_detail.is_none() && s.model.selected_capture.is_none());
            assert!(s.confirm.is_none() && s.confirm_ticket.is_none() && s.restore_focus.is_none());
        }
        assert!(!lease.is_current());
        assert!(!s.model.event(Event::Read {
            ticket,
            revision: 200,
            payload: Payload::CaptureDetail(fixtures::capture("synthetic-capture".into()))
        }));
    });
    // Keyboard tab navigation uses the same private review cleanup. An open
    // confirmation traps keyboard focus, so this path starts without a dialog.
    entity.update(cx, |s, _| {
        s.model.selected_capture = Some("synthetic-capture".into());
        s.model.capture_detail = Some(fixtures::capture("synthetic-capture".into()));
    });
    let tabs = entity.read_with(cx, |s, _| s.tabs.clone());
    cx.update(|w, cx| w.focus(&tabs.read(cx).focus_handle(cx), cx));
    cx.simulate_keystrokes("home");
    draw(cx);
    entity.read_with(cx, |s, _| {
        assert!(s.model.capture_detail.is_none() && s.confirm.is_none())
    });
    // A queued route must cancel confirmation before the Confirm handler can
    // issue a side effect, even when it drains before the next paint/sync.
    cx.update(|w, cx| {
        entity.update(cx, |s, cx| {
            s.open_confirm(Confirm::Events, Lane::Events, w, cx);
            let events_ticket = s.model.ticket(Lane::Events);
            assert!(
                s.inbox
                    .publish(Event::Notification {
                        attachment: events_ticket.attachment,
                        lane: Lane::Route,
                        revision: 102,
                        store_epoch: 0,
                        payload: Payload::Route(Tab::Runs),
                    })
                    .is_ok()
            );
            s.confirm(w, cx);
            assert_eq!(s.model.ticket(Lane::Events), events_ticket);
            assert!(s.confirm.is_none() && s.confirm_ticket.is_none());
        })
    });
}
#[gpui::test]
fn stale_capture_reads_retry_original_uuid_only_for_current_selection(cx: &mut TestAppContext) {
    use super::host::{Host, Request, RequestKind, Subscription};
    use std::cell::RefCell;
    #[derive(Default)]
    struct Probe {
        requests: RefCell<Vec<Request>>,
    }
    struct Stop;
    impl Subscription for Stop {
        fn stop(&mut self) {}
    }
    impl Host for Probe {
        fn subscribe(
            &self,
            _: murmur_core::events::Consumer,
            _: u64,
            _: Inbox,
        ) -> Box<dyn Subscription> {
            Box::new(Stop)
        }
        fn request(&self, r: Request, _: Inbox) {
            self.requests.borrow_mut().push(r);
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
    entity.unwrap().update(cx, |s, cx| {
        s.model.attach();
        s.model.navigate(Tab::Dictation);
        s.model.selected_capture = Some("synthetic-capture".into());
        s.request(Lane::Captures, RequestKind::ReadCapture { capture_id: "synthetic-capture".into() });
        let ticket = s.model.ticket(Lane::Captures);
        assert!(s.inbox.publish(Event::Notification { attachment: ticket.attachment, lane: Lane::Captures, revision: 2, store_epoch: 0, payload: fixtures::payload(Lane::Captures) }).is_ok());
        assert!(s.inbox.publish(Event::Read { ticket, revision: 3, payload: Payload::CaptureDetail(fixtures::capture("synthetic-capture".into())) }).is_ok());
        s.drain(cx);
        assert_eq!(host.requests.borrow().len(), 2);
        assert!(matches!(&host.requests.borrow()[1].kind, RequestKind::ReadCapture { capture_id } if capture_id == "synthetic-capture"));
        let retry = s.model.ticket(Lane::Captures);
        assert_ne!(ticket, retry);
        assert!(s.inbox.publish(Event::Read { ticket: retry, revision: 4, payload: Payload::CaptureDetail(fixtures::capture("synthetic-capture".into())) }).is_ok());
        s.drain(cx);
        assert!(s.model.capture_detail.is_some());
        s.request(Lane::Captures, RequestKind::ReadCapture { capture_id: "synthetic-capture".into() });
        let stale = s.model.ticket(Lane::Captures);
        s.model.clear_capture_review();
        assert!(s.inbox.publish(Event::Read { ticket: stale, revision: 5, payload: Payload::CaptureDetail(fixtures::capture("synthetic-capture".into())) }).is_ok());
        s.drain(cx);
        assert_eq!(host.requests.borrow().len(), 3);
        assert!(s.model.capture_detail.is_none());
        // An older request cannot supersede a newly selected capture's read.
        s.model.selected_capture = Some("second-synthetic".into());
        s.request(Lane::Captures, RequestKind::ReadCapture { capture_id: "second-synthetic".into() });
        assert!(s.inbox.publish(Event::Read { ticket: stale, revision: 6, payload: Payload::CaptureDetail(fixtures::capture("synthetic-capture".into())) }).is_ok());
        s.drain(cx);
        assert_eq!(host.requests.borrow().len(), 4);
    });
}

#[test]
fn latency_identity_and_route_strings_are_bounded_before_retention() {
    let mut m = active();
    let Payload::Latency(rows) = fixtures::payload(Lane::Latency) else {
        unreachable!()
    };
    for field in 0..3 {
        let mut row = rows[0].clone();
        *match field {
            0 => &mut row.build,
            1 => &mut row.from,
            _ => &mut row.to,
        } = "x".repeat(257);
        assert!(notify(
            &mut m,
            Lane::Latency,
            field + 1,
            0,
            Payload::Latency(vec![row])
        ));
        assert!(m.latency.is_empty());
    }
    assert!(notify(&mut m, Lane::Latency, 4, 0, Payload::Latency(rows)));
    assert_eq!(m.latency.len(), 1);
}

#[test]
fn synthetic_live_fixture_coalesces_and_stops_with_subscription() {
    use super::host::Host;
    let host = fixtures::FixtureHost::default();
    let inbox = Inbox::default();
    let mut subscription =
        host.subscribe(murmur_core::events::Consumer::Diagnostics, 1, inbox.clone());
    assert!(host.emit_live());
    assert!(host.emit_live());
    assert_eq!(inbox.drain().len(), 1);
    assert!(host.emit_live());
    let event = inbox.drain().pop_back().unwrap();
    assert!(matches!(
        event,
        Event::Notification {
            revision: 2,
            payload: Payload::Event(_),
            ..
        }
    ));
    subscription.stop();
    assert!(!host.emit_live());
    let _subscription =
        host.subscribe(murmur_core::events::Consumer::Diagnostics, 2, inbox.clone());
    inbox.close();
    assert!(!host.emit_live());
}
