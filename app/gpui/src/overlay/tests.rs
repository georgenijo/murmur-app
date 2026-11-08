use super::model::*;
#[cfg(feature = "a11y-tests")]
use super::{Host, Overlay};
#[cfg(feature = "a11y-tests")]
use gpui::{AccessibleAction, Role, TestAppContext, prelude::*};
use murmur_core::dictation::Status;
use murmur_core::overlay_geometry::*;
#[cfg(feature = "a11y-tests")]
use std::cell::RefCell;
#[cfg(feature = "a11y-tests")]
use std::rc::Rc;

fn active() -> Model {
    Model::new(Snapshot {
        generation: 7,
        status: Status::Recording,
        ..Snapshot::default()
    })
}
#[test]
fn cue_priority_and_expiry_match_current_overlay() {
    let mut m = active();
    assert_eq!(m.indicator(0), Indicator::Recording);
    m.event(Event::Failure(7, Failure::ChooseInput), 10);
    assert_eq!(m.indicator(10), Indicator::Failure(Failure::ChooseInput));
    assert!(!m.waveform(10));
    m.event(Event::Cancelled(7), 20);
    assert_eq!(m.indicator(20), Indicator::Cancelled);
    m.event(Event::Calibration(true), 21);
    assert_eq!(m.indicator(21), Indicator::Calibrating);
    m.event(Event::Calibration(false), 22);
    assert_eq!(m.indicator(821), Indicator::Failure(Failure::ChooseInput));
    assert_eq!(m.indicator(5011), Indicator::Recording);
}
#[test]
fn lifecycle_generation_rejects_stale_cues_clicks_and_retry_answers() {
    let mut m = active();
    m.event(
        Event::Lifecycle {
            generation: 7,
            status: Status::Idle,
            at_ms: 0,
        },
        0,
    );
    m.event(Event::Delivery(7, Delivery::Clipboard), 0);
    assert!(m.retry(1).is_some());
    let attempt = m.retry_attempt;
    m.event(
        Event::Lifecycle {
            generation: 8,
            status: Status::Starting,
            at_ms: 2,
        },
        2,
    );
    m.event(
        Event::RetryResult {
            generation: 7,
            attempt,
            result: Delivery::Pasted,
        },
        3,
    );
    m.event(Event::Failure(7, Failure::Retry), 3);
    m.event(Event::Delivery(7, Delivery::Clipboard), 3);
    m.event(Event::Cancelled(7), 3);
    assert_eq!(m.indicator(3), Indicator::Starting);
    m.event(
        Event::Lifecycle {
            generation: 7,
            status: Status::Idle,
            at_ms: 4,
        },
        4,
    );
    assert_eq!(m.snapshot.generation, 8);
    assert_eq!(m.snapshot.status, Status::Starting);
}
#[test]
fn delivery_duplicates_never_extend_cue_and_only_confirmed_clipboard_installs_it() {
    let mut m = Model::new(Snapshot {
        generation: 1,
        ..Snapshot::default()
    });
    m.event(Event::Delivery(1, Delivery::Clipboard), 10);
    m.event(Event::Delivery(1, Delivery::Clipboard), 5000);
    assert_eq!(m.indicator(5011), Indicator::Idle);
    for d in [Delivery::Pasted, Delivery::Nothing, Delivery::Retry] {
        m.snapshot.generation += 1;
        m.event(Event::Delivery(m.snapshot.generation, d), 6000);
        assert_eq!(m.indicator(6000), Indicator::Idle);
    }
}
#[test]
fn retry_is_single_flight_and_uses_a_fresh_bounded_result() {
    let mut m = Model::new(Snapshot {
        generation: 1,
        ..Snapshot::default()
    });
    m.event(Event::Delivery(1, Delivery::Clipboard), 0);
    m.pause_delivery(1);
    assert_eq!(m.indicator(6000), Indicator::Delivery(Delivery::Clipboard));
    m.resume_delivery(6000);
    let intent = m.retry(6001).unwrap();
    assert!(m.retry(2).is_none());
    assert_eq!(
        intent,
        Intent::RetryDelivery {
            generation: 1,
            attempt: m.retry_attempt
        }
    );
    m.event(
        Event::RetryResult {
            generation: 1,
            attempt: m.retry_attempt,
            result: Delivery::Retry,
        },
        6000,
    );
    assert_eq!(m.indicator(6001), Indicator::Delivery(Delivery::Retry));
    assert_eq!(m.indicator(11001), Indicator::Idle);
}
#[test]
fn click_delay_double_click_and_generation_ownership() {
    let mut m = active();
    assert!(m.click(1, 0).is_none());
    assert!(m.tick(249, false, |_, _| true).is_none());
    assert_eq!(
        m.tick(250, false, |_, _| true),
        Some(Intent::Control {
            generation: 7,
            input: Control::Stop
        })
    );
    m.click(1, 300);
    assert_eq!(
        m.click(2, 350),
        Some(Intent::Control {
            generation: 7,
            input: Control::Lock
        })
    );
    assert!(m.tick(600, false, |_, _| true).is_none());
    m.click(1, 700);
    m.event(
        Event::Lifecycle {
            generation: 8,
            status: Status::Recording,
            at_ms: 710,
        },
        710,
    );
    assert!(m.tick(950, false, |_, _| true).is_none());
    m.phase = Phase::Open;
    assert!(m.click(2, 1000).is_none());
}
#[test]
fn starting_click_cancels_and_disabled_recording_remains_stoppable() {
    let mut m = active();
    m.snapshot.status = Status::Starting;
    m.click(1, 0);
    assert_eq!(
        m.tick(250, false, |_, _| true),
        Some(Intent::Control {
            generation: 7,
            input: Control::Cancel
        })
    );
    m.snapshot.status = Status::Recording;
    m.snapshot.disabled = true;
    m.click(1, 300);
    assert_eq!(
        m.tick(550, false, |_, _| true),
        Some(Intent::Control {
            generation: 7,
            input: Control::Stop
        })
    );
    m.snapshot.status = Status::Idle;
    assert!(m.click(2, 600).is_none());
}
#[test]
fn hover_dwell_ack_leave_shrink_and_failed_grow() {
    let mut m = active();
    m.hover(true, 0);
    m.tick(149, false, |_, _| panic!("no early grow"));
    assert_eq!(m.phase, Phase::Collapsed);
    m.tick(150, false, |_, _| false);
    assert_eq!(m.phase, Phase::Collapsed);
    m.hover(true, 200);
    m.tick(350, false, |frame, _| {
        assert_eq!(frame.height, 101.);
        true
    });
    assert_eq!(m.phase, Phase::Open);
    m.hover(false, 400);
    m.tick(699, false, |_, _| panic!("no early shrink"));
    assert_eq!(m.phase, Phase::Open);
    m.tick(700, false, |_, _| true);
    assert_eq!(m.phase, Phase::Closing);
    m.hover(true, 710);
    m.hover(false, 720);
    m.tick(1080, false, |frame, _| {
        assert_eq!(frame.height, 37.);
        true
    });
    assert_eq!(m.phase, Phase::Collapsed);
}
#[test]
fn hidden_and_topology_reset_do_not_reopen_transparent_hit_area() {
    let mut m = active();
    m.hover(true, 0);
    m.event(Event::Visible(false), 10);
    m.tick(1000, false, |_, _| panic!("hidden"));
    assert_eq!(m.phase, Phase::Collapsed);
    m.event(Event::Visible(true), 1001);
    m.hover(true, 2001);
    m.event(
        Event::Screen(Screen {
            bounds: (-2560., 0., 0., 1440.),
            ..Screen::default()
        }),
        2010,
    );
    m.tick(3000, false, |_, _| panic!("reset"));
    assert_eq!(m.phase, Phase::Collapsed);
}
#[test]
fn elapsed_starts_at_recording_not_hover_and_done_is_preemptible() {
    let mut m = Model::new(Snapshot::default());
    m.event(
        Event::Lifecycle {
            generation: 1,
            status: Status::Recording,
            at_ms: 100,
        },
        100,
    );
    assert_eq!(m.elapsed(63100), "1:03");
    m.event(
        Event::Lifecycle {
            generation: 1,
            status: Status::Processing,
            at_ms: 64000,
        },
        64000,
    );
    m.event(
        Event::Lifecycle {
            generation: 1,
            status: Status::Idle,
            at_ms: 65000,
        },
        65000,
    );
    assert_eq!(m.indicator(65399), Indicator::Done);
    assert_eq!(m.indicator(65400), Indicator::Idle);
    m.event(
        Event::Lifecycle {
            generation: 2,
            status: Status::Starting,
            at_ms: 65100,
        },
        65100,
    );
    assert_eq!(m.indicator(65100), Indicator::Starting);
}
#[test]
fn waveform_is_bounded_and_only_the_current_visible_owner_can_supply_samples() {
    let mut m = active();
    m.sample(6, 1., 0);
    assert_eq!(m.level, 0.);
    m.sample(7, f32::NAN, 0);
    assert_eq!(m.level, 0.);
    m.sample(7, 1., 0);
    for count in [7, 18] {
        let bars = m.bars(count);
        assert_eq!(bars.len(), count);
        assert!(bars.iter().all(|v| (0.14..=1.).contains(v)));
    }
    m.event(Event::Visible(false), 1);
    m.sample(7, 0.1, 1);
    assert_eq!(m.level, 1.);
}
#[test]
fn pill_waveform_retains_per_bar_rhythm_and_responds_without_saturation() {
    let mut m = active();
    m.snapshot.placement = OverlayPlacement::FloatingPill;
    m.sample(7, 1., 0);
    let loud = m.bars(18);
    assert_eq!(loud, m.bars(18)); // repeatable offscreen fixtures
    assert_eq!(&loud[..5], &[0.3, 0.55, 0.8, 1., 0.7]);
    m.sample(7, 0.1, 0);
    assert_eq!(m.bars(18), loud); // speech-level RMS saturates the live gain
    m.sample(7, 0.03, 0);
    let quiet = m.bars(18);
    m.sample(7, 0.04, 0);
    let louder = m.bars(18);
    assert!(quiet.iter().any(|v| *v > 0.14));
    assert!(quiet.iter().zip(&louder).all(|(a, b)| a <= b));
    assert_ne!(quiet, louder);
    m.sample(7, 0., 0);
    assert_eq!(m.bars(18), vec![0.14; 18]);
    assert!(m.bars(0).is_empty());
}
#[test]
fn both_placements_use_the_shared_geometry_and_live_dock_limit() {
    let mut m = active();
    m.screen = Screen {
        bounds: (-2560., -100., 0., 1340.),
        work: (-2560., -70., 0., 1300.),
        ..Screen::default()
    };
    let f = m.frame(false, 0);
    assert_eq!(f.x, -1400.5);
    assert_eq!(f.y, -100.);
    m.snapshot.placement = OverlayPlacement::FloatingPill;
    let f = m.frame(false, 0);
    assert_eq!((f.width, f.height), (120., 34.));
    assert_eq!(f.y, 1252.);
    m.phase = Phase::Open;
    m.event(Event::Dock(DockLimit::Above(1100.)), 0);
    assert_eq!(m.phase, Phase::Open);
    let mut dock_only = m.screen.clone();
    dock_only.hidden_dock = Some(HiddenDock {
        tile: 128.,
        magnified: None,
    });
    m.event(Event::Screen(dock_only), 0);
    assert_eq!(m.phase, Phase::Open);
    assert_eq!(m.frame(false, 0).y, 1052.);
    assert_eq!(
        m.frame(true, 0).y + m.frame(true, 0).height,
        m.frame(false, 0).y + 34.
    );
}
#[test]
fn suggestion_requires_idle_and_retains_expanded_bounds_until_dismissal() {
    let mut m = Model::new(Snapshot::default());
    m.event(
        Event::Suggestion(Some(Suggestion {
            token: "synthetic".into(),
            title: "Synthetic meeting".into(),
            busy: false,
            failed: false,
        })),
        0,
    );
    m.tick(0, true, |frame, _| {
        assert_eq!(frame.width, 320.);
        assert_eq!(frame.height, 173.);
        true
    });
    assert_eq!(m.phase, Phase::Open);
    assert!(m.click(2, 1).is_none());
    m.event(
        Event::Lifecycle {
            generation: 1,
            status: Status::Starting,
            at_ms: 2,
        },
        2,
    );
    assert!(m.suggestion.is_none());
}
#[cfg(feature = "a11y-tests")]
#[derive(Default)]
struct FakeHost {
    intents: RefCell<Vec<Intent>>,
}
#[cfg(feature = "a11y-tests")]
impl Host for FakeHost {
    fn submit(&self, intent: Intent) {
        self.intents.borrow_mut().push(intent);
    }
    fn waveform_interest(&self, _: murmur_core::events::WaveformConsumer, _: bool) {}
    fn preview_anchor(&self, _: OverlayFrame, _: bool, _: f64) {}
}
#[cfg(feature = "a11y-tests")]
fn adapter_press(cx: &mut gpui::VisualTestContext, name: &str) {
    let window = cx.update(|w, _| w.window_handle());
    cx.update(|w, cx| w.draw(cx).clear(cx));
    let tree = cx.accessibility_tree(window);
    let id = tree
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some(name))
        .unwrap()
        .0;
    // Run the emitted tree through the vendored macOS adapter's role/title
    // mapping and production AXPress handler. Only its returned request crosses
    // the test-window transport; no test constructs a Click request.
    let (role, title, pressable, request) = accesskit_macos::test_press(tree, id);
    assert_eq!(role, "AXButton");
    assert_eq!(title.as_deref(), Some(name));
    assert!(pressable, "adapter must support AXPress for {name}");
    let request = request.expect("AXPress must dispatch");
    assert_eq!(request.action, AccessibleAction::Click);
    assert_eq!(request.target_node, id);
    assert_eq!(request.target_tree, gpui::accesskit::TreeId::ROOT);
    cx.dispatch_accessibility_action(window, request);
    cx.run_until_parked();
    cx.update(|w, cx| w.draw(cx).clear(cx));
}

