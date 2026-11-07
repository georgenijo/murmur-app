//! Synthetic native Notetaker review app. No production identity or data access.
//!
//! Gallery/QA-only launch parameters (never read by production startup):
//! - MURMUR_MEETINGS_STATE=compact uses a 720×560 content window (default: review).
//! - MURMUR_MEETINGS_COUNT=N creates N meetings, initially 50, then Load more
//!   pages of 50 (0..=10,000; default 2). End reaches the last loaded row.
//! - MURMUR_MEETINGS_SEGMENTS=N creates N transcript segments, paged 24 at a
//!   time (0..=10,000; default 4), including final, pending and failed text.
//! - MURMUR_MEETINGS_TEXT_CHARS=N gives each final segment N synthetic
//!   characters (0..=32,768; default 0 keeps short text); >2,048 exercises
//!   transcript text pages.
//!
//! Example (from app/gpui):
//! ```sh
//! MURMUR_MEETINGS_COUNT=250 MURMUR_MEETINGS_SEGMENTS=80 \
//! MURMUR_MEETINGS_TEXT_CHARS=5000 cargo run --features gallery --bin meetings-qa
//! ```
//!
//! The combined fixture must fit the 16 MiB projection limit.
//! All names/text/PCM are synthetic; no microphone, production storage or
//! pasteboard is accessed.
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
        let host = Rc::new(fixtures::FixtureHost::with_corpus(
            fixture_count("MURMUR_MEETINGS_COUNT", 2, 10_000),
            fixture_count("MURMUR_MEETINGS_SEGMENTS", 4, 10_000),
            fixture_count("MURMUR_MEETINGS_TEXT_CHARS", 0, 32_768),
        ));
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
                        appears_transparent: true,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |w, cx| {
                    let view = cx.new(|cx| Meetings::new(host, w, cx));
                    entity = Some(view.clone());
                    if std::env::var_os("MURMUR_QA_WORKSPACE").is_some() {
                        view.update(cx, |s, _| s.embedded = true);
                        let home = cx.new(|cx| {
                            murmur_ui::home::Home::new(
                                Rc::new(murmur_ui::home::fixtures::FixtureHost::default()),
                                Rc::new(murmur_ui::history::fixtures::FixtureHost::default()),
                                w,
                                cx,
                            )
                        });
                        home.update(cx, |s, cx| {
                            s.workspace = Some((
                                murmur_ui::home::model::Destination::Meetings,
                                view.clone().into(),
                            ));
                            s.attach(cx);
                            s.drain(cx);
                        });
                        cx.new(|cx| gpui_base::Root::new(home, w, cx))
                    } else {
                        cx.new(|cx| gpui_base::Root::new(view, w, cx))
                    }
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

fn fixture_count(name: &str, default: usize, max: usize) -> usize {
    std::env::var(name).ok().map_or(default, |value| {
        value
            .parse::<usize>()
            .ok()
            .filter(|n| *n <= max)
            .unwrap_or_else(|| panic!("{name} must be an integer from 0 to {max}"))
    })
}
