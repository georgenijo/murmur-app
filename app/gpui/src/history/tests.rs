use super::fixtures::allocation_probe::{RenderAllocator, count_large_render_allocations};
use super::{History, fixtures::*, host::Authority, inbox::Inbox, model::*};
use gpui::{Entity, Focusable, TestAppContext, VisualTestContext, prelude::*};
use std::{
    rc::Rc,
    time::{Duration, Instant},
};
#[global_allocator]
static RENDER_ALLOCATOR: RenderAllocator = RenderAllocator;
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
    harness_after(cx, None)
}
struct WithAfterHistory {
    history: Entity<History>,
    after: gpui::FocusHandle,
}
impl gpui::Render for WithAfterHistory {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        _: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        gpui::div()
            .size_full()
            .flex()
            .flex_col()
            .child(gpui::div().flex_1().min_h_0().child(self.history.clone()))
            .child(crate::Button::new("after-history", "After History").track_focus(&self.after))
    }
}
fn harness_after(
    cx: &mut TestAppContext,
    after: Option<gpui::FocusHandle>,
) -> (Entity<History>, Rc<FixtureHost>, &mut VisualTestContext) {
    cx.update(crate::init);
    let host = Rc::new(FixtureHost::default());
    let mut entity = None;
    let (_, cx) = cx.add_window_view(|w, cx| {
        let d = cx.new(|cx| History::new(host.clone(), w, cx));
        entity = Some(d.clone());
        let view: gpui::AnyView = if let Some(after) = after {
            cx.new(|_| WithAfterHistory { history: d, after }).into()
        } else {
            d.into()
        };
        gpui_base::Root::new(view, w, cx)
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
    assert!(painted > 0 && painted < 40, "visible rows {painted}");
    tab_to_list(&e, cx);
    cx.simulate_keystrokes("home down up space");
    e.update(cx, |s, cx| s.drain(cx));
    draw(cx);
    assert_eq!(*host.clipboard.borrow(), entry(200).text);
    cx.simulate_keystrokes("end");
    draw(cx);
    assert_eq!(e.read_with(cx, |s, _| s.model.selected), Some(1));
    assert!(e.read_with(cx, |s, _| s.scroll.state.logical_scroll_top().item_ix) > 150);
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
    pub(super) fn tree(cx: &mut VisualTestContext) -> TreeUpdate {
        let w = cx.update(|w, _| w.window_handle());
        cx.activate_accessibility(w);
        draw(cx);
        cx.accessibility_tree(w)
    }
    pub(super) fn click(cx: &mut VisualTestContext, id: NodeId) {
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
    pub(super) fn named(t: &TreeUpdate, name: &str) -> NodeId {
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
        click(cx, named(&t, "More transcript actions"));
        let t = tree(cx);
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

#[cfg(feature = "a11y-tests")]
#[gpui::test]
fn nested_row_keyboard_and_open_picker_escape_keep_their_own_behavior(cx: &mut TestAppContext) {
    use gpui::Focusable;
    let (e, host, cx) = harness(cx);
    use gpui::{
        AccessibleAction,
        accesskit::{ActionRequest, TreeId},
    };
    let window = cx.update(|w, _| w.window_handle());
    cx.activate_accessibility(window);
    draw(cx);
    let tree = cx.accessibility_tree(window);
    let id = tree
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Show more for transcript 1"))
        .unwrap()
        .0;
    cx.dispatch_accessibility_action(
        window,
        ActionRequest {
            action: AccessibleAction::Focus,
            target_tree: TreeId::ROOT,
            target_node: id,
            data: None,
        },
    );
    draw(cx);
    assert_eq!(cx.accessibility_tree(window).focus, id);
    cx.update(|w, cx| assert!(e.read(cx).row_focus[&200].is_focused(w)));
    cx.simulate_keystrokes("space");
    draw(cx);
    assert_eq!(e.read_with(cx, |s, _| s.model.detail), Some(200));
    assert!(host.clipboard.borrow().is_empty());
    cx.simulate_keystrokes("escape");
    draw(cx);
    let actions = e.read_with(cx, |s, _| s.actions.clone());
    cx.update(|w, cx| w.focus(&actions.read(cx).focus_handle(cx), cx));
    cx.simulate_keystrokes("space");
    draw(cx);
    assert!(actions.read_with(cx, |s, _| s.is_open()));
    cx.simulate_keystrokes("escape");
    draw(cx);
    assert!(!actions.read_with(cx, |s, _| s.is_open()));
}

/// Traverse the actual Root tab order from the window start, never focus the list.
fn tab_to_list(e: &Entity<History>, cx: &mut VisualTestContext) {
    cx.update(|w, cx| {
        w.blur(cx);
        // With no focus, TestWindow has no contextual key dispatch path. Enter
        // the order using Root's first-Tab operation, then send real Tab keys.
        w.focus_next(cx);
    });
    draw(cx);
    let mut saw_search =
        cx.update(|w, cx| e.read(cx).search.read(cx).focus_handle(cx).is_focused(w));
    for _ in 0..40 {
        cx.simulate_keystrokes("tab");
        draw(cx);
        let (list, search) = cx.update(|w, cx| {
            e.read_with(cx, |s, cx| {
                (
                    s.focus.is_focused(w),
                    s.search.read(cx).focus_handle(cx).is_focused(w),
                )
            })
        });
        saw_search |= search;
        if list {
            assert!(
                saw_search,
                "Tab must pass the visible search before the list"
            );
            return;
        }
    }
    panic!("normal Tab traversal skipped the transcript list");
}

#[gpui::test]
fn command_f_from_detail_returns_to_visible_search_preserving_selection(cx: &mut TestAppContext) {
    let (e, _, cx) = harness(cx);
    tab_to_list(&e, cx);
    cx.simulate_keystrokes("home down right");
    draw(cx);
    assert_eq!(e.read_with(cx, |s, _| s.model.detail), Some(199));
    cx.simulate_keystrokes("cmd-f");
    draw(cx);
    assert_eq!(e.read_with(cx, |s, _| s.model.detail), None);
    assert_eq!(e.read_with(cx, |s, _| s.model.selected), Some(199));
    cx.update(|w, cx| assert!(e.read(cx).search.read(cx).focus_handle(cx).is_focused(w)));
    cx.simulate_input("transcript 199:");
    draw(cx);
    assert_eq!(e.read_with(cx, |s, _| s.model.matches.len()), 1);
}

#[cfg(feature = "a11y-tests")]
#[gpui::test]
fn detail_tab_scrolls_every_short_line_and_resets_next_page(cx: &mut TestAppContext) {
    let (e, _, cx) = harness(cx);
    // Nine hundred short lines fit in ONE scalar page, but many viewport pages.
    e.update(cx, |s, _| {
        s.model.entries[199].text = "x\n".repeat(900) + &"y".repeat(248) + &"z\n".repeat(900);
    });
    tab_to_list(&e, cx);
    cx.simulate_keystrokes("home right");
    draw(cx);
    cx.simulate_keystrokes("tab");
    draw(cx);
    cx.update(|w, cx| assert!(e.read(cx).detail_focus.is_focused(w)));
    let (height, max) = e.read_with(cx, |s, _| {
        (
            s.detail_scroll.bounds().size.height,
            s.detail_scroll.max_offset().y,
        )
    });
    assert!(max > height * 10., "short lines must actually overflow");
    let mut offset = gpui::px(0.);
    for _ in 0..200 {
        cx.simulate_keystrokes("pagedown");
        draw(cx);
        let next = e.read_with(cx, |s, _| s.detail_scroll.offset().y);
        assert!(
            offset - next <= height,
            "viewport paging skipped unread text"
        );
        assert_eq!(e.read_with(cx, |s, _| s.detail_page), 0);
        if next == offset {
            break;
        }
        offset = next;
    }
    assert_eq!(offset, -max);
    cx.simulate_keystrokes("home down");
    draw(cx);
    assert_eq!(
        e.read_with(cx, |s, _| s.detail_scroll.offset().y),
        -gpui::px(24.)
    );
    cx.simulate_keystrokes("up");
    draw(cx);
    assert_eq!(
        e.read_with(cx, |s, _| s.detail_scroll.offset().y),
        gpui::px(0.)
    );
    cx.simulate_keystrokes("space");
    draw(cx);
    assert!(e.read_with(cx, |s, _| s.detail_scroll.offset().y) < gpui::px(0.));
    cx.simulate_keystrokes("shift-space");
    draw(cx);
    assert_eq!(
        e.read_with(cx, |s, _| s.detail_scroll.offset().y),
        gpui::px(0.)
    );
    cx.simulate_keystrokes("end pageup");
    draw(cx);
    let y = e.read_with(cx, |s, _| s.detail_scroll.offset().y);
    assert!(y > -max && y < gpui::px(0.));
    let t = ax::tree(cx);
    ax::click(cx, ax::named(&t, "Next text page"));
    assert_eq!(e.read_with(cx, |s, _| s.detail_page), 1);
    assert_eq!(
        e.read_with(cx, |s, _| s.detail_scroll.offset().y),
        gpui::px(0.)
    );
    // The second scalar page overflows too: its scroll position must not be
    // inherited on Next, and returning from its bottom must also start at zero.
    for _ in 0..20 {
        if cx.update(|w, cx| e.read(cx).detail_focus.is_focused(w)) {
            break;
        }
        cx.simulate_keystrokes("tab");
        draw(cx);
    }
    cx.update(|w, cx| assert!(e.read(cx).detail_focus.is_focused(w)));
    cx.simulate_keystrokes("end");
    draw(cx);
    assert!(e.read_with(cx, |s, _| s.detail_scroll.offset().y) < gpui::px(0.));
    let t = ax::tree(cx);
    ax::click(cx, ax::named(&t, "Previous text page"));
    assert_eq!(e.read_with(cx, |s, _| s.detail_page), 0);
    assert_eq!(
        e.read_with(cx, |s, _| s.detail_scroll.offset().y),
        gpui::px(0.)
    );
    e.read_with(cx, |s, _| {
        let text = &s.model.entries[199].text;
        let reassembled: String = s
            .detail_pages
            .boundaries
            .windows(2)
            .map(|r| &text[r[0]..r[1]])
            .collect();
        assert_eq!(&reassembled, text);
    });
}

#[cfg(feature = "a11y-tests")]
#[gpui::test]
fn reopening_resets_visible_filter_choices_and_export_scope(cx: &mut TestAppContext) {
    let (e, host, cx) = harness(cx);
    let t = ax::tree(cx);
    ax::click(cx, ax::named(&t, "Filter transcripts: Everything"));
    let t = ax::tree(cx);
    ax::click(cx, ax::named(&t, "Show transcripts"));
    cx.simulate_keystrokes("end space");
    draw(cx);
    let t = ax::tree(cx);
    ax::click(cx, ax::named(&t, "From date"));
    cx.simulate_keystrokes("home down space");
    draw(cx);
    assert_eq!(e.read_with(cx, |s, _| s.model.query.source), Source::Files);
    assert_eq!(e.read_with(cx, |s, _| s.model.query.date), Date::Today);
    cx.update(|w, cx| e.update(cx, |s, cx| s.hide(w, cx)));
    e.update(cx, |s, cx| {
        s.attach(cx);
        s.drain(cx);
    });
    draw(cx);
    let t = ax::tree(cx);
    ax::click(cx, ax::named(&t, "Filter transcripts: Everything"));
    e.read_with(cx, |s, cx| {
        assert_eq!(s.source.read(cx).value().map(|s| s.as_ref()), Some("0"));
        assert_eq!(s.date.read(cx).value().map(|s| s.as_ref()), Some("0"));
        assert_eq!(s.model.matches.len(), 200);
    });
    let t = ax::tree(cx);
    ax::click(cx, ax::named(&t, "More history actions"));
    cx.simulate_keystrokes("home down down down down down down down space");
    e.update(cx, |s, cx| s.drain(cx));
    draw(cx);
    assert_eq!(host.saved.borrow().len(), 1);
    assert!(host.saved.borrow()[0].1.contains("Synthetic transcript 1:"));
    assert!(
        host.saved.borrow()[0]
            .1
            .contains("Synthetic transcript 199:")
    );
}

#[cfg(feature = "a11y-tests")]
#[gpui::test]
fn calendar_gap_and_ambiguous_midnight_filter_the_real_view(cx: &mut TestAppContext) {
    use chrono::{TimeZone, Timelike};
    let (e, _, cx) = harness(cx);
    for (zone, y, m, d, first_hour) in [
        (chrono_tz::America::Santiago, 2026, 9, 6, 1),
        (chrono_tz::America::Havana, 2026, 11, 1, 0),
    ] {
        let date = chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap();
        let now = zone.with_ymd_and_hms(y, m, d, 12, 0, 0).unwrap();
        let clock = super::calendar_clock(now);
        let first = zone.timestamp_millis_opt(clock.today).unwrap();
        assert_eq!(first.hour(), first_hour);
        assert_eq!(first.minute(), 0);
        assert_eq!(first.date_naive(), date);
        if first_hour == 0 {
            let ambiguous = zone.from_local_datetime(&date.and_hms_opt(0, 0, 0).unwrap());
            assert!(matches!(ambiguous, chrono::LocalResult::Ambiguous(_, _)));
            assert_eq!(
                clock.today,
                ambiguous.earliest().unwrap().timestamp_millis()
            );
        }
        e.update(cx, |s, cx| {
            s.model.entries = [clock.today - 1, clock.today, clock.today + 1]
                .iter()
                .enumerate()
                .map(|(i, stamp)| {
                    Entry::project(
                        i as u64 + 1,
                        serde_json::json!({
                            "id": format!("calendar-{i}"), "text": "synthetic date boundary",
                            "timestamp": stamp, "duration": 1
                        }),
                    )
                    .unwrap()
                })
                .collect();
            s.model.set_clock(clock);
            s.model.set_query(Query::default());
            s.filters_open = true;
            s.date.update(cx, |s, cx| s.set_value("0", cx));
            s.prepare(cx);
        });
        draw(cx);
        let t = ax::tree(cx);
        ax::click(cx, ax::named(&t, "From date"));
        cx.simulate_keystrokes("home down space");
        draw(cx);
        assert_eq!(e.read_with(cx, |s, _| s.model.matches.len()), 2);
        assert_eq!(
            e.read_with(cx, |s, _| s.model.entries[s.model.matches[1]].key),
            2
        );
    }
}

#[gpui::test]
fn multi_megabyte_detail_redraws_only_shape_one_cached_slice(cx: &mut TestAppContext) {
    let (e, _, cx) = harness(cx);
    let text = "synthetic 🐱\n".repeat(200_000);
    let bytes = text.len();
    e.update(cx, |s, _| {
        s.model.entries[199] = Entry::project(
            200,
            serde_json::json!({
                "id":"large-synthetic", "text":text, "rawText":"original JSON ".repeat(200_000),
                "timestamp":NOW, "duration":1
            }),
        )
        .unwrap();
    });
    tab_to_list(&e, cx);
    cx.simulate_keystrokes("home right");
    draw(cx);
    let scanned = e.read_with(cx, |s, _| s.detail_pages.scanned_bytes);
    assert_eq!(scanned, bytes);
    let before = e.read_with(cx, |s, _| s.detail_pages.shaped_bytes);
    let whole_transcript_allocations = count_large_render_allocations(bytes, || {
        for _ in 0..8 {
            e.update(cx, |_, cx| cx.notify());
            draw(cx);
        }
    });
    assert_eq!(
        whole_transcript_allocations, 0,
        "redraw cloned retained transcript data"
    );
    assert_eq!(
        e.read_with(cx, |s, _| s.detail_pages.scanned_bytes),
        scanned
    );
    let copied = e.read_with(cx, |s, _| s.detail_pages.shaped_bytes) - before;
    assert!(
        copied > 0,
        "the redraws must exercise the real detail render"
    );
    assert!(
        copied <= 8 * super::DETAIL_PAGE * 4,
        "only bounded slices may be shaped"
    );
    e.update(cx, |s, cx| {
        s.last_detail_page();
        cx.notify();
    });
    draw(cx);
    assert_eq!(
        e.read_with(cx, |s, _| s.detail_pages.scanned_bytes),
        scanned
    );
    // A same-key edit with a newer service revision must invalidate boundaries.
    e.update(cx, |s, cx| {
        let mut replacement = entry(200);
        replacement.text = "replacement".into();
        s.receive(
            Event::Changed {
                attachment: s.model.ticket().attachment,
                revision: s.model.ticket().revision + 1,
                clear_epoch: 0,
                snapshot: Snapshot {
                    entries: vec![replacement],
                    retain_history: true,
                    retry_import: false,
                },
            },
            cx,
        );
    });
    draw(cx);
    assert_eq!(e.read_with(cx, |s, _| s.detail_page), 0);
    assert_eq!(
        e.read_with(cx, |s, _| s.detail_pages.scanned_bytes),
        scanned + "replacement".len()
    );
    cx.update(|w, cx| e.update(cx, |s, cx| s.hide(w, cx)));
    assert!(e.read_with(cx, |s, _| s.detail_pages.boundaries.is_empty()));
}

/// Reproduce H10 at the native QA viewport, then continue beyond every initially
/// rendered row. Inspect real scroll geometry, not merely the rendered range.
#[gpui::test]
fn tab_row_actions_reveal_forward_reverse_and_unrendered_rows(cx: &mut TestAppContext) {
    let (e, _, cx) = harness(cx);
    cx.update(|w, cx| {
        w.resize(gpui::size(gpui::px(880.), gpui::px(720.)));
        w.bounds_changed(cx);
        w.focus(&e.read(cx).search.read(cx).focus_handle(cx), cx);
    });
    draw(cx);
    let initial_end = e.read_with(cx, |s, _| s.rendered.end);
    let mut visited = Vec::new();
    for _ in 0..415 {
        cx.simulate_keystrokes("tab");
        draw(cx);
        let position = cx.update(|w, cx| {
            e.read_with(cx, |s, _| {
                s.row_focus.iter().find_map(|(key, f)| {
                    f.is_focused(w).then(|| {
                        s.model
                            .matches
                            .iter()
                            .position(|&i| s.model.entries[i].key == *key)
                            .unwrap()
                    })
                })
            })
        });
        if let Some(position) = position {
            e.read_with(cx, |s, _| {
                s.scroll.assert_visible(position);
            });
            visited.push(position);
            if position == 199 {
                break;
            }
        }
    }
    assert_eq!(visited, (0..200).collect::<Vec<_>>());
    assert!(initial_end < 199);
    for position in (0..199).rev() {
        cx.simulate_keystrokes("shift-tab shift-tab");
        draw(cx);
        cx.update(|w, cx| {
            e.read_with(cx, |s, _| {
                let key = s.model.entries[s.model.matches[position]].key;
                assert!(s.row_focus[&key].is_focused(w));
                s.scroll.assert_visible(position);
            })
        });
    }
}

/// Wheel scrolling may unmount a focused row without changing window focus.
#[gpui::test]
fn focused_row_remount_does_not_reveal_again(cx: &mut TestAppContext) {
    let (e, _, cx) = harness(cx);
    cx.update(|w, cx| {
        w.resize(gpui::size(gpui::px(880.), gpui::px(720.)));
        w.bounds_changed(cx);
        e.update(cx, |s, cx| {
            s.scroll.scroll_to_item(30, gpui::ScrollStrategy::Top);
            cx.notify();
        });
    });
    draw(cx);
    let key = e.read_with(cx, |s, _| s.model.entries[s.model.matches[30]].key);
    cx.update(|w, cx| {
        let focus = e.read(cx).row_focus[&key].clone();
        w.focus(&focus, cx);
    });
    draw(cx);
    draw(cx);
    e.update(cx, |s, cx| {
        s.scroll.scroll_to_item(50, gpui::ScrollStrategy::Top);
        cx.notify();
    });
    draw(cx);
    draw(cx);
    assert!(!e.read_with(cx, |s, _| s.rendered.contains(&30)));
    cx.update(|w, cx| assert!(e.read(cx).row_focus[&key].is_focused(w)));
    let partial = gpui::ListOffset {
        item_ix: 30,
        offset_in_item: gpui::px(20.),
    };
    e.update(cx, |s, cx| {
        s.scroll.state.scroll_to(partial);
        cx.notify();
    });
    draw(cx);
    draw(cx);
    assert!(e.read_with(cx, |s, _| s.rendered.contains(&30)));
    assert_eq!(
        e.read_with(cx, |s, _| s.scroll.state.logical_scroll_top()),
        partial,
        "unchanged focus must not jump when a clipped row remounts"
    );
    // A real focus exit and re-entry must still reveal that same clipped action.
    cx.update(|w, cx| w.focus(&e.read(cx).search.read(cx).focus_handle(cx), cx));
    draw(cx);
    cx.update(|w, cx| {
        let focus = e.read(cx).row_focus[&key].clone();
        w.focus(&focus, cx);
    });
    draw(cx);
    draw(cx);
    assert!(
        e.read_with(cx, |s, _| s
            .scroll
            .state
            .logical_scroll_top()
            .offset_in_item)
            < partial.offset_in_item
    );
}

#[gpui::test]
fn fresh_reverse_entry_wraps_to_unrendered_last_row(cx: &mut TestAppContext) {
    let (e, _, cx) = harness(cx);
    cx.update(|w, cx| {
        w.resize(gpui::size(gpui::px(880.), gpui::px(720.)));
        w.bounds_changed(cx);
        w.focus(&e.read(cx).search.read(cx).focus_handle(cx), cx);
    });
    draw(cx);
    assert!(e.read_with(cx, |s, _| s.rendered.end < 199));
    cx.simulate_keystrokes("shift-tab");
    draw(cx);
    draw(cx);
    assert_row_action_visible(&e, 199, cx);
    cx.simulate_keystrokes("shift-tab shift-tab");
    draw(cx);
    assert_row_action_visible(&e, 198, cx);
    cx.simulate_keystrokes("tab tab tab");
    draw(cx);
    cx.update(|w, cx| assert!(e.read(cx).search.read(cx).focus_handle(cx).is_focused(w)));
}

#[gpui::test]
fn fresh_reverse_entry_from_control_after_list_reaches_last_row(cx: &mut TestAppContext) {
    let after = cx.update(|cx| cx.focus_handle());
    let (e, _, cx) = harness_after(cx, Some(after.clone()));
    cx.update(|w, cx| {
        w.resize(gpui::size(gpui::px(880.), gpui::px(720.)));
        w.bounds_changed(cx);
        w.focus(&after, cx);
    });
    draw(cx);
    assert!(e.read_with(cx, |s, _| s.rendered.end < 199));
    cx.simulate_keystrokes("shift-tab");
    draw(cx);
    draw(cx);
    assert_row_action_visible(&e, 199, cx);
    cx.simulate_keystrokes("tab");
    draw(cx);
    cx.update(|w, _| assert!(after.is_focused(w)));
}

fn assert_row_action_visible(e: &Entity<History>, position: usize, cx: &mut VisualTestContext) {
    cx.update(|w, cx| {
        e.read_with(cx, |s, _| {
            let key = s.model.entries[s.model.matches[position]].key;
            assert!(s.row_focus.get(&key).is_some_and(|focus| focus.is_focused(w)),
                "expected transcript {} action to own focus; boundary focused={}, focused rows={:?}", position + 1,
                s.list_end_focus.is_focused(w),
                s.row_focus.iter().filter_map(|(key, focus)| focus.is_focused(w).then_some(*key)).collect::<Vec<_>>());
            s.scroll.assert_visible(position);
        });
    });
}

/// No explicit drain/receive after setup: every response must travel through
/// the gallery's attach/follow loop, including owner snapshots and action acks.
#[gpui::test]
fn gallery_follow_delivers_pin_copy_save_delete_import_and_reopen(cx: &mut TestAppContext) {
    let (e, host, cx) = harness(cx);
    cx.update(|w, cx| e.update(cx, |s, cx| s.hide(w, cx)));
    host.retry_import.set(true);
    e.update(cx, |s, cx| {
        s.fixture_clock = Some(clock());
        super::fixtures::attach(s, cx);
    });
    let tick = |cx: &mut VisualTestContext| {
        draw(cx);
        cx.executor().advance_clock(Duration::from_millis(64));
        draw(cx);
    };
    e.update(cx, |s, _| {
        s.request(super::RequestKind::Pin { entry: entry(200) })
    });
    tick(cx);
    assert!(!host.rows.borrow()[199].pinned);
    assert!(!e.read_with(cx, |s, _| s.model.entries[199].pinned));
    e.update(cx, |s, _| s.copy(200, false));
    tick(cx);
    assert_eq!(*host.clipboard.borrow(), entry(200).text);
    assert_eq!(e.read_with(cx, |s, _| s.model.notice), Some("Copied"));
    for format in 0..3 {
        cx.update(|w, cx| e.update(cx, |s, cx| s.action(&format!("save-{format}"), w, cx)));
        tick(cx);
        assert_eq!(e.read_with(cx, |s, _| s.model.notice), Some("Export saved"));
    }
    assert_eq!(host.saved.borrow().len(), 3);
    let exported: serde_json::Value = serde_json::from_str(&host.saved.borrow()[2].1).unwrap();
    assert_eq!(exported["entries"].as_array().unwrap().len(), 200);
    host.cancel_save.set(true);
    cx.update(|w, cx| e.update(cx, |s, cx| s.action("save-2", w, cx)));
    tick(cx);
    assert_eq!(host.saved.borrow().len(), 3);
    assert_eq!(e.read_with(cx, |s, _| s.model.notice), None);
    cx.update(|w, cx| {
        e.update(cx, |s, cx| {
            s.delete(DeleteScope::Row(200), w, cx);
            s.delete(DeleteScope::Row(200), w, cx);
        })
    });
    tick(cx);
    assert_eq!(e.read_with(cx, |s, _| s.model.entries.len()), 199);
    e.update(cx, |s, _| s.request(super::RequestKind::RetryImport));
    tick(cx);
    assert!(!e.read_with(cx, |s, _| s.model.retry_import));
    assert!(e.read_with(cx, |s, _| s.model.entries.iter().any(|e| e.key == 201)));
    cx.update(|w, cx| e.update(cx, |s, cx| s.hide(w, cx)));
    e.update(cx, super::fixtures::attach);
    e.update(cx, |s, _| s.copy(201, false));
    tick(cx);
    assert_eq!(e.read_with(cx, |s, _| s.model.notice), Some("Copied"));
    assert_eq!(host.stops.get(), 2);
    e.update(cx, |s, cx| {
        s.query(
            Query {
                text: "transcript 199:".into(),
                ..Default::default()
            },
            cx,
        )
    });
    cx.update(|w, cx| {
        e.update(cx, |s, cx| {
            s.delete(DeleteScope::Shown, w, cx);
            s.delete(DeleteScope::Shown, w, cx);
        })
    });
    tick(cx);
    assert!(e.read_with(cx, |s, _| s.model.matches.is_empty()));
    assert!(!host.rows.borrow().iter().any(|e| e.key == 199));
    cx.update(|w, cx| {
        e.update(cx, |s, cx| {
            s.delete(DeleteScope::All, w, cx);
            s.delete(DeleteScope::All, w, cx);
        })
    });
    tick(cx);
    assert!(host.rows.borrow().is_empty());
    assert!(e.read_with(cx, |s, _| s.model.entries.is_empty()));
    assert!(e.read_with(cx, |s, _| s.model.ticket().clear_epoch > 0));
}

#[test]
fn calendar_gallery_fixtures_straddle_real_local_day_boundaries() {
    use chrono::TimeZone;
    for (name, hour, zone_name) in [
        ("santiago", 1, "-03"),
        ("havana", 0, "CDT"),
        ("tokyo", 0, "JST"),
    ] {
        let (zone, clock, rows) = super::fixtures::calendar(name);
        let start = zone.timestamp_millis_opt(clock.today).unwrap();
        assert_eq!(start.format("%H").to_string(), format!("{hour:02}"));
        assert_eq!(start.format("%Z").to_string(), zone_name);
        let q = Query {
            date: Date::Today,
            ..Default::default()
        };
        assert_eq!(q.matching(&rows, clock).len(), 3);
        assert_eq!(Query::default().matching(&rows, clock).len(), 4);
    }
}

#[cfg(feature = "a11y-tests")]
#[gpui::test]
fn ax_focus_reveals_partially_clipped_row_action(cx: &mut TestAppContext) {
    use gpui::{
        AccessibleAction,
        accesskit::{ActionRequest, TreeId},
    };
    let (e, _, cx) = harness(cx);
    cx.update(|w, cx| {
        w.resize(gpui::size(gpui::px(880.), gpui::px(720.)));
        w.bounds_changed(cx);
    });
    let partial = gpui::ListOffset {
        item_ix: 5,
        offset_in_item: gpui::px(20.),
    };
    e.update(cx, |s, cx| {
        s.scroll.state.scroll_to(partial);
        cx.notify();
    });
    draw(cx);
    let tree = ax::tree(cx);
    let id = ax::named(&tree, "Show more for transcript 6");
    let window = cx.update(|w, _| w.window_handle());
    assert_eq!(
        e.read_with(cx, |s, _| s.scroll.state.logical_scroll_top()),
        partial
    );
    cx.dispatch_accessibility_action(
        window,
        ActionRequest {
            action: AccessibleAction::Focus,
            target_tree: TreeId::ROOT,
            target_node: id,
            data: None,
        },
    );
    draw(cx);
    draw(cx);
    let tree = ax::tree(cx);
    let node = tree
        .nodes
        .iter()
        .find(|(node, _)| *node == id)
        .unwrap()
        .1
        .clone();
    let bounds = node.bounds().unwrap();
    let scale = cx.update(|w, _| w.scale_factor()) as f64;
    assert!(
        bounds.y1 <= 704. * scale,
        "AX-focused action below list viewport: {bounds:?}"
    );
    assert!(
        e.read_with(cx, |s, _| s
            .scroll
            .state
            .logical_scroll_top()
            .offset_in_item)
            < partial.offset_in_item
    );
}

#[gpui::test]
fn row_copy_is_a_keyboard_stop_and_activates_once_with_space_and_enter(cx: &mut TestAppContext) {
    let (e, host, cx) = harness(cx);
    tab_to_list(&e, cx);
    cx.simulate_keystrokes("tab");
    draw(cx);
    let key = e.read_with(cx, |s, _| s.model.entries[s.model.matches[0]].key);
    cx.update(|w, cx| assert!(e.read(cx).row_copy_focus[&key].is_focused(w)));
    cx.simulate_keystrokes("space");
    draw(cx);
    assert!(e.read_with(cx, |s, _| s.model.detail.is_none()));
    assert_eq!(e.read_with(cx, |s, _| s.copied_key), Some(key));
    let count = host.calls.borrow().len();
    cx.simulate_keystrokes("enter");
    draw(cx);
    assert!(e.read_with(cx, |s, _| s.model.detail.is_none()));
    assert_eq!(host.calls.borrow().len(), count + 1);
    cx.simulate_keystrokes("tab");
    draw(cx);
    cx.update(|w, cx| assert!(e.read(cx).row_focus[&key].is_focused(w)));
    cx.simulate_keystrokes("shift-tab");
    draw(cx);
    cx.update(|w, cx| assert!(e.read(cx).row_copy_focus[&key].is_focused(w)));
}

#[test]
fn initial_empty_history_epoch_zero_finishes_loading() {
    let mut model = Model::default();
    model.attach();
    let ticket = model.begin();
    assert!(model.event(Event::Read {
        ticket,
        revision: 0,
        clear_epoch: 0,
        result: Ok(Snapshot {
            entries: vec![],
            retain_history: true,
            retry_import: false
        }),
    }));
    assert!(!model.loading);
    assert!(model.entries.is_empty());
    assert!(!model.event(Event::Read {
        ticket,
        revision: 0,
        clear_epoch: 0,
        result: Err(Problem::Unavailable),
    }));
}
