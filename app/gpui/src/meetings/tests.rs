use super::{
    Meetings, fixtures,
    host::{Action, PlaybackAction},
    model::*,
};
use crate::workspace::Projection;
use fixtures::FixtureHost;
use gpui::{AppContext, Entity, TestAppContext, VisualTestContext};
use std::rc::Rc;
fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|w, cx| w.draw(cx).clear(cx));
}
fn harness(cx: &mut TestAppContext) -> (Entity<Meetings>, Rc<FixtureHost>, &mut VisualTestContext) {
    cx.update(crate::init);
    let h = Rc::new(FixtureHost::default());
    let mut view = None;
    let (_, cx) = cx.add_window_view(|w, cx| {
        let e = cx.new(|cx| Meetings::new(h.clone(), w, cx));
        view = Some(e.clone());
        gpui_base::Root::new(e, w, cx)
    });
    let e = view.unwrap();
    e.update(cx, |s, cx| s.attach(cx));
    draw(cx);
    (e, h, cx)
}
#[test]
fn private_projection_bounds_canonical_channels_and_copy() {
    let mut s = fixtures::snapshot();
    assert!(s.valid());
    assert_eq!(Phase::Starting.label(), "Connecting both channels…");
    assert_eq!(
        Phase::Processing.label(),
        "Finishing pending transcript chunks…"
    );
    assert_eq!(duration(65000), "1 min");
    assert_eq!(timestamp(3601000), "1:00:01");
    let d = s.detail.as_ref().unwrap();
    assert_eq!(d.label(&d.segments[1]), "Speaker 1");
    s.detail.as_mut().unwrap().segments[0].remote_speaker = Some(1);
    assert!(!s.valid());
}
#[test]
fn command_contracts_preserve_existing_review_revision_and_calendar_token() {
    let a = Action::SaveLabels {
        id: "fixture".into(),
        revision: Some(7),
        me: "Me".into(),
        them: "Them".into(),
    };
    let (c, p) = a.command().unwrap();
    assert_eq!(c, "save_meeting_review");
    assert_eq!(p["request"]["expectedReviewRevision"], 7);
    assert_eq!(p["request"]["base"]["kind"], "labels_only");
    assert!(p["request"]["document"].is_null());
    assert_eq!(
        Action::CalendarApply {
            id: "fixture".into(),
            token: "opaque".into()
        }
        .command()
        .unwrap()
        .1["selectionToken"],
        "opaque"
    );
}
#[gpui::test]
fn subscribe_precedes_passive_read_and_no_capture_or_calendar(cx: &mut TestAppContext) {
    let (e, h, cx) = harness(cx);
    assert_eq!(h.calls.borrow()[0], "subscribe");
    assert!(matches!(h.actions.borrow().as_slice(), [Action::Read]));
    assert!(e.read_with(cx, |s, _| s.model.snapshot.as_ref().unwrap().valid()));
}
#[gpui::test]
fn generation_stop_and_processing_exclusion(cx: &mut TestAppContext) {
    let (e, h, cx) = harness(cx);
    e.update(cx, |s, cx| s.record(cx));
    draw(cx);
    let g = h.state.borrow().recording_generation;
    e.update(cx, |s, cx| s.record(cx));
    draw(cx);
    assert!(matches!(h.actions.borrow().last(),Some(Action::Stop{generation}) if *generation==g));
    let count = h.actions.borrow().len();
    e.update(cx, |s, cx| s.record(cx));
    assert_eq!(h.actions.borrow().len(), count);
}
#[gpui::test]
fn calendar_requires_explicit_choice_and_apply(cx: &mut TestAppContext) {
    let (e, h, cx) = harness(cx);
    e.update(cx, |s, cx| {
        s.request(Action::CalendarLookup("fixture-1".into()), cx)
    });
    draw(cx);
    assert_eq!(
        h.state
            .borrow()
            .detail
            .as_ref()
            .unwrap()
            .session
            .title_source
            .as_deref(),
        Some("manual")
    );
    assert!(e.read_with(cx, |s, _| s.calendar_selection.is_none()));
    e.update(cx, |s, cx| {
        s.request(
            Action::CalendarApply {
                id: "fixture-1".into(),
                token: "wrong".into(),
            },
            cx,
        )
    });
    assert!(!matches!(
        h.actions.borrow().last(),
        Some(Action::CalendarApply { .. })
    ));
    e.update(cx, |s, cx| {
        s.request(
            Action::CalendarApply {
                id: "fixture-1".into(),
                token: "synthetic-selection-token".into(),
            },
            cx,
        )
    });
    draw(cx);
    assert_eq!(
        h.state
            .borrow()
            .detail
            .as_ref()
            .unwrap()
            .session
            .title_source
            .as_deref(),
        Some("calendar")
    );
}
#[gpui::test]
fn selection_revokes_calendar_reply_and_releases_old_private_fields(cx: &mut TestAppContext) {
    let (e, h, cx) = harness(cx);
    h.hold.set(true);
    e.update(cx, |s, cx| {
        s.request(Action::CalendarLookup("fixture-1".into()), cx)
    });
    let (r, inbox) = h.held.borrow_mut().pop().unwrap();
    e.update(cx, |s, cx| s.select("fixture-2".into(), cx));
    assert!(!r.lease.is_current());
    let mut snapshot = fixtures::snapshot();
    snapshot.revision = 10;
    let _ = inbox.publish(Event::Reply {
        ticket: r.ticket,
        result: Ok(snapshot),
    });
    e.update(cx, |s, cx| s.drain(cx));
    assert!(e.read_with(cx, |s, _| {
        s.model.snapshot.as_ref().unwrap().detail.is_none()
    }));
}
#[gpui::test]
fn hide_closes_inbox_revokes_pending_and_reopen_subscribes(cx: &mut TestAppContext) {
    let (e, h, cx) = harness(cx);
    let lease = e.read_with(cx, |s, _| s.authority.lease(s.model.ticket()));
    cx.update(|w, cx| e.update(cx, |s, cx| s.hide(w, cx)));
    assert!(!lease.is_current());
    assert_eq!(h.stops.get(), 1);
    assert!(e.read_with(cx, |s, _| s.model.snapshot.is_none()
        && s.fields.is_empty()
        && s.inbox.is_closed()));
    e.update(cx, |s, cx| s.attach(cx));
    draw(cx);
    assert!(e.read_with(cx, |s, _| s.model.snapshot.is_some()));
}
#[gpui::test]
fn export_captions_and_bundle_admission(cx: &mut TestAppContext) {
    let (e, _, cx) = harness(cx);
    e.update(cx, |s, cx| {
        let state = s.model.snapshot.as_mut().unwrap();
        state.detail.as_mut().unwrap().session.status = "active".into();
        assert!(!s.allowed(&Action::Export {
            id: "fixture-1".into(),
            format: Format::Srt
        }));
        assert!(s.allowed(&Action::Export {
            id: "fixture-1".into(),
            format: Format::Text
        }));
        assert!(!s.allowed(&Action::BundlePreview("fixture-1".into())));
        s.model
            .snapshot
            .as_mut()
            .unwrap()
            .detail
            .as_mut()
            .unwrap()
            .session
            .status = "complete".into();
        s.request(Action::BundlePreview("fixture-1".into()), cx);
    });
    draw(cx);
    assert!(e.read_with(cx, |s, _| s.allowed(&Action::BundleExport {
        id: "fixture-1".into(),
        format: Format::Markdown
    })));
    e.update(cx, |s, _| {
        s.model
            .snapshot
            .as_mut()
            .unwrap()
            .bundle
            .as_mut()
            .unwrap()
            .max_bytes = 1;
        assert!(!s.allowed(&Action::BundleExport {
            id: "fixture-1".into(),
            format: Format::Markdown
        }));
    });
}
#[gpui::test]
fn deletion_arms_then_confirms_and_never_defaults(cx: &mut TestAppContext) {
    let (e, h, cx) = harness(cx);
    e.update(cx, |s, cx| s.delete(false, cx));
    assert!(matches!(h.actions.borrow().last(), Some(Action::Read)));
    cx.simulate_keystrokes("enter");
    draw(cx);
    assert!(matches!(h.actions.borrow().last(), Some(Action::Read)));
    e.update(cx, |s, cx| s.delete(false, cx));
    draw(cx);
    assert_eq!(h.state.borrow().meetings.len(), 1);
    assert!(h.pauses.get() > 0);
}
#[gpui::test]
fn metadata_and_speaker_names_are_scoped_to_session_and_keep_raw_text(cx: &mut TestAppContext) {
    let (e, h, cx) = harness(cx);
    let text = h.state.borrow().detail.as_ref().unwrap().segments[1]
        .text
        .clone();
    e.update(cx, |s, cx| {
        s.request(
            Action::RenameSpeaker {
                id: "fixture-1".into(),
                speaker: 1,
                label: "Fixture guest".into(),
            },
            cx,
        )
    });
    draw(cx);
    let s = h.state.borrow();
    let d = s.detail.as_ref().unwrap();
    assert_eq!(d.label(&d.segments[1]), "Fixture guest");
    assert_eq!(d.segments[1].text, text);
    assert_eq!(d.segments[1].channel.wire(), "them");
    drop(s);
    e.update(cx, |s, cx| {
        s.request(
            Action::RenameSpeaker {
                id: "fixture-2".into(),
                speaker: 1,
                label: "Cross session".into(),
            },
            cx,
        )
    });
    assert!(h.state.borrow().detail.as_ref().unwrap().remote[0].label == "Fixture guest");
}
#[gpui::test]
fn draft_survives_failure_and_busy_blocks_playback_and_calendar(cx: &mut TestAppContext) {
    let (e, h, cx) = harness(cx);
    cx.update(|w, cx| {
        e.update(cx, |s, cx| {
            s.name(w, cx);
            s.fields["title"].update(cx, |f, cx| f.set_value("Synthetic edited title", w, cx));
        })
    });
    h.fail.set(true);
    e.update(cx, |s, cx| s.save_metadata(cx));
    draw(cx);
    assert!(e.read_with(cx, |s, cx| s.naming
        && s.value("title", cx) == "Synthetic edited title"));
    e.update(cx, |s, _| {
        s.model.snapshot.as_mut().unwrap().capture_busy = true;
        assert!(!s.allowed(&Action::CalendarLookup("fixture-1".into())));
        assert!(!s.allowed(&Action::Playback {
            id: "fixture-1".into(),
            action: PlaybackAction::Play
        }));
        assert!(s.allowed(&Action::Playback {
            id: "fixture-1".into(),
            action: PlaybackAction::Pause
        }));
    });
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
    fn native_ax_capture_and_stop_have_exact_generation(cx: &mut TestAppContext) {
        let (e, h, cx) = harness(cx);
        let t = tree(cx);
        named(&t, "Notetaker");
        named(&t, "Search meetings");
        click(cx, named(&t, "Start meeting"));
        let t = tree(cx);
        click(cx, named(&t, "Stop meeting"));
        assert!(matches!(
            h.actions.borrow().last(),
            Some(Action::Stop { .. })
        ));
        assert!(
            e.read_with(cx, |s, _| s.model.snapshot.as_ref().unwrap().phase
                == Phase::Processing)
        );
    }
    #[gpui::test]
    fn native_ax_calendar_requires_selection_and_has_cancel(cx: &mut TestAppContext) {
        let (_, h, cx) = harness(cx);
        let t = tree(cx);
        click(cx, named(&t, "Name from calendar…"));
        let t = tree(cx);
        named(&t, "Choose an overlapping event");
        if let Calendar::Choosing(events) = &h.state.borrow().calendar {
            let event = &events[0];
            named(
                &t,
                &format!(
                    "{} · {} – {} · Attendees: {}",
                    event.title,
                    super::super::view::local_datetime(event.start_ms),
                    super::super::view::local_datetime(event.end_ms),
                    event.attendees.join(", ")
                ),
            );
        } else {
            panic!("expected calendar candidates");
        }
        let apply = named(&t, "Apply event");
        assert!(
            t.nodes
                .iter()
                .find(|(id, _)| *id == apply)
                .unwrap()
                .1
                .is_disabled()
        );
        click(cx, named(&t, "Cancel calendar lookup"));
        assert_eq!(
            h.state
                .borrow()
                .detail
                .as_ref()
                .unwrap()
                .session
                .title_source
                .as_deref(),
            Some("manual")
        );
    }
}
#[gpui::test]
fn suggestion_acceptance_is_explicit_and_respects_native_busy_projection(cx: &mut TestAppContext) {
    let (e, h, cx) = harness(cx);
    e.update(cx, |s, cx| {
        s.model.snapshot.as_mut().unwrap().capture_busy = true;
        s.accept_suggestion("opaque-fixture".into(), cx);
    });
    assert!(matches!(h.actions.borrow().as_slice(), [Action::Read]));
    e.update(cx, |s, cx| {
        s.model.snapshot.as_mut().unwrap().capture_busy = false;
        s.accept_suggestion("opaque-fixture".into(), cx);
    });
    draw(cx);
    assert!(
        matches!(h.actions.borrow().last(),Some(Action::StartSuggested(token)) if token=="opaque-fixture")
    );
}
#[gpui::test]
fn cancelling_calendar_releases_candidates_and_rejects_delayed_reply(cx: &mut TestAppContext) {
    let (e, h, cx) = harness(cx);
    e.update(cx, |s, cx| {
        s.request(Action::CalendarLookup("fixture-1".into()), cx)
    });
    draw(cx);
    e.update(cx, |s, cx| s.close_calendar(cx));
    assert!(matches!(h.state.borrow().calendar, Calendar::Closed));
    h.hold.set(true);
    e.update(cx, |s, cx| {
        s.request(Action::CalendarLookup("fixture-1".into()), cx)
    });
    let (r, inbox) = h.held.borrow_mut().pop().unwrap();
    e.update(cx, |s, cx| s.close_calendar(cx));
    assert!(!r.lease.is_current());
    let _ = inbox.publish(Event::Reply {
        ticket: r.ticket,
        result: Ok(fixtures::snapshot()),
    });
    e.update(cx, |s, cx| s.drain(cx));
    assert!(e.read_with(cx, |s, _| matches!(
        s.model.snapshot.as_ref().unwrap().calendar,
        Calendar::Closed
    )));
}
#[gpui::test]
fn multiline_metadata_and_editor_change_are_submitted_as_visible(cx: &mut TestAppContext) {
    let (e, h, cx) = harness(cx);
    cx.update(|w, cx| {
        e.update(cx, |s, cx| {
            s.name(w, cx);
            s.attendees.as_ref().unwrap().update(cx, |input, cx| {
                input.set_value("Fixture one\nFixture two", w, cx)
            });
            s.fields["title"].update(cx, |input, cx| input.set_value("Edited fixture", w, cx));
        })
    });
    draw(cx);
    e.update(cx, |s, cx| s.save_metadata(cx));
    draw(cx);
    let state = h.state.borrow();
    let d = state.detail.as_ref().unwrap();
    assert_eq!(d.session.title.as_deref(), Some("Edited fixture"));
    assert_eq!(d.session.attendees, vec!["Fixture one", "Fixture two"]);
    assert!(!e.read_with(cx, |s, _| s.naming));
}
#[test]
fn live_segment_rechecks_recording_generation_and_session_before_mutating() {
    let mut state = fixtures::snapshot();
    let segment = state.detail.as_ref().unwrap().segments[0].clone();
    state.phase = Phase::Recording;
    state.recording_generation = 42;
    state.recording_session = Some("fixture-1".into());
    assert!(!state.live_segment(41, segment.clone()));
    let mut other = segment.clone();
    other.session_id = "fixture-2".into();
    assert!(!state.live_segment(42, other));
    assert!(state.live_segment(42, segment));
    assert_eq!(state.detail.as_ref().unwrap().segments.len(), 4);
    for i in 10..310 {
        let mut segment = state.detail.as_ref().unwrap().segments[0].clone();
        segment.id = i;
        segment.start_ms = i * 1000;
        segment.end_ms = i * 1000 + 500;
        assert!(state.live_segment(42, segment));
    }
    assert_eq!(state.detail.as_ref().unwrap().segments.len(), 200);
    state.phase = Phase::Idle;
    let segment = state.detail.as_ref().unwrap().segments[0].clone();
    assert!(!state.live_segment(42, segment));
}

