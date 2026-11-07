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
                c.kind = match i {
                    0 => ChartKind::Heatmap,
                    2 => ChartKind::Line,
                    _ => ChartKind::Bars,
                }
            });
        }
        Self {
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
        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(24.))
            .child(
                Button::new("insights-back", "Back to Home")
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
                    .text_size(px(20.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Insights"),
            )
            .child("Your usage patterns, computed on this Mac and never uploaded");
        if let Some(problem) = self.controller.model.problem {
            let e = cx.entity();
            body = body
                .child(
                    div()
                        .id("section-alert")
                        .role(Role::Alert)
                        .child(problem.message()),
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
                .flex()
                .flex_wrap()
                .gap(px(12.));
            for (label, value, detail) in snapshot.totals() {
                totals = totals.child(
                    div()
                        .w(px(230.))
                        .p(px(12.))
                        .border_1()
                        .border_color(p.border)
                        .rounded(tokens::CARD_RADIUS)
                        .child(div().text_color(p.muted).child(label))
                        .child(div().text_size(px(24.)).child(value))
                        .child(div().text_size(px(12.)).child(detail)),
                );
            }
            body = body.child(totals);
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
                body = body.child(
                    div()
                        .child(
                            [
                                "Activity · last 8 weeks",
                                "Words per day · last 7 days",
                                "WPM trend · last 7 days",
                            ][i],
                        )
                        .child(self.charts[i].clone()),
                );
            }
        } else {
            body = body.child("Loading insights…");
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
            .child(body)
    }
}
