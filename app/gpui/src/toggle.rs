use crate::tokens::{self, Palette};
use gpui::{prelude::*, *};

/// Controlled switch: the owner receives the requested value exactly once.
#[derive(IntoElement)]
pub struct Toggle {
    base: gpui_base::Switch,
    id: ElementId,
    checked: bool,
    disabled: bool,
    palette: Option<Palette>,
    focus: Option<FocusHandle>,
    scroll_parent: Option<ScrollHandle>,
}
impl Toggle {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>, checked: bool) -> Self {
        let label = label.into();
        assert!(
            !label.trim().is_empty(),
            "controls require an accessible name"
        );
        let id = id.into();
        Self {
            id: id.clone(),
            base: gpui_base::Switch::new(id)
                .accessibility_label(label)
                .checked(checked),
            checked,
            disabled: false,
            palette: None,
            focus: None,
            scroll_parent: None,
        }
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
    pub fn track_focus(mut self, f: &FocusHandle) -> Self {
        self.focus = Some(f.clone());
        self.base = self.base.track_focus(f);
        self
    }
    pub fn with_scroll_parent(mut self, parent: &ScrollHandle) -> Self {
        self.scroll_parent = Some(parent.clone());
        self
    }
    pub fn on_change(mut self, f: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.base = self.base.on_change(move |v, _, w, c| f(&v, w, c));
        self
    }
}
impl RenderOnce for Toggle {
    fn render(mut self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let reveal = if let Some(parent) = self.scroll_parent.take() {
            let focus = self.focus.clone().unwrap_or_else(|| {
                window
                    .use_keyed_state(self.id.clone(), cx, |_, cx| cx.focus_handle())
                    .read(cx)
                    .clone()
            });
            self.base = self.base.track_focus(&focus);
            Some(crate::data_components::focus_reveal(
                focus,
                Some(parent),
                None,
                window,
                cx,
            ))
        } else {
            None
        };
        let p = self.palette.unwrap_or_else(|| Palette::get(cx));
        let policy = crate::motion::Motion::Toggle.resolve(cx.reduce_motion());
        let position = gpui_base::transition(
            (self.id, "thumb"),
            px(if self.checked { 13. } else { 1. }),
            gpui_base::Transition::new(policy.duration),
            window,
            cx,
        );
        self.base
            .relative()
            .children(reveal)
            .disabled(self.disabled)
            .flex()
            .items_center()
            .w(tokens::SWITCH_WIDTH)
            .h(tokens::SWITCH_HEIGHT)
            .rounded_full()
            .border_1()
            .border_color(p.border)
            .bg(if self.checked { p.accent } else { p.raised })
            .when(self.disabled, |s| s.opacity(tokens::DISABLED_OPACITY))
            .focus_visible(|s| {
                s.border_color(p.focus).shadow(vec![BoxShadow {
                    inset: false,
                    color: p.focus.opacity(0.24),
                    offset: point(px(0.), px(0.)),
                    blur_radius: px(0.),
                    spread_radius: tokens::FOCUS_WIDTH,
                }])
            })
            .child(
                div()
                    .ml(position)
                    .size(tokens::SWITCH_THUMB)
                    .rounded_full()
                    .bg(p.surface),
            )
    }
}
