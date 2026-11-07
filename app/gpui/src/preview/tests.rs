use super::model::*;
use murmur_core::{dictation::Status, overlay_geometry::*};
fn anchor(above: bool) -> Anchor {
    Anchor {
        frame: OverlayFrame {
            x: 600.,
            y: if above { 800. } else { 0. },
            width: 240.,
            height: 37.,
        },
        above,
        lift: 0.,
        work: PopoverWorkArea {
            left: 0.,
            top: 30.,
            right: 1440.,
            bottom: 880.,
        },
    }
}
fn active() -> Model {
    let mut m = Model::default();
    m.event(Event::Anchor(anchor(false)));
    m.event(Event::ConsentRead {
        epoch: 0,
        enabled: true,
    });
    m.event(Event::Lifecycle {
        generation: 7,
        status: Status::Recording,
    });
    m.event(Event::Partial {
        generation: 7,
        text: "Synthetic sample words".into(),
    });
    m
}
#[test]
fn consent_is_durable_latest_read_only_and_opt_out_clears_before_reply() {
    let mut m = active();
    assert!(m.visible());
    m.event(Event::ConsentChanged { opted_out: false });
    let old = m.consent_epoch();
    m.event(Event::ConsentChanged { opted_out: true });
    assert!(!m.visible());
    assert!(m.text().is_empty());
    m.event(Event::ConsentRead {
        epoch: old,
        enabled: true,
    });
    assert!(!m.visible());
    m.event(Event::ConsentRead {
        epoch: m.consent_epoch(),
        enabled: false,
    });
    m.event(Event::Partial {
        generation: 7,
        text: "Synthetic hidden".into(),
    });
    assert!(m.text().is_empty());
    let mut fresh = Model::default();
    fresh.event(Event::ConsentChanged { opted_out: false });
    fresh.event(Event::Lifecycle {
        generation: 9,
        status: Status::Recording,
    });
    fresh.event(Event::Partial {
        generation: 9,
        text: "Synthetic forged consent".into(),
    });
    assert!(fresh.text().is_empty());
}
#[test]
fn disabled_lifecycle_floor_survives_reenable_and_status_reordering() {
    for status in [Status::Idle, Status::Processing, Status::Recovering] {
        let mut m = active();
        m.event(Event::ConsentChanged { opted_out: true });
        m.event(Event::Lifecycle {
            generation: 7,
            status,
        });
        m.event(Event::ConsentRead {
            epoch: m.consent_epoch(),
            enabled: true,
        });
        m.event(Event::Lifecycle {
            generation: 7,
            status: Status::Recording,
        });
        m.event(Event::Partial {
            generation: 7,
            text: "Synthetic stale".into(),
        });
        assert!(!m.visible());
        assert!(m.text().is_empty());
    }
}
#[test]
fn cancellation_and_new_generations_reject_old_text_and_presentation_callbacks() {
    let mut m = active();
    let old = m.ticket().unwrap();
    m.event(Event::Lifecycle {
        generation: 8,
        status: Status::Recording,
    });
    m.event(Event::Partial {
        generation: 8,
        text: "Synthetic successor".into(),
    });
    m.rejected(old);
    assert!(m.visible());
    assert!(!m.owns(old));
    m.event(Event::Partial {
        generation: 7,
        text: "Synthetic stale".into(),
    });
    assert_eq!(m.text(), "Synthetic successor");
    m.event(Event::Cancelled(7));
    assert!(m.visible());
    m.event(Event::Cancelled(8));
    assert!(m.text().is_empty());
    m.event(Event::Partial {
        generation: 8,
        text: "Synthetic cancelled".into(),
    });
    assert!(!m.visible());
}
#[test]
fn newer_partial_can_precede_generation_notice_but_not_stop_or_cancel_floor() {
    let mut m = active();
    m.event(Event::Partial {
        generation: 8,
        text: "Synthetic new".into(),
    });
    m.event(Event::Lifecycle {
        generation: 8,
        status: Status::Recording,
    });
    assert_eq!(m.text(), "Synthetic new");
    m.event(Event::Cancelled(9));
    m.event(Event::Lifecycle {
        generation: 9,
        status: Status::Recording,
    });
    m.event(Event::Partial {
        generation: 9,
        text: "Synthetic late".into(),
    });
    assert!(m.text().is_empty());
}
#[test]
fn empty_oversized_and_unicode_inputs_stay_bounded_and_current() {
    let mut m = active();
    m.event(Event::Partial {
        generation: 7,
        text: " ".into(),
    });
    assert!(!m.visible());
    m.event(Event::Partial {
        generation: 7,
        text: "x".repeat(4097),
    });
    assert!(m.text().is_empty());
    assert_eq!(visible_partial("  Synthetic sample  "), "Synthetic sample");
    assert_eq!(visible_partial(&"x".repeat(400)), "x".repeat(320));
    assert_eq!(visible_partial(&"😀".repeat(200)), "😀".repeat(160));
    assert!(
        visible_partial(&format!("{} final synthetic words", "word ".repeat(200)))
            .ends_with("final synthetic words")
    );
    let text = format!(
        "{} supercalifragilistic {}",
        "a".repeat(200),
        "b".repeat(200)
    );
    assert!(visible_partial(&text).starts_with("supercalifragilistic "));
}
#[test]
fn anchor_uses_shared_geometry_keeps_window_fixed_during_lift_and_clamps() {
    for above in [false, true] {
        let mut a = anchor(above);
        let base = a.window(LIFT_RESERVE).unwrap();
        a.lift = 64.;
        assert_eq!(a.window(LIFT_RESERVE), Some(base));
        if above {
            assert_eq!(base.y + base.height, a.frame.y - 6.);
        } else {
            assert_eq!(base.y, a.frame.y + a.frame.height + 6.);
        }
        a.work = PopoverWorkArea {
            left: -900.,
            right: -600.,
            top: -300.,
            bottom: 100.,
        };
        a.frame.x = -850.;
        a.frame.y = -250.;
        let fit = a.window(LIFT_RESERVE).unwrap();
        assert_eq!(fit.width, 300.);
        assert_eq!(fit.x, -900.);
        assert!(fit.y >= -300. && fit.y + fit.height <= 100.);
        a.lift = f64::NAN;
        assert!(a.window(LIFT_RESERVE).is_none());
    }
    let mut m = active();
    let ticket = m.ticket().unwrap();
    m.event(Event::Detached);
    assert!(m.text().is_empty());
    assert!(!m.owns(ticket));
}
#[test]
fn ordered_bounded_inbox_never_coalesces_across_privacy_barriers() {
    let q = super::inbox::Inbox::default();
    for _ in 0..1000 {
        assert!(
            q.publish(Event::Partial {
                generation: 7,
                text: "Synthetic".into()
            })
            .is_ok()
        );
    }
    assert_eq!(q.drain().len(), 1);
    for _ in 0..256 {
        assert!(q.publish(Event::ConsentChanged { opted_out: true }).is_ok());
    }
    assert!(q.publish(Event::Cancelled(7)).is_err());
    q.drain();
    assert!(
        q.publish(Event::Partial {
            generation: 7,
            text: "x".repeat(4097)
        })
        .is_err()
    );
    assert!(
        q.publish(Event::Partial {
            generation: 7,
            text: "Synthetic old".into()
        })
        .is_ok()
    );
    assert!(q.publish(Event::Cancelled(7)).is_ok());
    assert!(
        q.publish(Event::Partial {
            generation: 7,
            text: "Synthetic late".into()
        })
        .is_ok()
    );
    let mut m = active();
    for event in q.drain() {
        m.event(event);
    }
    assert!(m.text().is_empty());
}