#[gpui::test]
fn playback_progress_preserves_pointer_seek_until_release(cx: &mut TestAppContext) {
    let (e, h, cx) = harness(cx);
    let seek = e.read_with(cx, |s, _| s.seek.clone());
    let slider = seek.read_with(cx, |r, _| r.state.clone());
    cx.update(|w, cx| {
        slider.update(cx, |s, cx| {
            let bounds = s.bounds();
            assert!(
                bounds.size.width > gpui::px(0.),
                "seek track must record pointer mapping bounds"
            );
            s.update_value_by_position(
                gpui::Axis::Horizontal,
                gpui::point(bounds.left() + bounds.size.width * 0.8, bounds.top()),
                false,
                w,
                cx,
            );
        })
    });
    e.update(cx, |s, cx| {
        s.model.snapshot.as_mut().unwrap().audio.position_ms = 1200;
        cx.notify();
    });
    draw(cx);
    assert_eq!(seek.read_with(cx, |r, cx| r.value(cx)), 52000);
    slider.update(cx, |s, cx| s.handle_release(cx));
    draw(cx);
    assert!(matches!(
        h.actions.borrow().last(),
        Some(Action::Playback {
            action: PlaybackAction::Seek(52000),
            ..
        })
    ));
}

#[gpui::test]
fn playback_progress_keeps_delete_armed_but_owner_changes_disarm(cx: &mut TestAppContext) {
    let (e, h, cx) = harness(cx);
    e.update(cx, |s, cx| {
        s.delete(false, cx);
        let mut snapshot = s.model.snapshot.clone().unwrap();
        snapshot.audio.state = AudioState::Playing;
        snapshot.audio.position_ms += 32;
        snapshot.revision += 1;
        h.state.replace(snapshot.clone());
        assert!(
            s.inbox
                .publish(Event::Changed {
                    attachment: s.model.attachment,
                    snapshot
                })
                .is_ok()
        );
        s.drain(cx);
        assert!(s.confirmation.is_some());
        s.delete(false, cx);
    });
    draw(cx);
    assert!(matches!(h.actions.borrow().last(), Some(Action::Delete(id)) if id == "fixture-1"));
    e.update(cx, |s, cx| {
        s.delete(true, cx);
        let mut snapshot = s.model.snapshot.clone().unwrap();
        snapshot.capture_busy = true;
        snapshot.revision += 1;
        assert!(
            s.inbox
                .publish(Event::Changed {
                    attachment: s.model.attachment,
                    snapshot
                })
                .is_ok()
        );
        s.drain(cx);
        assert!(s.confirmation.is_none());
    });
}

