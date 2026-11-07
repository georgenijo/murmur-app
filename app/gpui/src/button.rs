use crate::tokens::{self, Palette};
use gpui::{prelude::*, *};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonIntent {
    Primary,
    #[default]
    Secondary,
    Ghost,
    Danger,
}
#[derive(Clone, Copy, Debug, Default)]
pub enum ButtonSize {
    #[default]
    Standard,
    Compact,
    ExtraSmall,
}
impl ButtonSize {
    pub fn height(self) -> Pixels {
        match self {
            Self::Standard => tokens::STANDARD_HEIGHT,
            Self::Compact => tokens::COMPACT_HEIGHT,
            Self::ExtraSmall => tokens::XS_HEIGHT,
        }
    }
    fn inset(self) -> Pixels {
        px(match self {
            Self::Standard => 11.,
            Self::Compact => 9.,
            Self::ExtraSmall => 7.,
        })
    }
}

/// A named action. Return is deliberately suppressed: owners opt into a
/// recoverable dialog default explicitly. Never designate destructive defaults.
#[derive(IntoElement)]
pub struct Button {
    base: gpui_base::Button,
    id: ElementId,
    focus: Option<FocusHandle>,
    scroll_parent: Option<ScrollHandle>,
    label: SharedString,
    intent: ButtonIntent,
    size: ButtonSize,
    disabled: bool,
    palette: Option<Palette>,
    default_confirm: bool,
}
impl Button {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        let id = id.into();
        let label = label.into();
        assert!(
            !label.trim().is_empty(),
            "controls require an accessible name"
        );
        Self {
            base: gpui_base::Button::new(id.clone()).accessibility_label(label.clone()),
            id,
            focus: None,
            scroll_parent: None,
            label,
            intent: ButtonIntent::Secondary,
            size: ButtonSize::Standard,
            disabled: false,
            palette: None,
            default_confirm: false,
        }
    }
    /// Change visible copy while retaining the constructor's accessible name.
    pub fn caption(mut self, caption: impl Into<SharedString>) -> Self {
        self.label = caption.into();
        self
    }
    /// Persistent filter state, exposed independently from its visible caption.
    pub fn toggled(mut self, value: bool) -> Self {
        self.base = self
            .base
            .aria_toggled(if value { Toggled::True } else { Toggled::False });
        self
    }
    pub fn intent(mut self, intent: ButtonIntent) -> Self {
        self.intent = intent;
        self
    }
    pub fn size(mut self, size: ButtonSize) -> Self {
        self.size = size;
        self
    }
    /// Window-local roles (utility surfaces do not follow the main theme).
    pub fn palette(mut self, palette: Palette) -> Self {
        self.palette = Some(palette);
        self
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
    /// Only for an enabled, recoverable confirming action inside a dialog.
    pub fn default_confirm(mut self, enabled: bool) -> Self {
        self.default_confirm = enabled;
        self
    }
    pub fn with_scroll_parent(mut self, parent: Option<&ScrollHandle>) -> Self {
        self.scroll_parent = parent.cloned();
        self
    }
    pub fn track_focus(mut self, focus: &FocusHandle) -> Self {
        self.focus = Some(focus.clone());
        self.base = self.base.track_focus(focus);
        self
    }
    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.base = self.base.on_click(f);
        self
    }
}
impl RenderOnce for Button {
    fn render(mut self, w: &mut Window, cx: &mut App) -> impl IntoElement {
        let reveal = if self.scroll_parent.is_some() {
            let focus = self.focus.clone().unwrap_or_else(|| {
                w.use_keyed_state(self.id.clone(), cx, |_, cx| cx.focus_handle())
                    .read(cx)
                    .clone()
            });
            self.base = self.base.track_focus(&focus);
            Some(crate::data_components::focus_reveal(
                focus,
                self.scroll_parent.clone(),
                w,
                cx,
            ))
        } else {
            None
        };
        let p = self.palette.unwrap_or_else(|| Palette::get(cx));
        let (bg, fg) = match self.intent {
            ButtonIntent::Primary => (p.accent, p.on_accent),
            ButtonIntent::Danger => (p.danger, p.on_accent),
            ButtonIntent::Ghost => (p.surface, p.text),
            ButtonIntent::Secondary => (p.raised, p.text),
        };
        let allow_return =
            self.default_confirm && self.intent != ButtonIntent::Danger && !self.disabled;
        self.base
            .relative()
            .children(reveal)
            .disabled(self.disabled)
            .h(self.size.height())
            .px(self.size.inset())
            .rounded(tokens::CONTROL_RADIUS)
            .border_1()
            .border_color(p.border)
            .bg(bg)
            .text_color(fg)
            .text_size(tokens::BODY_SIZE)
            .line_height(tokens::BODY_LEADING)
            .font_weight(FontWeight::MEDIUM)
            .when(self.disabled, |s| s.opacity(tokens::DISABLED_OPACITY))
            .when(!self.disabled, |s| {
                s.hover(|s| s.bg(bg.blend(fg.opacity(0.08))))
                    .active(|s| s.bg(bg.blend(fg.opacity(0.12))))
            })
            .focus_visible(|s| {
                s.border_color(p.focus).shadow(vec![BoxShadow {
                    inset: false,
                    color: p.focus.opacity(0.24),
                    offset: point(px(0.), px(0.)),
                    blur_radius: px(0.),
                    spread_radius: tokens::FOCUS_WIDTH,
                }])
            })
            .capture_key_down(move |e, w, cx| {
                if e.keystroke.key == "enter"
                    && (!allow_return
                        || e.is_held
                        || e.keystroke.modifiers.number_of_modifiers() != 0
                        || e.prefer_character_input)
                {
                    w.prevent_default();
                    cx.stop_propagation();
                }
            })
            .child(self.label)
    }
}