#[cfg(feature = "a11y-tests")]
#[gpui::test]
fn actual_preview_adapter_tree_is_informational_and_lifecycle_clears_text(
    cx: &mut gpui::TestAppContext,
) {
    use gpui::{prelude::*, *};
    use std::rc::Rc;
    struct Current;
    impl super::Host for Current {
        fn is_current(&self, id: u64) -> bool {
            id == 7
        }
    }
    cx.update(|cx| {
        crate::init_with_preferences(
            cx,
            Some(crate::preferences::DisplayPreferences {
                reduce_motion: true,
                ..Default::default()
            }),
        )
    });
    let mut entity = None;
    let (_, cx) = cx.add_window_view(|w, cx| {
        let p = cx.new(|_| {
            let mut p = super::Preview::new(Rc::new(Current));
            p.model = active();
            p
        });
        entity = Some(p.clone());
        gpui_base::Root::new(p, w, cx)
    });
    let entity = entity.unwrap();
    let handle = cx.update(|w, _| w.window_handle());
    cx.activate_accessibility(handle);
    cx.run_until_parked();
    cx.update(|w, cx| w.draw(cx).clear(cx));
    let tree = cx.accessibility_tree(handle);
    let focus = tree.focus;
    let id = tree
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Words recognized so far"))
        .expect("status node")
        .0;
    assert!(tree.nodes.iter().any(|(_, n)| {
        n.value()
            .is_some_and(|s| s.contains("Synthetic sample words"))
    }));
    let content_id = tree
        .nodes
        .iter()
        .find(|(_, n)| n.value() == Some("Synthetic sample words"))
        .expect("actual content node")
        .0;
    let (content_role, _, content_pressable, content_action) =
        accesskit_macos::test_press(tree.clone(), content_id);
    assert_eq!(content_role, "AXStaticText");
    assert!(!content_pressable);
    assert!(content_action.is_none());
    let (role, _, pressable, action) = accesskit_macos::test_press(tree, id);
    assert_eq!(role, "AXGroup");
    assert!(!pressable);
    assert!(action.is_none());
    cx.update(|w, cx| {
        entity.update(cx, |s, cx| {
            s.receive(Event::ConsentChanged { opted_out: true }, w, cx)
        })
    });
    cx.update(|w, cx| w.draw(cx).clear(cx));
    let tree = cx.accessibility_tree(handle);
    assert_eq!(tree.focus, focus);
    assert!(!tree.nodes.iter().any(|(_, n)| {
        n.value()
            .is_some_and(|s| s.contains("Synthetic sample words"))
    }));
}

