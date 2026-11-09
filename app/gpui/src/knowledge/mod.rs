#[cfg(any(test, feature = "gallery"))]
pub mod fixtures;
pub mod host;
pub mod model;
#[cfg(test)]
mod tests;
use crate::{
    Button, ButtonIntent,
    form::{Changed, Form, Spec},
    section::{self, Controller},
    tokens::{self, Palette},
};
use gpui::{prelude::*, *};
use host::Action;
use model::*;
use serde_json::{Value, json};
use std::rc::Rc;
enum Dialog {
    Edit(Option<Value>),
    Delete(Value),
    DeleteAll(u64),
    Bulk(Bulk, BulkAction),
    Import(Value),
}
pub struct Knowledge {
    _forms: Vec<Subscription>,
    pub controller: Controller<Snapshot, Action>,
    pub filter: Filter,
    pub filters: Entity<Form>,
    pub editor: Entity<Form>,
    pub phrase: Entity<Form>,
    dialog: Option<Dialog>,
    confirming: bool,
    scroll: ScrollHandle,
    notice: Option<&'static str>,
    restore: Option<FocusHandle>,
    save_focus: FocusHandle,
    _filters: Subscription,
}
impl Knowledge {
    pub fn new(host: Rc<host::Host>, w: &mut Window, cx: &mut Context<Self>) -> Self {
        let filters = cx.new(|cx| {
            Form::new(
                vec![
                    Spec::text("query", "Search knowledge"),
                    Spec::select(
                        "kind",
                        "Type",
                        &[
                            ("all", "All types"),
                            ("replacement_rule", "Replacement"),
                            ("vocabulary_term", "Vocabulary"),
                            ("snippet", "Snippet"),
                            ("transform", "Transform"),
                        ],
                    ),
                    Spec::select(
                        "enabled",
                        "State",
                        &[
                            ("all", "Any state"),
                            ("true", "Enabled"),
                            ("false", "Disabled"),
                        ],
                    ),
                    Spec::select(
                        "scope",
                        "Visibility",
                        &[
                            ("all", "Any visibility"),
                            ("global", "Global"),
                            ("app", "App"),
                            ("project", "Project"),
                        ],
                    ),
                ],
                w,
                cx,
            )
        });
        filters.update(cx, |s, cx| {
            s.set(
                json!({"query":"","kind":"all","enabled":"all","scope":"all"}),
                w,
                cx,
            )
        });
        let editor = cx.new(|cx| {
            Form::new(
                vec![
                    Spec::select(
                        "kind",
                        "Knowledge type",
                        &[
                            ("replacement_rule", "Replacement rule"),
                            ("vocabulary_term", "Vocabulary term"),
                            ("snippet", "Snippet"),
                            ("transform", "Transform"),
                        ],
                    ),
                    Spec::text("title", "Heard phrase / written term / trigger / name"),
                    Spec {
                        multiline: true,
                        ..Spec::text(
                            "detail",
                            "Replacement / aliases (one per line) / body / instruction",
                        )
                    },
                    Spec::select(
                        "scope",
                        "Scope",
                        &[("global", "Global"), ("app", "App"), ("project", "Project")],
                    ),
                    Spec::text("bundleId", "App bundle ID"),
                    Spec::text("root", "Project root"),
                    Spec::boolean("enabled", "Enabled"),
                ],
                w,
                cx,
            )
        });
        let phrase =
            cx.new(|cx| Form::new(vec![Spec::text("phrase", "Type DELETE to confirm")], w, cx));
        let sub = cx.subscribe(&filters, |s, _, _: &Changed, cx| {
            if s.dialog.is_none() && !s.controller.model.pending {
                s.filter = Self::read_filter(s.filters.read(cx));
                s.controller.invalidate();
                s.controller.request(Action::List {
                    filter: s.filter.clone(),
                    offset: 0,
                });
                cx.notify();
            }
        });
        let forms = vec![
            cx.observe(&editor, |_, _, cx| cx.notify()),
            cx.observe(&phrase, |_, _, cx| cx.notify()),
        ];
        Self {
            _forms: forms,
            controller: Controller::new(host),
            filter: Filter::default(),
            filters,
            editor,
            phrase,
            dialog: None,
            confirming: false,
            scroll: ScrollHandle::new(),
            notice: None,
            restore: None,
            save_focus: cx.focus_handle(),
            _filters: sub,
        }
    }
    fn read_filter(f: &Form) -> Filter {
        let optional = |s: &str| {
            if s.is_empty() || s == "all" {
                None
            } else {
                Some(s.to_string())
            }
        };
        Filter {
            query: f.value("query").into(),
            kind: optional(f.value("kind")),
            enabled: match f.value("enabled") {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            },
            scope: optional(f.value("scope")),
        }
    }
    pub fn attach(&mut self, cx: &mut Context<Self>) {
        self.controller.attach(Action::List {
            filter: self.filter.clone(),
            offset: 0,
        });
        cx.notify();
    }
    pub fn follow(&self, cx: &mut Context<Self>) {
        section::follow(cx, self.controller.model.attachment, |s| &mut s.controller)
    }
    pub fn hide(&mut self, cx: &mut Context<Self>) {
        self.controller.detach();
        self.dialog = None;
        self.notice = None;
        cx.notify();
    }
    fn request(&mut self, a: Action, cx: &mut Context<Self>) {
        if self.controller.request(a) {
            self.notice = None;
            cx.notify();
        }
    }
    fn button(&self, id: String, label: String, a: Action, cx: &mut Context<Self>) -> Button {
        let e = cx.entity();
        Button::new(SharedString::from(id), label)
            .with_scroll_parent(Some(&self.scroll))
            .disabled(self.controller.model.pending)
            .on_click(move |_, _, cx| e.update(cx, |s, cx| s.request(a.clone(), cx)))
    }
    fn begin_dialog(&mut self, d: Dialog, w: &mut Window, cx: &mut Context<Self>) {
        self.restore = w.focused(cx);
        self.phrase
            .update(cx, |s, cx| s.set(json!({"phrase":""}), w, cx));
        if let Dialog::Edit(entry) = &d {
            let blank = json!({"payload":{"kind":"replacement_rule","source":"","replacement":""},"scope":{"kind":"global"},"enabled":true});
            let v = entry.as_ref().unwrap_or(&blank);
            self.editor.update(cx,|s,cx|s.set(json!({"kind":v["payload"]["kind"],"title":title(v),"detail":editor_detail(v),"scope":v["scope"]["kind"],"bundleId":v["scope"]["bundleId"],"root":v["scope"]["root"],"enabled":v["enabled"].as_bool().unwrap_or(true).to_string()}),w,cx));
            self.editor.update(cx, |s, cx| {
                s.disabled_keys.clear();
                if entry.is_some() {
                    s.disabled_keys.insert("kind");
                }
                s.focus_first(w, cx)
            });
        } else {
            self.phrase.update(cx, |s, cx| s.focus_first(w, cx));
        }
        self.dialog = Some(d);
        cx.notify();
    }
    fn cancel(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        self.dialog = None;
        self.confirming = false;
        self.controller.invalidate();
        self.controller.request(Action::Cancel);
        if let Some(f) = self.restore.take() {
            w.focus(&f, cx)
        }
        cx.notify();
    }
    fn confirm(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        if self.controller.model.pending {
            return;
        }
        let action = match &self.dialog {
            Some(Dialog::Edit(entry)) => {
                let f = self.editor.read(cx);
                let k = f.value("kind");
                let t = f.value("title");
                let d = f.value("detail");
                let payload = match k {
                    "replacement_rule" => json!({"kind":k,"source":t,"replacement":d}),
                    "vocabulary_term" => {
                        json!({"kind":k,"written":t,"aliases":d.lines().map(str::trim).filter(|s|!s.is_empty()).collect::<Vec<_>>()})
                    }
                    "snippet" => json!({"kind":k,"trigger":t,"body":d}),
                    _ => json!({"kind":"transform","name":t,"instruction":d}),
                };
                let mut scope = json!({"kind":f.value("scope")});
                if f.value("scope") != "global" {
                    scope["bundleId"] = json!(f.value("bundleId").trim())
                }
                if f.value("scope") == "project" {
                    scope["root"] = json!(f.value("root").trim())
                }
                let mut draft = json!({"payload":payload,"scope":scope,"enabled":f.value("enabled")=="true","voiceCommand":entry.as_ref().map(|v|&v["voiceCommand"]).unwrap_or(&Value::Null)});
                if let Some(v) = entry {
                    draft["id"] = v["id"].clone();
                    draft["expectedRevision"] = v["revision"].clone();
                }
                if valid_draft(&draft).is_err() {
                    self.notice = Some("Enter the required content and scope fields.");
                    cx.notify();
                    return;
                }
                Action::Upsert(draft)
            }
            Some(Dialog::Delete(v)) => Action::Delete(v.clone()),
            Some(Dialog::DeleteAll(r)) => {
                if self.phrase.read(cx).value("phrase") != "DELETE" {
                    return;
                }
                Action::DeleteAll(*r)
            }
            Some(Dialog::Bulk(b, a)) => {
                if *a == BulkAction::Delete && self.phrase.read(cx).value("phrase") != "DELETE" {
                    return;
                }
                Action::Apply {
                    bulk: b.clone(),
                    action: *a,
                }
            }
            Some(Dialog::Import(preview)) => Action::Import {
                preview: preview.clone(),
            },
            None => return,
        };
        self.confirming = true;
        self.request(action, cx);
        let _ = w;
    }
}
impl Render for Knowledge {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.controller.drain();
        if self.confirming && !self.controller.model.pending {
            self.confirming = false;
            if self.controller.model.problem.is_none() {
                self.dialog = None;
                if let Some(f) = self.restore.take() {
                    w.focus(&f, cx)
                }
            }
        }
        let p = Palette::get(cx);
        let snapshot = self.controller.model.snapshot.clone();
        if self.dialog.is_none() && !self.controller.model.pending {
            if let Some(preview) = snapshot.as_ref().and_then(|s| s.import_preview.clone()) {
                self.begin_dialog(Dialog::Import(preview), w, cx);
            } else if let Some(b) = snapshot
                .as_ref()
                .and_then(|s| s.bulk.clone())
                .filter(|b| b.complete)
            {
                self.begin_dialog(Dialog::Bulk(b, BulkAction::Delete), w, cx);
            }
        }
        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(div().text_size(px(20.)).child(gpui::text!("Knowledge")));
        if let Some(problem) = self.controller.model.problem {
            body = body.child(
                div()
                    .id("knowledge-error")
                    .role(Role::Alert)
                    .child(gpui::text!(problem.message())),
            );
        }
        if let Some(message) = self.notice {
            body = body.child(
                div()
                    .id("knowledge-validation")
                    .role(Role::Alert)
                    .child(gpui::text!(message)),
            );
        }
        if let Some(dialog) = &self.dialog {
            let label = match dialog {
                Dialog::Edit(Some(_)) => "Edit knowledge",
                Dialog::Edit(None) => "Create knowledge…",
                Dialog::Delete(_) => "Delete knowledge",
                Dialog::DeleteAll(_) => "Delete all knowledge",
                Dialog::Bulk(_, _) => "Delete matching knowledge",
                Dialog::Import(_) => "Import knowledge preview",
            };
            let mut form = div()
                .id("knowledge-dialog")
                .role(Role::Dialog)
                .aria_label(label)
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(gpui::text!(label));
            self.phrase
                .update(cx, |f, _| f.scroll_parent = Some(self.scroll.clone()));
            match dialog {
                Dialog::Edit(_) => {
                    self.editor
                        .update(cx, |s, _| s.scroll_parent = Some(self.scroll.clone()));
                    form=form.child(gpui::text!("Stored only on this Mac. Content is never sent to a cloud service or written to logs.")).child(self.editor.clone());
                }
                Dialog::Delete(v) => form = form.child(gpui::text!(title(v).to_string())),
                Dialog::Bulk(b, _) => {
                    form = form
                        .child(gpui::text!(format!("Permanently remove these {} matching records, including unloaded pages.", b.entries.len())))
                        .child(gpui::text!(b.filter.label()))
                        .child(gpui::text!("Only the collected records will be deleted. Records changed since collection will be kept and reported as failures."))
                        .child(self.phrase.clone())
                }
                Dialog::DeleteAll(_) => form = form
                    .child(gpui::text!("This permanently deletes all knowledge plus local recovery backups. Export first if you may need it later."))
                    .child(self.phrase.clone()),
                Dialog::Import(v) => {
                    form = form
                        .child(gpui::text!(format!(
                            "{} records inspected: {} new, {} duplicates, and {} trigger conflicts.",
                            v["total"], v["new"], v["duplicates"], v["conflicts"]
                        )))
                        .child(gpui::text!("Existing records are never overwritten. Same-ID conflicts reject the import."))
                }
            };
            let e = cx.entity();
            let cancel = e.clone();
            let confirm_label = match dialog {
                Dialog::Edit(_) => "Save".to_string(),
                Dialog::Import(_) => "Import".to_string(),
                Dialog::Bulk(b, BulkAction::Delete) => {
                    format!("Delete {} matching records…", b.entries.len())
                }
                _ => "Confirm".to_string(),
            };
            form = form
                .child(
                    Button::new("knowledge-confirm", confirm_label)
                        .intent(
                            if matches!(
                                dialog,
                                Dialog::Delete(_)
                                    | Dialog::DeleteAll(_)
                                    | Dialog::Bulk(_, BulkAction::Delete)
                            ) {
                                ButtonIntent::Danger
                            } else {
                                ButtonIntent::Primary
                            },
                        )
                        .disabled(self.controller.model.pending)
                        .track_focus(&self.save_focus)
                        .default_confirm(matches!(dialog, Dialog::Edit(_)))
                        .with_scroll_parent(Some(&self.scroll))
                        .on_click(move |_, w, cx| e.update(cx, |s, cx| s.confirm(w, cx))),
                )
                .child(
                    Button::new("knowledge-cancel", "Cancel")
                        .with_scroll_parent(Some(&self.scroll))
                        .on_click(move |_, w, cx| cancel.update(cx, |s, cx| s.cancel(w, cx))),
                );
            body = body.child(form);
        } else {
            if let Some(s) = snapshot {
                body = body.child(div().id("knowledge-status").role(Role::Status).child(
                    gpui::text!(match s.availability.as_str() {
                        "ready" => "Local store ready",
                        "recovered" => "Recovered from a local backup.",
                        "reinitialized" => "Started with a new local store.",
                        _ => "The local knowledge store is unavailable.",
                    }),
                ));
                if s.availability == "unavailable" {
                    body = body.child(self.button(
                        "knowledge-retry".into(),
                        "Retry".into(),
                        Action::Retry,
                        cx,
                    ));
                } else {
                    self.filters.update(cx, |f, _| {
                        f.disabled = self.controller.model.pending;
                        f.scroll_parent = Some(self.scroll.clone())
                    });
                    body = body.child(self.filters.clone());
                    let e = cx.entity();
                    body = body.child(
                        Button::new("knowledge-create", "Create knowledge…")
                            .disabled(self.controller.model.pending)
                            .with_scroll_parent(Some(&self.scroll))
                            .on_click(move |_, w, cx| {
                                e.update(cx, |s, cx| s.begin_dialog(Dialog::Edit(None), w, cx))
                            }),
                    );
                    let mut actions = div().flex().flex_wrap().gap(px(8.));
                    for (id, label, a) in [
                        ("import", "Import…", Action::InspectImport),
                        ("export", "Export…", Action::Export),
                        (
                            "enable",
                            "Enable matches",
                            Action::Collect {
                                filter: self.filter.clone(),
                                action: BulkAction::Enable,
                            },
                        ),
                        (
                            "disable",
                            "Disable matches",
                            Action::Collect {
                                filter: self.filter.clone(),
                                action: BulkAction::Disable,
                            },
                        ),
                        (
                            "delete-filter",
                            "Delete matches…",
                            Action::Collect {
                                filter: self.filter.clone(),
                                action: BulkAction::Delete,
                            },
                        ),
                    ] {
                        actions = actions.child(self.button(id.into(), label.into(), a, cx));
                    }
                    let e = cx.entity();
                    let revision = s.store_revision;
                    actions = actions.child(
                        Button::new("knowledge-delete-all", "Delete all…")
                            .with_scroll_parent(Some(&self.scroll))
                            .disabled(self.controller.model.pending)
                            .on_click(move |_, w, cx| {
                                e.update(cx, |s, cx| {
                                    s.begin_dialog(Dialog::DeleteAll(revision), w, cx)
                                })
                            }),
                    );
                    body = body
                        .child(actions)
                        .child(gpui::text!(format!("{} records", s.total)));
                    if s.records.is_empty() {
                        body = body.child(gpui::text!("No knowledge records match your filters."));
                    }
                    for v in s.records {
                        let id = v["id"].as_str().unwrap_or_default();
                        let edit = v.clone();
                        let del = v.clone();
                        let e = cx.entity();
                        let e2 = e.clone();
                        body =
                            body.child(
                                div()
                                    .id(SharedString::from(format!("knowledge-record-{id}")))
                                    .p(px(12.))
                                    .border_1()
                                    .border_color(p.border)
                                    .rounded(tokens::CARD_RADIUS)
                                    .flex()
                                    .flex_col()
                                    .gap(px(8.))
                                    .child(gpui::text!(title(&v).to_string()))
                                    .child(gpui::text!(detail(&v)))
                                    .child(gpui::text!(format!(
                                        "{} · {} · updated {}",
                                        v["scope"]["kind"], v["provenance"], v["updatedAtMs"]
                                    )))
                                    .child(
                                        div()
                                            .flex()
                                            .flex_wrap()
                                            .gap(px(8.))
                                            .child(
                                                Button::new(
                                                    SharedString::from(format!("edit-{id}")),
                                                    format!("Edit {}", title(&v)),
                                                )
                                                .with_scroll_parent(Some(&self.scroll))
                                                .on_click(move |_, w, cx| {
                                                    e.update(cx, |s, cx| {
                                                        s.begin_dialog(
                                                            Dialog::Edit(Some(edit.clone())),
                                                            w,
                                                            cx,
                                                        )
                                                    })
                                                }),
                                            )
                                            .child(self.button(
                                                format!("enable-{id}"),
                                                format!(
                                                    "{} {}",
                                                    if v["enabled"] == true {
                                                        "Disable"
                                                    } else {
                                                        "Enable"
                                                    },
                                                    title(&v)
                                                ),
                                                Action::Enable {
                                                    enabled: v["enabled"] != true,
                                                    entry: v.clone(),
                                                },
                                                cx,
                                            ))
                                            .child(
                                                Button::new(
                                                    SharedString::from(format!("delete-{id}")),
                                                    format!("Delete {}…", title(&v)),
                                                )
                                                .with_scroll_parent(Some(&self.scroll))
                                                .on_click(move |_, w, cx| {
                                                    e2.update(cx, |s, cx| {
                                                        s.begin_dialog(
                                                            Dialog::Delete(del.clone()),
                                                            w,
                                                            cx,
                                                        )
                                                    })
                                                }),
                                            ),
                                    ),
                            );
                    }
                    let mut pages = div().flex().gap(px(8.));
                    if s.offset > 0 {
                        pages = pages.child(self.button(
                            "knowledge-prev".into(),
                            "Previous page".into(),
                            Action::List {
                                filter: self.filter.clone(),
                                offset: s.offset.saturating_sub(50),
                            },
                            cx,
                        ));
                    }
                    if let Some(next) = s.next {
                        pages = pages.child(self.button(
                            "knowledge-next".into(),
                            "Next page".into(),
                            Action::List {
                                filter: self.filter.clone(),
                                offset: next,
                            },
                            cx,
                        ));
                    }
                    body = body.child(pages);
                    if let Some(notice) = s.notice {
                        body = body.child(
                            div()
                                .id("knowledge-notice")
                                .role(Role::Status)
                                .child(gpui::text!(notice)),
                        );
                    }
                }
            } else {
                body = body.child(gpui::text!("Loading knowledge…"));
            }
        }
        div()
            .id("knowledge")
            .capture_key_down(cx.listener(|s, e: &KeyDownEvent, w, cx| {
                if e.keystroke.key == "escape" && s.dialog.is_some() {
                    s.cancel(w, cx);
                    w.prevent_default();
                    cx.stop_propagation();
                } else if e.keystroke.key == "enter"
                    && e.keystroke.modifiers.number_of_modifiers() == 0
                    && !e.is_held
                    && !e.prefer_character_input
                    && matches!(s.dialog, Some(Dialog::Edit(_)))
                    && (s.save_focus.is_focused(w) || s.editor.read(cx).single_line_focused(w, cx))
                {
                    s.confirm(w, cx);
                    w.prevent_default();
                    cx.stop_propagation();
                }
            }))
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
