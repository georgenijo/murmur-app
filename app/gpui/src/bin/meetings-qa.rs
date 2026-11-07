//! Synthetic native Notetaker review app. No production identity or data access.
use gpui::*;
use murmur_ui::{
    meetings::{Meetings, fixtures},
    preferences::DisplayPreferences,
};
use std::rc::Rc;
fn main() {
    murmur_ui::overlay::native::start_webkit_policy();
    if std::env::var_os("MURMUR_WEBKIT_BASELINE_ONLY").is_some() {
        murmur_ui::overlay::native::assert_webkit_policy("MURMUR_MEETINGS_INVENTORY");
        return;
    }
    println!(
        "MURMUR_MEETINGS_REVISION={}",
        option_env!("MURMUR_QA_REVISION").unwrap_or("development")
    );
    if std::env::var_os("MURMUR_AUDIO_SELF_TEST").is_some() {
        audio_self_test();
        murmur_ui::overlay::native::assert_webkit_policy("MURMUR_MEETINGS_INVENTORY");
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
        cx.set_quit_mode(QuitMode::LastWindowClosed);
        murmur_ui::init_with_preferences(
            cx,
            headless.then_some(DisplayPreferences {
                reduce_motion: true,
                reduce_transparency: true,
                increase_contrast: std::env::var_os("MURMUR_CONTRAST").is_some(),
            }),
        );
        murmur_ui::tokens::apply_appearance(std::env::var_os("MURMUR_DARK").is_some(), cx);
        let state = std::env::var("MURMUR_MEETINGS_STATE").unwrap_or_else(|_| "review".into());
        let mut entity = None;
        let host = Rc::new(fixtures::FixtureHost::default());
        host.native_audio.set(!headless);
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
                        title: Some("Murmur Meetings (synthetic)".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |w, cx| {
                    let view = cx.new(|cx| Meetings::new(host, w, cx));
                    entity = Some(view.clone());
                    cx.new(|cx| gpui_base::Root::new(view, w, cx))
                },
            )
            .expect("synthetic meetings window");
        let entity = entity.unwrap();
        handle
            .update(cx, |_, w, cx| {
                entity.update(cx, |s, cx| {
                    s.attach(cx);
                    s.drain(cx);
                    s.fixture_state(&state, w, cx);
                    if !headless {
                        s.follow(cx);
                    }
                })
            })
            .expect("synthetic meetings state");
        if std::env::var_os("MURMUR_NATIVE_TEXT_CHECK").is_some() {
            let text_entity = entity.clone();
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(500))
                    .await;
                cx.update_window(handle.into(), |_, w, cx| {
                    text_entity.update(cx, |s, cx| s.fixture_type(w, cx))
                })
                .expect("native text focus and input hook");
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(1200))
                    .await;
                cx.update(|cx| {
                    println!(
                        "MURMUR_MEETINGS_TEXT=PASS native field focus and internal text handler"
                    );
                    cx.quit();
                });
            })
            .detach();
        }
        if let Some(path) = output {
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(100))
                    .await;
                cx.update_window(handle.into(), |_, w, cx| {
                    w.dispatch_event(
                        MouseMoveEvent {
                            position: point(px(-1.), px(-1.)),
                            pressed_button: None,
                            modifiers: Modifiers::default(),
                        }
                        .to_platform_input(),
                        cx,
                    );
                    if state == "text-input" {
                        entity.update(cx, |s, cx| s.fixture_type(w, cx));
                    }
                    w.draw(cx).clear(cx);
                })
                .expect("settle meetings controls");
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(32))
                    .await;
                cx.update_window(handle.into(), |_, w, cx| {
                    w.draw(cx).clear(cx);
                    let image = w.render_to_image().expect("offscreen meetings");
                    assert!(
                        image.pixels().any(|p| *p != *image.get_pixel(0, 0)),
                        "blank meetings capture"
                    );
                    murmur_ui::overlay::native::assert_webkit_policy("MURMUR_MEETINGS_INVENTORY");
                    image.save(path).expect("synthetic capture");
                    cx.quit();
                })
                .expect("meetings capture");
            })
            .detach();
        } else {
            cx.on_app_quit(|_| async {
                murmur_ui::overlay::native::assert_webkit_policy("MURMUR_MEETINGS_INVENTORY");
            })
            .detach();
        }
    });
}
#[cfg(target_os = "macos")]
fn audio_self_test() {
    use murmur_ui::meetings::playback::NativeSink;
    let output = std::sync::Arc::new(
        NativeSink::new().unwrap_or_else(|_| panic!("native output unavailable")),
    );
    fixtures::audio_self_test(output)
        .unwrap_or_else(|_| panic!("native playback self-test failed"));
    println!(
        "MURMUR_MEETINGS_AUDIO=PASS native AVAudioEngine play/pause/seek/channel; synthetic PCM only"
    );
}
#[cfg(not(target_os = "macos"))]
fn audio_self_test() {
    panic!("native playback requires macOS")
}
