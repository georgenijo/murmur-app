use gpui::{prelude::*, *};
use murmur_ui::{
    gallery::Gallery,
    preferences::{self, DisplayPreferences},
};

fn main() {
    let output = std::env::var_os("MURMUR_GOLDEN");
    let headless = output.is_some();
    let application = if headless {
        gpui_platform::application().with_windowing(WindowingRequest::Headless)
    } else {
        gpui_platform::application()
    };
    application.run(move |cx| {
        if headless {
            murmur_ui::init_with_preferences(cx, Some(DisplayPreferences::default()));
        } else {
            murmur_ui::init(cx);
        }
        let dark = std::env::var_os("MURMUR_DARK").is_some();
        murmur_ui::tokens::apply_appearance(dark, cx);
        if headless {
            // Fixed preferences; never reads or changes the GUI owner's settings.
            preferences::apply(
                DisplayPreferences {
                    reduce_motion: true,
                    increase_contrast: std::env::var_os("MURMUR_CONTRAST").is_some(),
                    reduce_transparency: true,
                },
                cx,
            );
        }
        let mut gallery_entity = None;
        let handle = cx
            .open_window(
                WindowOptions {
                    show: !headless,
                    focus: !headless,
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(0.), px(0.)),
                        size(px(1120.), px(820.)),
                    ))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Murmur control gallery (synthetic)".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |w, cx| {
                    let gallery = cx.new(|cx| Gallery::new(w, cx));
                    gallery_entity = Some(gallery.clone());
                    cx.new(|cx| gpui_base::Root::new(gallery, w, cx))
                },
            )
            .expect("create gallery");
        let gallery = gallery_entity.unwrap();
        let state = std::env::var("MURMUR_GALLERY_STATE").unwrap_or_else(|_| "rest".into());
        cx.update_window(handle.into(), |_, w, cx| {
            w.draw(cx).clear(cx);
            if state != "rest" {
                // Dispatch outside every entity update: key observers may
                // update their own gallery/control entity while handling it.
                let key = Keystroke::parse_with_os("f6", cx.platform_info().os).unwrap();
                w.dispatch_event(
                    KeyDownEvent {
                        keystroke: key.clone(),
                        is_held: false,
                        prefer_character_input: false,
                    }
                    .to_platform_input(),
                    cx,
                );
                w.dispatch_event(KeyUpEvent { keystroke: key }.to_platform_input(), cx);
            }
            gallery.update(cx, |g, cx| g.prepare_snapshot(&state, w, cx))
        })
        .unwrap();
        if let Some(path) = output {
            cx.spawn(async move |cx| {
                // Match Phase 0: allow font/asset initialization, then render
                // directly to a Metal texture. No visible NSWindow is opened.
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(100))
                    .await;
                cx.update_window(handle.into(), |_, window, cx| {
                    window.draw(cx).clear(cx);
                    if state == "hover" || state == "pressed" {
                        // Gallery's first 32px button, using the shared 24px
                        // inset and heading rhythm. Input stays inside GPUI.
                        let position = point(px(50.), px(112.));
                        window.dispatch_event(
                            MouseMoveEvent {
                                position,
                                pressed_button: None,
                                modifiers: Modifiers::default(),
                            }
                            .to_platform_input(),
                            cx,
                        );
                        if state == "pressed" {
                            window.dispatch_event(
                                MouseDownEvent {
                                    position,
                                    button: MouseButton::Left,
                                    modifiers: Modifiers::default(),
                                    click_count: 1,
                                    first_mouse: false,
                                }
                                .to_platform_input(),
                                cx,
                            );
                        }
                        window.draw(cx).clear(cx);
                    }
                    let image = window.render_to_image().expect("offscreen Metal");
                    assert!(
                        image.pixels().any(|p| *p != *image.get_pixel(0, 0)),
                        "blank frame"
                    );
                    image.save(&path).expect("save golden");
                    cx.quit();
                })
                .expect("capture gallery");
            })
            .detach();
        }
    });
}