#[cfg(feature = "a11y-tests")]
#[gpui::test]
fn actual_overlay_tree_controls_and_actions(cx: &mut TestAppContext) {
    cx.update(crate::init);
    cx.update(|cx| {
        crate::preferences::apply(
            crate::preferences::DisplayPreferences {
                reduce_motion: true,
                ..Default::default()
            },
            cx,
        )
    });
    let host = Rc::new(FakeHost::default());
    let cloned = host.clone();
    let (_, cx) = cx.add_window_view(|w, cx| {
        let overlay = cx.new(|_| Overlay::new(Snapshot::default(), cloned));
        gpui_base::Root::new(overlay, w, cx)
    });
    let window = cx.update(|w, _| w.window_handle());
    cx.activate_accessibility(window);
    cx.run_until_parked();
    cx.update(|w, cx| w.draw(cx).clear(cx));
    assert!(
        !cx.accessibility_tree(window)
            .nodes
            .iter()
            .any(|(_, n)| n.role() == Role::Group && n.label() == Some("Quick settings"))
    );
    let focus = cx.accessibility_tree(window).focus;
    adapter_press(cx, "Quick settings, collapsed");
    assert_eq!(
        cx.accessibility_tree(window).focus,
        focus,
        "disclosure must not move key focus"
    );
    assert!(
        host.intents.borrow().is_empty(),
        "disclosure must not start dictation"
    );
    let tree = cx.accessibility_tree(window);
    for (name, role) in [
        ("Quick settings, expanded", Role::Button),
        ("Quick settings", Role::Group),
        ("Mode: Everyday. Click to cycle", Role::Button),
        ("Disable Murmur", Role::Button),
        ("Auto-paste", Role::Switch),
        ("Open settings", Role::Button),
        ("Ready", Role::Status),
    ] {
        assert!(
            tree.nodes
                .iter()
                .any(|(_, n)| n.label() == Some(name) && n.role() == role),
            "missing {name}"
        );
    }
    for name in [
        "Mode: Everyday. Click to cycle",
        "Disable Murmur",
        "Open settings",
    ] {
        adapter_press(cx, name);
    }
    // Switches map to AXCheckBox rather than AXButton, but share AXPress.
    let tree = cx.accessibility_tree(window);
    let id = tree
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Auto-paste"))
        .unwrap()
        .0;
    let (role, _, pressable, request) = accesskit_macos::test_press(tree, id);
    assert_eq!(role, "AXCheckBox");
    assert!(pressable);
    cx.dispatch_accessibility_action(window, request.unwrap());
    cx.run_until_parked();
    assert_eq!(
        *host.intents.borrow(),
        vec![
            Intent::CycleMode,
            Intent::Disabled(true),
            Intent::OpenSettings,
            Intent::AutoPaste(false)
        ]
    );
    adapter_press(cx, "Quick settings, expanded");
    let tree = cx.accessibility_tree(window);
    assert!(
        !tree
            .nodes
            .iter()
            .any(|(_, n)| n.label() == Some("Quick settings")),
        "collapsed controls must leave AX tree"
    );
    assert!(
        tree.nodes
            .iter()
            .any(|(_, n)| n.label() == Some("Quick settings, collapsed"))
    );
    let id = tree
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Ready"))
        .unwrap()
        .0;
    let (role, _, pressable, request) = accesskit_macos::test_press(tree, id);
    assert_eq!(role, "AXGroup");
    assert!(pressable);
    cx.dispatch_accessibility_action(window, request.unwrap());
    cx.run_until_parked();
    assert_eq!(
        host.intents.borrow().last(),
        Some(&Intent::Control {
            generation: 0,
            input: Control::Lock
        })
    );
}

#[test]
fn suggestion_actions_recheck_current_token_and_are_single_flight() {
    let mut m = Model::new(Snapshot::default());
    m.event(
        Event::Suggestion(Some(Suggestion {
            token: "new".into(),
            title: "Synthetic".into(),
            busy: false,
            failed: false,
        })),
        0,
    );
    assert!(m.suggestion_action("old", true).is_none());
    assert_eq!(
        m.suggestion_action("new", true),
        Some(Intent::AcceptSuggestion("new".into()))
    );
    assert!(m.suggestion_action("new", true).is_none());
    assert!(m.suggestion_action("new", false).is_none());
}

#[test]
fn hydration_cannot_roll_back_a_same_generation_event_and_restores_elapsed() {
    let mut m = Model::new(Snapshot {
        generation: 1,
        status: Status::Recording,
        elapsed_ms: 61000,
        ..Default::default()
    });
    assert_eq!(m.elapsed(1000), "1:02");
    let observed_epoch = m.hydration_epoch();
    m.event(
        Event::Lifecycle {
            generation: 1,
            status: Status::Processing,
            at_ms: 1000,
        },
        1000,
    );
    m.event(
        Event::Hydrate {
            snapshot: Snapshot {
                generation: 1,
                status: Status::Recording,
                revision: 100,
                ..Default::default()
            },
            observed_epoch,
        },
        1001,
    );
    assert_eq!(m.snapshot.status, Status::Processing);
}

#[test]
fn state_contractions_retain_native_bounds_then_release_without_anchor_drift() {
    let mut writer = SurfaceWriter::default();
    let large = OverlayFrame {
        x: 100.,
        y: 0.,
        width: 241.,
        height: 37.,
    };
    let small = OverlayFrame {
        x: 128.,
        y: 0.,
        width: 185.,
        height: 37.,
    };
    assert_eq!(writer.resolve(small, Some(large), 0, false, true), large);
    assert_eq!(writer.resolve(small, Some(large), 379, false, true), large);
    assert_eq!(writer.resolve(small, Some(large), 380, false, true), small);
    assert!(!writer.pending());
    assert_eq!(writer.resolve(small, Some(large), 0, false, false), small);
    let moved = OverlayFrame { y: 100., ..small };
    assert_eq!(writer.resolve(moved, Some(large), 0, false, true), moved);
}

#[test]
fn disabled_hover_can_reopen_quick_settings_but_cannot_start_recording() {
    let mut m = Model::new(Snapshot::default());
    m.hover(true, 0);
    m.tick(150, false, |_, _| true);
    assert_eq!(m.phase, Phase::Open);
    m.event(
        Event::Snapshot(Snapshot {
            disabled: true,
            revision: 1,
            ..m.snapshot.clone()
        }),
        200,
    );
    m.hover(false, 400);
    m.tick(700, false, |_, _| true);
    m.tick(1080, false, |_, _| true);
    assert_eq!(m.phase, Phase::Collapsed);
    assert!(m.accessible_activate(1100).is_none());
    m.hover(true, 1200);
    m.tick(1349, false, |_, _| panic!("dwell still pending"));
    m.tick(1350, false, |_, _| true);
    assert_eq!(m.phase, Phase::Open);
}

