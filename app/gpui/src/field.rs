use crate::tokens::{self, Palette};
use gpui::{prelude::*, *};
use gpui_component::input::{Input, InputState};

pub use gpui_component::input::{InputEvent, InputState as TextFieldState};

/// A real component editor (clipboard, undo, composition and selection), with
/// one focus owner and mandatory accessible name. No shadow editor state.
#[derive(IntoElement)]
pub struct TextField {
    state: Entity<InputState>,
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
            label,
            disabled: false,
            readonly: false,
        }
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
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = Palette::get(cx);
        Input::new(&self.state)
            .aria_label(self.label)
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
            .when(self.disabled, |s| s.opacity(tokens::DISABLED_OPACITY))
    }
}
