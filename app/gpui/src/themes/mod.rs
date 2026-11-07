pub mod appearance;
pub mod archive;
pub mod color;
pub mod compile;
#[cfg(any(test, feature = "gallery"))]
pub mod fixtures;
pub mod host;
pub mod import;
pub mod library;
pub mod model;
pub mod open_vsx;
#[cfg(test)]
mod tests;
use crate::{
    Button, ButtonIntent,
    form::{Form, Spec},
    section::{self, Controller},
    tokens::{self, Palette},
};
use gpui::{prelude::*, *};
use host::Action;
use model::*;
use serde_json::{Value, json};
use std::rc::Rc;
#[derive(Clone)]
enum Dialog {
    Import,
    Community,
    Edit { create: bool },
    Preview(Vec<Value>),
    Remove(String),
    Update(Box<open_vsx::Extension>, Vec<Value>),
}
pub struct Themes {
    _forms: Vec<Subscription>,
    pub controller: Controller<Snapshot, Action>,
    pub editor: Entity<Form>,
    pub search: Entity<Form>,
    dialog: Option<Dialog>,
    scroll: ScrollHandle,
    applied: Option<Value>,
    base_theme: Value,
    editor_revision: u64,
    confirming: bool,
    notice: Option<&'static str>,
    restore: Option<FocusHandle>,
}
impl Themes {
    pub fn new(host: Rc<host::Host>, w: &mut Window, cx: &mut Context<Self>) -> Self {
        let editor = cx.new(|cx| {
            Form::new(
                vec![
                    Spec::text("label", "Theme name"),
                    Spec::text("accent", "Accent color (#rrggbb)"),
                    Spec::text("background", "Background color (#rrggbb)"),
                    Spec::text("foreground", "Foreground color (#rrggbb)"),
                    Spec::text("contrast", "Contrast (-100 to 100)"),
                ],
                w,
                cx,
            )
        });
        let search = cx.new(|cx| {
            Form::new(
                vec![
                    Spec::text("query", "Search community themes"),
                    Spec::select(
                        "sort",
                        "Sort themes",
                        &[
                            ("downloadCount", "Most downloaded"),
                            ("rating", "Highest rated"),
                            ("timestamp", "Recently updated"),
                            ("relevance", "Relevance"),
                        ],
                    ),
                ],
                w,
                cx,
            )
        });
        search.update(cx, |s, cx| {
            s.set(json!({"query":"","sort":"downloadCount"}), w, cx)
        });
        let forms = vec![
            cx.observe(&editor, |_, _, cx| cx.notify()),
            cx.observe(&search, |_, _, cx| cx.notify()),
        ];
        Self {
            _forms: forms,
            controller: Controller::new(host),
            editor,
            search,
            dialog: None,
            scroll: ScrollHandle::new(),
            applied: None,
            base_theme: Value::Null,
            editor_revision: 0,
            confirming: false,
            notice: None,
            restore: None,
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
        self.dialog = None;
        self.confirming = false;
        self.notice = None;
        cx.notify();
    }
    fn request(&mut self, a: Action, cx: &mut Context<Self>) {
        self.controller.request(a);
        cx.notify();
    }
    fn button(&self, id: String, label: String, a: Action, cx: &mut Context<Self>) -> Button {
        let e = cx.entity();
        Button::new(SharedString::from(id), label)
            .with_scroll_parent(Some(&self.scroll))
            .disabled(self.controller.model.pending)
            .on_click(move |_, _, cx| e.update(cx, |s, cx| s.request(a.clone(), cx)))
    }
    fn dialog(&mut self, d: Dialog, w: &mut Window, cx: &mut Context<Self>) {
        self.restore = w.focused(cx);
        self.notice = None;
        if let Dialog::Edit { create } = &d {
            let theme = self
                .controller
                .model
                .snapshot
                .as_ref()
                .map(|s| s.appearance["theme"].clone())
                .unwrap_or(json!({"version":1,"presetId":"sonic"}));
            self.base_theme = theme.clone();
            self.editor_revision = self.controller.model.revision;
            self.editor.update(cx,|f,cx|f.set(json!({"label":if *create{"New theme"}else{"Custom"},"accent":theme["accent"].as_str().unwrap_or(""),"background":theme["background"].as_str().unwrap_or(""),"foreground":theme["foreground"].as_str().unwrap_or(""),"contrast":theme["contrast"].as_i64().unwrap_or(0).to_string()}),w,cx));
            self.editor.update(cx, |f, cx| f.focus_first(w, cx));
        }
        if matches!(d, Dialog::Community | Dialog::Import) {
            self.search.update(cx, |f, cx| f.focus_first(w, cx));
        }
        self.dialog = Some(d);
        cx.notify();
    }
    fn close(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        self.controller.invalidate();
        self.dialog = None;
        self.confirming = false;
        self.notice = None;
        self.request(Action::Cancel, cx);
        if let Some(f) = self.restore.take() {
            w.focus(&f, cx)
        }
    }
    fn save_editor(&mut self, cx: &mut Context<Self>) {
        let Some(Dialog::Edit { create }) = self.dialog else {
            return;
        };
        let f = self.editor.read(cx);
        if self.editor_revision != self.controller.model.revision {
            self.notice = Some(
                "Appearance changed elsewhere. Cancel this edit and review the current theme.",
            );
            cx.notify();
            return;
        }
        let mut theme = self.base_theme.clone();
        theme["presetId"] = json!("custom");
        for key in ["accent", "background", "foreground"] {
            let raw = f.value(key).trim();
            theme.as_object_mut().unwrap().remove(key);
            if !raw.is_empty() {
                let Some(c) = color::parse(raw).filter(|_| raw.len() == 7) else {
                    self.notice = Some("Enter a six-digit hex color, such as #036785.");
                    cx.notify();
                    return;
                };
                theme[key] = json!(color::hex(c));
            }
        }
        let Ok(contrast) = f.value("contrast").parse::<i64>() else {
            self.notice = Some("Contrast must be between -100 and 100.");
            cx.notify();
            return;
        };
        if !(-100..=100).contains(&contrast) {
            self.notice = Some("Contrast must be between -100 and 100.");
            cx.notify();
            return;
        }
        theme.as_object_mut().unwrap().remove("contrast");
        if contrast != 0 {
            theme["contrast"] = json!(contrast)
        }
        let action = if create {
            let label = f.value("label").trim();
            if label.is_empty() || label.chars().count() > 64 {
                self.notice = Some("Enter a theme name of 1–64 characters.");
                cx.notify();
                return;
            }
            Action::Install(vec![
                json!({"version":1,"id":format!("local-{}",chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()),"label":label,"modes":["light","dark"],"theme":theme,"source":{"kind":"local"}}),
            ])
        } else {
            let Some(s) = &self.controller.model.snapshot else {
                return;
            };
            let mut doc = s.appearance.clone();
            doc["theme"] = theme;
            doc["selection"] = json!({"light":"custom","dark":"custom"});
            doc["revision"] = json!(library::next_revision(
                doc["revision"].as_u64().unwrap_or(0)
            ));
            Action::Appearance(doc)
        };
        self.confirming = true;
        self.request(action, cx);
    }
}
impl Render for Themes {
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
        let snapshot = self.controller.model.snapshot.clone();
        if let Some(s) = &snapshot {
            if self.applied.as_ref() != Some(&s.appearance) {
                appearance::install(s.appearance.clone(), cx);
                self.applied = Some(s.appearance.clone());
            }
            if self
                .dialog
                .as_ref()
                .is_none_or(|d| matches!(d, Dialog::Community | Dialog::Import))
                && !self.controller.model.pending
                && let Some(preview) = &s.import_preview
            {
                self.dialog(Dialog::Preview(preview.clone()), w, cx);
            }
        }
        let p = Palette::get(cx);
        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(div().text_size(px(20.)).child("Appearance"));
        if let Some(problem) = self.controller.model.problem {
            body = body
                .child(
                    div()
                        .id("themes-error")
                        .role(Role::Alert)
                        .child(problem.message()),
                )
                .child(self.button("themes-retry".into(), "Retry".into(), Action::Read, cx));
        }
        if let Some(n) = self.notice {
            body = body.child(div().id("theme-validation").role(Role::Alert).child(n));
        }
        if let Some(dialog) = self.dialog.clone() {
            let import_choice = matches!(dialog, Dialog::Import);
            let mut content = div()
                .id("theme-dialog")
                .role(Role::Dialog)
                .aria_label(match dialog {
                    Dialog::Community => "Community themes",
                    Dialog::Edit { .. } => "Theme editor",
                    Dialog::Import => "Import theme",
                    Dialog::Preview(_) => "Theme import preview",
                    Dialog::Remove(_) => "Remove theme",
                    Dialog::Update(..) => "Update theme collection",
                })
                .flex()
                .flex_col()
                .gap(px(12.));
            match dialog {
                Dialog::Import => {
                    let e = cx.entity();
                    let local = e.clone();
                    content = content
                        .child(self.button(
                            "theme-local-import".into(),
                            "Choose local file…".into(),
                            Action::InspectLocal,
                            cx,
                        ))
                        .child(
                            Button::new("theme-community", "Browse community…")
                                .with_scroll_parent(Some(&self.scroll))
                                .on_click(move |_, w, cx| {
                                    e.update(cx, |s, cx| s.dialog(Dialog::Community, w, cx))
                                }),
                        )
                        .child(
                            Button::new("theme-import-close", "Cancel")
                                .with_scroll_parent(Some(&self.scroll))
                                .on_click(move |_, w, cx| local.update(cx, |s, cx| s.close(w, cx))),
                        );
                }
                Dialog::Community => {
                    self.search
                        .update(cx, |f, _| f.scroll_parent = Some(self.scroll.clone()));
                    let query = self.search.read(cx).value("query").to_string();
                    let sort = self.search.read(cx).value("sort").to_string();
                    content=content.child("Search sends your query and ordinary connection metadata directly to https://open-vsx.org. Downloads occur only when you choose Add or Update. No account, proxy, background polling, or automatic updates.").child(self.search.clone()).child(self.button("community-search".into(),"Search Open VSX".into(),Action::Search{query,sort},cx));
                    if let Some(s) = &snapshot {
                        if s.results.is_empty() {
                            content = content.child("No community themes to show.");
                        }
                        for e in &s.results {
                            let installed: Vec<_> = s.library["themes"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter(|v| v["source"]["extensionId"] == e.id)
                                .cloned()
                                .collect();
                            let mut row = div()
                                .child(e.label.clone())
                                .child(format!("{} · {} · {}", e.publisher, e.version, e.license));
                            if installed.is_empty() {
                                row = row.child(self.button(
                                    format!("add-{}", e.id),
                                    "Add".into(),
                                    Action::Add(e.clone()),
                                    cx,
                                ));
                            } else {
                                let entity = cx.entity();
                                let extension = e.clone();
                                row = row.child(
                                    Button::new(
                                        SharedString::from(format!("update-{}", e.id)),
                                        "Update…",
                                    )
                                    .with_scroll_parent(Some(&self.scroll))
                                    .on_click(
                                        move |_, w, cx| {
                                            entity.update(cx, |s, cx| {
                                                s.dialog(
                                                    Dialog::Update(
                                                        Box::new(extension.clone()),
                                                        installed.clone(),
                                                    ),
                                                    w,
                                                    cx,
                                                )
                                            })
                                        },
                                    ),
                                );
                            }
                            content = content.child(row);
                        }
                    }
                }
                Dialog::Edit { .. } => {
                    self.editor
                        .update(cx, |f, _| f.scroll_parent = Some(self.scroll.clone()));
                    let e = cx.entity();
                    content=content.child(self.editor.clone()).child("Colors resolve to accessible semantic roles. Leave a color empty to use Sonic.").child(Button::new("theme-save","Save theme").with_scroll_parent(Some(&self.scroll)).on_click(move|_,_,cx|e.update(cx,|s,cx|s.save_editor(cx))));
                }
                Dialog::Preview(entries) => {
                    for entry in &entries {
                        let c = compile::compile(&entry["theme"]);
                        content = content.child(format!(
                            "{} · {} contrast adjustments",
                            entry["label"].as_str().unwrap_or("Imported theme"),
                            c.adjustments
                        ));
                    }
                    let e = cx.entity();
                    content = content.child(
                        Button::new("theme-import-confirm", "Confirm import")
                            .intent(ButtonIntent::Primary)
                            .with_scroll_parent(Some(&self.scroll))
                            .on_click(move |_, w, cx| {
                                e.update(cx, |s, cx| {
                                    s.confirming = true;
                                    s.request(Action::ConfirmImport(entries.clone()), cx);
                                    if let Some(f) = s.restore.take() {
                                        w.focus(&f, cx)
                                    }
                                })
                            }),
                    );
                }
                Dialog::Remove(id) => {
                    let e = cx.entity();
                    content = content.child("Remove this saved theme?").child(
                        Button::new("theme-remove-confirm", "Confirm remove…")
                            .with_scroll_parent(Some(&self.scroll))
                            .intent(ButtonIntent::Danger)
                            .on_click(move |_, _, cx| {
                                e.update(cx, |s, cx| {
                                    s.dialog = None;
                                    s.request(Action::Remove(vec![id.clone()]), cx)
                                })
                            }),
                    );
                }
                Dialog::Update(extension, expected) => {
                    let e = cx.entity();
                    content=content.child("Replace the installed collection with this version? Local changes since this preview refuse the update.").child(Button::new("theme-update-confirm","Confirm update").with_scroll_parent(Some(&self.scroll)).on_click(move|_,_,cx|e.update(cx,|s,cx|{s.dialog=None;s.request(Action::Update{extension:(*extension).clone(),expected:expected.clone()},cx)})));
                }
            }
            if !import_choice {
                let e = cx.entity();
                content = content.child(
                    Button::new("theme-dialog-close", "Cancel")
                        .with_scroll_parent(Some(&self.scroll))
                        .on_click(move |_, w, cx| e.update(cx, |s, cx| s.close(w, cx))),
                );
            }
            body = body.child(content);
        } else if let Some(s) = snapshot {
            let mut mode_buttons = div().flex().flex_wrap().gap(px(8.));
            for (mode, label) in [("system", "System"), ("light", "Light"), ("dark", "Dark")] {
                let mut doc = s.appearance.clone();
                doc["mode"] = json!(mode);
                doc["revision"] = json!(library::next_revision(
                    doc["revision"].as_u64().unwrap_or(0)
                ));
                mode_buttons = mode_buttons.child(
                    self.button(
                        format!("appearance-{mode}"),
                        label.into(),
                        Action::Appearance(doc),
                        cx,
                    )
                    .toggled(s.appearance["mode"] == mode),
                );
            }
            body = body.child(mode_buttons);
            let mut reset = s.appearance.clone();
            reset["theme"] = json!({"version":1,"presetId":"sonic"});
            reset["selection"] = json!({"light":"sonic","dark":"sonic"});
            reset["revision"] = json!(library::next_revision(
                reset["revision"].as_u64().unwrap_or(0)
            ));
            body = body.child(self.button(
                "theme-reset".into(),
                "Reset to Sonic".into(),
                Action::Appearance(reset),
                cx,
            ));
            let create = cx.entity();
            let edit = create.clone();
            let import = create.clone();
            body = body.child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .child(
                        Button::new("theme-create", "Create theme…")
                            .with_scroll_parent(Some(&self.scroll))
                            .on_click(move |_, w, cx| {
                                create.update(cx, |s, cx| {
                                    s.dialog(Dialog::Edit { create: true }, w, cx)
                                })
                            }),
                    )
                    .child(
                        Button::new("theme-edit", "Edit colors…")
                            .with_scroll_parent(Some(&self.scroll))
                            .on_click(move |_, w, cx| {
                                edit.update(cx, |s, cx| {
                                    s.dialog(Dialog::Edit { create: false }, w, cx)
                                })
                            }),
                    )
                    .child(
                        Button::new("theme-import", "Import theme…")
                            .with_scroll_parent(Some(&self.scroll))
                            .on_click(move |_, w, cx| {
                                import.update(cx, |s, cx| s.dialog(Dialog::Import, w, cx))
                            }),
                    )
                    .child(self.button(
                        "theme-export".into(),
                        "Export current…".into(),
                        Action::ExportCurrent,
                        cx,
                    )),
            );
            if !s.library_available {
                body = body.child(div().id("theme-library-error").role(Role::Alert).child(
                    "Theme library storage is unavailable. Your active appearance is kept.",
                ));
            }
            for entry in library::entries(&s.library) {
                let id = entry["id"].as_str().unwrap().to_string();
                let mut row = div()
                    .p(px(16.))
                    .border_1()
                    .border_color(p.border)
                    .rounded(tokens::CARD_RADIUS)
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(entry["label"].as_str().unwrap_or("Theme").to_string());
                let mut variants = div().flex().flex_wrap().gap(px(8.));
                for mode in entry["modes"].as_array().unwrap() {
                    let mode = mode.as_str().unwrap();
                    if let Ok(doc) = library::selection(&s.appearance, &s.library, &id, Some(mode))
                    {
                        variants = variants.child(self.button(
                            format!("theme-{id}-{mode}"),
                            format!("Use {} {mode}", entry["label"].as_str().unwrap()),
                            Action::Appearance(doc),
                            cx,
                        ));
                    }
                }
                if let Ok(doc) = library::selection(&s.appearance, &s.library, &id, None) {
                    variants = variants.child(self.button(
                        format!("theme-{id}-pair"),
                        "Use theme".into(),
                        Action::Appearance(doc),
                        cx,
                    ));
                }
                variants = variants
                    .child(self.button(
                        format!("theme-{id}-duplicate"),
                        "Duplicate".into(),
                        Action::Duplicate(id.clone()),
                        cx,
                    ))
                    .child(self.button(
                        format!("theme-{id}-export"),
                        "Export…".into(),
                        Action::ExportEntry(id.clone()),
                        cx,
                    ));
                if !["sonic", "opencode"].contains(&id.as_str()) {
                    let e = cx.entity();
                    variants = variants.child(
                        Button::new(SharedString::from(format!("theme-{id}-remove")), "Remove…")
                            .with_scroll_parent(Some(&self.scroll))
                            .on_click(move |_, w, cx| {
                                e.update(cx, |s, cx| s.dialog(Dialog::Remove(id.clone()), w, cx))
                            }),
                    );
                }
                if let Some(url) = entry["source"]["sourceUrl"].as_str() {
                    variants = variants.child(self.button(
                        format!("source-{}", entry["id"].as_str().unwrap()),
                        "Source…".into(),
                        Action::OpenSource(url.into()),
                        cx,
                    ));
                }
                row = row.child(variants);
                body = body.child(row);
            }
            if let Some(n) = s.notice {
                body = body.child(div().id("themes-notice").role(Role::Status).child(n));
            }
        } else {
            body = body.child("Loading appearance…");
        }
        div()
            .id("themes")
            .capture_key_down(cx.listener(|s, e: &KeyDownEvent, w, cx| {
                if e.keystroke.key == "escape" && s.dialog.is_some() {
                    s.close(w, cx);
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