#[test]
fn accepted_recording_snapshots_rebase_elapsed_even_without_lifecycle_change() {
    let mut m = Model::new(Snapshot {
        elapsed_ms: 61000,
        ..active().snapshot
    });
    m.event(
        Event::Snapshot(Snapshot {
            elapsed_ms: 62000,
            revision: 2,
            ..m.snapshot.clone()
        }),
        1000,
    );
    assert_eq!(m.elapsed(1000), "1:02");
    assert_eq!(m.elapsed(2000), "1:03");
    m.event(
        Event::Hydrate {
            snapshot: Snapshot {
                elapsed_ms: 64000,
                revision: 3,
                ..m.snapshot.clone()
            },
            observed_epoch: m.hydration_epoch(),
        },
        3000,
    );
    assert_eq!(m.elapsed(3000), "1:04");
    assert_eq!(m.elapsed(4000), "1:05");
    // Rejected stale snapshots cannot move the current anchor or elapsed base.
    for snapshot in [
        Snapshot {
            revision: 2,
            ..m.snapshot.clone()
        },
        Snapshot {
            generation: 6,
            ..m.snapshot.clone()
        },
    ] {
        m.event(Event::Snapshot(snapshot), 5000);
        assert_eq!(m.elapsed(5000), "1:06");
    }
}

#[test]
fn shrink_preserves_a_pending_dwell_in_the_persistent_top_band() {
    let mut m = active();
    m.hover(true, 0);
    m.tick(150, false, |_, _| true);
    m.hover(false, 400);
    m.tick(700, false, |_, _| true);
    assert_eq!(m.phase, Phase::Closing);
    m.hover(true, 1000);
    m.tick(1080, false, |frame, _| {
        assert_eq!(frame.height, 37.);
        true
    });
    assert_eq!(m.phase, Phase::Collapsed);
    assert!(m.needs_tick());
    m.tick(1149, false, |_, _| panic!("no early grow"));
    // No second enter event: the pointer has remained inside throughout.
    m.tick(1150, false, |frame, _| {
        assert_eq!(frame.height, 101.);
        true
    });
    assert_eq!(m.phase, Phase::Open);
    assert!(!m.needs_tick());
}

#[cfg(feature = "a11y-tests")]
#[gpui::test]
fn painted_status_content_stays_in_notch_wings_and_centers_in_the_pill(cx: &mut TestAppContext) {
    cx.update(crate::init);
    cx.update(|cx| {
        crate::preferences::apply(
            crate::preferences::DisplayPreferences {
                reduce_motion: true,
                ..Default::default()
            },
            cx,
        )
    });
    let mut entity = None;
    let (_, cx) = cx.add_window_view(|w, cx| {
        let overlay = cx.new(|_| Overlay::new(active().snapshot, Rc::new(FakeHost::default())));
        entity = Some(overlay.clone());
        gpui_base::Root::new(overlay, w, cx)
    });
    let entity = entity.unwrap();
    for placement in [OverlayPlacement::Notch, OverlayPlacement::FloatingPill] {
        for physical in [true, false] {
            for expanded in [false, true] {
                for delivery in [false, true] {
                    entity.update(cx, |s, cx| {
                        s.model = active();
                        s.model.screen.notch = Some(NotchInfo {
                            width: 185.,
                            height: 37.,
                            physical_notch: physical,
                            menu_bar_visible: true,
                        });
                        s.model.snapshot.placement = placement;
                        s.model.phase = if expanded {
                            Phase::Open
                        } else {
                            Phase::Collapsed
                        };
                        if delivery {
                            s.model.snapshot.status = Status::Idle;
                            s.model.event(Event::Delivery(7, Delivery::Clipboard), 0);
                        }
                        cx.notify();
                    });
                    cx.run_until_parked();
                    cx.update(|w, cx| w.draw(cx).clear(cx));
                    let row = cx.debug_bounds("overlay-status").unwrap();
                    let geometry = entity.read_with(cx, |s, _| s.model.geometry(expanded, 0));
                    assert_eq!(row.size.width, gpui::px(geometry.window_w as f32));
                    assert_eq!(row.size.height, gpui::px(geometry.collapsed_h as f32));
                    let leading = cx.debug_bounds("overlay-leading").unwrap();
                    let trailing = cx.debug_bounds("overlay-trailing").unwrap();
                    let content = cx
                        .debug_bounds(if delivery {
                            "delivery-action"
                        } else {
                            "overlay-waveform"
                        })
                        .unwrap();
                    assert!(
                        leading.left() >= row.left(),
                        "placement={placement:?} physical={physical} expanded={expanded} delivery={delivery} row={row:?} leading={leading:?} trailing={trailing:?}"
                    );
                    assert!(content.left() >= trailing.left());
                    assert!(content.right() <= trailing.right());
                    assert!(trailing.right() <= row.right());
                    let center = |b: gpui::Bounds<gpui::Pixels>| (b.left() + b.right()) / 2.;
                    // GPUI snaps painted origins to the device pixel grid.
                    let half_pixel = gpui::px(0.5 / cx.update(|w, _| w.scale_factor()));
                    assert!((center(content) - center(trailing)).abs() <= half_pixel);
                    if placement == OverlayPlacement::Notch {
                        // The 185pt cutout is shared by the physical and pretend notch.
                        let notch_left = center(row) - gpui::px(92.5);
                        let notch_right = center(row) + gpui::px(92.5);
                        assert!(leading.right() <= notch_left);
                        assert!(trailing.left() >= notch_right);
                        assert_eq!(leading.size.width, gpui::px(WING as f32));
                        assert_eq!(trailing.size.width, gpui::px(WING as f32));
                        if !delivery {
                            assert_eq!(content.size.width, gpui::px(23.));
                        }
                    } else {
                        assert!(
                            (center(row) - (leading.left() + trailing.right()) / 2.).abs()
                                <= half_pixel,
                            "pill layout physical={physical} expanded={expanded} delivery={delivery}: row={row:?} leading={leading:?} trailing={trailing:?}"
                        );
                        if !delivery {
                            assert_eq!(content.size.width, gpui::px(61.5));
                        }
                    }
                }
            }
        }
    }
}

#[cfg(all(feature = "a11y-tests", target_os = "macos"))]
#[gpui::test]
fn indicator_cues_are_single_line_and_inside_the_minimum_card(cx: &mut TestAppContext) {
    // Use the production CoreText shaper: the default test font cannot expose
    // the width/wrapping regression in the two-character clipboard shortcut.
    let mut shaped = TestAppContext::build_with_text_system(
        cx.dispatcher.clone(),
        None,
        std::sync::Arc::new(gpui_apple::AppleTextSystem::new()),
    );
    shaped.update(crate::init);
    shaped.update(|cx| {
        crate::preferences::apply(
            crate::preferences::DisplayPreferences {
                reduce_motion: true,
                ..Default::default()
            },
            cx,
        )
    });
    let mut entity = None;
    let (_, cx) = shaped.add_window_view(|w, cx| {
        let overlay = cx.new(|_| Overlay::new(active().snapshot, Rc::new(FakeHost::default())));
        entity = Some(overlay.clone());
        gpui_base::Root::new(overlay, w, cx)
    });
    let overlay = entity.unwrap();
    let window = cx.update(|w, _| w.window_handle());
    cx.activate_accessibility(window);
    let indicators = [
        Indicator::Idle,
        Indicator::Starting,
        Indicator::Slow,
        Indicator::Recording,
        Indicator::Recovering,
        Indicator::Processing,
        Indicator::Done,
        Indicator::Cancelled,
        Indicator::TapMissed,
        Indicator::Failure(Failure::Retry),
        Indicator::Failure(Failure::Permission),
        Indicator::Failure(Failure::ChooseInput),
        Indicator::Failure(Failure::Partial),
        Indicator::Calibrating,
        Indicator::Meeting,
        Indicator::MeetingProcessing,
        Indicator::Delivery(Delivery::Clipboard),
        Indicator::Delivery(Delivery::Retrying),
        Indicator::Delivery(Delivery::Pasted),
        Indicator::Delivery(Delivery::Nothing),
        Indicator::Delivery(Delivery::Retry),
    ];
    for placement in [OverlayPlacement::Notch, OverlayPlacement::FloatingPill] {
        for physical in [true, false] {
            for expanded in [false, true] {
                for indicator in indicators {
                    if placement == OverlayPlacement::FloatingPill && indicator == Indicator::Idle {
                        continue; // The idle pill is intentionally hidden.
                    }
                    cx.update(|w, cx| {
                        overlay.update(cx, |s, cx| {
                            s.now = 0;
                            s.model = Model::new(Snapshot {
                                generation: 7,
                                hotkey_feedback: true,
                                placement,
                                ..Snapshot::default()
                            });
                            s.model.screen.notch = Some(NotchInfo {
                                width: 185.,
                                height: 24.,
                                physical_notch: physical,
                                menu_bar_visible: true,
                            });
                            s.model.phase = if expanded {
                                Phase::Open
                            } else {
                                Phase::Collapsed
                            };
                            match indicator {
                                Indicator::Starting | Indicator::Slow => {
                                    s.model.snapshot.status = Status::Starting;
                                    if indicator == Indicator::Slow {
                                        s.model.event(Event::Stalled(7), 0);
                                    }
                                }
                                Indicator::Recording => s.model.snapshot.status = Status::Recording,
                                Indicator::Recovering => {
                                    s.model.snapshot.status = Status::Recovering
                                }
                                Indicator::Processing => {
                                    s.model.snapshot.status = Status::Processing
                                }
                                Indicator::Done => {
                                    s.model.snapshot.status = Status::Processing;
                                    s.model.event(
                                        Event::Lifecycle {
                                            generation: 7,
                                            status: Status::Idle,
                                            at_ms: 0,
                                        },
                                        0,
                                    );
                                }
                                Indicator::Cancelled => s.model.event(Event::Cancelled(7), 0),
                                Indicator::TapMissed => s.model.event(Event::TapMissed, 0),
                                Indicator::Failure(f) => s.model.event(Event::Failure(7, f), 0),
                                Indicator::Calibrating => s.model.calibrating = true,
                                Indicator::Meeting => s.model.snapshot.meeting = Meeting::Recording,
                                Indicator::MeetingProcessing => {
                                    s.model.snapshot.meeting = Meeting::Processing
                                }
                                Indicator::Delivery(d) => {
                                    s.model.event(Event::Delivery(7, Delivery::Clipboard), 0);
                                    if d != Delivery::Clipboard {
                                        s.model.retry(0).unwrap();
                                        if d != Delivery::Retrying {
                                            s.model.event(
                                                Event::RetryResult {
                                                    generation: 7,
                                                    attempt: s.model.retry_attempt,
                                                    result: d,
                                                },
                                                0,
                                            );
                                        }
                                    }
                                }
                                Indicator::Idle => {}
                            }
                            assert_eq!(s.model.indicator(0), indicator);
                            cx.notify();
                        });
                        w.draw(cx).clear(cx);
                    });
                    let card = cx.debug_bounds("overlay-surface").unwrap();
                    let row = cx.debug_bounds("overlay-status").unwrap();
                    assert_eq!(
                        row.size.height,
                        gpui::px(if placement == OverlayPlacement::Notch {
                            24.
                        } else {
                            34.
                        })
                    );
                    let contains =
                        |outer: gpui::Bounds<gpui::Pixels>, inner: gpui::Bounds<gpui::Pixels>| {
                            assert!(
                                inner.left() >= outer.left()
                                    && inner.right() <= outer.right()
                                    && inner.top() >= outer.top()
                                    && inner.bottom() <= outer.bottom(),
                                "{indicator:?} {placement:?} physical={physical} expanded={expanded}: {inner:?} outside {outer:?}"
                            );
                        };
                    contains(card, row);
                    contains(card, cx.debug_bounds("overlay-leading").unwrap());
                    assert_eq!(
                        cx.debug_bounds("overlay-state-symbol").is_some(),
                        !matches!(
                            indicator,
                            Indicator::Processing
                                | Indicator::Recovering
                                | Indicator::MeetingProcessing
                        ) && (placement == OverlayPlacement::FloatingPill
                            || !matches!(indicator, Indicator::Starting | Indicator::Slow)),
                        "{indicator:?} must expose its rendered symbol bounds"
                    );
                    if let Some(symbol) = cx.debug_bounds("overlay-state-symbol") {
                        // Measure the intrinsic text child, not just its fixed
                        // 8 pt parent: nowrap must not conceal horizontal spill.
                        assert_eq!(
                            symbol.size.height,
                            if indicator == Indicator::Recording
                                || (indicator == Indicator::Meeting
                                    && placement == OverlayPlacement::FloatingPill)
                            {
                                gpui::px(10.)
                            } else if indicator == Indicator::Meeting {
                                gpui::px(8.)
                            } else {
                                gpui::px(12.)
                            },
                            "{indicator:?}"
                        );
                        contains(card, symbol);
                        contains(cx.debug_bounds("overlay-leading").unwrap(), symbol);
                        let disclosure = cx
                            .debug_bounds("overlay-quick-settings-disclosure")
                            .unwrap();
                        // The silent AX target no longer participates in wing
                        // layout or adds a visible caret beside the status.
                        assert_eq!(disclosure.size.width, gpui::px(20.));
                        if placement == OverlayPlacement::Notch
                            && indicator == Indicator::Delivery(Delivery::Clipboard)
                        {
                            contains(cx.debug_bounds("overlay-state-glyph").unwrap(), symbol);
                        }
                    }
                    if let Some(trailing) = cx.debug_bounds("overlay-trailing") {
                        contains(card, trailing);
                        assert!(
                            trailing.size.height <= gpui::px(24.),
                            "{indicator:?}: {trailing:?}"
                        );
                    }
                    let tree = cx.accessibility_tree(window);
                    assert!(
                        tree.nodes.iter().any(|(_, n)| n.role() == Role::Status
                            && n.label() == Some(indicator.label()))
                    );
                    if indicator == Indicator::Delivery(Delivery::Clipboard) {
                        assert!(
                            tree.nodes
                                .iter()
                                .any(|(_, n)| n.label()
                                    == Some("Copied. Paste manually or try again"))
                        );
                    }
                }
            }
        }
    }
}