#[cfg(feature = "a11y-tests")]
#[gpui::test]
fn queued_show_rechecks_host_ownership_and_layout_shrink_wraps(cx: &mut gpui::TestAppContext) {
    use gpui::{prelude::*, *};
    use std::{cell::Cell, rc::Rc};
    struct Current(Rc<Cell<bool>>);
    impl super::Host for Current {
        fn is_current(&self, _: u64) -> bool {
            self.0.get()
        }
    }
    let current = Rc::new(Cell::new(true));
    let gate = current.clone();
    cx.update(|cx| crate::init_with_preferences(cx, Some(Default::default())));
    let mut entity = None;
    let (_, cx) = cx.add_window_view(|w, cx| {
        let p = cx.new(|_| {
            let mut p = super::Preview::new(Rc::new(Current(gate)));
            p.model = active();
            p
        });
        entity = Some(p.clone());
        gpui_base::Root::new(p, w, cx)
    });
    let entity = entity.unwrap();
    cx.update(|w, cx| {
        let short = super::card_size("Synthetic", 460., w);
        let long = super::card_size(&"Synthetic wrapped words ".repeat(30), 460., w);
        assert!(short.0 < long.0);
        assert!(short.1 < long.1);
        assert!(long.1 <= 85.);
        w.draw(cx).clear(cx);
    });
    current.set(false);
    cx.update(|w, cx| entity.update(cx, |s, cx| s.advance(w, cx)));
    cx.run_until_parked();
    assert!(entity.read_with(cx, |s, _| s.model.text().is_empty()));
}

#[test]
fn stable_boundary_is_only_accepted_from_explicit_authority_and_cleared_with_content() {
    let mut m = active();
    assert_eq!(m.stable_bytes(), 0);
    m.event(Event::AuthoritativePartial {
        generation: 7,
        stable: "Synthetic stable".into(),
        tentative: "synthetic tentative".into(),
    });
    assert_eq!(m.stable_bytes(), "Synthetic stable".len());
    assert_eq!(m.text(), "Synthetic stable synthetic tentative");
    m.event(Event::Partial {
        generation: 7,
        text: "Synthetic wholly provisional".into(),
    });
    assert_eq!(m.stable_bytes(), 0);
    m.event(Event::Detached);
    assert_eq!(m.stable_bytes(), 0);
    assert!(m.text().is_empty());
}

#[test]
fn closed_consumer_releases_queued_text_and_refuses_late_producers() {
    let q = super::inbox::Inbox::default();
    assert!(
        q.publish(Event::Partial {
            generation: 7,
            text: "Synthetic queued".into()
        })
        .is_ok()
    );
    let producer = q.clone();
    q.close();
    assert!(producer.is_closed());
    assert!(q.drain().is_empty());
    assert!(
        producer
            .publish(Event::Partial {
                generation: 7,
                text: "Synthetic late".into()
            })
            .is_err()
    );
}

