#[cfg(any(test, feature = "gallery"))]
pub mod fixtures;
pub mod host;
pub mod model;
#[cfg(test)]
mod tests;
use crate::{
    Button, ButtonIntent,
    section::{self, Controller},
    tokens::{self, Palette},
};
use gpui::{prelude::*, *};
use host::Action;
use model::*;
use std::rc::Rc;
pub struct Models {
    pub controller: Controller<Snapshot, Action>,
    confirm: Option<(String, u64, std::time::Instant)>,
    scroll: ScrollHandle,
}
impl Models {
    pub fn new(host: Rc<host::Host>, _: &mut Window, _: &mut Context<Self>) -> Self {
        Self {
            controller: Controller::new(host),
            confirm: None,
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
        self.confirm = None;
        cx.notify();
    }
    pub fn act(&mut self, a: Action, cx: &mut Context<Self>) {
        if let Action::Cancel { model, request_id } = &a {
            if self.controller.model.snapshot.as_ref().is_some_and(|s| {
                s.attempt.as_ref() == Some(request_id) && s.attempt_model.as_ref() == Some(model)
            }) {
                self.controller.invalidate();
                self.controller.request(a);
                cx.notify();
            }
            return;
        }

        let Some(s) = &self.controller.model.snapshot else {
            return;
        };
        if self.controller.model.pending {
            return;
        }
        match &a {
            Action::Remove(id) => {
                let Some(m) = s.model(id) else { return };
                if !s.can_remove(m) {
                    return;
                }
                let armed = self.confirm.as_ref().is_some_and(|(i, r, t)| {
                    i == id && *r == s.revision && t.elapsed() < std::time::Duration::from_secs(4)
                });
                if !armed {
                    self.confirm = Some((id.clone(), s.revision, std::time::Instant::now()));
                    cx.notify();
                    return;
                }
            }
            Action::Download(id) => {
                if !s.model(id).is_some_and(|m| s.can_download(m)) {
                    return;
                }
            }
            Action::Select { id, .. } => {
                if s.busy || s.attempt.is_some() || !s.model(id).is_some_and(|m| m.supported) {
                    return;
                }
            }
            Action::Read | Action::Cancel { .. } => (),
        };
        self.confirm = None;
        self.controller.request(a);
        cx.notify();
    }
}
impl Render for Models {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.controller.drain();
        let p = Palette::get(cx);
        let mut body=div().flex().flex_col().gap(px(12.)).child(div().text_size(px(20.)).child(gpui::text!("Model storage"))).child(gpui::text!("Remove unused models to free storage. Choose another model before removing the selected one."));
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
                    Button::new("models-retry", "Retry")
                        .with_scroll_parent(Some(&self.scroll))
                        .on_click(move |_, _, cx| e.update(cx, |s, cx| s.act(Action::Read, cx))),
                );
        }
        if let Some(snapshot) = self.controller.model.snapshot.clone() {
            if let Some(h) = &snapshot.hardware {
                body = body.child(gpui::text!(h.clone()));
            }
            for m in &snapshot.catalog {
                let active = m.id == snapshot.selected
                    || !matches!(m.lifecycle, Lifecycle::Unloaded | Lifecycle::Failed);
                let status = match m.install {
                    Install::Installed => "Installed",
                    Install::Installing | Install::Validating => "Installing…",
                    _ => "Not installed",
                };
                let mut row = div()
                    .id(SharedString::from(format!("model-{}", m.id)))
                    .p(px(12.))
                    .border_1()
                    .border_color(p.border)
                    .rounded(tokens::CARD_RADIUS)
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(gpui::text!(m.label.clone()))
                    .child(gpui::text!(format!(
                        "{} · {} · {} · {status}{}",
                        m.size,
                        m.backend,
                        m.accelerator,
                        if m.id == snapshot.selected {
                            " · Selected"
                        } else if active {
                            " · In use"
                        } else {
                            ""
                        }
                    )))
                    .child(gpui::text!(m.capabilities.join(" · ")));
                if !m.supported {
                    row = row.child(gpui::text!("Unavailable on this Mac"));
                }
                if snapshot.recommended.as_ref() == Some(&m.id) {
                    row = row.child(gpui::text!("Recommended for this Mac"));
                }
                if snapshot.warned.contains(&m.id) {
                    row = row.child(gpui::text!("Higher memory"));
                }
                let mut actions = div().flex().flex_wrap().gap(px(8.));
                if m.supported && m.id != snapshot.selected {
                    let id = m.id.clone();
                    let e = cx.entity();
                    actions = actions.child(
                        Button::new(
                            SharedString::from(format!("select-{}", m.id)),
                            format!("Select {}", m.label),
                        )
                        .with_scroll_parent(Some(&self.scroll))
                        .disabled(
                            snapshot.busy
                                || snapshot.attempt.is_some()
                                || self.controller.model.pending,
                        )
                        .on_click(move |_, _, cx| {
                            e.update(cx, |s, cx| {
                                if let Some(snapshot) = &s.controller.model.snapshot {
                                    let action = snapshot.select(id.clone());
                                    s.act(action, cx);
                                }
                            })
                        }),
                    );
                }
                if snapshot.can_download(m) {
                    let e = cx.entity();
                    let id = m.id.clone();
                    actions = actions.child(
                        Button::new(
                            SharedString::from(format!("download-{}", m.id)),
                            format!("Download {}", m.label),
                        )
                        .with_scroll_parent(Some(&self.scroll))
                        .disabled(self.controller.model.pending)
                        .on_click(move |_, _, cx| {
                            e.update(cx, |s, cx| s.act(Action::Download(id.clone()), cx))
                        }),
                    );
                }
                if snapshot.can_remove(m) {
                    let e = cx.entity();
                    let id = m.id.clone();
                    let armed = self.confirm.as_ref().is_some_and(|(i, r, t)| {
                        i == &m.id
                            && *r == snapshot.revision
                            && t.elapsed() < std::time::Duration::from_secs(4)
                    });
                    actions = actions.child(
                        Button::new(
                            SharedString::from(format!("remove-{}", m.id)),
                            format!(
                                "{} {}…",
                                if armed { "Confirm remove" } else { "Remove" },
                                m.label
                            ),
                        )
                        .intent(if armed {
                            ButtonIntent::Danger
                        } else {
                            ButtonIntent::Secondary
                        })
                        .with_scroll_parent(Some(&self.scroll))
                        .disabled(self.controller.model.pending)
                        .on_click(move |_, _, cx| {
                            e.update(cx, |s, cx| s.act(Action::Remove(id.clone()), cx))
                        }),
                    );
                }
                body = body.child(row.child(actions));
            }
            if snapshot.attempt.is_some() {
                body = body.child(div().id("section-status").role(Role::Status).child(
                    gpui::text!(format!(
                        "{} {}",
                        snapshot.phase.as_deref().unwrap_or("Preparing…"),
                        snapshot.percent.map_or(String::new(), |p| format!("{p}%"))
                    )),
                ));
            }
            if let (Some(request_id), Some(model)) = (&snapshot.attempt, &snapshot.attempt_model) {
                let e = cx.entity();
                let action = Action::Cancel {
                    model: model.clone(),
                    request_id: request_id.clone(),
                };
                body = body.child(
                    Button::new("cancel-model-download", "Cancel download")
                        .with_scroll_parent(Some(&self.scroll))
                        .on_click(move |_, _, cx| e.update(cx, |s, cx| s.act(action.clone(), cx))),
                );
            }
        } else {
            body = body.child(gpui::text!("Loading models…"));
        }
        div()
            .id("model-storage")
            .role(Role::Group)
            .aria_label("Model storage")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .p(px(24.))
            .bg(p.surface)
            .text_color(p.text)
            .text_size(tokens::BODY_SIZE)
            .line_height(tokens::BODY_LEADING)
            .child(body)
    }
}