#[cfg(feature = "a11y-tests")]
#[gpui::test]
fn disabled_collapsed_overlay_opens_and_closes_through_macos_adapter(cx: &mut TestAppContext) {
    cx.update(crate::init);
    cx.update(|cx| {
        crate::preferences::apply(
            crate::preferences::DisplayPreferences {
                reduce_motion: true,
                ..Default::default()
            },
            cx,
        )
    });
    let host = Rc::new(FakeHost::default());
    let (_, cx) = cx.add_window_view(|w, cx| {
        let overlay = cx.new(|_| {
            Overlay::new(
                Snapshot {
                    disabled: true,
                    ..Snapshot::default()
                },
                host.clone(),
            )
        });
        gpui_base::Root::new(overlay, w, cx)
    });
    let window = cx.update(|w, _| w.window_handle());
    cx.activate_accessibility(window);
    cx.run_until_parked();
    cx.update(|w, cx| w.draw(cx).clear(cx));
    let focus = cx.accessibility_tree(window).focus;
    assert!(
        !cx.accessibility_tree(window)
            .nodes
            .iter()
            .any(|(_, n)| n.label() == Some("Enable Murmur"))
    );
    adapter_press(cx, "Quick settings, collapsed");
    assert_eq!(cx.accessibility_tree(window).focus, focus);
    assert!(host.intents.borrow().is_empty());
    adapter_press(cx, "Enable Murmur");
    assert_eq!(*host.intents.borrow(), vec![Intent::Disabled(false)]);
    adapter_press(cx, "Quick settings, expanded");
    let tree = cx.accessibility_tree(window);
    assert!(
        tree.nodes
            .iter()
            .any(|(_, n)| n.label() == Some("Quick settings, collapsed"))
    );
    assert!(!tree.nodes.iter().any(|(_, n)| n.label() == Some("Enable Murmur") || n.label() == Some("Quick settings")));
    assert_eq!(tree.focus, focus);
    assert_eq!(
        *host.intents.borrow(),
        vec![Intent::Disabled(false)],
        "collapse must not send a recording or settings intent"
    );
}

#[test]
fn duplicate_snapshot_and_hydration_do_not_rewind_the_recording_timer() {
    let snapshot = Snapshot {
        revision: 1,
        generation: 7,
        status: Status::Recording,
        elapsed_ms: 61000,
        ..Default::default()
    };
    let mut m = Model::new(snapshot.clone());
    assert_eq!(m.elapsed(1000), "1:02");
    m.event(Event::Snapshot(snapshot.clone()), 1000);
    assert_eq!(m.elapsed(1000), "1:02");
    m.event(
        Event::Hydrate {
            snapshot: snapshot.clone(),
            observed_epoch: m.hydration_epoch(),
        },
        2000,
    );
    assert_eq!(m.elapsed(2000), "1:03");
    m.event(Event::Visible(true), 2500);
    m.event(
        Event::Hydrate {
            snapshot,
            observed_epoch: m.hydration_epoch(),
        },
        3000,
    );
    assert_eq!(m.elapsed(3000), "1:04");
    assert_eq!(m.started_ms, Some(0));
}

#[test]
fn initial_hydration_and_current_epoch_refresh_accept_newer_revisions() {
    let mut m = Model::new(Snapshot::default());
    m.event(
        Event::Hydrate {
            snapshot: Snapshot {
                revision: 1,
                generation: 7,
                status: Status::Recording,
                elapsed_ms: 61000,
                ..Default::default()
            },
            observed_epoch: m.hydration_epoch(),
        },
        5000,
    );
    assert_eq!(m.elapsed(6000), "1:02");
    let old_epoch = m.hydration_epoch();
    m.event(Event::Visible(true), 6000);
    let refresh = Snapshot {
        revision: 2,
        elapsed_ms: 63000,
        ..m.snapshot.clone()
    };
    m.event(
        Event::Hydrate {
            snapshot: refresh.clone(),
            observed_epoch: old_epoch,
        },
        7000,
    );
    assert_eq!(m.snapshot.revision, 1);
    assert_eq!(m.elapsed(7000), "1:03");
    m.event(
        Event::Hydrate {
            snapshot: refresh,
            observed_epoch: m.hydration_epoch(),
        },
        8000,
    );
    assert_eq!(m.snapshot.revision, 2);
    assert_eq!(m.elapsed(9000), "1:04");
}

#[test]
fn sampled_pointer_recovers_missed_exit_and_releases_expanded_hit_area() {
    for reduced in [false, true] {
        let mut m = active();
        m.snapshot.placement = OverlayPlacement::FloatingPill;
        let mut pointer = PointerPresence::default();
        let mut sample = |m: &mut Model, inside, now| {
            if pointer.observe(inside) {
                m.hover(inside, now);
            }
        };
        sample(&mut m, true, 0);
        sample(&mut m, true, 149);
        m.tick(149, reduced, |_, _| panic!("dwell"));
        m.tick(150, reduced, |_, _| true);
        assert_eq!(m.phase, Phase::Open);
        // No tracking exit is delivered. The next watchdog sample finds the
        // pointer outside even if resize or rapid movement lost the event.
        sample(&mut m, false, 183);
        for now in [216, 249, 282, 315, 348, 381, 414, 447] {
            sample(&mut m, false, now);
            m.tick(now, reduced, |_, _| panic!("leave delay"));
        }
        let mut applied = m.frame(true, 150);
        m.tick(483, reduced, |frame, _| {
            applied = frame;
            true
        });
        m.tick(863, reduced, |frame, _| {
            applied = frame;
            true
        });
        assert_eq!(m.phase, Phase::Collapsed);
        assert_eq!(applied, m.frame(false, 863));
        assert_eq!((applied.width, applied.height), (120., 34.));
    }
}

#[test]
fn pointer_samples_do_not_undo_ax_disclosure_or_restart_stationary_dwell() {
    let mut m = active();
    let mut pointer = PointerPresence::default();
    assert!(!pointer.observe(false));
    m.expand(0);
    m.tick(0, false, |_, _| true);
    assert!(!pointer.observe(false), "AX opening is not a pointer exit");
    assert_eq!(m.phase, Phase::Open);
    assert!(pointer.observe(true));
    m.hover(true, 1);
    m.close(2, true);
    m.tick(2, false, |_, _| true);
    assert!(
        !pointer.observe(true),
        "stationary pointer must not reopen AX close"
    );
    m.tick(382, false, |_, _| true);
    assert_eq!(m.phase, Phase::Collapsed);
    assert!(pointer.observe(false));
    m.hover(false, 400);
    assert!(pointer.observe(true));
    m.hover(true, 500);
    assert!(pointer.observe(false));
    m.hover(false, 510);
    m.tick(1000, false, |_, _| panic!("a graze cannot open"));
    assert_eq!(m.phase, Phase::Collapsed);
}

#[test]
fn rapid_disclosure_close_rejects_late_stationary_enter_until_real_exit() {
    rapid_disclosure_close_for_motion(false);
}

#[test]
fn rapid_disclosure_close_rejects_late_stationary_enter_with_reduced_motion() {
    rapid_disclosure_close_for_motion(true);
}

