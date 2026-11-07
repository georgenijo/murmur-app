//! Persistent native windows, menu dispatch and coordinated process lifetime.
use super::{
    lifecycle::{Lifecycle, ProcessLifecycle},
    menu::{Intent, TrayState},
    native,
};
use gpui::*;
use std::{
    rc::Rc,
    sync::{Arc, mpsc},
    time::Duration,
};
actions!(
    murmur_host,
    [
        Show,
        Diagnostics,
        About,
        Settings,
        CheckUpdates,
        Quit,
        Hide,
        HideOthers,
        ShowAll,
        ZoomIn,
        ZoomOut,
        ZoomReset
    ]
);
/// Production service composition is injected by the executable. This boundary
/// never constructs fixtures or starts a second recording/repository owner.
pub trait ApplicationHost {
    fn main_view(&self, window: &mut Window, cx: &mut App) -> AnyView;
    fn diagnostics_view(&self, window: &mut Window, cx: &mut App) -> AnyView;
    /// Revoke UI leases synchronously before ordering a persistent window out.
    /// Accepted durable owner work remains admitted; capture keeps running.
    fn window_hidden(&self, label: &'static str, window: &mut Window, cx: &mut App);
    fn tick(&self, _cx: &mut App) {}
    fn intent(&self, intent: Intent);
    fn tray(&self) -> TrayState;
    /// The coordinator alone may call this after committing its restart permit.
    fn relaunch(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), &'static str>>>>;
}
fn persistent(window: &mut Window, cx: &App, host: Rc<dyn ApplicationHost>, main: bool) {
    window.on_window_should_close(cx, move |window, cx| {
        host.window_hidden(if main { "main" } else { "diagnostics" }, window, cx);
        native::show(window, false);
        false
    });
}
fn show(handle: AnyWindowHandle, cx: &mut App) {
    let _ = handle.update(cx, |_, w, _| native::show(w, true));
}
/// Called after the executable's worker dispatch and service initialization.
pub fn run(
    host: Rc<dyn ApplicationHost>,
    lifecycle: Lifecycle,
    terminal: mpsc::Receiver<murmur_core::shutdown::Action>,
) {
    run_with(move |_| (host, Arc::new(lifecycle), terminal));
}
/// Services are composed inside the GPUI launch callback. No duplicate drain
/// driver is created: production supplies the same service-owned gate used by
/// process commands, signals, update restart and AppKit termination.
pub fn run_with(
    factory: impl FnOnce(
        &mut App,
    ) -> (
        Rc<dyn ApplicationHost>,
        Arc<dyn ProcessLifecycle>,
        mpsc::Receiver<murmur_core::shutdown::Action>,
    ) + 'static,
) {
    let app = gpui_platform::application();
    let reopen = Rc::new(std::cell::Cell::new(None::<AnyWindowHandle>));
    let main = reopen.clone();
    app.on_reopen(move |cx| {
        // Match Tauri's visible-window guard: overlay activation must not open Main.
        let visible = cx
            .windows()
            .into_iter()
            .any(|h| h.update(cx, |_, w, _| native::visible(w)).unwrap_or(false));
        if !visible && let Some(main) = main.get() {
            show(main, cx);
        }
    });
    app.run(move |cx| {
        cx.set_quit_mode(QuitMode::Explicit);
        crate::init(cx);
        let (host, lifecycle, terminal) = factory(cx);
        let termination = native::Termination::install(lifecycle.clone())
            .expect("coordinated native termination");
        let (mut tray, tray_intents) = native::Tray::new();
        tray.update(host.tray());
        let h = host.clone();
        let main = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(1120.), px(820.)),
                        cx,
                    ))),
                    window_min_size: Some(size(px(720.), px(560.))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Murmur".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                move |w, cx| {
                    persistent(w, cx, h.clone(), true);
                    let view = h.main_view(w, cx);
                    cx.new(|cx| gpui_base::Root::new(view, w, cx))
                },
            )
            .expect("native main window")
            .into();
        reopen.set(Some(main));
        let h = host.clone();
        let diagnostics: AnyWindowHandle = cx
            .open_window(
                WindowOptions {
                    show: false,
                    focus: false,
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(1040.), px(760.)),
                        cx,
                    ))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Murmur Diagnostics".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                move |w, cx| {
                    persistent(w, cx, h.clone(), false);
                    let view = h.diagnostics_view(w, cx);
                    cx.new(|cx| gpui_base::Root::new(view, w, cx))
                },
            )
            .expect("native diagnostics window")
            .into();
        cx.on_action(move |_: &Diagnostics, cx| show(diagnostics, cx));

        cx.on_action(|_: &ZoomIn, cx| zoom(cx, 0.1, false));
        cx.on_action(|_: &ZoomOut, cx| zoom(cx, -0.1, false));
        cx.on_action(|_: &ZoomReset, cx| zoom(cx, 0., true));
        cx.bind_keys([
            KeyBinding::new("cmd-+", ZoomIn, None),
            KeyBinding::new("cmd-=", ZoomIn, None),
            KeyBinding::new("cmd--", ZoomOut, None),
            KeyBinding::new("cmd-0", ZoomReset, None),
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-,", Settings, None),
            KeyBinding::new("cmd-h", Hide, None),
            KeyBinding::new("cmd-alt-h", HideOthers, None),
        ]);
        cx.on_action(move |_: &Show, cx| show(main, cx));
        let h = host.clone();
        cx.on_action(move |_: &Settings, cx| {
            show(main, cx);
            h.intent(Intent::Settings)
        });
        let h = host.clone();
        cx.on_action(move |_: &About, cx| {
            show(main, cx);
            h.intent(Intent::About)
        });
        let h = host.clone();
        cx.on_action(move |_: &CheckUpdates, cx| {
            show(main, cx);
            h.intent(Intent::CheckUpdates)
        });
        let l = lifecycle.clone();
        cx.on_action(move |_: &Quit, _| l.request(murmur_core::shutdown::Request::Quit));
        let h = host.clone();
        cx.on_action(move |_: &Hide, cx| {
            let h = h.clone();
            let _ = main.update(cx, |_, w, cx| h.window_hidden("main", w, cx));
            let _ = diagnostics.update(cx, |_, w, cx| h.window_hidden("diagnostics", w, cx));
            cx.hide();
        });
        cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
        cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
        use gpui_component::input::{Copy, Cut, Paste, Redo, SelectAll, Undo};
        cx.set_menus(vec![
            Menu::new("Murmur").items([
                MenuItem::action("About Murmur", About),
                MenuItem::action("Settings…", Settings),
                MenuItem::action("Check for Updates…", CheckUpdates),
                MenuItem::action("Diagnostics…", Diagnostics),
                MenuItem::separator(),
                MenuItem::os_submenu("Services", SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action("Hide Murmur", Hide),
                MenuItem::action("Hide Others", HideOthers),
                MenuItem::action("Show All", ShowAll),
                MenuItem::separator(),
                MenuItem::action("Quit Murmur", Quit),
            ]),
            Menu::new("Edit").items([
                MenuItem::os_action("Undo", Undo, OsAction::Undo),
                MenuItem::os_action("Redo", Redo, OsAction::Redo),
                MenuItem::separator(),
                MenuItem::os_action("Cut", Cut, OsAction::Cut),
                MenuItem::os_action("Copy", Copy, OsAction::Copy),
                MenuItem::os_action("Paste", Paste, OsAction::Paste),
                MenuItem::os_action("Select All", SelectAll, OsAction::SelectAll),
            ]),
        ]);
        cx.set_dock_menu(vec![
            MenuItem::action("Show Murmur", Show),
            MenuItem::action("Settings…", Settings),
        ]);
        cx.on_app_quit({
            let lifecycle = lifecycle.clone();
            move |_| {
                lifecycle.exited();
                async {}
            }
        })
        .detach();
        cx.spawn(async move |cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                cx.update(|cx| {
                    host.tick(cx);
                    let snapshot = host.tray();
                    while let Ok(intent) = tray_intents.try_recv() {
                        if !snapshot.accepts(&intent) {
                            continue;
                        }
                        match intent {
                            Intent::Show => show(main, cx),
                            Intent::Diagnostics => show(diagnostics, cx),
                            Intent::Quit => lifecycle.request(murmur_core::shutdown::Request::Quit),
                            Intent::Settings | Intent::About | Intent::CheckUpdates => {
                                show(main, cx);
                                host.intent(intent);
                            }
                            _ => host.intent(intent),
                        }
                    }
                    tray.update(host.tray());
                    if let Ok(action) = terminal.try_recv() {
                        if !lifecycle.permitted() {
                            return;
                        }
                        if action == murmur_core::shutdown::Action::Restart {
                            let launch = host.relaunch();
                            let termination = termination.clone();
                            cx.spawn(async move |cx| {
                                // Keep AppKit termination deferred until the
                                // parent-exit launcher acknowledges readiness.
                                let _ = launch.await;
                                cx.update(|cx| {
                                    if !termination.reply() {
                                        cx.quit();
                                    }
                                });
                            })
                            .detach();
                        } else if !termination.reply() {
                            cx.quit();
                        }
                    }
                });
            }
        })
        .detach();
        cx.activate(true);
    });
}

fn zoom(cx: &mut App, delta: f32, reset: bool) {
    if let Some(handle) = cx.active_window() {
        let _ = handle.update(cx, |_, w, cx| {
            let next = if reset {
                1.
            } else {
                ((w.ui_zoom() + delta) * 10.).round() / 10.
            };
            w.set_ui_zoom(next.clamp(0.5, 3.), cx);
        });
    }
}
