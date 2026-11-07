//! Gallery-only review composition. Synthetic hosts, no production services.
use crate::{Button, ButtonSize, tokens::Palette};
use gpui::{prelude::*, *};
pub const SECTIONS: [&str; 6] = [
    "Command palette",
    "Insights",
    "Models",
    "Knowledge",
    "Modes",
    "Themes",
];
pub struct Review {
    pub customize: Entity<crate::customize::Customize>,
    pub custom_active: bool,
    _custom_back: Subscription,
    pub palette: Entity<crate::palette::CommandPalette>,
    pub insights: Entity<crate::insights::Insights>,
    pub models: Entity<crate::models::Models>,
    pub knowledge: Entity<crate::knowledge::Knowledge>,
    pub modes: Entity<crate::modes::Modes>,
    pub themes: Entity<crate::themes::Themes>,
    pub selected: usize,
}
impl Review {
    pub fn new(w: &mut Window, cx: &mut Context<Self>) -> Self {
        let customize = cx
            .new(|cx| crate::customize::Customize::new(crate::customize::fixtures::host(), w, cx));
        let back = cx.subscribe(&customize, |s, _, _: &crate::customize::Back, cx| {
            s.custom_active = false;
            s.knowledge.update(cx, |s, cx| {
                s.attach(cx);
                s.follow(cx)
            });
            cx.notify();
        });
        Self {
            customize,
            custom_active: false,
            _custom_back: back,
            palette: cx.new(|cx| {
                crate::palette::CommandPalette::new(crate::palette::fixtures::host(), w, cx)
            }),
            insights: cx
                .new(|cx| crate::insights::Insights::new(crate::insights::fixtures::host(), w, cx)),
            models: cx.new(|cx| crate::models::Models::new(crate::models::fixtures::host(), w, cx)),
            knowledge: cx.new(|cx| {
                crate::knowledge::Knowledge::new(crate::knowledge::fixtures::host(), w, cx)
            }),
            modes: cx.new(|cx| crate::modes::Modes::new(crate::modes::fixtures::host(), w, cx)),
            themes: cx.new(|cx| crate::themes::Themes::new(crate::themes::fixtures::host(), w, cx)),
            selected: usize::MAX,
        }
    }
    pub fn select(&mut self, index: usize, w: &mut Window, cx: &mut Context<Self>) {
        if self.custom_active {
            self.customize.update(cx, |s, cx| s.hide(cx));
            self.custom_active = false;
            if self.selected == index {
                self.knowledge.update(cx, |s, cx| {
                    s.attach(cx);
                    s.follow(cx)
                });
                cx.notify();
                return;
            }
        }
        if index >= 6 || self.selected == index {
            return;
        }
        match self.selected {
            0 => self.palette.update(cx, |s, cx| s.hide(w, cx)),
            1 => self.insights.update(cx, |s, cx| s.hide(cx)),
            2 => self.models.update(cx, |s, cx| s.hide(cx)),
            3 => self.knowledge.update(cx, |s, cx| s.hide(cx)),
            4 => self.modes.update(cx, |s, cx| s.hide(cx)),
            5 => self.themes.update(cx, |s, cx| s.hide(cx)),
            _ => (),
        };
        self.selected = index;
        match index {
            0 => self.palette.update(cx, |s, cx| {
                s.attach(cx);
                s.show(w, cx);
                s.follow(cx)
            }),
            1 => self.insights.update(cx, |s, cx| {
                s.attach(cx);
                s.follow(cx)
            }),
            2 => self.models.update(cx, |s, cx| {
                s.attach(cx);
                s.follow(cx)
            }),
            3 => self.knowledge.update(cx, |s, cx| {
                s.attach(cx);
                s.follow(cx)
            }),
            4 => self.modes.update(cx, |s, cx| {
                s.attach(cx);
                s.follow(cx)
            }),
            _ => self.themes.update(cx, |s, cx| {
                s.attach(cx);
                s.follow(cx)
            }),
        };
        cx.notify();
    }
    pub fn select_customize(
        &mut self,
        page: crate::customize::model::Page,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected != 3 {
            self.select(3, w, cx)
        }
        self.knowledge.update(cx, |s, cx| s.hide(cx));
        self.custom_active = true;
        self.customize.update(cx, |s, cx| {
            s.return_to = w.focused(cx);
            s.set_page(page, w, cx);
            s.follow(cx)
        });
        cx.notify();
    }
    pub fn fixture_state(&mut self, state: &str, w: &mut Window, cx: &mut Context<Self>) {
        if self.custom_active {
            self.customize
                .update(cx, |s, cx| s.fixture_state(state, w, cx));
            return;
        }
        match self.selected {
            0 => self
                .palette
                .update(cx, |s, cx| s.fixture_state(state, w, cx)),
            1 => self
                .insights
                .update(cx, |s, cx| s.fixture_state(state, w, cx)),
            2 => self
                .models
                .update(cx, |s, cx| s.fixture_state(state, w, cx)),
            3 => self
                .knowledge
                .update(cx, |s, cx| s.fixture_state(state, w, cx)),
            4 => self.modes.update(cx, |s, cx| s.fixture_state(state, w, cx)),
            _ => self
                .themes
                .update(cx, |s, cx| s.fixture_state(state, w, cx)),
        };
        cx.notify();
    }
}
impl Render for Review {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::get(cx);
        let mut nav = div()
            .id("remaining-navigation")
            .role(Role::Navigation)
            .aria_label("Remaining sections")
            .flex()
            .flex_wrap()
            .gap(px(6.))
            .p(px(8.));
        for (i, label) in SECTIONS.iter().enumerate() {
            let e = cx.entity();
            nav = nav.child(
                Button::new(
                    SharedString::from(format!("review-{i}")),
                    format!("Review {label}"),
                )
                .caption(*label)
                .size(ButtonSize::Compact)
                .toggled(i == self.selected)
                .on_click(move |_, w, cx| e.update(cx, |s, cx| s.select(i, w, cx))),
            );
        }
        if self.selected == 3 {
            for page in crate::customize::model::Page::ALL {
                let e = cx.entity();
                nav = nav.child(
                    Button::new(
                        SharedString::from(format!("review-custom-{page:?}")),
                        format!("Review {}", page.label()),
                    )
                    .caption(page.label())
                    .size(ButtonSize::Compact)
                    .on_click(move |_, w, cx| {
                        e.update(cx, |s, cx| s.select_customize(page, w, cx))
                    }),
                );
            }
        }
        let child = if self.custom_active {
            self.customize.clone().into_any_element()
        } else {
            match self.selected {
                0 => self.palette.clone().into_any_element(),
                1 => self.insights.clone().into_any_element(),
                2 => self.models.clone().into_any_element(),
                3 => self.knowledge.clone().into_any_element(),
                4 => self.modes.clone().into_any_element(),
                _ => self.themes.clone().into_any_element(),
            }
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(p.surface)
            .text_color(p.text)
            .child(nav)
            .child(div().flex_1().min_h(px(0.)).child(child))
    }
}