fn rapid_disclosure_close_for_motion(reduced: bool) {
    for placement in [OverlayPlacement::Notch, OverlayPlacement::FloatingPill] {
        for interval in [49, 101, 197] {
            let mut m = active();
            m.snapshot.placement = placement;
            for cycle in 0..10 {
                let start = cycle * 2000;
                m.expand(start);
                assert!(m.tick(start, reduced, |_, _| true).is_none());
                assert_eq!(m.phase, Phase::Open);
                // Both releases precede the end of the 360 ms animation.
                m.close(start + interval, true);
                m.tick(start + interval, reduced, |_, _| true);
                // A never-key panel can deliver its first enter/poll after
                // both clicks. It is still the same stationary pointer,
                // not permission to undo the explicit disclosure close.
                m.hover(true, start + interval + 33);
                for elapsed in (33..=1254).step_by(33) {
                    assert!(
                        m.tick(start + interval + elapsed, reduced, |_, _| true)
                            .is_none()
                    );
                    assert_ne!(
                        m.phase,
                        Phase::Open,
                        "cycle={cycle}, reduced={reduced}, interval={interval}"
                    );
                }
                assert_eq!(m.phase, Phase::Collapsed);
                // A deliberate click can reopen without a pointer exit.
                m.expand(start + 1500);
                m.tick(start + 1500, reduced, |_, _| true);
                assert_eq!(m.phase, Phase::Open);
                m.close(start + 1550, true);
                m.tick(start + 1550, reduced, |_, _| true);
                m.tick(start + 1930, reduced, |_, _| true);
                m.hover(false, start + 1940);
            }
            // Actual exit/re-entry restores the ordinary 150 ms dwell.
            m.hover(true, 21000);
            m.tick(21149, reduced, |_, _| panic!("early dwell"));
            m.tick(21150, reduced, |_, _| true);
            assert_eq!(m.phase, Phase::Open);
        }
    }
}

#[cfg(feature = "a11y-tests")]
#[gpui::test]
fn explicit_close_without_an_enter_rearms_on_an_outside_sample(cx: &mut TestAppContext) {
    cx.update(crate::init);
    cx.update(|cx| {
        let overlay = cx.new(|_| Overlay::new(active().snapshot, Rc::new(FakeHost::default())));
        overlay.update(cx, |s, _| {
            s.model.expand(0);
            s.model.tick(0, false, |_, _| true);
            s.model.close(50, true);
            s.model.tick(50, false, |_, _| true);
            s.model.tick(430, false, |_, _| true);
            // AX close while outside: no enter/exit transition was delivered.
            assert!(!s.observe_pointer(false, 450));
            assert!(s.observe_pointer(true, 500));
            s.model.tick(649, false, |_, _| panic!("early dwell"));
            s.model.tick(650, false, |_, _| true);
            assert_eq!(s.model.phase, Phase::Open);
        });
    });
}

#[cfg(feature = "a11y-tests")]
#[gpui::test]
fn ax_close_outside_allows_entry_before_next_watchdog_sample(cx: &mut TestAppContext) {
    close_visit_regression(cx, false);
}

#[cfg(feature = "a11y-tests")]
#[gpui::test]
fn delayed_exit_and_reentry_release_close_even_when_native_sample_is_inside(
    cx: &mut TestAppContext,
) {
    close_visit_regression(cx, true);
}

#[cfg(feature = "a11y-tests")]
fn close_visit_regression(test_cx: &mut TestAppContext, close_inside: bool) {
    use super::native::TEST_POINTER_SAMPLE;
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_POINTER_SAMPLE.set(None);
        }
    }
    let _reset = Reset;
    test_cx.update(crate::init);
    for placement in [OverlayPlacement::Notch, OverlayPlacement::FloatingPill] {
        for reduced in [false, true] {
            TEST_POINTER_SAMPLE.set(Some(false));
            test_cx.update(|cx| {
                crate::preferences::apply(
                    crate::preferences::DisplayPreferences {
                        reduce_motion: reduced,
                        ..Default::default()
                    },
                    cx,
                )
            });
            let host = Rc::new(FakeHost::default());
            let mut entity = None;
            let (_, cx) = test_cx.add_window_view(|w, cx| {
                let overlay = cx.new(|_| {
                    let mut s = Overlay::new(active().snapshot, host.clone());
                    s.model.snapshot.placement = placement;
                    s
                });
                entity = Some(overlay.clone());
                gpui_base::Root::new(overlay, w, cx)
            });
            let overlay = entity.unwrap();
            let handle = cx.update(|w, cx| {
                overlay.update(cx, |s, cx| s.advance(w, cx));
                w.window_handle()
            });
            cx.activate_accessibility(handle);
            adapter_press(cx, "Quick settings, collapsed");
            assert_eq!(overlay.read_with(cx, |s, _| s.model.phase), Phase::Open);
            cx.update(|w, cx| {
                overlay.update(cx, |s, _| {
                    s.epoch = std::time::Instant::now() - std::time::Duration::from_millis(50);
                    // Deliberately stale stored presence: close must use the
                    // native sample, not the most recent watchdog observation.
                    s.pointer.observe(true);
                });
                w.draw(cx).clear(cx);
            });
            TEST_POINTER_SAMPLE.set(Some(close_inside));
            adapter_press(cx, "Quick settings, expanded");
            assert_ne!(overlay.read_with(cx, |s, _| s.model.phase), Phase::Open);
            // Keep the current native sample INSIDE while queued outside and
            // inside mouse events are dispatched. No outside watchdog sample.
            TEST_POINTER_SAMPLE.set(Some(true));
            let band = cx.debug_bounds("overlay-status").unwrap().center();
            cx.update(|w, cx| {
                overlay.update(cx, |s, _| {
                    s.epoch = std::time::Instant::now() - std::time::Duration::from_millis(60);
                });
                if close_inside {
                    w.simulate_mouse_move(gpui::point(gpui::px(-10.), gpui::px(-10.)), cx);
                }
                w.simulate_mouse_move(band, cx);
                overlay.update(cx, |s, _| {
                    let entered = s.epoch.elapsed().as_millis() as u64;
                    s.model.tick(entered + 149, reduced, |_, _| true);
                    assert_ne!(s.model.phase, Phase::Open, "full dwell is required");
                    s.model.tick(entered + 150, reduced, |_, _| true);
                    assert_eq!(
                        s.model.phase,
                        Phase::Open,
                        "placement={placement:?}, reduced={reduced}, close_inside={close_inside}"
                    );
                });
            });
            assert!(
                host.intents.borrow().is_empty(),
                "hover/disclosure cannot send recording intents"
            );
        }
    }
}

#[cfg(feature = "a11y-tests")]
#[gpui::test]
fn delivered_hover_exits_between_watchdog_ticks_restart_continuous_dwell(
    test_cx: &mut TestAppContext,
) {
    test_cx.update(crate::init);
    for placement in [OverlayPlacement::Notch, OverlayPlacement::FloatingPill] {
        for reduced in [false, true] {
            test_cx.update(|cx| {
                crate::preferences::apply(
                    crate::preferences::DisplayPreferences {
                        reduce_motion: reduced,
                        ..Default::default()
                    },
                    cx,
                )
            });
            let mut entity = None;
            let (_, cx) = test_cx.add_window_view(|w, cx| {
                let overlay = cx.new(|_| {
                    let mut overlay = Overlay::new(active().snapshot, Rc::new(FakeHost::default()));
                    overlay.model.snapshot.placement = placement;
                    overlay
                });
                entity = Some(overlay.clone());
                gpui_base::Root::new(overlay, w, cx)
            });
            let overlay = entity.unwrap();
            cx.update(|w, cx| {
                overlay.update(cx, |s, cx| s.advance(w, cx));
                w.draw(cx).clear(cx);
                w.simulate_mouse_move(gpui::point(gpui::px(-10.), gpui::px(-10.)), cx);
                overlay.update(cx, |s, _| {
                    s.model.collapse();
                    s.pointer = PointerPresence::default();
                });
            });
            let band = cx.debug_bounds("overlay-status").unwrap().center();
            let deliver = |cx: &mut gpui::VisualTestContext, inside, at| {
                cx.update(|w, cx| {
                    overlay.update(cx, |s, _| {
                        s.epoch = std::time::Instant::now() - std::time::Duration::from_millis(at);
                    });
                    // Deliver the actual GPUI hover callback, not an outside
                    // sample: the watchdog sees only inside on both sides.
                    w.simulate_mouse_move(
                        if inside {
                            band
                        } else {
                            gpui::point(gpui::px(-10.), gpui::px(-10.))
                        },
                        cx,
                    );
                    overlay.read(cx).now
                })
            };
            deliver(cx, true, 0);
            cx.update(|_, cx| {
                overlay.update(cx, |s, _| {
                    assert!(
                        !s.observe_pointer(true, 99),
                        "event and poll share presence"
                    );
                    s.model.tick(99, reduced, |_, _| panic!("early dwell"));
                });
            });
            // Recreate 99/132 ms samples surrounding an exit at 110 and a
            // re-entry at 120. Also repeat brief grazes across later ticks.
            let mut last_entry = 0;
            for (exit, enter, sample) in [(110, 120, 132), (175, 185, 198), (241, 251, 264)] {
                deliver(cx, false, exit);
                let entered = deliver(cx, true, enter);
                assert!(
                    entered >= enter,
                    "delivered events must use the current clock"
                );
                last_entry = enter;
                cx.update(|_, cx| {
                    overlay.update(cx, |s, _| {
                        assert!(!s.observe_pointer(true, sample));
                        s.model
                            .tick(sample, reduced, |_, _| panic!("a graze cannot open"));
                        s.model
                            .tick(sample + 33, reduced, |_, _| panic!("cancelled dwell"));
                        assert_eq!(s.model.phase, Phase::Collapsed);
                    });
                });
            }
            let entered = overlay.read_with(cx, |s, _| s.now);
            cx.update(|_, cx| {
                overlay.update(cx, |s, _| {
                    assert!(!s.observe_pointer(true, last_entry + 149));
                    // Bound the event timestamp between clock setup and the
                    // completed callback, allowing a slow frame on CI.
                    s.model.tick(last_entry + 149, reduced, |_, _| {
                        panic!("full dwell required")
                    });
                    s.model.tick(entered + 150, reduced, |_, _| true);
                    assert_eq!(s.model.phase, Phase::Open);
                });
            });
        }
    }
}