#[gpui::test]
fn list_navigation_reaches_offscreen_rows_and_retains_row_focus(cx: &mut TestAppContext) {
    let (e, h, cx) = harness(cx);
    let mut snapshot = h.state.borrow().clone();
    snapshot.meetings = (0..80)
        .map(|i| fixtures::meeting(&format!("row-{i}")))
        .collect();
    snapshot.total = 80;
    h.state.replace(snapshot.clone());
    e.update(cx, |s, cx| {
        s.model.snapshot = Some(snapshot);
        cx.notify();
    });
    draw(cx);
    cx.update(|w, cx| {
        e.update(cx, |s, cx| {
            let focus = s.focus_for("meeting-row-0", cx);
            w.focus(&focus, cx);
        })
    });
    for (key, expected) in [("end", 79), ("up", 78), ("home", 0), ("down", 1)] {
        cx.simulate_keystrokes(key);
        draw(cx);
        assert!(
            matches!(h.actions.borrow().last(), Some(Action::Select(id)) if id == &format!("row-{expected}"))
        );
        if key == "end" {
            assert!(
                e.read_with(cx, |s, _| s.list_scroll.0.borrow().base_handle.offset().y)
                    < gpui::px(0.)
            );
        } else if key == "home" {
            assert_eq!(
                e.read_with(cx, |s, _| s.list_scroll.0.borrow().base_handle.offset().y),
                gpui::px(0.)
            );
        }
        cx.update(|w, cx| e.read(cx).focus[&format!("meeting-row-{expected}")].is_focused(w))
            .then_some(())
            .expect("new row owns focus");
    }
}

