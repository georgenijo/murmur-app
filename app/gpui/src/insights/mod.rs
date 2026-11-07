#[cfg(any(test, feature = "gallery"))]
pub mod fixtures;
pub mod host;
pub mod model;
#[cfg(test)]
mod tests;
use crate::{
    Button,
    data_components::{Chart, ChartKind, ChartPoint},
    section::{self, Controller},
    tokens::{self, Palette},
};
use gpui::{prelude::*, *};
use host::Action;
use model::*;
use std::rc::Rc;
pub struct Insights {
    pub embedded: bool,
    pub controller: Controller<Snapshot, Action>,
    charts: Vec<Entity<Chart>>,
    scroll: ScrollHandle,
}
impl Insights {
    pub fn new(host: Rc<host::Host>, w: &mut Window, cx: &mut Context<Self>) -> Self {
        let charts: Vec<Entity<Chart>> = [
            ("Words per day heatmap", "words"),
            ("Words per day bar chart", "words"),
            ("Words-per-minute trend line", "wpm"),
        ]
        .into_iter()
        .map(|(l, u)| cx.new(|cx| Chart::new(l, u, w, cx)))
        .collect();
        for (i, chart) in charts.iter().enumerate() {
            chart.update(cx, |c, _| {
                c.heading = Some(
                    [
                        "ACTIVITY · LAST 8 WEEKS",
                        "WORDS PER DAY · LAST 7 DAYS",
                        "WPM TREND · LAST 7 DAYS",
                    ][i]
                        .into(),
                );
                c.kind = match i {
                    0 => ChartKind::Heatmap,
                    2 => ChartKind::Line,
                    _ => ChartKind::Bars,
                }
            });
        }
        Self {
            embedded: false,
            controller: Controller::new(host),
            charts,
            scroll: ScrollHandle::new(),
        }
    }
    pub fn attach(&mut self, cx: &mut Context<Self>) {
        self.controller.attach(Action::Read);
        cx.notify();
    }
    pub fn follow(&self, cx: &mut Context<Self>) {
        section::follow(cx, self.controller.model.attachment, |s| &mut s.controller)
    }
    pub fn hide(&mut self, cx: &mut Context<Self>) {
        self.controller.detach();
        cx.notify();
    }
}
impl Render for Insights {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.controller.drain();
        let p = Palette::get(cx);
        let e = cx.entity();
        let mut body = div().flex().flex_col().gap(px(12.)).child(
            div()
                .flex()
                .items_center()
                .gap(px(16.))
                .child(
                    Button::new("insights-back", "Back to Home")
                        .intent(crate::ButtonIntent::Ghost)
                        .icon(crate::icons::Glyph::ChevronLeft)
                        .size(crate::ButtonSize::Compact)
                        .with_scroll_parent(Some(&self.scroll))
                        .on_click(move |_, _, cx| {
                            e.update(cx, |s, cx| {
                                s.controller.request(Action::Back);
                                cx.notify()
                            })
                        }),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(
                            div()
                                .text_size(px(20.))
                                .font_weight(FontWeight::MEDIUM)
                                .child("Insights"),
                        )
                        .child(
                            div()
                                .text_size(tokens::CAPTION_SIZE)
                                .text_color(p.muted)
                                .child(
                                    "Your usage patterns, computed on this Mac and never uploaded",
                                ),
                        ),
                ),
        );
        if let Some(problem) = self.controller.model.problem {
            let e = cx.entity();
            body = body
                .child(
                    div()
                        .id("section-alert")
                        .role(Role::Alert)
                        .child(gpui::text!(problem.message())),
                )
                .child(
                    Button::new("insights-retry", "Retry")
                        .with_scroll_parent(Some(&self.scroll))
                        .on_click(move |_, _, cx| {
                            e.update(cx, |s, cx| {
                                s.controller.request(Action::Read);
                                cx.notify()
                            })
                        }),
                );
        }
        if let Some(snapshot) = &self.controller.model.snapshot {
            let mut totals = div()
                .id("section-group")
                .role(Role::Group)
                .aria_label("Usage totals")
                .grid()
                .grid_cols(4)
                .gap(px(12.));
            for (label, value, detail) in snapshot.totals() {
                totals = totals.child(
                    div()
                        .id(label)
                        .role(Role::Group)
                        .aria_label(label)
                        .min_w_0()
                        .p(px(16.))
                        .border_1()
                        .border_color(p.hairline_strong)
                        .bg(p.tint_raised)
                        .shadow(p.card_shadow())
                        .rounded(tokens::CARD_RADIUS)
                        .child(
                            div()
                                .text_size(tokens::CAPTION_SIZE)
                                .line_height(tokens::CAPTION_LEADING)
                                .text_color(p.muted)
                                .child(gpui::text!(label)),
                        )
                        .child(
                            div()
                                .text_size(px(24.))
                                .line_height(px(28.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .font_features(tokens::tabular_numbers())
                                .child(gpui::text!(value)),
                        )
                        .child(
                            div()
                                .text_size(tokens::CAPTION_SIZE)
                                .line_height(tokens::CAPTION_LEADING)
                                .text_color(p.muted)
                                .child(gpui::text!(detail)),
                        ),
                );
            }
            body = body.child(totals);
            let mut charts = div()
                .grid()
                .grid_cols(2)
                .gap(px(12.))
                .p(px(12.))
                .border_1()
                .border_color(p.hairline)
                .rounded(tokens::CARD_RADIUS)
                .bg(p.tint_raised);
            for (i, days) in [snapshot.heatmap(), snapshot.recent(7), snapshot.recent(7)]
                .into_iter()
                .enumerate()
            {
                self.charts[i].update(cx, |c, cx| {
                    c.scroll_parent = Some(self.scroll.clone());
                    c.set_points(
                        days.into_iter()
                            .map(|d| ChartPoint {
                                label: d.date.to_string().into(),
                                value: if d.future {
                                    None
                                } else {
                                    Some(if i == 2 { d.wpm } else { d.words as f64 })
                                },
                            })
                            .collect(),
                        cx,
                    )
                });
                charts = charts.child(
                    div()
                        .id(("insights-chart", i))
                        .child(self.charts[i].clone()),
                );
            }
            body = body.child(charts);
        } else {
            body = body.child(gpui::text!("Loading insights…"));
        }
        div()
            .id("insights")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .bg(p.surface)
            .text_color(p.text)
            .text_size(tokens::BODY_SIZE)
            .line_height(tokens::BODY_LEADING)
            .p(px(24.))
            .pt(px(if self.embedded { 16. } else { 56. }))
            .child(body)
    }
}
