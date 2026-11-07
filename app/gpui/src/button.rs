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
    scroll_parent: Vec<ScrollHandle>,
    label: SharedString,
    intent: ButtonIntent,
    size: ButtonSize,
    disabled: bool,
    palette: Option<Palette>,
    default_confirm: bool,
    selected: bool,
    icon: Option<crate::icons::Glyph>,
    icon_only: bool,
    pill: bool,
    nav: bool,
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
            scroll_parent: vec![],
            label,
            intent: ButtonIntent::Secondary,
            size: ButtonSize::Standard,
            disabled: false,
            palette: None,
            default_confirm: false,
            selected: false,
            icon: None,
            icon_only: false,
            pill: false,
            nav: false,
        }
    }
    /// Change visible copy while retaining the constructor's accessible name.
    pub fn caption(mut self, caption: impl Into<SharedString>) -> Self {
        self.label = caption.into();
        self
    }
    /// Persistent filter state, exposed independently from its visible caption.
    pub fn toggled(mut self, value: bool) -> Self {
        self.selected = value;
        self.base = self
            .base
            .aria_toggled(if value { Toggled::True } else { Toggled::False });
        self
    }
    pub fn icon(mut self, glyph: crate::icons::Glyph) -> Self {
        self.icon = Some(glyph);
        self
    }
    pub fn icon_only(mut self, glyph: crate::icons::Glyph) -> Self {
        self.icon = Some(glyph);
        self.icon_only = true;
        self
    }
    pub fn pill(mut self) -> Self {
        self.pill = true;
        self
    }
    pub fn nav_row(mut self) -> Self {
        self.nav = true;
        self.intent = ButtonIntent::Ghost;
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
        self.scroll_parent = parent.cloned().into_iter().collect();
        self
    }
    /// Reveal through each viewport, ordered from innermost to outermost.
    pub(crate) fn with_scroll_parents(mut self, parents: Vec<ScrollHandle>) -> Self {
        self.scroll_parent = parents;
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
        let reveal = if !self.scroll_parent.is_empty() {
            let focus = self.focus.clone().unwrap_or_else(|| {
                w.use_keyed_state(self.id.clone(), cx, |_, cx| cx.focus_handle())
                    .read(cx)
                    .clone()
            });
            self.base = self.base.track_focus(&focus);
            Some(crate::data_components::focus_reveal(
                focus,
                self.scroll_parent.clone(),
                None,
                w,
                cx,
            ))
        } else {
            None
        };
        let tooltip_label = self.label.clone();
        let p = self.palette.unwrap_or_else(|| Palette::get(cx));
        let (bg, fg) = match self.intent {
            ButtonIntent::Primary => (p.accent, p.on_accent),
            ButtonIntent::Danger => (p.danger, p.on_accent),
            ButtonIntent::Ghost => (gpui::transparent_black(), p.text),
            ButtonIntent::Secondary => (p.tint_raised, p.text),
        };
        let bg = if self.selected {
            if self.nav {
                p.tint_raised
            } else if self.intent == ButtonIntent::Primary {
                p.accent.blend(p.on_accent.opacity(0.08))
            } else {
                p.raised
            }
        } else {
            bg
        };
        let allow_return =
            self.default_confirm && self.intent != ButtonIntent::Danger && !self.disabled;
        self.base
            .relative()
            .when(self.icon_only, |s| {
                s.tooltip(move |w, cx| {
                    gpui_component::tooltip::Tooltip::new(tooltip_label.clone()).build(w, cx)
                })
            })
            .children(reveal)
            .disabled(self.disabled)
            .flex_shrink_0()
            .max_w_full()
            .min_w_0()
            .overflow_hidden()
            .self_start()
            .h(self.size.height())
            .px(if self.nav { px(8.) } else { self.size.inset() })
            .rounded(if self.pill {
                px(999.)
            } else {
                tokens::CONTROL_RADIUS
            })
            .when(self.intent != ButtonIntent::Ghost, |s| {
                s.border_1().border_color(p.hairline_strong)
            })
            .when(self.intent == ButtonIntent::Secondary, |s| {
                s.shadow(p.card_shadow())
            })
            .when(
                matches!(self.intent, ButtonIntent::Primary | ButtonIntent::Danger),
                |s| {
                    s.shadow(vec![BoxShadow {
                        inset: true,
                        color: white().opacity(0.16),
                        offset: point(px(0.), px(1.)),
                        blur_radius: px(0.),
                        spread_radius: px(0.),
                    }])
                },
            )
            .when(self.icon_only, |s| s.w(self.size.height()).px(px(0.)))
            .when(self.nav, |s| s.w_full().text_size(tokens::LABEL_SIZE))
            .bg(bg)
            .text_color(fg)
            .text_size(if self.nav {
                tokens::LABEL_SIZE
            } else {
                match self.size {
                    ButtonSize::Standard => tokens::BODY_SIZE,
                    ButtonSize::Compact => tokens::LABEL_SIZE,
                    ButtonSize::ExtraSmall => tokens::CAPTION_SIZE,
                }
            })
            .line_height(tokens::BODY_LEADING)
            .font_weight(FontWeight::MEDIUM)
            .when(self.disabled, |s| s.opacity(tokens::DISABLED_OPACITY))
            .when(!self.disabled, |s| {
                s.hover(|s| s.bg(bg.blend(fg.opacity(0.05))))
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
            .child(
                div()
                    .flex()
                    .min_w_0()
                    .max_w_full()
                    .items_center()
                    .gap(px(8.))
                    .when(self.nav, |s| s.w_full())
                    .when(self.icon_only, |s| s.justify_center())
                    .when(self.nav && self.selected && !self.icon_only, |s| {
                        s.child(
                            div()
                                .absolute()
                                .left_0()
                                .w(px(3.))
                                .h(px(16.))
                                .rounded(px(2.))
                                .bg(p.accent),
                        )
                    })
                    .when_some(self.icon, |s, glyph| {
                        s.child(crate::icons::icon(glyph).text_color(
                            if self.nav && self.selected {
                                p.accent
                            } else {
                                fg
                            },
                        ))
                    })
                    .when(!self.icon_only, |s| {
                        s.child(div().min_w_0().truncate().child(self.label))
                    }),
            )
    }
}
