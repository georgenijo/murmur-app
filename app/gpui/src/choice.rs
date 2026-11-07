use crate::tokens::{self, Palette};
use gpui::{prelude::*, *};
use gpui_base::ElementExt as _;

#[derive(Clone, Debug)]
pub struct Choice {
    pub value: SharedString,
    pub label: SharedString,
    pub disabled: bool,
}
impl Choice {
    pub fn new(value: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        let label = label.into();
        assert!(
            !label.trim().is_empty(),
            "choices require an accessible name"
        );
        Self {
            value: value.into(),
            label,
            disabled: false,
        }
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChoiceKind {
    Dropdown,
    List,
    Navigation,
}
#[derive(Clone, Debug)]
pub struct SelectionChanged(pub SharedString);

/// A single-selection composite with one Tab stop, roving AX active descendant,
/// explicit selection (Space/Return), wrapping arrows, Home/End, and Escape.
/// The owner subscribes to SelectionChanged; labels and stable values are separate.
pub struct Choices {
    pub(crate) focus: FocusHandle,
    label: SharedString,
    kind: ChoiceKind,
    choices: Vec<Choice>,
    selected: Option<usize>,
    active: Option<usize>,
    open: bool,
    disabled: bool,
    scroll: ScrollHandle,
    _blur: Subscription,
    popup_bounds: std::rc::Rc<std::cell::Cell<Option<Bounds<Pixels>>>>,
}
impl EventEmitter<SelectionChanged> for Choices {}
impl Focusable for Choices {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Choices {
    pub fn new(
        label: impl Into<SharedString>,
        kind: ChoiceKind,
        choices: Vec<Choice>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let label = label.into();
        assert!(
            !label.trim().is_empty(),
            "controls require an accessible name"
        );
        let active = choices.iter().position(|c| !c.disabled);
        let focus = cx.focus_handle();
        let blur = cx.on_blur(&focus, window, |s, _, cx| {
            if s.open {
                s.open = false;
                s.active = s.selected;
                cx.notify();
            }
        });
        Self {
            popup_bounds: Default::default(),
            focus,
            _blur: blur,
            label,
            kind,
            choices,
            selected: None,
            active,
            open: false,
            disabled: false,
            scroll: ScrollHandle::new(),
        }
    }
    pub fn value(&self) -> Option<&SharedString> {
        self.selected.map(|i| &self.choices[i].value)
    }
    pub fn is_open(&self) -> bool {
        self.open
    }
    pub fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.kind == ChoiceKind::Dropdown && !self.disabled {
            self.open = true;
            window.focus(&self.focus, cx);
            cx.notify();
        }
    }
    pub fn set_disabled(&mut self, disabled: bool, cx: &mut Context<Self>) {
        self.disabled = disabled;
        if disabled {
            self.open = false;
        }
        cx.notify();
    }
    /// Set an existing enabled value without emitting an event.
    pub fn set_value(&mut self, value: &str, cx: &mut Context<Self>) {
        self.selected = self
            .choices
            .iter()
            .position(|c| c.value == value && !c.disabled);
        self.active = self
            .selected
            .or_else(|| self.choices.iter().position(|c| !c.disabled));
        cx.notify();
    }
    fn activate(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled || self.choices.get(index).is_none_or(|c| c.disabled) {
            return;
        }
        self.selected = Some(index);
        self.active = Some(index);
        self.open = false;
        window.focus(&self.focus, cx);
        crate::events::publish(cx, SelectionChanged(self.choices[index].value.clone()));
        cx.notify();
    }
    fn move_active(&mut self, backwards: bool) {
        let enabled: Vec<_> = self
            .choices
            .iter()
            .enumerate()
            .filter_map(|(i, c)| (!c.disabled).then_some(i))
            .collect();
        if enabled.is_empty() {
            self.active = None;
            return;
        }
        let position = enabled.iter().position(|i| Some(*i) == self.active);
        let next = match position {
            Some(i) if backwards => (i + enabled.len() - 1) % enabled.len(),
            Some(i) => (i + 1) % enabled.len(),
            None if backwards => enabled.len() - 1,
            None => 0,
        };
        self.active = Some(enabled[next]);
    }
    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled
            || event.prefer_character_input
            || event.keystroke.modifiers.number_of_modifiers() != 0
        {
            return;
        }
        let popup = self.kind == ChoiceKind::Dropdown;
        match event.keystroke.key.as_str() {
            "tab" => {
                self.open = false;
                cx.notify();
                return;
            }
            "escape" if popup && self.open => {
                self.open = false;
                self.active = self.selected;
            }
            "space" | "enter" if !event.is_held => {
                if popup && !self.open {
                    self.open = true;
                } else if let Some(i) = self.active {
                    self.activate(i, window, cx);
                }
            }
            "down" | "up" | "left" | "right" => {
                if popup && !self.open {
                    self.open = true;
                    if event.keystroke.key == "up" {
                        self.active = self.choices.iter().rposition(|c| !c.disabled);
                    }
                } else {
                    self.move_active(matches!(event.keystroke.key.as_str(), "up" | "left"));
                }
            }
            "home" => self.active = self.choices.iter().position(|c| !c.disabled),
            "end" => self.active = self.choices.iter().rposition(|c| !c.disabled),
            _ => return,
        }
        if let Some(i) = self.active {
            self.scroll.scroll_to_item(i);
        }
        window.prevent_default();
        cx.stop_propagation();
        cx.notify();
    }
}
impl Render for Choices {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Also close during a focus-triggered redraw: the pinned engine can
        // coalesce a Root Tab action and its blur notification into one frame.
        if self.open && !self.focus.is_focused(window) {
            self.open = false;
            self.active = self.selected;
        }
        let p = Palette::get(cx);
        let popup = self.kind == ChoiceKind::Dropdown;
        let visible = !popup || self.open;
        let disabled = self.disabled;
        let role = match self.kind {
            ChoiceKind::Dropdown => Role::ComboBox,
            ChoiceKind::List => Role::ListBox,
            ChoiceKind::Navigation => Role::TabList,
        };
        let mut root = div()
            .id("choices")
            .relative()
            .w(tokens::SELECT_MAX)
            .role(role)
            .when(popup, |s| s.h(tokens::STANDARD_HEIGHT))
            .aria_label(self.label.clone())
            .aria_value(
                self.selected
                    .map(|i| self.choices[i].label.clone())
                    .unwrap_or_default(),
            )
            .when(popup, |s| s.aria_expanded(self.open))
            .a11y_synthetic_children(move |b| {
                if disabled {
                    b.parent_node().set_disabled();
                }
            })
            .when(!disabled || self.focus.is_focused(window), |s| {
                s.track_focus(&self.focus.clone().tab_stop(!disabled))
            })
            .when(disabled, |s| s.opacity(tokens::DISABLED_OPACITY))
            .text_size(tokens::BODY_SIZE)
            .line_height(tokens::BODY_LEADING)
            .text_color(p.text)
            .bg(p.surface)
            .rounded(tokens::CONTROL_RADIUS)
            .border_1()
            .border_color(p.border)
            .focus_visible(|s| {
                s.border_color(p.focus).shadow(vec![BoxShadow {
                    inset: false,
                    color: p.focus.opacity(0.24),
                    offset: point(px(0.), px(0.)),
                    blur_radius: px(0.),
                    spread_radius: tokens::FOCUS_WIDTH,
                }])
            })
            .capture_key_down(cx.listener(Self::key));
        let mut content = None;
        if visible {
            let mut list = div()
                .id("options")
                .max_h(px(240.))
                .overflow_y_scroll()
                .track_scroll(&self.scroll)
                .flex()
                .flex_col()
                .gap(tokens::LIST_GAP)
                .p(px(4.))
                .when(popup, |s| {
                    s.role(Role::ListBox)
                        .aria_label(self.label.clone())
                        .w(tokens::SELECT_MAX)
                        .rounded(tokens::CONTROL_RADIUS)
                        .border_1()
                        .border_color(p.border)
                        .bg(p.popover(cx))
                });
            for (i, choice) in self.choices.iter().enumerate() {
                let row_disabled = disabled || choice.disabled;
                let row = div()
                    .id(i)
                    .role(if self.kind == ChoiceKind::Navigation {
                        Role::Tab
                    } else {
                        Role::ListBoxOption
                    })
                    .aria_label(choice.label.clone())
                    .aria_selected(self.selected == Some(i))
                    .when(self.active == Some(i) && !row_disabled, |s| {
                        s.aria_active_descendant()
                    })
                    .a11y_synthetic_children(move |b| {
                        if row_disabled {
                            b.parent_node().set_disabled();
                        }
                    })
                    .h(tokens::STANDARD_HEIGHT)
                    .px(px(10.))
                    .rounded(if popup {
                        tokens::MENU_RADIUS
                    } else {
                        tokens::CONTROL_RADIUS
                    })
                    .flex()
                    .items_center()
                    .font_weight(FontWeight::MEDIUM)
                    .when(self.selected == Some(i), |s| s.bg(p.raised))
                    .when(
                        self.active == Some(i) && self.focus.is_focused(window),
                        |s| s.border_1().border_color(p.focus),
                    )
                    .when(choice.disabled && !disabled, |s| {
                        s.opacity(tokens::DISABLED_OPACITY)
                    })
                    .when(!row_disabled, |s| {
                        s.hover(|s| s.bg(p.raised))
                            .on_click(cx.listener(move |s, _, w, cx| {
                                cx.stop_propagation();
                                s.activate(i, w, cx);
                            }))
                            .on_a11y_action(AccessibleAction::Click, {
                                let entity = cx.entity();
                                move |_, w, cx| entity.update(cx, |s, cx| s.activate(i, w, cx))
                            })
                    })
                    .child(choice.label.clone());
                list = list.child(row);
            }
            let bounds = self.popup_bounds.clone();
            list = list.on_prepaint(move |rect, _, _| bounds.set(Some(rect)));
            content = Some(list);
        }
        if popup {
            let trigger = div()
                .h(tokens::STANDARD_HEIGHT)
                .px(tokens::CONTROL_INSET)
                .flex()
                .items_center()
                .justify_between()
                .child(
                    self.selected
                        .map(|i| self.choices[i].label.clone())
                        .unwrap_or_else(|| "Choose…".into()),
                )
                .child("⌄");
            root = root
                .child(
                    gpui_base::Popup::new("picker-position", trigger)
                        .offset(px(4.))
                        .when_some(content, |s, content| s.content(content)),
                )
                .when(!disabled, |s| {
                    s.on_click(cx.listener(|s, _, w, cx| {
                        s.open = !s.open;
                        w.focus(&s.focus, cx);
                        cx.notify();
                    }))
                })
                .when(!disabled, |s| {
                    s.on_a11y_action(AccessibleAction::Click, {
                        let entity = cx.entity();
                        move |_, w, cx| {
                            entity.update(cx, |s, cx| {
                                s.open = !s.open;
                                w.focus(&s.focus, cx);
                                cx.notify();
                            })
                        }
                    })
                })
                .when(self.open, |s| {
                    s.on_mouse_down_out(cx.listener(|s, event: &MouseDownEvent, _, cx| {
                        if !s
                            .popup_bounds
                            .get()
                            .is_some_and(|b| b.contains(&event.position))
                        {
                            s.open = false;
                            cx.notify();
                        }
                    }))
                });
        } else {
            root = root.children(content);
        }
        root
    }
}
