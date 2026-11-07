//! Synthetic offscreen/native read-only QA. Never accesses application storage.
use gpui::*;
use murmur_ui::{
    home::{Home, fixtures, model::*},
    preferences::DisplayPreferences,
};
use std::rc::Rc;
fn main() {
    murmur_ui::overlay::native::start_webkit_policy();
    if std::env::var_os("MURMUR_WEBKIT_BASELINE_ONLY").is_some() {
        murmur_ui::overlay::native::assert_webkit_policy("MURMUR_HOME_INVENTORY");
        return;
    }
    println!(
        "MURMUR_HOME_REVISION={}",
        option_env!("MURMUR_QA_REVISION").unwrap_or("development")
    );
    let output = std::env::var_os("MURMUR_GOLDEN");
    let headless = output.is_some();
    let app = if headless {
        gpui_platform::application().with_windowing(WindowingRequest::Headless)
    } else {
        gpui_platform::application()
    };
    app.run(move |cx| {
        cx.set_quit_mode(QuitMode::LastWindowClosed);
        if headless {
            murmur_ui::init_with_preferences(
                cx,
                Some(DisplayPreferences {
                    reduce_motion: true,
                    reduce_transparency: true,
                    increase_contrast: std::env::var_os("MURMUR_CONTRAST").is_some(),
                }),
            );
        } else {
            murmur_ui::init(cx);
        }
        murmur_ui::tokens::apply_appearance(std::env::var_os("MURMUR_DARK").is_some(), cx);
        if std::env::var_os("MURMUR_MIDNIGHT").is_some() {
            murmur_ui::gallery::apply_midnight(cx);
        }
        let state = std::env::var("MURMUR_HOME_STATE").unwrap_or_else(|_| "checklist".into());
        let compact = state == "compact";
        let mut entity = None;
        let handle = cx
            .open_window(
                WindowOptions {
                    show: !headless,
                    focus: false,
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(0.), px(0.)),
                        if compact {
                            size(px(720.), px(560.))
                        } else {
                            size(px(880.), px(720.))
                        },
                    ))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Murmur Home (synthetic)".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |w, cx| {
                    if !headless {
                        let apply = |w: &mut Window, cx: &mut App| {
                            murmur_ui::tokens::apply_appearance(
                                matches!(
                                    w.appearance(),
                                    WindowAppearance::Dark | WindowAppearance::VibrantDark
                                ),
                                cx,
                            );
                            cx.refresh_windows();
                        };
                        apply(w, cx);
                        w.observe_window_appearance(apply).detach();
                    }
                    let d = cx.new(|cx| {
                        Home::new(
                            Rc::new(fixtures::FixtureHost::default()),
                            Rc::new(murmur_ui::history::fixtures::FixtureHost::default()),
                            w,
                            cx,
                        )
                    });
                    entity = Some(d.clone());
                    cx.new(|cx| gpui_base::Root::new(d, w, cx))
                },
            )
            .expect("synthetic Home window");
        let entity = entity.unwrap();
        handle
            .update(cx, |_, _w, cx| {
                entity.update(cx, |s, cx| {
                    s.attach(cx);
                    s.drain(cx);
                    if let Some(snapshot) = s.model.snapshot.as_mut() {
                        snapshot.discovery["checklistDismissed"] =
                            serde_json::json!(state != "checklist");
                        match state.as_str() {
                            "starting" => {
                                snapshot.status = Status::Starting;
                                snapshot.recording_generation = Some(80);
                            }
                            "recording" | "destination" => {
                                snapshot.status = Status::Recording;
                                snapshot.recording_generation = Some(80);
                                snapshot.recording_seconds = 65;
                                snapshot.audio_level = 0.03;
                            }
                            "processing" => snapshot.status = Status::Processing,
                            "recovering" => snapshot.status = Status::Recovering,
                            "meeting" => snapshot.meeting_busy = true,
                            "initializing" => snapshot.initialized = false,
                            "denied" => {
                                snapshot.permissions[0] =
                                    murmur_ui::onboarding::model::Access::Denied
                            }
                            _ => (),
                        }
                        if state.starts_with("hint-") {
                            snapshot.discovery["pendingHints"] =
                                serde_json::json!([state.trim_start_matches("hint-")]);
                        }
                        if state == "destination" {
                            snapshot.destination = Some("Clipboard only".into());
                        }
                    }
                    if state == "collapsed" {
                        s.desktop_collapsed = true;
                    }
                    if state == "loading" {
                        s.model.snapshot = None;
                        s.model.pending = true;
                    }
                    if state == "error" {
                        s.model.problem = Some(Problem::Unavailable);
                    }
                    if !headless {
                        s.follow(cx);
                    }
                })
            })
            .expect("initialize synthetic Home");
        if let Some(path) = output {
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(100))
                    .await;
                // Let controlled pickers/ranges finish their first layout and
                // deferred notifications before the captured presentation turn.
                cx.update_window(handle.into(), |_, w, cx| {
                    // Fix only this headless view's hover state; never move the
                    // shared desktop pointer or dispatch native HID input.
                    w.dispatch_event(
                        MouseMoveEvent {
                            position: point(px(-1.), px(-1.)),
                            pressed_button: None,
                            modifiers: Modifiers::default(),
                        }
                        .to_platform_input(),
                        cx,
                    );
                    w.draw(cx).clear(cx);
                })
                .expect("settle Home controls");
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(32))
                    .await;
                cx.update_window(handle.into(), |_, w, cx| {
                    w.draw(cx).clear(cx);
                    let image = w.render_to_image().expect("offscreen Home");
                    assert!(
                        image.pixels().any(|p| *p != *image.get_pixel(0, 0)),
                        "blank Home capture"
                    );
                    murmur_ui::overlay::native::assert_webkit_policy("MURMUR_HOME_INVENTORY");
                    image.save(path).expect("synthetic capture");
                    cx.quit();
                })
                .expect("Home capture");
            })
            .detach();
        } else {
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(1))
                    .await;
                murmur_ui::overlay::native::assert_webkit_policy("MURMUR_HOME_INVENTORY");
            })
            .detach();
        }
    });
}