#[cfg(feature = "a11y-tests")]
#[gpui::test]
fn resized_overlay_ax_frames_match_viewport_and_mode_pointer_target(cx: &mut TestAppContext) {
    cx.update(crate::init);
    cx.update(|cx| {
        crate::preferences::apply(
            crate::preferences::DisplayPreferences {
                reduce_motion: true,
                ..Default::default()
            },
            cx,
        )
    });
    let host = Rc::new(FakeHost::default());
    let mut entity = None;
    let (_, cx) = cx.add_window_view(|w, cx| {
        let overlay = cx.new(|_| {
            let mut overlay = Overlay::new(active().snapshot, host.clone());
            overlay.model.snapshot.placement = OverlayPlacement::FloatingPill;
            overlay.model.screen.bounds = (-1920., -200., 0., 880.);
            overlay.model.screen.work = (-1920., -176., 0., 840.);
            overlay
        });
        entity = Some(overlay.clone());
        gpui_base::Root::new(overlay, w, cx)
    });
    let overlay = entity.unwrap();
    let window = cx.update(|w, _| w.window_handle());
    cx.activate_accessibility(window);
    for expanded in [false, true, false, true] {
        cx.update(|w, cx| {
            overlay.update(cx, |s, cx| {
                s.model.phase = if expanded {
                    Phase::Open
                } else {
                    Phase::Collapsed
                };
                s.advance(w, cx);
                let frame = s.applied.unwrap().frame;
                assert_eq!(
                    w.viewport_size(),
                    gpui::size(gpui::px(frame.width as f32), gpui::px(frame.height as f32)),
                    "layout must refresh before returning from native transaction"
                );
            })
        });
        cx.run_until_parked();
        cx.update(|w, cx| w.draw(cx).clear(cx));
        let tree = cx.accessibility_tree(window);
        let (frame, scale) = cx.update(|w, cx| {
            (
                overlay.read(cx).applied.unwrap().frame,
                w.scale_factor() as f64,
            )
        });
        for (_, node) in &tree.nodes {
            if let Some(bounds) = node.bounds() {
                assert!(
                    bounds.x0 >= 0. && bounds.y0 >= 0.,
                    "outside window: {:?} {bounds:?}",
                    node.label()
                );
                assert!(
                    bounds.x1 / scale <= frame.width + 0.5
                        && bounds.y1 / scale <= frame.height + 0.5,
                    "outside window: {:?} {bounds:?}, {frame:?}",
                    node.label()
                );
            }
        }
        if expanded {
            let mode = tree
                .nodes
                .iter()
                .find(|(_, n)| n.label() == Some("Mode: Everyday. Click to cycle"))
                .unwrap()
                .1
                .bounds()
                .unwrap();
            let before = host.intents.borrow().len();
            cx.simulate_click(
                gpui::point(
                    gpui::px(((mode.x0 + mode.x1) / (2. * scale)) as f32),
                    gpui::px(((mode.y0 + mode.y1) / (2. * scale)) as f32),
                ),
                gpui::Modifiers::default(),
            );
            cx.run_until_parked();
            assert_eq!(host.intents.borrow().len(), before + 1);
            assert!(matches!(
                host.intents.borrow().last(),
                Some(Intent::CycleMode)
            ));
        }
    }
}

#[cfg(feature = "a11y-tests")]
#[gpui::test]
fn disclosure_ax_frame_is_its_pointer_target_in_both_placements(cx: &mut TestAppContext) {
    cx.update(crate::init);
    cx.update(|cx| {
        crate::preferences::apply(
            crate::preferences::DisplayPreferences {
                reduce_motion: true,
                ..Default::default()
            },
            cx,
        )
    });
    let host = Rc::new(FakeHost::default());
    let mut entity = None;
    let (_, cx) = cx.add_window_view(|w, cx| {
        let overlay = cx.new(|_| Overlay::new(active().snapshot, host.clone()));
        entity = Some(overlay.clone());
        gpui_base::Root::new(overlay, w, cx)
    });
    let overlay = entity.unwrap();
    let window = cx.update(|w, _| w.window_handle());
    cx.activate_accessibility(window);
    for placement in [OverlayPlacement::Notch, OverlayPlacement::FloatingPill] {
        for disabled in [false, true] {
            for expanded in [false, true, false, true] {
                for (x, y) in [(0.5, 0.5), (0.1, 0.1), (0.9, 0.9)] {
                    cx.update(|w, cx| {
                        overlay.update(cx, |s, cx| {
                            s.model = active();
                            s.model.snapshot.placement = placement;
                            s.model.snapshot.disabled = disabled;
                            s.model.phase = if expanded {
                                Phase::Open
                            } else {
                                Phase::Collapsed
                            };
                            s.advance(w, cx);
                        });
                        w.draw(cx).clear(cx);
                    });
                    let tree = cx.accessibility_tree(window);
                    let focus = tree.focus;
                    let name = if expanded {
                        "Quick settings, expanded"
                    } else {
                        "Quick settings, collapsed"
                    };
                    let (id, node) = tree
                        .nodes
                        .iter()
                        .find(|(_, n)| n.label() == Some(name))
                        .unwrap();
                    assert_eq!(node.role(), Role::Button);
                    let ax = node.bounds().unwrap();
                    let id = *id;
                    let hit = cx
                        .debug_bounds("overlay-quick-settings-disclosure")
                        .unwrap();
                    let scale = cx.update(|w, _| w.scale_factor()) as f64;
                    assert_eq!(ax.x0 / scale, f32::from(hit.left()) as f64);
                    assert_eq!(ax.y0 / scale, f32::from(hit.top()) as f64);
                    assert_eq!(ax.width() / scale, 20.);
                    assert_eq!(ax.height() / scale, 24.);
                    // The disclosure owns no Mode/power/paste/settings area.
                    for (_, other) in &tree.nodes {
                        if other.label().is_some_and(|label| {
                            label.starts_with("Mode:")
                                || matches!(
                                    label,
                                    "Disable Murmur"
                                        | "Enable Murmur"
                                        | "Auto-paste"
                                        | "Open settings"
                                )
                        }) {
                            let bounds = other.bounds().unwrap();
                            assert!(
                                ax.x1 <= bounds.x0
                                    || ax.x0 >= bounds.x1
                                    || ax.y1 <= bounds.y0
                                    || ax.y0 >= bounds.y1
                            );
                        }
                    }
                    cx.simulate_mouse_move(
                        gpui::point(gpui::px(-10.), gpui::px(-10.)),
                        None,
                        gpui::Modifiers::default(),
                    );
                    cx.simulate_mouse_move(
                        gpui::point(hit.right() - gpui::px(1.), hit.center().y),
                        None,
                        gpui::Modifiers::default(),
                    );
                    overlay.update(cx, |s, _| {
                        assert!(
                            !s.observe_pointer(true, s.now),
                            "hovering the enlarged disclosure counts as inside the card"
                        );
                    });
                    let (_, _, pressable, request) = accesskit_macos::test_press(tree, id);
                    assert!(pressable);
                    assert!(request.is_some());
                    let before = host.intents.borrow().len();
                    cx.simulate_click(
                        gpui::point(
                            gpui::px(((ax.x0 + ax.width() * x) / scale) as f32),
                            gpui::px(((ax.y0 + ax.height() * y) / scale) as f32),
                        ),
                        gpui::Modifiers::default(),
                    );
                    cx.run_until_parked();
                    cx.update(|_, cx| {
                        overlay.update(cx, |s, _| {
                            let action = s.model.tick(s.now + 300, true, |_, _| true);
                            s.send(action);
                        });
                    });
                    assert_eq!(
                        overlay.read_with(cx, |s, _| s.model.phase),
                        if expanded {
                            Phase::Collapsed
                        } else {
                            Phase::Open
                        }
                    );
                    assert_eq!(
                        host.intents.borrow().len(),
                        before,
                        "disclosure cannot activate dictation or quick controls"
                    );
                    assert_eq!(cx.accessibility_tree(window).focus, focus);
                    // The same exported native AXPress target reverses the click.
                    adapter_press(
                        cx,
                        if expanded {
                            "Quick settings, collapsed"
                        } else {
                            "Quick settings, expanded"
                        },
                    );
                    assert_eq!(
                        overlay.read_with(cx, |s, _| s.model.phase),
                        if expanded {
                            Phase::Open
                        } else {
                            Phase::Collapsed
                        }
                    );
                    assert_eq!(host.intents.borrow().len(), before);
                }
            }
        }
    }
    // The adjacent state indicator still stops a recording, without toggling
    // disclosure or sending the action twice through its parent status row.
    cx.update(|w, cx| {
        overlay.update(cx, |s, cx| {
            s.model = active();
            s.model.snapshot.placement = OverlayPlacement::FloatingPill;
            s.advance(w, cx);
        });
        w.draw(cx).clear(cx);
    });
    let glyph = cx.debug_bounds("overlay-state-glyph").unwrap();
    let before = host.intents.borrow().len();
    cx.simulate_click(glyph.center(), gpui::Modifiers::default());
    cx.update(|_, cx| {
        overlay.update(cx, |s, _| {
            assert_eq!(s.model.phase, Phase::Collapsed);
            let action = s.model.tick(s.now + 300, true, |_, _| true);
            s.send(action);
        });
    });
    assert_eq!(host.intents.borrow().len(), before + 1);
    assert_eq!(
        host.intents.borrow().last(),
        Some(&Intent::Control {
            generation: 7,
            input: Control::Stop
        })
    );
    // A hidden floating surface exports neither a disclosure nor a hitbox.
    cx.update(|w, cx| {
        overlay.update(cx, |s, cx| {
            s.model = Model::new(Snapshot {
                placement: OverlayPlacement::FloatingPill,
                ..Snapshot::default()
            });
            s.advance(w, cx);
        });
        w.draw(cx).clear(cx);
    });
    assert!(!cx.accessibility_tree(window).nodes.iter().any(|(_, n)| {
        n.label()
            .is_some_and(|label| label.starts_with("Quick settings"))
    }));
    assert!(
        cx.debug_bounds("overlay-quick-settings-disclosure")
            .is_none()
    );
}

