//! Packaging entry point. Native service/startup integration follows in #996.
//! This shell deliberately performs no profile, capture, agent or clipboard I/O.
use gpui::*;

struct PackagingShell;
impl Render for PackagingShell {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(format!(
                "Murmur {} · native packaging rehearsal",
                env!("MURMUR_APP_VERSION")
            ))
    }
}

fn main() {
    if std::env::args().any(|arg| arg == "--version") {
        println!("{}", env!("MURMUR_APP_VERSION"));
        return;
    }
    gpui_platform::application().run(|cx| {
        cx.set_quit_mode(QuitMode::LastWindowClosed);
        murmur_ui::init(cx);
        cx.open_window(
            WindowOptions {
                focus: false,
                titlebar: Some(TitlebarOptions {
                    title: Some("Murmur".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| {
                let shell = cx.new(|_| PackagingShell);
                cx.new(|cx| gpui_base::Root::new(shell, window, cx))
            },
        )
        .expect("native packaging window");
    });
}
