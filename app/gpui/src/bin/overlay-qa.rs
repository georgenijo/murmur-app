//! Synthetic native-parity host. Never opens user data, capture or pasteboard.
use gpui::{prelude::*, *};
use murmur_core::{dictation::Status, overlay_geometry::*};
use murmur_ui::overlay::{Host, Overlay, model::*};
use std::rc::Rc;
struct Synthetic {
    inbox: murmur_ui::overlay::inbox::Inbox,
    snapshot: std::cell::RefCell<Snapshot>,
    control: std::cell::RefCell<murmur_core::dictation::Controller>,
}
impl Host for Synthetic {
    fn submit(&self, intent: Intent) {
        use murmur_core::dictation::{Effect, Origin};
        let mut snapshot = self.snapshot.borrow_mut();
        match intent {
            Intent::Control { generation, input } if generation == snapshot.generation => {
                let mut control = self.control.borrow_mut();
                control.observe(snapshot.generation, snapshot.status, Origin::Toggle);
                match control.input(input.input()) {
                    Some(Effect::Start { .. }) => {
                        snapshot.generation += 1;
                        snapshot.status = Status::Recording;
                    }
                    Some(Effect::Stop { .. } | Effect::Cancel { .. }) => {
                        snapshot.status = Status::Idle
                    }
                    _ => {}
                }
                snapshot.locked = control.locked;
            }
            Intent::Disabled(disabled) => snapshot.disabled = disabled,
            Intent::AutoPaste(auto_paste) => snapshot.auto_paste = auto_paste,
            Intent::CycleMode => {
                snapshot.mode = if snapshot.mode == "Everyday" {
                    "Verbatim".into()
                } else {
                    "Everyday".into()
                }
            }
            Intent::RetryDelivery {
                generation,
                attempt,
            } => {
                let _ = self.inbox.publish(Event::RetryResult {
                    generation,
                    attempt,
                    result: Delivery::Nothing,
                });
            }
            Intent::AcceptSuggestion(_) | Intent::DismissSuggestion(_) => {
                let _ = self.inbox.publish(Event::Suggestion(None));
            }
            _ => {}
        }
        snapshot.revision += 1;
        let _ = self.inbox.publish(Event::Snapshot(snapshot.clone()));
    }
    fn waveform_interest(&self, _: murmur_core::events::WaveformConsumer, _: bool) {}
    fn preview_anchor(&self, _: OverlayFrame, _: bool, _: f64) {}
}
fn main() {
    murmur_ui::overlay::native::start_webkit_policy();
    if std::env::var_os("MURMUR_WEBKIT_BASELINE_ONLY").is_some() {
        murmur_ui::overlay::native::assert_webkit_policy("MURMUR_OVERLAY_INVENTORY");
        return;
    }
    let output = std::env::var_os("MURMUR_GOLDEN");
    let headless = output.is_some();
    let app = if headless {
        gpui_platform::application().with_windowing(WindowingRequest::Headless)
    } else {
        gpui_platform::application()
    };
    app.run(move |cx| {
        if headless {
            murmur_ui::init_with_preferences(
                cx,
                Some(murmur_ui::preferences::DisplayPreferences {
                    reduce_motion: true,
                    reduce_transparency: true,
                    increase_contrast: false,
                }),
            );
        } else {
            murmur_ui::init(cx);
        }
        let dark = std::env::var_os("MURMUR_DARK").is_some();
        murmur_ui::tokens::apply_appearance(dark, cx);
        let state = std::env::var("MURMUR_OVERLAY_STATE").unwrap_or_else(|_| "recording".into());
        let floating = std::env::var_os("MURMUR_PILL").is_some();
        let expanded = matches!(
            state.as_str(),
            "expanded" | "suggestion" | "disabled-expanded"
        );
        let mut snapshot = Snapshot {
            generation: 1,
            disabled: state.starts_with("disabled"),
            placement: if floating {
                OverlayPlacement::FloatingPill
            } else {
                OverlayPlacement::Notch
            },
            ..Snapshot::default()
        };
        snapshot.status = match state.as_str() {
            "idle" | "suggestion" | "disabled" | "disabled-expanded" => Status::Idle,
            "starting" => Status::Starting,
            "processing" => Status::Processing,
            "recovering" => Status::Recovering,
            _ => Status::Recording,
        };
        let mut model = Model::new(snapshot.clone());
        model.screen.notch = Some(NotchInfo {
            width: 185.,
            height: if state == "delivery-min-height" {
                24.
            } else if std::env::var_os("MURMUR_PRETEND_NOTCH").is_some() {
                30.
            } else {
                37.
            },
            physical_notch: std::env::var_os("MURMUR_PRETEND_NOTCH").is_none(),
            menu_bar_visible: true,
        });
        model.level = 0.04;
        if expanded {
            model.phase = Phase::Open;
        }
        match state.as_str() {
            "cancelled" => model.event(Event::Cancelled(1), 0),
            "failure" => model.event(Event::Failure(1, Failure::ChooseInput), 0),
            "delivery" | "delivery-min-height" => {
                model.snapshot.status = Status::Idle;
                model.event(Event::Delivery(1, Delivery::Clipboard), 0);
            }
            "suggestion" => model.event(
                Event::Suggestion(Some(Suggestion {
                    token: "synthetic-token".into(),
                    title: "Synthetic meeting".into(),
                    busy: false,
                    failed: false,
                })),
                0,
            ),
            _ => {}
        }
        let frame = model.frame(expanded, 0);
        let host = Rc::new(Synthetic {
            inbox: Default::default(),
            snapshot: std::cell::RefCell::new(snapshot.clone()),
            control: std::cell::RefCell::new({
                let mut control = murmur_core::dictation::Controller::default();
                control.gate = murmur_core::dictation::LaunchGate {
                    configured: true,
                    onboarding_complete: true,
                    model_installed: true,
                    accessibility: true,
                    ..Default::default()
                };
                control
            }),
        });
        let mut entity = None;
        let handle = cx
            .open_window(
                WindowOptions {
                    show: !headless,
                    focus: false,
                    kind: WindowKind::PopUp,
                    titlebar: None,
                    is_resizable: false,
                    is_movable: false,
                    window_background: WindowBackgroundAppearance::Transparent,
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(frame.x as f32), px(frame.y as f32)),
                        size(px(frame.width as f32), px(frame.height as f32)),
                    ))),
                    ..WindowOptions::default()
                },
                |w, cx| {
                    let overlay = cx.new(|_| {
                        let mut overlay = Overlay::new(snapshot, host.clone());
                        overlay.model = model;
                        overlay.inbox = host.inbox.clone();
                        overlay
                    });
                    entity = Some(overlay.clone());
                    cx.new(|cx| gpui_base::Root::new(overlay, w, cx).bg(transparent_black()))
                },
            )
            .expect("synthetic overlay");
        let overlay = entity.unwrap();
        if let Some(path) = output {
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(100))
                    .await;
                cx.update_window(handle.into(), |_, w, cx| {
                    w.draw(cx).clear(cx);
                    let image = w.render_to_image().expect("offscreen overlay");
                    assert!(
                        image.pixels().any(|p| *p != *image.get_pixel(0, 0)),
                        "blank overlay"
                    );
                    murmur_ui::overlay::native::assert_webkit_policy("MURMUR_OVERLAY_INVENTORY");
                    image.save(path).expect("write synthetic capture");
                    cx.quit();
                })
                .expect("offscreen capture");
            })
            .detach();
        } else {
            // Optional bounded diagnostic run, no input or system-setting changes.
            if let Ok(seconds) = std::env::var("MURMUR_WEBKIT_QA_SECONDS") {
                let seconds: u64 = seconds.parse().expect("QA seconds");
                cx.spawn(async move |cx| {
                    cx.background_executor()
                        .timer(std::time::Duration::from_secs(seconds))
                        .await;
                    murmur_ui::overlay::native::assert_webkit_policy("MURMUR_OVERLAY_INVENTORY");
                    cx.update(|cx| cx.quit());
                })
                .detach();
            }
            handle
                .update(cx, |_, w, cx| {
                    overlay.update(cx, |s, cx| {
                        if let Some(screen) = murmur_ui::overlay::native::screen() {
                            s.model.screen = screen;
                        }
                        s.advance(w, cx);
                        s.follow(w, cx);
                    })
                })
                .unwrap();
        }
    });
}
