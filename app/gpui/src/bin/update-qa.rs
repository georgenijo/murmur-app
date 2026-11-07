//! Synthetic updater presentation, no storage, network, pasteboard or capture access.
use gpui::{prelude::*, *};
use murmur_ui::{Button, ButtonIntent, icons::Glyph, tokens::Palette, update_dialog};
use std::rc::Rc;
struct Review {
    home: Entity<murmur_ui::home::Home>,
    open: bool,
    downloading: bool,
}
impl Render for Review {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::get(cx);
        let header = update_dialog::header(
            "Update available",
            "Version 0.43.1",
            Some(
                Button::new("update-close", "Close update dialog")
                    .icon_only(Glyph::Close)
                    .intent(ButtonIntent::Ghost)
                    .on_click(cx.listener(|s, _, _, cx| {
                        s.open = false;
                        cx.notify();
                    })),
            ),
            p,
        );
        let notes = update_dialog::notes().children(
            [
                "Faster local transcription.",
                "More reliable microphone startup.",
                "Clearer recording controls.",
            ]
            .map(|text| div().text_color(p.muted).child(format!("•  {text}"))),
        );
        let primary = Button::new(
            "update-download",
            if self.downloading {
                "Downloading…"
            } else {
                "Download update"
            },
        )
        .pill()
        .full_width()
        .intent(ButtonIntent::Primary)
        .disabled(self.downloading)
        .on_click(cx.listener(|s, _, _, cx| {
            s.downloading = true;
            cx.notify();
        }));
        let later = Button::new("update-later", "Later")
            .pill()
            .full_width()
            .intent(ButtonIntent::Secondary)
            .on_click(cx.listener(|s, _, _, cx| {
                s.open = false;
                cx.notify();
            }));
        let footer = update_dialog::footer(p)
            .child(div().flex_1().child(primary))
            .child(div().flex_1().child(later));
        div()
            .size_full()
            .relative()
            .child(self.home.clone())
            .when(self.open, |root| {
                root.child(
                    update_dialog::scrim().child(
                        update_dialog::card(p)
                            .child(header)
                            .child(notes)
                            .child(footer),
                    ),
                )
            })
    }
}
fn main() {
    println!(
        "MURMUR_UPDATE_REVISION={}",
        option_env!("MURMUR_QA_REVISION").unwrap_or("development")
    );
    gpui_platform::application().run(|cx| {
        cx.set_quit_mode(QuitMode::LastWindowClosed);
        murmur_ui::init_with_preferences(
            cx,
            Some(murmur_ui::preferences::DisplayPreferences {
                reduce_motion: true,
                reduce_transparency: true,
                increase_contrast: false,
            }),
        );
        murmur_ui::tokens::apply_appearance(false, cx);
        cx.open_window(
            WindowOptions {
                focus: false,
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(0.), px(30.)),
                    size(px(880.), px(720.)),
                ))),
                titlebar: Some(TitlebarOptions {
                    appears_transparent: true,
                    ..Default::default()
                }),
                ..Default::default()
            },
            |w, cx| {
                let home = cx.new(|cx| {
                    murmur_ui::home::Home::new(
                        Rc::new(murmur_ui::home::fixtures::FixtureHost::default()),
                        Rc::new(murmur_ui::history::fixtures::FixtureHost::default()),
                        w,
                        cx,
                    )
                });
                home.update(cx, |s, cx| {
                    s.attach(cx);
                    s.drain(cx);
                    s.follow(cx);
                });
                let review = cx.new(|_| Review {
                    home,
                    open: true,
                    downloading: false,
                });
                cx.new(|cx| gpui_base::Root::new(review, w, cx))
            },
        )
        .expect("synthetic updater window");
    });
}