#[cfg(feature = "a11y-tests")]
#[gpui::test]
fn first_frame_callback_cannot_show_after_queued_opt_out_or_clear_successor(
    cx: &mut gpui::TestAppContext,
) {
    use gpui::{prelude::*, *};
    use std::rc::Rc;
    struct Current;
    impl super::Host for Current {
        fn is_current(&self, _: u64) -> bool {
            true
        }
    }
    cx.update(|cx| crate::init_with_preferences(cx, Some(Default::default())));
    let mut entity = None;
    let (_, cx) = cx.add_window_view(|w, cx| {
        let p = cx.new(|_| {
            let mut p = super::Preview::new(Rc::new(Current));
            p.model = active();
            p
        });
        entity = Some(p.clone());
        gpui_base::Root::new(p, w, cx)
    });
    let entity = entity.unwrap();
    cx.update(|w, cx| {
        entity.update(cx, |s, cx| {
            let old = s.model.ticket().unwrap();
            assert!(
                s.inbox
                    .publish(Event::ConsentChanged { opted_out: true })
                    .is_ok()
            );
            s.admit_frame(old, w, cx);
            assert!(s.presented.is_none());
            assert!(s.model.text().is_empty());
            s.model = active();
            let old = s.model.ticket().unwrap();
            s.model.event(Event::Lifecycle {
                generation: 8,
                status: Status::Recording,
            });
            s.model.event(Event::Partial {
                generation: 8,
                text: "Synthetic successor".into(),
            });
            s.admit_frame(old, w, cx);
            assert!(s.presented.is_none());
            assert_eq!(s.model.text(), "Synthetic successor");
        })
    });
}

#[cfg(feature = "a11y-tests")]
#[gpui::test]
fn overlay_applied_anchor_interface_drives_preview_lift_without_moving_window(
    cx: &mut gpui::TestAppContext,
) {
    use gpui::{prelude::*, *};
    use std::{cell::Cell, rc::Rc};
    struct Bridge(Cell<Option<(OverlayFrame, bool, f64)>>);
    impl crate::overlay::Host for Bridge {
        fn submit(&self, _: crate::overlay::model::Intent) {}
        fn waveform_interest(&self, _: murmur_core::events::WaveformConsumer, _: bool) {}
        fn preview_anchor(&self, frame: OverlayFrame, above: bool, lift: f64) {
            self.0.set(Some((frame, above, lift)));
        }
    }
    cx.update(|cx| {
        crate::init_with_preferences(
            cx,
            Some(crate::preferences::DisplayPreferences {
                reduce_motion: true,
                ..Default::default()
            }),
        )
    });
    let bridge = Rc::new(Bridge(Cell::new(None)));
    let host = bridge.clone();
    let mut entity = None;
    let (_, cx) = cx.add_window_view(|w, cx| {
        let overlay = cx.new(|_| {
            crate::overlay::Overlay::new(
                crate::overlay::model::Snapshot {
                    generation: 7,
                    status: Status::Recording,
                    ..Default::default()
                },
                host,
            )
        });
        entity = Some(overlay.clone());
        gpui_base::Root::new(overlay, w, cx)
    });
    let entity = entity.unwrap();
    cx.update(|w, cx| entity.update(cx, |s, cx| s.advance(w, cx)));
    let (frame, above, lift) = bridge.0.get().expect("acknowledged collapsed anchor");
    assert_eq!(lift, 0.);
    let mut preview = active();
    let a = super::model::Anchor {
        frame,
        above,
        lift,
        ..anchor(above)
    };
    preview.event(Event::Anchor(a));
    let initial = preview.anchor.unwrap().window(preview.reserve);
    cx.update(|w, cx| {
        entity.update(cx, |s, cx| {
            s.model.expand(0);
            s.advance(w, cx);
        })
    });
    let (frame, above, lift) = bridge.0.get().expect("acknowledged hover anchor");
    assert_eq!(lift, 64.);
    preview.event(Event::Anchor(super::model::Anchor {
        frame,
        above,
        lift,
        ..a
    }));
    assert_eq!(preview.anchor.unwrap().window(preview.reserve), initial);
}

#[test]
fn identical_partial_does_not_repeat_speech_or_invalidate_its_painted_ticket() {
    let mut m = active();
    let ticket = m.ticket();
    m.event(Event::Partial {
        generation: 7,
        text: "Synthetic sample words".into(),
    });
    assert_eq!(m.ticket(), ticket);
    m.event(Event::Partial {
        generation: 7,
        text: "Synthetic revised words".into(),
    });
    assert_ne!(m.ticket(), ticket);
}
