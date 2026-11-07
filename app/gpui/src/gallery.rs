//! Synthetic gallery shared by the demo, interaction tests and Metal goldens.
use crate::{
    tokens::{self, Palette},
    *,
};
use gpui::{prelude::*, *};
use gpui_component::input::InputState;

pub struct Gallery {
    fields: Vec<Entity<InputState>>,
    selections: Vec<Entity<Choices>>,
    checked: bool,
    off_checked: bool,
    action_count: usize,
    button_focus: FocusHandle,
    toggle_focus: FocusHandle,
}
impl Gallery {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let fields = ["", "Synthetic editable text", "Read only", "Disabled"]
            .into_iter()
            .map(|text| {
                cx.new(|cx| {
                    InputState::new(window, cx)
                        .default_value(text)
                        .placeholder("Synthetic placeholder")
                })
            })
            .collect();
        let selections = [
            ChoiceKind::Dropdown,
            ChoiceKind::List,
            ChoiceKind::Navigation,
        ]
        .into_iter()
        .map(|kind| {
            cx.new(|cx| {
                let mut choices = Choices::new(
                    match kind {
                        ChoiceKind::Dropdown => "Language",
                        ChoiceKind::List => "History examples",
                        ChoiceKind::Navigation => "Sections",
                    },
                    kind,
                    vec![
                        Choice::new("one", "First example"),
                        Choice::new("two", "Second example"),
                        Choice::new("disabled", "Unavailable").disabled(true),
                    ],
                    window,
                    cx,
                );
                choices.set_value("one", cx);
                choices
            })
        })
        .collect();
        Self {
            fields,
            selections,
            checked: true,
            off_checked: false,
            action_count: 0,
            button_focus: cx.focus_handle(),
            toggle_focus: cx.focus_handle(),
        }
    }
    pub fn prepare_snapshot(&mut self, state: &str, window: &mut Window, cx: &mut Context<Self>) {
        let focus = match state {
            "button" => Some(self.button_focus.clone()),
            "toggle" => Some(self.toggle_focus.clone()),
            "field" => Some(self.fields[1].read(cx).focus_handle(cx)),
            "dropdown" | "popup" => Some(self.selections[0].read(cx).focus_handle(cx)),
            "list" => Some(self.selections[1].read(cx).focus_handle(cx)),
            "navigation" => Some(self.selections[2].read(cx).focus_handle(cx)),
            _ => None,
        };
        if let Some(focus) = focus {
            window.focus(&focus, cx);
        }
        if state == "popup" {
            self.selections[0].update(cx, |s, cx| s.open(window, cx));
        }
        cx.notify();
    }
}
impl Render for Gallery {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::get(cx);
        let mut buttons =
            div().flex().flex_col().gap(px(8.)).child(
                "Buttons: primary, secondary, ghost, danger; standard, compact, extra small",
            );
        for (row, size) in [
            ButtonSize::Standard,
            ButtonSize::Compact,
            ButtonSize::ExtraSmall,
        ]
        .into_iter()
        .enumerate()
        {
            let mut line = div().flex().gap(px(8.));
            for (col, intent) in [
                ButtonIntent::Primary,
                ButtonIntent::Secondary,
                ButtonIntent::Ghost,
                ButtonIntent::Danger,
            ]
            .into_iter()
            .enumerate()
            {
                for disabled in [false, true] {
                    line = line.child(
                        Button::new(
                            ("button", row * 8 + col * 2 + usize::from(disabled)),
                            if disabled { "Disabled" } else { "Action" },
                        )
                        .intent(intent)
                        .size(size)
                        .disabled(disabled)
                        .on_click(cx.listener(|s, _, _, cx| {
                            s.action_count += 1;
                            cx.notify();
                        }))
                        .when(row == 0 && col == 0 && !disabled, |s| {
                            s.track_focus(&self.button_focus)
                        }),
                    );
                }
            }
            buttons = buttons.child(line);
        }
        let mut fields = div().flex().gap(px(16.));
        for (i, field) in self.fields.iter().enumerate() {
            fields = fields.child(
                div().w(px(210.)).child(
                    TextField::new(field, format!("Example field {}", i + 1))
                        .readonly(i == 2)
                        .disabled(i == 3),
                ),
            );
        }
        div().id("gallery").tab_group().size_full().bg(p.surface).text_color(p.text)
            .font_family(".SystemUIFont").text_size(tokens::BODY_SIZE).line_height(tokens::BODY_LEADING)
            .p(px(24.)).flex().flex_col().gap(px(24.))
            .child(format!("Murmur · Accessible controls · Synthetic data · Actions: {}", self.action_count))
            .child(buttons)
            .child(div().flex().gap(px(24.)).items_center().child("Switches")
                .child(Toggle::new("off", "Off switch", self.off_checked).on_change(cx.listener(|s, next, _, cx| { s.off_checked = *next; cx.notify(); })))
                .child(Toggle::new("on", "On switch", self.checked).track_focus(&self.toggle_focus).on_change(cx.listener(|s, next, _, cx| { s.checked = *next; cx.notify(); })))
                .child(Toggle::new("disabled-off", "Disabled off switch", false).disabled(true))
                .child(Toggle::new("disabled-on", "Disabled on switch", true).disabled(true)))
            .child(fields)
            .child(div().flex().gap(px(16.)).children(self.selections.clone()))
            .child("Focus: Tab / Shift-Tab · Select: arrows, Home, End, Space · Dropdown: Return / Escape")
    }
}

/// Imported Midnight fixture: semantic roles only, no appearance persistence.
/// Used by review hosts alongside Sonic Light/Dark and Increase Contrast.
pub fn apply_midnight(cx: &mut gpui::App) {
    crate::tokens::apply_appearance(true, cx);
    gpui_component::Theme::update(cx, |theme| {
        let c = &mut theme.colors;
        c.background = gpui::rgb(0x000000).into();
        c.secondary = gpui::rgb(0x10121a).into();
        c.foreground = gpui::rgb(0xf5f5f5).into();
        c.muted_foreground = gpui::rgb(0xb7b8d0).into();
        c.border = gpui::rgb(0x53566e).into();
        c.input = c.border;
        c.primary = gpui::rgb(0x74a9d8).into();
        c.primary_foreground = gpui::rgb(0x140d2a).into();
        c.ring = c.primary;
        c.popover = c.secondary;
        c.popover_foreground = c.foreground;
    });
}