#[cfg(feature = "a11y-tests")]
#[gpui::test]
fn disclosure_stays_hittable_and_precedes_controls_during_motion(cx: &mut TestAppContext) {
    use std::time::Duration;

    cx.update(crate::init);
    let host = Rc::new(FakeHost::default());
    let mut entity = None;
    let (_, cx) = cx.add_window_view(|w, cx| {
        let overlay = cx.new(|_| Overlay::new(active().snapshot, host.clone()));
        entity = Some(overlay.clone());
        gpui_base::Root::new(overlay, w, cx)
    });
    let overlay = entity.unwrap();
    let window = cx.update(|w, _| w.window_handle());
    cx.activate_accessibility(window);
    for placement in [OverlayPlacement::Notch, OverlayPlacement::FloatingPill] {
        for disabled in [false, true] {
            for opening in [true, false] {
                for sample_ms in [0, 16, 60, 120, 180, 240, 270, 360] {
                    for (x, y) in [(0.05, 0.05), (0.5, 0.5), (0.95, 0.95)] {
                        // Settle the starting geometry, then enable the real
                        // transition engine and advance its simulated clock.
                        cx.update(|w, cx| {
                            crate::preferences::apply(
                                crate::preferences::DisplayPreferences {
                                    reduce_motion: true,
                                    ..Default::default()
                                },
                                cx,
                            );
                            overlay.update(cx, |s, cx| {
                                s.model = active();
                                s.model.snapshot.placement = placement;
                                s.model.snapshot.disabled = disabled;
                                s.model.phase = if opening {
                                    Phase::Collapsed
                                } else {
                                    Phase::Open
                                };
                                s.advance(w, cx);
                            });
                            w.draw(cx).clear(cx);
                            crate::preferences::apply(Default::default(), cx);
                            assert!(!cx.reduce_motion());
                            overlay.update(cx, |s, cx| s.toggle_quick_settings(w, cx));
                            w.draw(cx).clear(cx);
                        });
                        cx.executor()
                            .advance_clock(Duration::from_millis(sample_ms));
                        cx.update(|w, cx| w.draw(cx).clear(cx));
                        let card = cx.debug_bounds("overlay-surface").unwrap();
                        let hit = cx
                            .debug_bounds("overlay-quick-settings-disclosure")
                            .unwrap();
                        let g = overlay.read_with(cx, |s, _| s.model.geometry(opening, s.now));
                        let height = f32::from(card.size.height) as f64;
                        if sample_ms > 0 && sample_ms < 360 {
                            assert!(
                                height > 0.
                                    && height != if opening { g.collapsed_h } else { g.expanded_h },
                                "must sample a real intermediate frame: {height}, sample={sample_ms}, opening={opening}, placement={placement:?}"
                            );
                        }
                        // Overflow hit-testing clips to this rectangle. The
                        // entire AX target must stay within it, not just its center.
                        assert!(hit.left() >= card.left() && hit.right() <= card.right());
                        assert!(hit.top() >= card.top() && hit.bottom() <= card.bottom());
                        if placement == OverlayPlacement::FloatingPill {
                            assert_eq!(hit.bottom() + gpui::px(5.), card.bottom());
                        } else {
                            assert_eq!(
                                hit.top() - card.top(),
                                gpui::px((g.collapsed_h as f32 - 24.) / 2.)
                            );
                        }
                        let tree = cx.accessibility_tree(window);
                        let focus = tree.focus;
                        let name = if opening {
                            "Quick settings, expanded"
                        } else {
                            "Quick settings, collapsed"
                        };
                        let (disclosure_id, node) = tree
                            .nodes
                            .iter()
                            .find(|(_, n)| n.label() == Some(name))
                            .unwrap();
                        let scale = cx.update(|w, _| w.scale_factor()) as f64;
                        let ax = node.bounds().unwrap();
                        assert_eq!(ax.x0 / scale, f32::from(hit.left()) as f64);
                        assert_eq!(ax.y0 / scale, f32::from(hit.top()) as f64);
                        assert_eq!(ax.width() / scale, 20.);
                        assert_eq!(ax.height() / scale, 24.);
                        if opening {
                            // Walk actual AX child links. Node emission order
                            // alone is not the navigation order consumed by VO.
                            fn walk(
                                tree: &gpui::accesskit::TreeUpdate,
                                id: gpui::accesskit::NodeId,
                                order: &mut Vec<gpui::accesskit::NodeId>,
                            ) {
                                order.push(id);
                                let node = &tree.nodes.iter().find(|(n, _)| *n == id).unwrap().1;
                                for child in node.children() {
                                    walk(tree, *child, order);
                                }
                            }
                            let mut order = Vec::new();
                            walk(&tree, tree.tree.as_ref().unwrap().root, &mut order);
                            let disclosure_index =
                                order.iter().position(|id| id == disclosure_id).unwrap();
                            for label in [
                                "Mode: Everyday. Click to cycle",
                                if disabled {
                                    "Enable Murmur"
                                } else {
                                    "Disable Murmur"
                                },
                                "Auto-paste",
                                "Open settings",
                            ] {
                                let id = tree
                                    .nodes
                                    .iter()
                                    .find(|(_, n)| n.label() == Some(label))
                                    .unwrap()
                                    .0;
                                assert!(
                                    disclosure_index < order.iter().position(|n| *n == id).unwrap(),
                                    "disclosure must precede {label} in AX child order"
                                );
                            }
                        }
                        let before = host.intents.borrow().len();
                        cx.simulate_click(
                            gpui::point(
                                gpui::px(((ax.x0 + ax.width() * x) / scale) as f32),
                                gpui::px(((ax.y0 + ax.height() * y) / scale) as f32),
                            ),
                            gpui::Modifiers::default(),
                        );
                        assert_eq!(
                            overlay.read_with(cx, |s, _| s.model.phase),
                            if opening { Phase::Closing } else { Phase::Open }
                        );
                        adapter_press(
                            cx,
                            if opening {
                                "Quick settings, collapsed"
                            } else {
                                "Quick settings, expanded"
                            },
                        );
                        assert_eq!(
                            overlay.read_with(cx, |s, _| s.model.phase),
                            if opening { Phase::Open } else { Phase::Closing }
                        );
                        assert_eq!(cx.accessibility_tree(window).focus, focus);
                        assert_eq!(host.intents.borrow().len(), before);
                    }
                }
            }
        }
    }
}

#[cfg(feature = "a11y-tests")]
#[gpui::test]
fn stationary_pointer_exit_notifications_cannot_undo_disclosure_close(
    test_cx: &mut TestAppContext,
) {
    use super::native::{TEST_POINTER_POSITION, TEST_POINTER_SAMPLE};
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_POINTER_SAMPLE.set(None);
            TEST_POINTER_POSITION.set(None);
        }
    }
    let _reset = Reset;
    test_cx.update(crate::init);
    for reduced in [false, true] {
        TEST_POINTER_SAMPLE.set(Some(true));
        test_cx.update(|cx| {
            crate::preferences::apply(
                crate::preferences::DisplayPreferences {
                    reduce_motion: reduced,
                    ..Default::default()
                },
                cx,
            )
        });
        let host = Rc::new(FakeHost::default());
        let mut entity = None;
        let (_, cx) = test_cx.add_window_view(|w, cx| {
            let overlay = cx.new(|_| {
                let mut s = Overlay::new(active().snapshot, host.clone());
                s.model.snapshot.placement = OverlayPlacement::FloatingPill;
                s
            });
            entity = Some(overlay.clone());
            gpui_base::Root::new(overlay, w, cx)
        });
        let overlay = entity.unwrap();
        let handle = cx.update(|w, cx| {
            overlay.update(cx, |s, cx| s.advance(w, cx));
            w.window_handle()
        });
        cx.activate_accessibility(handle);
        // Synthetic global coordinates stay fixed even while TestWindow resizes
        // (unlike NSWindow, TestWindow does not preserve the floating anchor).
        let stationary = gpui::point(gpui::px(937.), gpui::px(869.));
        TEST_POINTER_POSITION.set(Some(stationary));
        // AX uses the same toggle handler as clicks. Twelve rapid activations
        // must each toggle exactly once, without changing the pointer sample.
        for click in 0..12 {
            adapter_press(
                cx,
                if click % 2 == 0 {
                    "Quick settings, collapsed"
                } else {
                    "Quick settings, expanded"
                },
            );
            assert_eq!(
                overlay.read_with(cx, |s, _| s.model.phase == Phase::Open),
                click % 2 == 0
            );
        }
        let closed_at = overlay.read_with(cx, |s, _| s.now);
        cx.update(|w, cx| {
            overlay.update(cx, |s, cx| {
                // Tracking-area reconciliation can report an exit at the
                // unchanged cursor position during contraction. Feed that
                // event and subsequent 33 ms samples through the same reducer
                // used by the native watchdog and GPUI hover callbacks.
                s.observe_pointer_at(false, stationary, closed_at + 1);
                for elapsed in (33..=1254).step_by(33) {
                    let at = closed_at + elapsed;
                    s.observe_pointer_at(true, stationary, at);
                    s.model.tick(at, reduced, |_, _| true);
                    assert_ne!(
                        s.model.phase,
                        Phase::Open,
                        "stationary close reopened at {elapsed}ms, reduced={reduced}"
                    );
                }
                assert_eq!(s.model.phase, Phase::Collapsed);
                // Another explicit action can reopen without a pointer exit.
                s.toggle_quick_settings(w, cx);
            });
        });
        assert_eq!(overlay.read_with(cx, |s, _| s.model.phase), Phase::Open);
        adapter_press(cx, "Quick settings, expanded");
        cx.update(|_, cx| {
            overlay.update(cx, |s, _| {
                let at = s.now + 1500;
                // Real movement outside, then re-entry between watchdog ticks,
                // releases suppression and requires a fresh full 150 ms dwell.
                s.observe_pointer_at(
                    false,
                    stationary + gpui::point(gpui::px(300.), gpui::px(0.)),
                    at,
                );
                s.observe_pointer_at(true, stationary, at + 1);
                s.model.tick(at + 150, reduced, |_, _| true);
                assert_ne!(s.model.phase, Phase::Open, "full dwell required");
                s.model.tick(at + 151, reduced, |_, _| true);
                assert_eq!(s.model.phase, Phase::Open);
            });
        });
        assert!(host.intents.borrow().is_empty());
    }
}

#[test]
fn idle_has_no_deadline_and_motion_matches_the_shipped_tokens() {
    let mut model = Model::new(Snapshot::default());
    assert!(!model.needs_tick());
    assert_eq!(model.next_deadline(0), None);
    assert_eq!(HOVER_OPEN_DWELL_MS, 150);
    assert_eq!(COLLAPSE_DELAY_MS, 300);
    assert_eq!(SIZE_ANIMATION_MS, 360);
    assert_eq!(SHRINK_GUARD_MS, 20);
    assert_eq!(SHRINK_DELAY_MS, 380);
    model.hover(true, 100);
    assert_eq!(model.next_deadline(100), Some(250));
    model.tick(250, false, |_, _| true);
    assert_eq!(model.next_deadline(250), None);
    model.hover(false, 300);
    assert_eq!(model.next_deadline(300), Some(600));
    model.tick(600, false, |_, _| true);
    assert_eq!(model.next_deadline(600), Some(980));
    model.tick(980, false, |_, _| true);
    assert_eq!(model.next_deadline(980), None);
    assert_eq!(model.phase, Phase::Collapsed);
    model.expand(1000);
    model.tick(1000, true, |_, _| true);
    model.hover(false, 1100);
    model.tick(1400, true, |_, _| true);
    assert_eq!(model.phase, Phase::Collapsed);
    assert_eq!(model.next_deadline(1400), None);
}

#[test]
fn first_surface_uses_global_notch_geometry_on_either_side_of_primary() {
    for bounds in [(-1728., -120., 0., 997.), (1440., 80., 3168., 1197.)] {
        let mut model = active();
        model.screen = Screen {
            bounds,
            work: (bounds.0, bounds.1 + 37., bounds.2, bounds.3),
            scale_factor: 2.,
            notch: Some(NotchInfo {
                width: 185.,
                height: 37.,
                physical_notch: true,
                menu_bar_visible: true,
            }),
            ..Screen::default()
        };
        let frame = model.frame(false, 0);
        let mut writer = SurfaceWriter::default();
        assert_eq!(writer.resolve(frame, None, 0, false, true), frame);
        assert_eq!(frame.x + frame.width / 2., (bounds.0 + bounds.2) / 2.);
        assert_eq!(frame.y, bounds.1);
        assert_eq!((frame.width, frame.height), (241., 37.));
        assert!(!writer.pending());
        assert!(!model.geometry(false, 0).floating);
        model.screen.notch = None;
        assert!(
            !model.geometry(false, 0).floating,
            "unmeasured screens use a pretend notch, never a floating capsule"
        );
    }
}

