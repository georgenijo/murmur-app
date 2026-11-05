use gpui::{prelude::*, *};
struct Minimal;
impl Render for Minimal {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(0x132333))
            .text_color(rgb(0xffffff))
            .p_4()
            .child("Pinned GPUI minimal app")
    }
}
fn main() {
    gpui_platform::application().run(|cx| {
        cx.open_window(WindowOptions::default(), |_, cx| cx.new(|_| Minimal))
            .unwrap();
    });
}
