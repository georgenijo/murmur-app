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
                    assert!(leading.left() >= row.left());
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
    m.close(2);
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
