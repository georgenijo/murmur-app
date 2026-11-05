use gpui::{actions, prelude::*, *};
use std::{ffi::CString, time::Duration};
unsafe extern "C" {
    fn p0e_install();
    fn p0e_log(event: *const std::ffi::c_char);
}
fn log(event: &str) {
    unsafe { p0e_log(CString::new(event).unwrap().as_ptr()) }
}
actions!(prototype, [Quit, Relaunch]);
struct Probe {
    ticks: u64,
    golden: bool,
}
impl Render for Probe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(0x132333))
            .text_color(rgb(0xf5f8ff))
            .p_6()
            .flex()
            .flex_col()
            .gap_4()
            .child(div().text_xl().child("GPUI 0e — pinned build / quit probe"))
            .child("654efae441bc8eb117d0042bb28cd7cf3570bdc9")
            .child(if self.golden {
                "Golden frame: offscreen Metal render".to_string()
            } else {
                format!("Main-thread UI timer: {}", self.ticks)
            })
            .child("Quit waits for a 6 second background teardown.")
    }
}
fn main() {
    log("PROCESS_START");
    let golden = std::env::var_os("P0E_SCREENSHOT");
    let application = if golden.is_some() {
        gpui_platform::application().with_windowing(WindowingRequest::Headless)
    } else {
        gpui_platform::application()
    };
    application.run(move |cx| {
        unsafe { p0e_install() };
        cx.on_app_quit(|cx| {
            log("GPUI_QUIT_HANDLER_START");
            let timer = cx.background_executor().timer(Duration::from_millis(50));
            async move {
                timer.await;
                log("GPUI_QUIT_HANDLER_END");
            }
        })
        .detach();
        cx.on_app_restart(|_| log("GPUI_RESTART_OBSERVER")).detach();
        cx.on_action(|_: &Quit, cx| {
            log("MENU_QUIT_ACTION");
            cx.quit();
        });
        cx.on_action(|_: &Relaunch, cx| {
            log("PROGRAMMATIC_RELAUNCH_ACTION");
            // Explicit scratch bundle path: GPUI's unbundled fallback cannot be opened as an app.
            cx.set_restart_path(
                std::env::var_os("P0E_BUNDLE")
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| {
                        std::env::current_exe()
                            .unwrap()
                            .ancestors()
                            .nth(3)
                            .unwrap()
                            .to_path_buf()
                    }),
            );
            cx.restart();
        });
        cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
        cx.set_menus(vec![Menu::new("GPUI 0e").items([
            MenuItem::action("Relaunch Probe", Relaunch),
            MenuItem::separator(),
            MenuItem::action("Quit GPUI 0e", Quit),
        ])]);
        let h = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(80.), px(80.)),
                        size(px(600.), px(240.)),
                    ))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("GPUI 0e Quit Probe".into()),
                        ..Default::default()
                    }),
                    show: golden.is_none(),
                    focus: golden.is_none(),
                    ..Default::default()
                },
                |_, cx| {
                    cx.new(|_| Probe {
                        ticks: 0,
                        golden: golden.is_some(),
                    })
                },
            )
            .unwrap();
        if let Some(path) = golden {
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                cx.update_window(h.into(), |_, window, cx| {
                    window.draw(cx).clear(cx);
                    let image = window.render_to_image().expect("offscreen Metal capture");
                    assert!(
                        image.pixels().any(|p| *p != *image.get_pixel(0, 0)),
                        "blank golden"
                    );
                    image.save(&path).expect("write screenshot");
                    log(&format!(
                        "OFFSCREEN_SCREENSHOT {}x{} {:?}",
                        image.width(),
                        image.height(),
                        path
                    ));
                    cx.quit();
                })
                .unwrap();
            })
            .detach();
        } else {
            cx.spawn(async move |cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(250))
                        .await;
                    if h.update(cx, |probe, window, cx| {
                        probe.ticks += 1;
                        log(&format!("MAIN_UI_TICK {}", probe.ticks));
                        window.set_window_title(&format!("GPUI 0e — tick {}", probe.ticks));
                        cx.notify();
                    })
                    .is_err()
                    {
                        break;
                    }
                }
            })
            .detach();
        }
        log("READY");
    });
}
