//! Synthetic offscreen/native read-only QA. Never accesses application storage.
//!
//! Launch parameters (environment variables; gallery binary only):
//! - MURMUR_SETTINGS_STATE=general-advanced opens calibration; recording-advanced,
//!   delivery-advanced and shortcuts select the other QA destinations.
//! - MURMUR_SETTINGS_PREVIEW_FAILURE=1 fails the first calibration preview only
//!   (S05). Open Calibrate, observe failure, then Save: the saved offset survives.
//! - MURMUR_SETTINGS_PREVIEW_DELAY_MS=2000 delays each preview reply (S06).
//!   Open Calibrate, Escape while pending, then wait for the stale reply.
//! - MURMUR_SETTINGS_SHORTCUT_CAPTURE=1 exposes synthetic capture buttons in
//!   shortcuts (S10). Activate one, then F1–F20 for dictation or Command with
//!   Shift/Option/Control-V for Paste last; Escape cancels. No global hook.
//! - MURMUR_SETTINGS_SHORTCUT_CONFLICT=1 rejects shortcut registration attempts,
//!   including picker edits, without changing the old binding. Combine with
//!   SHORTCUT_CAPTURE to exercise capture failure. All services are in memory.
//!
//! Existing MURMUR_GOLDEN selects an offscreen capture; MURMUR_DARK and
//! MURMUR_CONTRAST select its appearance. No OS service or real pasteboard use.
use gpui::*;
use murmur_ui::{
    preferences::DisplayPreferences,
    settings::{Settings, fixtures, host::Host, model::*, schema::Page},
};
use std::rc::Rc;
fn main() {
    murmur_ui::overlay::native::start_webkit_policy();
    if std::env::var_os("MURMUR_WEBKIT_BASELINE_ONLY").is_some() {
        murmur_ui::overlay::native::assert_webkit_policy("MURMUR_SETTINGS_INVENTORY");
        return;
    }
    println!(
        "MURMUR_SETTINGS_REVISION={}",
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
        let state = std::env::var("MURMUR_SETTINGS_STATE").unwrap_or_else(|_| "general".into());
        let compact = state == "compact";
        let host = Rc::new(fixtures::FixtureHost::default());
        host.fail_preview
            .set(std::env::var("MURMUR_SETTINGS_PREVIEW_FAILURE").as_deref() == Ok("1"));
        host.fail_shortcut
            .set(std::env::var("MURMUR_SETTINGS_SHORTCUT_CONFLICT").as_deref() == Ok("1"));
        let preview_delay = std::env::var("MURMUR_SETTINGS_PREVIEW_DELAY_MS")
            .ok()
            .map(|s| {
                s.parse::<u64>()
                    .expect("preview delay must be milliseconds")
            })
            .unwrap_or(0);
        assert!(
            preview_delay <= 60_000,
            "preview delay must be at most 60000 ms"
        );
        host.delay_preview.set(preview_delay > 0);
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
                        appears_transparent: true,
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
                    let d = cx.new(|cx| Settings::new(host.clone(), w, cx));
                    entity = Some(d.clone());
                    cx.new(|cx| gpui_base::Root::new(d, w, cx))
                },
            )
            .expect("synthetic Settings window");
        let entity = entity.unwrap();
        handle
            .update(cx, |_, w, cx| {
                entity.update(cx, |s, cx| {
                    s.fixture = true;
                    s.fixture_shortcuts =
                        std::env::var("MURMUR_SETTINGS_SHORTCUT_CAPTURE").as_deref() == Ok("1");
                    s.attach(cx);
                    s.drain(cx);
                    s.follow(cx);
                    match state.as_str() {
                        "loading" => {
                            s.model.snapshot = None;
                            s.model.pending = true;
                        }
                        "error" => s.model.problem = Some(Problem::Unavailable),
                        "search" => {
                            s.model.query = "silence".into();
                            s.search
                                .update(cx, |field, cx| field.set_value("silence", w, cx));
                        }
                        "compact" => (),
                        id => {
                            s.navigate(Page::resolve(id.trim_end_matches("-advanced")), None, w, cx)
                        }
                    }
                    if let Some(snapshot) = s.model.snapshot.as_mut() {
                        if state.ends_with("-advanced") {
                            snapshot.disclosure["panes"][s.model.page.id()] =
                                serde_json::json!(true);
                        }
                        if state == "recording-advanced" {
                            for key in ["captureStandbyEnabled", "smartAutoMicrophoneEnabled"] {
                                snapshot.settings[key] = serde_json::json!(true);
                            }
                            snapshot.runtime.meter = Some((0.25, 0.5));
                        }
                        if state == "delivery-advanced" {
                            snapshot.settings["autoPaste"] = serde_json::json!(true);
                            snapshot.settings["saveTranscript"] = serde_json::json!(true);
                        }
                        if state == "text-advanced" {
                            snapshot.settings["cleanupEnabled"] = serde_json::json!(true);
                        }
                    }
                })
            })
            .expect("initialize synthetic Settings");
        if preview_delay > 0 {
            let entity = entity.downgrade();
            cx.spawn(async move |cx| {
                loop {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(16))
                        .await;
                    let pending = host.delayed.borrow_mut().drain(..).collect::<Vec<_>>();
                    if pending.is_empty() {
                        continue;
                    }
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(preview_delay))
                        .await;
                    host.delay_preview.set(false);
                    for (request, inbox) in pending {
                        // FixtureHost rechecks the original lease before any effect.
                        host.request(request, inbox);
                    }
                    host.delay_preview.set(true);
                    if entity.update(cx, |s, cx| s.drain(cx)).is_err() {
                        break;
                    }
                }
            })
            .detach();
        }
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
                .expect("settle Settings controls");
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(32))
                    .await;
                cx.update_window(handle.into(), |_, w, cx| {
                    w.draw(cx).clear(cx);
                    let image = w.render_to_image().expect("offscreen Settings");
                    assert!(
                        image.pixels().any(|p| *p != *image.get_pixel(0, 0)),
                        "blank Settings capture"
                    );
                    murmur_ui::overlay::native::assert_webkit_policy("MURMUR_SETTINGS_INVENTORY");
                    image.save(path).expect("synthetic capture");
                    cx.quit();
                })
                .expect("Settings capture");
            })
            .detach();
        } else {
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(30))
                    .await;
                murmur_ui::overlay::native::assert_webkit_policy("MURMUR_SETTINGS_INVENTORY");
                cx.update(|cx| cx.quit());
            })
            .detach();
        }
    });
}