#[gpui::test]
fn detail_fields_and_playback_controls_reveal_in_their_scroll_parent(cx: &mut TestAppContext) {
    use gpui::Focusable;
    let (e, _, cx) = harness(cx);
    cx.simulate_resize(gpui::size(gpui::px(720.), gpui::px(560.)));
    e.update(cx, |s, cx| {
        s.speakers = true;
        cx.notify();
    });
    draw(cx);
    cx.update(|w, cx| w.simulate_next_frame(cx));
    draw(cx);
    cx.update(|w, cx| {
        e.update(cx, |s, cx| {
            w.focus(&s.fields["remote-1"].read(cx).focus_handle(cx), cx);
        })
    });
    draw(cx);
    cx.update(|w, cx| w.simulate_next_frame(cx));
    draw(cx);
    let scrolled = e.read_with(cx, |s, _| s.detail_scroll.offset().y);
    assert!(
        scrolled < gpui::px(0.),
        "offscreen speaker field must scroll into view: {:?}",
        e.read_with(cx, |s, _| (
            s.detail_scroll.bounds(),
            s.detail_scroll.max_offset()
        ))
    );
    cx.update(|w, cx| {
        e.update(cx, |s, cx| {
            w.focus(&s.channel_control.read(cx).focus_handle(cx), cx)
        })
    });
    draw(cx);
    cx.update(|w, cx| w.simulate_next_frame(cx));
    draw(cx);
    assert!(e.read_with(cx, |s, _| s.detail_scroll.offset().y) > scrolled);
    cx.update(|w, cx| e.update(cx, |s, cx| w.focus(&s.seek.read(cx).focus_handle(cx), cx)));
    draw(cx);
    cx.update(|w, cx| w.simulate_next_frame(cx));
    draw(cx);
    e.read_with(cx, |s, cx| {
        let bounds = s.seek.read(cx).state.read(cx).bounds();
        let viewport = s.detail_scroll.bounds();
        assert!(
            bounds.top() >= viewport.top() && bounds.bottom() <= viewport.bottom(),
            "focused seek track {bounds:?} outside detail viewport {viewport:?}"
        );
    });
}

#[test]
fn calendar_and_detail_labels_use_local_wall_clock() {
    let ms = 1700000000000;
    let expected = chrono::DateTime::from_timestamp_millis(ms)
        .unwrap()
        .with_timezone(&chrono::Local)
        .format("%a, %b %e, %Y %H:%M")
        .to_string();
    assert_eq!(super::view::local_datetime(ms), expected);
    assert!(!super::view::local_datetime(ms).contains(&ms.to_string()));
}
