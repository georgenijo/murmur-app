//! Shared updater presentation; the service owner supplies state and actions.
use crate::{
    Button,
    icons::{self, Glyph},
    tokens::{self, Palette},
};
use gpui::{prelude::*, *};
pub fn card(p: Palette) -> Stateful<Div> {
    div()
        .id("update-dialog")
        .role(Role::Dialog)
        .aria_label("Murmur update")
        .w(px(560.))
        .max_w_full()
        .h(px(620.))
        .max_h_full()
        .flex()
        .flex_col()
        .overflow_hidden()
        .bg(p.tint_raised)
        .border_1()
        .border_color(p.hairline_strong)
        .rounded(tokens::CARD_RADIUS)
        .shadow(p.card_shadow())
}
pub fn header(
    title: impl Into<SharedString>,
    caption: impl Into<SharedString>,
    close: Option<Button>,
    p: Palette,
) -> Div {
    let title: SharedString = title.into();
    let caption: SharedString = caption.into();
    div()
        .flex()
        .items_start()
        .gap(px(12.))
        .p(px(24.))
        .border_b_1()
        .border_color(p.hairline)
        .flex_shrink_0()
        .child(
            div()
                .size(px(40.))
                .rounded(tokens::CARD_RADIUS)
                .bg(p.surface_container_low)
                .flex()
                .items_center()
                .justify_center()
                .child(icons::icon(Glyph::Download).size(px(20.))),
        )
        .child(
            div()
                .flex_1()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(
                    div()
                        .text_size(px(18.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .id("update-dialog-heading")
                        .role(Role::Heading)
                        .aria_label(title.clone())
                        .child(title),
                )
                .child(
                    div()
                        .text_size(tokens::LABEL_SIZE)
                        .text_color(p.muted)
                        .child(gpui::text!(caption)),
                ),
        )
        .children(close)
}
pub fn notes() -> Stateful<Div> {
    div()
        .id("release-notes")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .p(px(24.))
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .child(gpui::text!("What's Changed")),
        )
}
pub fn footer(p: Palette) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .p(px(16.))
        .border_t_1()
        .border_color(p.hairline)
        .bg(p.surface_container_low)
        .flex_shrink_0()
}
pub fn scrim() -> Stateful<Div> {
    div()
        .id("update-dialog-scrim")
        .absolute()
        .inset_0()
        .p(px(24.))
        .flex()
        .items_center()
        .justify_center()
        .bg(black().opacity(0.36))
}
