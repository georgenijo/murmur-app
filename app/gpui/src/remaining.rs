//! Gallery-only review composition. Synthetic hosts, no production services.
use crate::{Button, ButtonSize, tokens::Palette};
use gpui::{prelude::*, *};
use std::rc::Rc;
pub const SECTIONS: [&str; 6] = [
    "Command palette",
    "Insights",
    "Models",
    "Knowledge",
    "Modes",
    "Themes",
];
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub enum Scenario {
    #[default]
    Ready,
    PartialBulk,
    StaleImport,
    ConflictingImport,
}
#[derive(Clone, Copy, Default, Debug)]
pub struct QaOptions {
    pub knowledge: Scenario,
    pub spellings: Scenario,
    pub mode_files: bool,
    pub normal_motion: bool,
}
impl QaOptions {
    pub fn preferences(self, increase_contrast: bool) -> crate::preferences::DisplayPreferences {
        crate::preferences::DisplayPreferences {
            reduce_motion: !self.normal_motion,
            reduce_transparency: true,
            increase_contrast,
        }
    }
    /// Parsing lives only in the gallery/test module; production reads no QA variables.
    pub fn parse(
        knowledge: &str,
        spellings: &str,
        mode_files: &str,
        motion: &str,
    ) -> Result<Self, String> {
        fn scenario(value: &str, bulk: bool) -> Result<Scenario, String> {
            match value {
                "" | "ready" => Ok(Scenario::Ready),
                "partial-bulk" if bulk => Ok(Scenario::PartialBulk),
                "stale-import" => Ok(Scenario::StaleImport),
                "conflicting-import" => Ok(Scenario::ConflictingImport),
                _ => Err(format!("Unknown remaining-qa scenario: {value}")),
            }
        }
        Ok(Self {
            knowledge: scenario(knowledge, true)?,
            spellings: scenario(spellings, false)?,
            mode_files: match mode_files {
                "" | "0" => false,
                "1" => true,
                _ => return Err("MURMUR_MODE_FILES must be 0 or 1".into()),
            },
            normal_motion: match motion {
                "" | "reduced" => false,
                "normal" => true,
                _ => return Err("MURMUR_MOTION must be reduced or normal".into()),
            },
        })
    }
}
pub struct Review {
    home: Entity<crate::home::Home>,
    pub settings: Entity<crate::settings::Settings>,
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
        Self::with_options(QaOptions::default(), w, cx)
    }
    pub fn with_options(options: QaOptions, w: &mut Window, cx: &mut Context<Self>) -> Self {
        let customize = cx.new(|cx| {
            crate::customize::Customize::new(
                Rc::new(crate::customize::fixtures::FixtureHost::with_scenario(
                    options.spellings,
                )),
                w,
                cx,
            )
        });
        let back = cx.subscribe(&customize, |s, _, _: &crate::customize::Back, cx| {
            s.custom_active = false;
            s.knowledge.update(cx, |s, cx| {
                s.attach(cx);
                s.follow(cx)
            });
            cx.notify();
        });
        let review = Self {
            home: cx.new(|cx| {
                crate::home::Home::new(
                    Rc::new(crate::home::fixtures::FixtureHost::default()),
                    Rc::new(crate::history::fixtures::FixtureHost::default()),
                    w,
                    cx,
                )
            }),
            settings: cx.new(|cx| {
                crate::settings::Settings::new(
                    Rc::new(crate::settings::fixtures::FixtureHost::default()),
                    w,
                    cx,
                )
            }),
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
                crate::knowledge::Knowledge::new(
                    Rc::new(crate::knowledge::fixtures::FixtureHost::with_scenario(
                        options.knowledge,
                    )),
                    w,
                    cx,
                )
            }),
            modes: cx.new(|cx| {
                crate::modes::Modes::new(
                    Rc::new({
                        let mut host =
                            crate::modes::fixtures::FixtureHost::with_files(options.mode_files);
                        if std::env::var("MURMUR_REMAINING_STATE").as_deref() == Ok("sites") {
                            host.seed_sites();
                        }
                        host
                    }),
                    w,
                    cx,
                )
            }),
            themes: cx.new(|cx| crate::themes::Themes::new(crate::themes::fixtures::host(), w, cx)),
            selected: usize::MAX,
        };
        review.settings.update(cx, |s, cx| {
            s.embed(
                crate::settings::schema::Page::Appearance,
                review.themes.clone(),
            );
            s.embed(crate::settings::schema::Page::Modes, review.modes.clone());
            s.attach(cx);
            s.follow(cx);
        });
        review.themes.update(cx, |s, _| s.embedded = true);
        review.modes.update(cx, |s, _| s.embedded = true);
        review
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
        if index == 1 && std::env::var_os("MURMUR_QA_HIDE_TABS").is_some() {
            self.home.update(cx, |s, cx| {
                s.workspace = Some((
                    crate::home::model::Destination::Insights,
                    self.insights.clone().into(),
                ));
                s.attach(cx);
                s.drain(cx);
            });
            self.insights.update(cx, |s, _| s.embedded = true);
        }
        if index == 4 || index == 5 {
            self.settings.update(cx, |s, cx| {
                s.navigate(
                    if index == 4 {
                        crate::settings::schema::Page::Modes
                    } else {
                        crate::settings::schema::Page::Appearance
                    },
                    None,
                    w,
                    cx,
                )
            });
        }
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
                1 => {
                    if std::env::var_os("MURMUR_QA_HIDE_TABS").is_some() {
                        self.home.clone().into_any_element()
                    } else {
                        self.insights.clone().into_any_element()
                    }
                }
                2 => self.models.clone().into_any_element(),
                3 => self.knowledge.clone().into_any_element(),
                4 | 5 => self.settings.clone().into_any_element(),
                _ => self.themes.clone().into_any_element(),
            }
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(p.surface)
            .text_color(p.text)
            .when(std::env::var_os("MURMUR_QA_HIDE_TABS").is_none(), |d| {
                d.child(nav)
            })
            .child(div().flex_1().min_h(px(0.)).child(child))
    }
}
