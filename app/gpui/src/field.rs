use crate::tokens::{self, Palette};
use gpui::{prelude::*, *};
use gpui_component::input::{Input, InputState};

pub use gpui_component::input::{InputEvent, InputState as TextFieldState};

/// A real component editor (clipboard, undo, composition and selection), with
/// one focus owner and mandatory accessible name. No shadow editor state.
#[derive(IntoElement)]
pub struct TextField {
    state: Entity<InputState>,
    scroll_parent: Option<ScrollHandle>,
    label: SharedString,
    disabled: bool,
    readonly: bool,
    search_chrome: bool,
}
impl TextField {
    pub fn new(state: &Entity<InputState>, label: impl Into<SharedString>) -> Self {
        let label = label.into();
        assert!(
            !label.trim().is_empty(),
            "controls require an accessible name"
        );
        Self {
            state: state.clone(),
            scroll_parent: None,
            label,
            disabled: false,
            readonly: false,
            search_chrome: false,
        }
    }
    pub fn with_scroll_parent(mut self, parent: &ScrollHandle) -> Self {
        self.scroll_parent = Some(parent.clone());
        self
    }
    pub fn search_chrome(mut self) -> Self {
        self.search_chrome = true;
        self
    }
    pub fn disabled(mut self, value: bool) -> Self {
        self.disabled = value;
        self
    }
    pub fn readonly(mut self, value: bool) -> Self {
        self.readonly = value;
        self
    }
}
impl RenderOnce for TextField {
    fn render(self, w: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = Palette::get(cx);
        let focus = self.state.read(cx).focus_handle(cx);
        let reveal = crate::data_components::focus_reveal(focus, self.scroll_parent, None, w, cx);
        div()
            .relative()
            .child(reveal)
            .child(
                Input::new(&self.state)
                    .aria_label(self.label)
                    .placeholder_color(p.muted)
                    .disabled(self.disabled)
                    .readonly(self.readonly)
                    .h(if self.search_chrome {
                        tokens::STANDARD_HEIGHT
                    } else {
                        tokens::INPUT_HEIGHT
                    })
                    .px(tokens::CONTROL_INSET)
                    .when(self.search_chrome, |s| s.pl(px(28.)).pr(px(28.)))
                    .rounded(if self.search_chrome {
                        px(999.)
                    } else {
                        tokens::CONTROL_RADIUS
                    })
                    .text_size(tokens::BODY_SIZE)
                    .line_height(tokens::BODY_LEADING)
                    .bg(p.surface)
                    .border_1()
                    .border_color(if self.search_chrome {
                        p.hairline_strong
                    } else {
                        p.border
                    })
                    .when(self.disabled, |s| s.opacity(tokens::DISABLED_OPACITY)),
            )
            .when(self.search_chrome, |d| {
                d.child(
                    div().absolute().left(px(10.)).top(px(8.)).child(
                        crate::icons::icon(crate::icons::Glyph::Search)
                            .size(px(16.))
                            .text_color(p.muted),
                    ),
                )
            })
    }
}
