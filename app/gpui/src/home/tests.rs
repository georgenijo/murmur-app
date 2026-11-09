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
fn tab_from_unfocused_home_enters_content_without_activation_focus(cx: &mut TestAppContext) {
    let (e, _, cx) = harness(cx);
    // Match focus:false in the native host. Merely drawing leaves focus alone.
    cx.update(|w, cx| w.blur(cx));
    draw(cx);
    assert!(cx.update(|w, cx| w.focused(cx).is_none()));
    keys(cx, "cmd-tab");
    assert!(cx.update(|w, cx| w.focused(cx).is_none()));
    keys(cx, "tab");
    assert!(cx.update(|w, cx| e.read(cx).focus["sidebar-toggle"].is_focused(w)));
    keys(cx, "tab");
    assert!(cx.update(|w, cx| e.read(cx).focus["settings"].is_focused(w)));
    cx.update(|w, cx| w.blur(cx));
    draw(cx);
    keys(cx, "shift-tab");
    assert!(cx.update(|w, cx| w.focused(cx).is_some()));
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
fn main_window_shortcuts_follow_editability_and_include_performance(cx: &mut TestAppContext) {
    let (e, h, cx) = harness(cx);
    cx.update(|w, cx| w.focus(&e.read(cx).focus["settings"].clone(), cx));
    // Outside a text field the Control form works like the Command form.
    keys(cx, "ctrl-l");
    assert_eq!(
        h.actions.borrow().last(),
        Some(&Action::Open(Destination::Performance))
    );
    keys(cx, "ctrl-,");
    assert_eq!(
        h.actions.borrow().last(),
        Some(&Action::Open(Destination::General))
    );
    // Option and Shift combinations pass through.
    let before = h.actions.borrow().len();
    keys(cx, "alt-k cmd-shift-k");
    assert_eq!(h.actions.borrow().len(), before);
    // In the search field only Command shortcuts apply: Control keeps editing.
    cx.update(|w, cx| {
        let search = e.read(cx).history.read(cx).search.read(cx).focus_handle(cx);
        w.focus(&search, cx)
    });
    keys(cx, "ctrl-k ctrl-l ctrl-,");
    assert_eq!(h.actions.borrow().len(), before);
    keys(cx, "cmd-l");
    assert_eq!(
        h.actions.borrow().last(),
        Some(&Action::Open(Destination::Performance))
    );
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
    fn monthly_totals_checklist_and_hints_have_static_text(cx: &mut TestAppContext) {
        let (e, _, cx) = harness(cx);
        for _ in 0..2 {
            let t = tree(cx);
            for text in [
                "This month 420 words · 7 dictations".to_string(),
                "Get more from Murmur · 0/5".to_string(),
                "Try what is ready when you are".to_string(),
            ]
            .into_iter()
            .chain(ITEMS.into_iter().flat_map(|(_, title, detail, _)| {
                [format!("{title}: Not completed"), detail.to_string()]
            })) {
                let matching: Vec<_> = t
                    .nodes
                    .iter()
                    .filter(|(_, n)| {
                        n.role() == gpui::accesskit::Role::Label && n.value() == Some(text.as_str())
                    })
                    .collect();
                assert_eq!(
                    matching.len(),
                    1,
                    "missing or duplicated static text: {text}"
                );
            }
        }
        for (id, _, detail, _) in HINTS {
            e.update(cx, |s, cx| {
                s.model.snapshot.as_mut().unwrap().discovery["pendingHints"] =
                    serde_json::json!([id]);
                cx.notify();
            });
            let t = tree(cx);
            assert!(t.nodes.iter().any(
                |(_, n)| n.role() == gpui::accesskit::Role::Label && n.value() == Some(detail)
            ));
        }
    }

    #[gpui::test]
    fn compact_checklist_keyboard_focus_stays_inside_scroll_viewport(cx: &mut TestAppContext) {
        let (e, _, cx) = harness(cx);
        cx.simulate_resize(gpui::size(gpui::px(720.), gpui::px(560.)));
        draw(cx);
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
            let origin = e.read_with(cx, |s, _| s.focus["sidebar-toggle"].clone());
            cx.update(|w, cx| w.focus(&origin, cx));
            let mut visited = std::collections::BTreeSet::new();
            for key in ["tab", "shift-tab"] {
                for _ in 0..45 {
                    keys(cx, key);
                    cx.update(|w, cx| w.simulate_next_frame(cx));
                    draw(cx);
                    let t = tree(cx);
                    let n = &t
                        .nodes
                        .iter()
                        .find(|(id, _)| *id == t.focus)
                        .expect("focus")
                        .1;
                    if let Some(label) = n.label()
                        && (super::ITEMS
                            .iter()
                            .any(|(_, title, _, _)| label.starts_with(title))
                            || label == "Dismiss Get more from Murmur")
                    {
                        visited.insert(label.to_owned());
                        let b = n.bounds().unwrap();
                        let scale = cx.update(|w, _| f64::from(w.scale_factor()));
                        let v = e.read_with(cx, |s, _| s.scroll.bounds());
                        assert!(
                            b.y0 >= scale * f64::from(f32::from(v.top()))
                                && b.y1 <= scale * f64::from(f32::from(v.bottom())),
                            "{label} {b:?} outside {v:?}"
                        );
                    }
                    if cx.update(|w, _| origin.is_focused(w)) {
                        break;
                    }
                }
            }
            assert_eq!(visited.len(), 6, "all checklist controls reached");
        }
    }
    #[gpui::test]
    fn sidebar_splitter_paints_focus_ring_and_keeps_ax_contract(cx: &mut TestAppContext) {
        let (e, _, cx) = harness(cx);
        let t = tree(cx);
        let id = named(&t, "Sidebar width");
        let node = &t.nodes.iter().find(|(i, _)| *i == id).unwrap().1;
        assert_eq!(node.role(), gpui::Role::Splitter);
        assert_eq!(node.numeric_value(), Some(256.));
        cx.update(|w, cx| w.focus(&e.read(cx).focus["nav-insights"].clone(), cx));
        keys(cx, "tab");
        assert!(
            cx.update(|w, cx| e.read(cx).focus["sidebar-width"].is_focused(w)),
            "Tab must reach the splitter"
        );
        keys(cx, "right");
        let t = tree(cx);
        let node = &t
            .nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Sidebar width"))
            .unwrap()
            .1;
        assert_eq!(node.numeric_value(), Some(272.));
        cx.update(|w, cx| {
            w.draw(cx).clear(cx);
            let p = crate::tokens::Palette::get(cx);
            assert!(
                w.painted_quads().iter().any(|q| q.bounds.size.width
                    == gpui::px(8.).scale(w.scale_factor())
                    && q.border_color == p.focus),
                "splitter has no painted focus border"
            );
        });
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

#[gpui::test]
fn home_command_f_exits_detail_and_searches_the_real_embedded_list(cx: &mut TestAppContext) {
    let (e, _, cx) = harness(cx);
    e.update(cx, |s, cx| {
        s.history.update(cx, |h, cx| {
            h.model.selected = Some(199);
            h.model.detail = Some(199);
            cx.notify();
        })
    });
    draw(cx);
    cx.update(|w, cx| w.focus(&e.read(cx).focus["settings"].clone(), cx));
    keys(cx, "cmd-f");
    assert!(e.read_with(cx, |s, cx| s.history.read(cx).model.detail.is_none()));
    assert_eq!(
        e.read_with(cx, |s, cx| s.history.read(cx).model.selected),
        Some(199)
    );
    cx.simulate_input("transcript 199:");
    draw(cx);
    assert_eq!(
        e.read_with(cx, |s, cx| s.history.read(cx).model.matches.len()),
        1
    );
}

#[gpui::test]
fn palette_shortcut_restores_the_originating_history_editor(cx: &mut TestAppContext) {
    let (e, _, cx) = harness(cx);
    cx.update(|w, cx| w.focus(&e.read(cx).focus["settings"].clone(), cx));
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
    keys(cx, "cmd-k");
    cx.update(|w, cx| e.update(cx, |s, cx| s.destination_closed(w, cx)));
    draw(cx);
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
fn command_f_in_embedded_meetings_focuses_visible_meeting_search(cx: &mut TestAppContext) {
    let (home, _, cx) = harness(cx);
    let meetings = cx.update(|w, cx| {
        cx.new(|cx| {
            crate::meetings::Meetings::new(
                Rc::new(crate::meetings::fixtures::FixtureHost::default()),
                w,
                cx,
            )
        })
    });
    home.update(cx, |s, cx| {
        s.workspace = Some((Destination::Meetings, meetings.clone().into()));
        cx.notify();
    });
    meetings.update(cx, |s, cx| {
        s.attach(cx);
        s.drain(cx);
    });
    draw(cx);
    cx.update(|w, cx| w.focus(&meetings.read(cx).search.read(cx).focus_handle(cx), cx));
    keys(cx, "tab");
    assert!(cx.update(|w, cx| {
        !meetings
            .read(cx)
            .search
            .read(cx)
            .focus_handle(cx)
            .is_focused(w)
    }));
    keys(cx, "cmd-f");
    cx.update(|w, cx| {
        assert!(
            meetings
                .read(cx)
                .search
                .read(cx)
                .focus_handle(cx)
                .is_focused(w)
        );
        assert!(
            !home
                .read(cx)
                .history
                .read(cx)
                .search
                .read(cx)
                .focus_handle(cx)
                .is_focused(w)
        );
    });
}
#[gpui::test]
fn talk_control_is_reached_by_keyboard_without_starting_recording(cx: &mut TestAppContext) {
    let (home, host, cx) = harness(cx);
    cx.update(|w, cx| w.focus(&home.read(cx).focus["settings"].clone(), cx));
    let count = host.actions.borrow().len();
    let mut reached = false;
    for _ in 0..20 {
        keys(cx, "tab");
        if cx.update(|w, cx| home.read(cx).focus["home-record"].is_focused(w)) {
            reached = true;
            break;
        }
    }
    assert!(reached);
    assert_eq!(host.actions.borrow().len(), count);
}