#[cfg(feature = "a11y-tests")]
#[gpui::test]
fn settled_idle_parks_without_scheduling_frames_or_timers(test_cx: &mut TestAppContext) {
    use std::time::Duration;
    test_cx.update(crate::init);
    super::native::TEST_POINTER_SAMPLE.set(Some(false));
    struct ResetPointer;
    impl Drop for ResetPointer {
        fn drop(&mut self) {
            super::native::TEST_POINTER_SAMPLE.set(None);
        }
    }
    let _reset = ResetPointer;
    let mut entity = None;
    let (_, cx) = test_cx.add_window_view(|w, cx| {
        let overlay = cx.new(|_| Overlay::new(Snapshot::default(), Rc::new(FakeHost::default())));
        entity = Some(overlay.clone());
        gpui_base::Root::new(overlay, w, cx)
    });
    let overlay = entity.unwrap();
    cx.update(|w, cx| {
        w.simulate_mouse_move(gpui::point(gpui::px(-100.), gpui::px(-100.)), cx);
        overlay.update(cx, |s, cx| {
            s.advance(w, cx);
            s.follow(w, cx);
        })
    });
    cx.run_until_parked();
    cx.update(|w, cx| {
        w.draw(cx).clear(cx);
        // Finish startup layout callbacks, then idle must create no new demand.
        w.simulate_next_frame(cx);
        w.draw(cx).clear(cx);
        assert_eq!(w.simulate_next_frame(cx), 0);
    });
    let before = overlay.read_with(cx, |s, _| {
        assert!(
            s.deadline.is_none(),
            "deadline={:?}, now={}, phase={:?}, model={:?}",
            s.deadline.as_ref().map(|d| d.0),
            s.now,
            s.model.phase,
            s.model.next_deadline(s.now)
        );
        s.now
    });
    super::native::TEST_APPLIES.with_borrow_mut(Vec::clear);
    cx.executor().advance_clock(Duration::from_secs(120));
    cx.run_until_parked();
    overlay.read_with(cx, |s, _| {
        assert_eq!(s.now, before, "no periodic watchdog is allowed to run");
        assert!(
            s.deadline.is_none(),
            "deadline={:?}, now={}, phase={:?}, model={:?}",
            s.deadline.as_ref().map(|d| d.0),
            s.now,
            s.model.phase,
            s.model.next_deadline(s.now)
        );
    });
    cx.update(|w, cx| assert_eq!(w.simulate_next_frame(cx), 0));
    super::native::TEST_APPLIES.with_borrow(|applies| assert!(applies.is_empty()));
    // A publisher must still wake the parked owner immediately.
    let inbox = overlay.read_with(cx, |s, _| s.inbox.clone());
    inbox
        .publish(Event::Lifecycle {
            generation: 1,
            status: Status::Starting,
            at_ms: 0,
        })
        .unwrap_or_else(|_| panic!("inbox"));
    cx.run_until_parked();
    assert_eq!(
        overlay.read_with(cx, |s, _| s.model.indicator(s.now)),
        Indicator::Starting
    );
}

#[test]
fn floating_visibility_eases_for_180ms_and_stops_at_the_endpoint() {
    let fade = VisibilityMotion {
        from: 0.,
        to: 1.,
        started: 100,
    };
    assert_eq!(PILL_FADE_MS, 180);
    assert_eq!(fade.value(100), 0.);
    assert!(fade.value(190) > 0.5 && fade.value(190) < 1.);
    assert!(fade.running(279));
    assert!(!fade.running(280));
    assert_eq!(fade.value(280), 1.);
    assert_eq!(fade.value(1000), 1.);
}

#[test]
fn notch_uses_the_shipped_overshooting_curve_and_pill_settles_without_overshoot() {
    let notch = super::size_easing(false);
    let pill = super::size_easing(true);
    // CSS easing solves the curve's x coordinate, rather than treating its
    // parameter as elapsed time. The notch overshoots; the pill cannot bounce.
    assert!(notch.sample(0.5) > 1.);
    assert!(pill.sample(0.5) > 0.9 && pill.sample(0.5) < 1.);
    for t in [0., 0.1, 0.25, 0.5, 0.75, 0.9, 1.] {
        assert!((0. ..=1.).contains(&pill.sample(t)));
    }
    assert_eq!(notch.sample(0.), 0.);
    assert_eq!(notch.sample(1.), 1.);
    assert_eq!(
        crate::motion::Motion::Overlay
            .resolve(false)
            .duration
            .as_millis(),
        SIZE_ANIMATION_MS as u128
    );
    assert!(
        crate::motion::Motion::Overlay
            .resolve(true)
            .duration
            .is_zero()
    );
}

#[cfg(feature = "a11y-tests")]
#[gpui::test]
fn calibration_dock_and_appearance_each_apply_once_then_park(test_cx: &mut TestAppContext) {
    use super::native::{TEST_APPLIES, TEST_SCREEN};
    use crate::preferences;
    use gpui::{WindowAppearance, point, px};
    use std::time::Duration;
    test_cx.update(crate::init);
    test_cx.update(|cx| {
        preferences::apply(
            preferences::DisplayPreferences {
                reduce_motion: true,
                ..Default::default()
            },
            cx,
        )
    });
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_APPLIES.with_borrow_mut(Vec::clear);
            TEST_SCREEN.set(None);
        }
    }
    let _reset = Reset;
    let mut entity = None;
    let (_, cx) = test_cx.add_window_view(|w, cx| {
        let overlay = cx.new(|_| Overlay::new(Snapshot::default(), Rc::new(FakeHost::default())));
        entity = Some(overlay.clone());
        gpui_base::Root::new(overlay, w, cx)
    });
    let overlay = entity.unwrap();
    let window = cx.update(|w, cx| {
        w.simulate_mouse_move(point(px(-100.), px(-100.)), cx);
        overlay.update(cx, |s, cx| {
            s.advance(w, cx);
            s.follow(w, cx);
        });
        w.window_handle()
    });
    cx.run_until_parked();
    TEST_APPLIES.with_borrow_mut(Vec::clear);
    // Invoke the exact Settings entry point, then inspect the surface passed
    // to the native writer, rather than only checking the model's offset.
    let before = overlay.read_with(cx, |s, _| s.applied.unwrap().frame);
    cx.update(|w, cx| overlay.update(cx, |s, cx| s.calibrate(Some(7.), false, w, cx)));
    cx.run_until_parked();
    TEST_APPLIES.with_borrow_mut(|applies| {
        assert_eq!(applies.len(), 1);
        assert_eq!(applies[0].frame.y, before.y + 7.);
        assert!(!applies[0].hidden);
        assert_eq!(
            applies[0].frame,
            overlay.read_with(cx, |s, _| s.model.frame(false, s.now))
        );
        applies.clear();
    });
    // Finish/cancel is the same geometry path, and restores quiet idle.
    cx.update(|w, cx| overlay.update(cx, |s, cx| s.calibrate(Some(0.), true, w, cx)));
    cx.run_until_parked();
    TEST_APPLIES.with_borrow_mut(|applies| {
        assert_eq!(applies.len(), 1);
        assert_eq!(applies[0].frame, before);
        applies.clear();
    });
    cx.update(|w, cx| {
        overlay.update(cx, |s, cx| {
            s.model.snapshot.placement = OverlayPlacement::FloatingPill;
            s.advance(w, cx);
            // Stale live limits must not survive a Dock preference change.
            s.model.screen.dock = murmur_core::overlay_geometry::DockLimit::Above(500.);
        })
    });
    cx.run_until_parked();
    TEST_APPLIES.with_borrow_mut(Vec::clear);
    let before = overlay.read_with(cx, |s, _| s.applied.unwrap().frame);
    // A preference notification must replace the cached work area AND discard
    // an old live Dock limit, including when AX cannot supply a fresh limit.
    let inbox = overlay.read_with(cx, |s, _| s.inbox.clone());
    let mut screen = overlay.read_with(cx, |s, _| s.model.screen.clone());
    screen.work.3 -= 120.;
    screen.hidden_dock = Some(murmur_core::overlay_geometry::HiddenDock {
        tile: 96.,
        magnified: Some(160.),
    });
    screen.dock = murmur_core::overlay_geometry::DockLimit::Unknown;
    TEST_SCREEN.set(Some(screen.clone()));
    inbox
        .publish(Event::RefreshDock)
        .unwrap_or_else(|_| panic!("inbox"));
    cx.run_until_parked();
    TEST_APPLIES.with_borrow_mut(|applies| {
        assert_eq!(applies.len(), 1);
        assert_eq!(
            applies[0].frame,
            overlay.read_with(cx, |s, _| s.model.frame(false, s.now))
        );
        assert_ne!(applies[0].frame.y, before.y);
        applies.clear();
    });
    assert_eq!(
        overlay.read_with(cx, |s, _| s.model.screen.work),
        screen.work
    );
    assert_eq!(
        overlay.read_with(cx, |s, _| s.model.screen.dock),
        murmur_core::overlay_geometry::DockLimit::Unknown
    );
    cx.simulate_window_appearance_change(window, WindowAppearance::Dark);
    cx.run_until_parked();
    TEST_APPLIES.with_borrow_mut(|applies| {
        assert_eq!(applies.len(), 1);
        assert!(applies[0].dark);
        applies.clear();
    });
    // Duplicate appearance and unchanged screen notifications cause no apply.
    cx.simulate_window_appearance_change(window, WindowAppearance::Dark);
    inbox
        .publish(Event::RefreshDock)
        .unwrap_or_else(|_| panic!("inbox"));
    cx.run_until_parked();
    TEST_APPLIES.with_borrow(|applies| assert!(applies.is_empty()));
    cx.update(|w, cx| {
        w.draw(cx).clear(cx);
        w.simulate_next_frame(cx);
        w.draw(cx).clear(cx);
        assert_eq!(w.simulate_next_frame(cx), 0);
    });
    cx.executor().advance_clock(Duration::from_secs(120));
    cx.run_until_parked();
    TEST_APPLIES.with_borrow(|applies| assert!(applies.is_empty()));
    overlay.read_with(cx, |s, _| assert!(s.deadline.is_none()));
    cx.update(|w, cx| assert_eq!(w.simulate_next_frame(cx), 0));
}
