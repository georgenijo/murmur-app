//! Synthetic offscreen/native read-only QA. Never accesses application storage.
use gpui::*;
use murmur_ui::{
    history::{History, fixtures, model::*},
    preferences::DisplayPreferences,
};
use std::rc::Rc;
fn main() {
    murmur_ui::overlay::native::start_webkit_policy();
    if std::env::var_os("MURMUR_WEBKIT_BASELINE_ONLY").is_some() {
        murmur_ui::overlay::native::assert_webkit_policy("MURMUR_HISTORY_INVENTORY");
        return;
    }
    println!(
        "MURMUR_HISTORY_REVISION={}",
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
        let state = std::env::var("MURMUR_HISTORY_STATE").unwrap_or_else(|_| "list".into());
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
                        title: Some("Murmur History (synthetic)".into()),
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
                    let d =
                        cx.new(|cx| History::new(Rc::new(fixtures::FixtureHost::default()), w, cx));
                    entity = Some(d.clone());
                    cx.new(|cx| gpui_base::Root::new(d, w, cx))
                },
            )
            .expect("synthetic History window");
        let entity = entity.unwrap();
        handle
            .update(cx, |_, _, cx| {
                entity.update(cx, |s, cx| {
                    s.fixture = true;
                    s.attach(cx);
                    s.drain(cx);
                    s.model.set_clock(fixtures::clock());
                    match state.as_str() {
                        "detail" => {
                            s.model.detail = Some(200);
                        }
                        "detail-last-page" => {
                            s.model.detail = Some(200);
                            s.last_detail_page();
                        }
                        "search" => {
                            s.model.query.text = "violet release".into();
                            s.model.query.source = Source::Files;
                        }
                        "empty" => s.model.entries.clear(),
                        "no-matches" => s.model.query.text = "no synthetic match".into(),
                        "loading" => {
                            s.model.entries.clear();
                            s.model.loading = true;
                        }
                        "retention-off" => s.model.retain_history = false,
                        "migration" => s.model.retry_import = true,
                        "error" => s.model.problem = Some(Problem::Unavailable),
                        "confirm" => {
                            s.model.arm(DeleteScope::All, std::time::Instant::now());
                        }
                        _ => (),
                    }
                    s.prepare(cx);
                })
            })
            .expect("initialize synthetic History");
        if let Some(path) = output {
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(100))
                    .await;
                cx.update_window(handle.into(), |_, w, cx| {
                    // Headless windows can inherit the shared desktop pointer.
                    // Keep golden hover state fixed using in-process GPUI input;
                    // this never moves the OS pointer or sends native HID input.
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
                    let image = w.render_to_image().expect("offscreen History");
                    assert!(
                        image.pixels().any(|p| *p != *image.get_pixel(0, 0)),
                        "blank History capture"
                    );
                    murmur_ui::overlay::native::assert_webkit_policy("MURMUR_HISTORY_INVENTORY");
                    image.save(path).expect("synthetic capture");
                    cx.quit();
                })
                .expect("History capture");
            })
            .detach();
        } else {
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(30))
                    .await;
                murmur_ui::overlay::native::assert_webkit_policy("MURMUR_HISTORY_INVENTORY");
                cx.update(|cx| cx.quit());
            })
            .detach();
        }
    });
}
