//! Synthetic review app and offscreen render driver. No production I/O.
use gpui::*;
use murmur_ui::{preferences::DisplayPreferences, remaining::Review};
fn main() {
    murmur_ui::overlay::native::start_webkit_policy();
    if std::env::var_os("MURMUR_WEBKIT_BASELINE_ONLY").is_some() {
        murmur_ui::overlay::native::assert_webkit_policy("MURMUR_REMAINING_INVENTORY");
        return;
    }
    println!(
        "MURMUR_REMAINING_REVISION={}",
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
        murmur_ui::init_with_preferences(
            cx,
            Some(DisplayPreferences {
                reduce_motion: true,
                reduce_transparency: true,
                increase_contrast: std::env::var_os("MURMUR_CONTRAST").is_some(),
            }),
        );
        let dark = std::env::var_os("MURMUR_DARK").is_some();
        murmur_ui::tokens::apply_appearance(dark, cx);
        let section = std::env::var("MURMUR_SECTION").unwrap_or_else(|_| "palette".into());
        let index = [
            "palette",
            "insights",
            "models",
            "knowledge",
            "modes",
            "themes",
            "spellings",
            "commands",
            "apps",
            "scan",
        ]
        .iter()
        .position(|s| *s == section)
        .expect("known section");
        let state = std::env::var("MURMUR_REMAINING_STATE").unwrap_or_else(|_| "ready".into());
        let mut entity = None;
        let handle = cx
            .open_window(
                WindowOptions {
                    show: !headless,
                    focus: false,
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(0.), px(0.)),
                        if state == "compact" {
                            size(px(720.), px(560.))
                        } else {
                            size(px(1120.), px(820.))
                        },
                    ))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Murmur remaining sections (synthetic)".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |w, cx| {
                    let e = cx.new(|cx| Review::new(w, cx));
                    entity = Some(e.clone());
                    cx.new(|cx| gpui_base::Root::new(e, w, cx))
                },
            )
            .expect("review window");
        let entity = entity.unwrap();
        handle
            .update(cx, |_, w, cx| {
                entity.update(cx, |s, cx| {
                    s.select(if index >= 6 { 3 } else { index }, w, cx);
                    if index >= 6 {
                        s.select_customize(
                            murmur_ui::customize::model::Page::ALL[index - 6],
                            w,
                            cx,
                        );
                    }
                    match index {
                        0 => {
                            s.palette.update(cx, |s, _| {
                                s.controller.drain();
                            });
                        }
                        1 => {
                            s.insights.update(cx, |s, _| {
                                s.controller.drain();
                            });
                        }
                        2 => {
                            s.models.update(cx, |s, _| {
                                s.controller.drain();
                            });
                        }
                        3 => {
                            s.knowledge.update(cx, |s, _| {
                                s.controller.drain();
                            });
                        }
                        4 => {
                            s.modes.update(cx, |s, _| {
                                s.controller.drain();
                            });
                        }
                        6..=9 => {
                            s.customize.update(cx, |s, _| {
                                s.controller.drain();
                            });
                        }
                        _ => {
                            s.themes.update(cx, |s, _| {
                                s.controller.drain();
                            });
                        }
                    }
                    s.fixture_state(&state, w, cx);
                });
                let mut appearance = murmur_ui::themes::fixtures::snapshot().appearance;
                appearance["mode"] = serde_json::json!(if dark { "dark" } else { "light" });
                let library = murmur_ui::themes::fixtures::snapshot().library;
                if std::env::var_os("MURMUR_MIDNIGHT").is_some() {
                    appearance = murmur_ui::themes::library::selection(
                        &appearance,
                        &library,
                        "midnight",
                        None,
                    )
                    .unwrap();
                }
                if std::env::var_os("MURMUR_IMPORTED").is_some() {
                    appearance = murmur_ui::themes::library::selection(
                        &appearance,
                        &library,
                        "synthetic-orchid",
                        None,
                    )
                    .unwrap();
                }
                if index == 5 {
                    entity.read(cx).themes.clone().update(cx, |s, _| {
                        if let Some(snapshot) = s.controller.model.snapshot.as_mut() {
                            snapshot.appearance = appearance.clone();
                        }
                    });
                }
                murmur_ui::themes::appearance::install(appearance, cx);
            })
            .expect("initial state");
        if std::env::var_os("MURMUR_NATIVE_TEXT_CHECK").is_some() {
            let e = entity.clone();
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(500))
                    .await;
                let before = murmur_ui::overlay::native::webkit_images();
                cx.update_window(handle.into(), |_, w, cx| {
                    e.read(cx)
                        .palette
                        .clone()
                        .update(cx, |s, cx| s.fixture_type(w, cx));
                })
                .expect("native text hook");
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(1000))
                    .await;
                let after = murmur_ui::overlay::native::webkit_images();
                println!("MURMUR_REMAINING_TEXT={{\"before\":{before},\"after\":{after}}}");
                assert_eq!(before, after, "palette focus/typing loaded Writing Tools");
                cx.update(|cx| cx.quit());
            })
            .detach();
        }
        if let Some(path) = output {
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(150))
                    .await;
                cx.update_window(handle.into(), |_, w, cx| {
                    if state == "text-input" {
                        entity
                            .read(cx)
                            .palette
                            .clone()
                            .update(cx, |s, cx| s.fixture_type(w, cx));
                    }
                    w.draw(cx).clear(cx);
                })
                .expect("settle");
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(32))
                    .await;
                cx.update_window(handle.into(), |_, w, cx| {
                    w.draw(cx).clear(cx);
                    let image = w.render_to_image().expect("offscreen image");
                    assert!(image.pixels().any(|p| *p != *image.get_pixel(0, 0)));
                    murmur_ui::overlay::native::assert_webkit_policy("MURMUR_REMAINING_INVENTORY");
                    image.save(path).expect("save image");
                    cx.quit();
                })
                .expect("capture");
            })
            .detach();
        } else {
            cx.on_app_quit(|_| async {
                murmur_ui::overlay::native::assert_webkit_policy("MURMUR_REMAINING_INVENTORY");
            })
            .detach();
        }
    });
}
