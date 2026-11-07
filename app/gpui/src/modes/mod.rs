pub mod exchange;
#[cfg(any(test, feature = "gallery"))]
pub mod fixtures;
pub mod host;
pub mod model;
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
pub struct Modes {
    _forms: Vec<Subscription>,
    pub controller: Controller<Snapshot, Action>,
    pub selected: String,
    pub editor: Entity<Form>,
    pub site: Entity<Form>,
    pub sample: Entity<Form>,
    editing: Option<Value>,
    edit_focus: FocusHandle,
    site_edit: Option<Value>,
    confirm: Option<String>,
    scroll: ScrollHandle,
    editing_source: Value,
    saving: bool,
    seen_revision: u64,
    notice: Option<&'static str>,
}
impl Modes {
    pub fn new(host: Rc<host::Host>, w: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut specs = vec![
            Spec::text("name", "Mode name"),
            Spec::boolean("enabled", "Enable mode"),
            Spec::select(
                "writingStyle",
                "Writing style",
                &[
                    ("global", "Use global"),
                    ("inherit", "Inherit"),
                    ("conversational", "Conversational"),
                    ("polished", "Polished"),
                    ("notes", "Notes"),
                    ("code_technical", "Code / technical"),
                    ("verbatim", "Verbatim"),
                ],
            ),
            Spec::select(
                "vocabularyPolicy",
                "Vocabulary policy",
                &[
                    ("inherit", "Inherit"),
                    ("general", "General"),
                    ("technical", "Technical"),
                ],
            ),
            Spec::select(
                "contextPolicy",
                "Context policy",
                &[("none", "None"), ("project", "Project")],
            ),
        ];
        for (key, label) in [("modelId", "Mode model"), ("language", "Mode language")] {
            let row = crate::settings::schema::ROWS
                .iter()
                .find(|r| {
                    r.key
                        == if key == "modelId" {
                            "model"
                        } else {
                            "language"
                        }
                })
                .unwrap();
            let mut spec = Spec::select(key, label, &[("global", "Use global")]);
            if let crate::settings::schema::Kind::Choice(o) = row.kind {
                spec.choices
                    .extend(o.iter().map(|(v, l)| (v.to_string(), l.to_string())))
            }
            specs.push(spec);
        }
        for (key, label) in [
            ("cleanupEnabled", "Cleanup"),
            ("smartFormattingEnabled", "Smart formatting"),
            ("cliFormattingEnabled", "CLI formatting"),
            ("autoPaste", "Auto-paste"),
        ] {
            specs.push(Spec::select(
                key,
                label,
                &[("global", "Use global"), ("true", "On"), ("false", "Off")],
            ));
        }
        let editor = cx.new(|cx| Form::new(specs, w, cx));
        let site = cx.new(|cx| {
            Form::new(
                vec![
                    Spec::select("browser", "Site rule browser", &BROWSERS),
                    Spec::text("host", "Site rule host"),
                ],
                w,
                cx,
            )
        });
        site.update(cx, |s, cx| {
            s.set(json!({"browser":BROWSERS[0].0,"host":""}), w, cx)
        });
        let sample = cx.new(|cx| {
            Form::new(
                vec![Spec {
                    multiline: true,
                    ..Spec::text("sample", "Mode test input")
                }],
                w,
                cx,
            )
        });
        sample.update(cx, |s, cx| {
            s.set(json!({"sample":"um draft a concise project update"}), w, cx)
        });
        let forms = vec![
            cx.observe(&editor, |_, _, cx| cx.notify()),
            cx.observe(&site, |_, _, cx| cx.notify()),
            cx.observe(&sample, |_, _, cx| cx.notify()),
        ];
        Self {
            _forms: forms,
            controller: Controller::new(host),
            selected: "builtin.everyday".into(),
            editor,
            site,
            sample,
            editing: None,
            edit_focus: cx.focus_handle(),
            site_edit: None,
            confirm: None,
            scroll: ScrollHandle::new(),
            editing_source: Value::Null,
            saving: false,
            seen_revision: 0,
            notice: None,
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
        self.editing = None;
        self.site_edit = None;
        self.confirm = None;

        cx.notify();
    }
    fn patch(&mut self, patch: Value, cx: &mut Context<Self>) {
        self.controller.request(Action::Patch(patch));

        self.confirm = None;
        cx.notify();
    }
    fn create(&mut self, source: Option<Value>, cx: &mut Context<Self>) {
        let Some(s) = &self.controller.model.snapshot else {
            return;
        };
        let id = format!(
            "mode.{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        let mut mode = source.map_or_else(
            || blank(&id, "New mode"),
            |mut m| {
                m["id"] = json!(id);
                m["name"] = json!(format!("{} copy", m["name"].as_str().unwrap_or("Mode")));
                m["builtIn"] = json!(false);
                m["enabled"] = json!(true);
                m
            },
        );
        mode["id"] = json!(id);
        let mut modes = s.settings["modes"].as_array().unwrap().clone();
        if modes.len() >= 100 {
            self.notice = Some("The mode library is full.");
            return;
        }
        modes.push(mode);
        self.selected = id;
        self.patch(json!({"modes":modes}), cx);
    }
    fn cancel_edit(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        self.editing = None;
        self.saving = false;
        self.notice = None;
        w.focus(&self.edit_focus, cx);
        cx.notify();
    }
    fn save(&mut self, cx: &mut Context<Self>) {
        if self
            .controller
            .model
            .snapshot
            .as_ref()
            .is_some_and(|s| self.editing_source != s.settings["modes"])
        {
            self.notice =
                Some("Modes changed elsewhere. Cancel this edit and review the current mode.");
            cx.notify();
            return;
        }
        let Some(mut m) = self.editing.clone() else {
            return;
        };
        let f = self.editor.read(cx);
        for key in [
            "name",
            "writingStyle",
            "vocabularyPolicy",
            "contextPolicy",
            "modelId",
            "language",
            "cleanupEnabled",
            "smartFormattingEnabled",
            "cliFormattingEnabled",
            "autoPaste",
            "enabled",
        ] {
            let v = f.value(key);
            m[key] = match v {
                "global" => Value::Null,
                "true" => json!(true),
                "false" => json!(false),
                _ => json!(v),
            };
        }
        let Some(s) = &self.controller.model.snapshot else {
            return;
        };
        let mut settings = s.settings.clone();
        let Some(modes) = settings["modes"].as_array_mut() else {
            return;
        };
        let Some(current) = modes.iter_mut().find(|v| v["id"] == m["id"]) else {
            return;
        };
        *current = m.clone();
        if !murmur_core::state::normalize("dictation-settings", settings.clone())
            .is_ok_and(|v| v["modes"] == settings["modes"])
        {
            self.notice = Some("Review the mode name and policy choices.");
            cx.notify();
            return;
        }
        self.saving = true;
        self.patch(json!({"modes":settings["modes"]}), cx);
    }
    fn button(&self, id: &str, label: &str, a: Action, cx: &mut Context<Self>) -> Button {
        let e = cx.entity();
        Button::new(SharedString::from(id.to_string()), label.to_string())
            .disabled(self.controller.model.pending)
            .with_scroll_parent(Some(&self.scroll))
            .on_click(move |_, _, cx| {
                e.update(cx, |s, cx| {
                    s.controller.request(a.clone());
                    cx.notify()
                })
            })
    }
    fn add_site(&mut self, cx: &mut Context<Self>) {
        let f = self.site.read(cx);
        let Some(host) = normalize_host(f.value("host")) else {
            self.notice = Some("Enter an exact host such as github.com.");
            cx.notify();
            return;
        };
        let browser = f.value("browser").to_string();
        let Some(s) = &self.controller.model.snapshot else {
            return;
        };
        let mut rules = s.settings["browserSiteRules"].as_array().unwrap().clone();
        if rules.iter().any(|r| {
            r["host"] == host
                && r["browserBundleId"] == browser
                && self.site_edit.as_ref().is_none_or(|e| e["id"] != r["id"])
        }) {
            self.notice = Some("That browser and host already have a rule.");
            cx.notify();
            return;
        }
        if let Some(mut edited) = self.site_edit.take() {
            edited["host"] = json!(host);
            edited["browserBundleId"] = json!(browser);
            if let Some(rule) = rules.iter_mut().find(|r| r["id"] == edited["id"]) {
                *rule = edited
            } else {
                self.notice = Some("The site rule changed. Review the current list.");
                cx.notify();
                return;
            }
        } else {
            rules.push(json!({"id":format!("site.{}",chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()),"browserBundleId":browser,"host":host,"modeId":self.selected,"enabled":true}));
        }
        self.patch(json!({"browserSiteRules":rules}), cx);
    }
}
impl Render for Modes {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.controller.drain();
        if self.seen_revision != self.controller.model.revision {
            self.confirm = None;
            self.seen_revision = self.controller.model.revision;
        }
        if self.saving && !self.controller.model.pending {
            self.saving = false;
            if self.controller.model.problem.is_none() {
                self.editing = None;
            }
        }
        let p = Palette::get(cx);
        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(div().text_size(px(20.)).child(gpui::text!("Modes")));
        if let Some(problem) = self.controller.model.problem {
            body = body.child(
                div()
                    .id("modes-error")
                    .role(Role::Alert)
                    .child(gpui::text!(problem.message())),
            );
        }
        if let Some(n) = self.notice {
            body = body.child(
                div()
                    .id("modes-validation")
                    .role(Role::Alert)
                    .child(gpui::text!(n)),
            );
        }
        if let Some(s) = self.controller.model.snapshot.clone() {
            let all = s.all();
            let selected = all
                .iter()
                .find(|m| m["id"] == self.selected)
                .cloned()
                .unwrap_or_else(|| builtins().remove(0));
            self.selected = selected["id"].as_str().unwrap().into();
            let custom = selected["builtIn"] == false;
            let mut list = div().flex().flex_wrap().gap(px(8.));
            for m in all {
                let id = m["id"].as_str().unwrap().to_string();
                let e = cx.entity();
                list = list.child(
                    Button::new(
                        SharedString::from(format!("mode-{id}")),
                        m["name"].as_str().unwrap_or("Untitled mode").to_string(),
                    )
                    .toggled(id == self.selected)
                    .with_scroll_parent(Some(&self.scroll))
                    .on_click(move |_, _, cx| {
                        e.update(cx, |s, cx| {
                            s.selected = id.clone();
                            s.editing = None;
                            s.confirm = None;

                            cx.notify()
                        })
                    }),
                );
            }
            body = body.child(gpui::text!("Library")).child(list);
            let create = cx.entity();
            let duplicate = create.clone();
            let source = selected.clone();
            body = body.child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .child(
                        Button::new("mode-create", "Create mode")
                            .with_scroll_parent(Some(&self.scroll))
                            .disabled(self.controller.model.pending)
                            .on_click(move |_, _, cx| {
                                create.update(cx, |s, cx| s.create(None, cx))
                            }),
                    )
                    .child(
                        Button::new("mode-duplicate", "Duplicate")
                            .with_scroll_parent(Some(&self.scroll))
                            .disabled(self.controller.model.pending)
                            .on_click(move |_, _, cx| {
                                duplicate.update(cx, |s, cx| s.create(Some(source.clone()), cx))
                            }),
                    )
                    .child(self.button("modes-import", "Import modes…", Action::InspectImport, cx))
                    .child(self.button("modes-export", "Export custom modes…", Action::Export, cx)),
            );
            if let Some(preview) = s.import_preview.clone() {
                let stale = preview.baseline != exchange::subset(&s.settings);
                body = body.child(
                    div()
                        .id("modes-import-preview")
                        .role(Role::Group)
                        .aria_label("Mode import preview")
                        .child(gpui::text!(format!(
                            "{} new modes · {} exact duplicates · {} missing apps · {} conflicts",
                            preview.additions,
                            preview.duplicates,
                            preview.missing_apps,
                            preview.conflicts
                        )))
                        .child(
                            self.button(
                                "modes-confirm-import",
                                "Confirm import",
                                Action::Import(preview.clone()),
                                cx,
                            )
                            .disabled(
                                stale || preview.conflicts > 0 || self.controller.model.pending,
                            ),
                        ),
                );
            }
            body = body.child(gpui::text!("Selected mode")).child(gpui::text!(
                selected["name"]
                    .as_str()
                    .unwrap_or("Untitled mode")
                    .to_string()
            ));
            if custom {
                if self.editing.is_none() {
                    let e = cx.entity();
                    let m = selected.clone();
                    body = body.child(
                        Button::new("mode-edit", "Edit mode…")
                            .track_focus(&self.edit_focus)
                            .with_scroll_parent(Some(&self.scroll))
                            .on_click(move |_, w, cx| {
                                e.update(cx, |s, cx| {
                                    let mut values = m.clone();
                                    for (_, v) in values.as_object_mut().unwrap() {
                                        if v.is_null() {
                                            *v = json!("global")
                                        } else if v.is_boolean() {
                                            *v = json!(v.to_string())
                                        }
                                    }
                                    s.editor.update(cx, |f, cx| f.set(values, w, cx));
                                    s.editing_source = s
                                        .controller
                                        .model
                                        .snapshot
                                        .as_ref()
                                        .map_or(Value::Null, |v| v.settings["modes"].clone());
                                    s.editing = Some(m.clone());
                                    s.editor.update(cx, |f, cx| f.focus_first(w, cx));
                                    cx.notify()
                                })
                            }),
                    );
                } else {
                    self.editor
                        .update(cx, |f, _| f.scroll_parent = Some(self.scroll.clone()));
                    let e = cx.entity();
                    let cancel = e.clone();
                    body = body
                        .child(self.editor.clone())
                        .child(
                            Button::new("mode-save", "Save mode")
                                .with_scroll_parent(Some(&self.scroll))
                                .on_click(move |_, _, cx| e.update(cx, |s, cx| s.save(cx))),
                        )
                        .child(
                            Button::new("mode-cancel", "Cancel edit")
                                .with_scroll_parent(Some(&self.scroll))
                                .on_click(move |_, w, cx| {
                                    cancel.update(cx, |s, cx| s.cancel_edit(w, cx))
                                }),
                        );
                }
                let e = cx.entity();
                let id = self.selected.clone();
                let patch = s.patch_without(&id);
                let armed = self.confirm.as_deref() == Some(&id);
                body = body.child(
                    Button::new(
                        "mode-delete",
                        if armed {
                            "Confirm delete mode…"
                        } else {
                            "Delete mode…"
                        },
                    )
                    .intent(if armed {
                        ButtonIntent::Danger
                    } else {
                        ButtonIntent::Secondary
                    })
                    .disabled(self.controller.model.pending)
                    .with_scroll_parent(Some(&self.scroll))
                    .on_click(move |_, _, cx| {
                        e.update(cx, |s, cx| {
                            if s.confirm.as_deref() == Some(&id) {
                                s.selected = "builtin.everyday".into();
                                s.patch(patch.clone(), cx)
                            } else {
                                s.confirm = Some(id.clone());
                                cx.notify()
                            }
                        })
                    }),
                );
            } else {
                body=body.child(gpui::text!("Built-in modes are read-only. Duplicate one to change its style, model, language, or vocabulary."));
            }
            body = body.child(gpui::text!("Where it applies"));
            let profiles = s.settings["appProfiles"].as_array().unwrap();
            if profiles.is_empty() {
                body = body.child(self.button(
                    "mode-add-app",
                    "Add an app",
                    Action::AddApp {
                        mode_id: self.selected.clone(),
                    },
                    cx,
                ));
            }
            for profile in profiles {
                let bundle = profile["bundleId"].as_str().unwrap_or_default().to_string();
                let mut next = profiles.clone();
                let bound = profile["modeId"] == self.selected;
                for v in &mut next {
                    if v["bundleId"] == bundle {
                        v["modeId"] = if bound {
                            Value::Null
                        } else {
                            json!(self.selected)
                        }
                    }
                }
                body = body.child(self.button(
                    &format!("bind-{bundle}"),
                    &format!(
                            "{} {}",
                            if bound { "Unbind" } else { "Bind" },
                            profile["label"]
                                .as_str()
                                .filter(|s| !s.is_empty())
                                .unwrap_or(&bundle)
                        ),
                    Action::Patch(json!({"appProfiles":next})),
                    cx,
                ));
            }
            let enabled = s.settings["siteModeLookupEnabled"] == true;
            body=body.child(gpui::text!("Browser sites")).child(gpui::text!("Reads only the current URL from an allowed browser, immediately discards everything except its exact host, and never reads page text or history. A host is saved only when you add a rule.")).child(self.button("site-activation",if enabled{"Disable browser site mode rules"}else{"Use browser site mode rules"},Action::Patch(json!({"siteModeLookupEnabled":!enabled})),cx));
            if enabled {
                self.site
                    .update(cx, |f, _| f.scroll_parent = Some(self.scroll.clone()));
                let e = cx.entity();
                body = body
                    .child(self.site.clone())
                    .child(
                        Button::new(
                            "site-add",
                            if self.site_edit.is_some() {
                                "Save site rule"
                            } else {
                                "Add site rule"
                            },
                        )
                        .with_scroll_parent(Some(&self.scroll))
                        .on_click(move |_, _, cx| e.update(cx, |s, cx| s.add_site(cx))),
                    )
                    .child(self.button("site-probe", "Test current site", Action::Probe, cx));
                let rules = s.settings["browserSiteRules"].as_array().unwrap();
                for r in rules.iter().filter(|r| r["modeId"] == self.selected) {
                    let id = r["id"].as_str().unwrap().to_string();
                    let host = r["host"].as_str().unwrap();
                    let mut toggled = rules.clone();
                    for v in &mut toggled {
                        if v["id"] == id {
                            v["enabled"] = json!(v["enabled"] != true)
                        }
                    }
                    let removed: Vec<_> = rules.iter().filter(|v| v["id"] != id).cloned().collect();
                    let edit = cx.entity();
                    let remove = edit.clone();
                    let rule = r.clone();
                    let removal_id = format!("site:{id}");
                    let armed = self.confirm.as_deref() == Some(&removal_id);
                    body = body.child(
                        div()
                            .id(SharedString::from(format!("site-row-{id}")))
                            .child(gpui::text!(format!("{host} · {}", r["browserBundleId"])))
                            .child(self.button(
                                &format!("toggle-site-{id}"),
                                &format!(
                                    "{} {host}",
                                    if r["enabled"] == true {
                                        "Disable"
                                    } else {
                                        "Enable"
                                    }
                                ),
                                Action::Patch(json!({"browserSiteRules":toggled})),
                                cx,
                            ))
                            .child(Button::new(SharedString::from(format!("edit-site-{id}")),format!("Edit {host}…")).with_scroll_parent(Some(&self.scroll)).on_click(move|_,w,cx|edit.update(cx,|s,cx|{s.site.update(cx,|f,cx|f.set(json!({"host":rule["host"],"browser":rule["browserBundleId"]}),w,cx));s.site_edit=Some(rule.clone());s.site.update(cx,|f,cx|f.focus_first(w,cx));cx.notify()})))
                            .child(Button::new(SharedString::from(format!("remove-site-{id}")),format!("{} {host}…",if armed{"Confirm remove"}else{"Remove"})).intent(if armed{ButtonIntent::Danger}else{ButtonIntent::Secondary}).with_scroll_parent(Some(&self.scroll)).on_click(move|_,_,cx|remove.update(cx,|s,cx|{if s.confirm.as_deref()==Some(&removal_id){s.patch(json!({"browserSiteRules":removed}),cx)}else{s.confirm=Some(removal_id.clone());cx.notify()}}))),
                    );
                }
                if let Some(probe) = s.probe {
                    body = body.child(
                        div()
                            .id("mode-probe-result")
                            .role(Role::Status)
                            .child(gpui::text!(probe)),
                    );
                }
            }
            self.sample
                .update(cx, |f, _| f.scroll_parent = Some(self.scroll.clone()));
            let text = self.sample.read(cx).value("sample").to_string();
            body=body.child(gpui::text!("Before / after test")).child(gpui::text!("Preview stays in this window. It never copies, pastes, or injects into another app.")).child(self.sample.clone()).child(gpui::text!(format!("Before: {text}"))).child(gpui::text!(format!("After: {}",preview(&text,&selected))));
            if let Some(n) = s.notice {
                body = body.child(
                    div()
                        .id("modes-notice")
                        .role(Role::Status)
                        .child(gpui::text!(n)),
                );
            }
        } else {
            body = body.child(gpui::text!("Loading modes…"));
        }
        div()
            .id("modes")
            .capture_key_down(cx.listener(|s, e: &KeyDownEvent, w, cx| {
                if e.keystroke.key == "escape" && s.editing.is_some() && !e.prefer_character_input {
                    s.cancel_edit(w, cx);
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
