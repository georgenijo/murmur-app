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
        }
    }
    pub fn with_scroll_parent(mut self, parent: &ScrollHandle) -> Self {
        self.scroll_parent = Some(parent.clone());
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
        let reveal = crate::data_components::focus_reveal(focus, self.scroll_parent, w, cx);
        div().relative().child(reveal).child(
            Input::new(&self.state)
                .aria_label(self.label)
                .placeholder_color(p.muted)
                .disabled(self.disabled)
                .readonly(self.readonly)
                .h(tokens::INPUT_HEIGHT)
                .px(tokens::CONTROL_INSET)
                .rounded(tokens::CONTROL_RADIUS)
                .text_size(tokens::BODY_SIZE)
                .line_height(tokens::BODY_LEADING)
                .bg(p.surface)
                .border_1()
                .border_color(p.border)
                .when(self.disabled, |s| s.opacity(tokens::DISABLED_OPACITY)),
        )
    }
}
