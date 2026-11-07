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
fn edited_site_rules(
    rules: &[Value],
    id: &str,
    field: &str,
    value: &str,
) -> Result<Option<Vec<Value>>, &'static str> {
    let Some(index) = rules.iter().position(|r| r["id"] == id) else {
        return Ok(None);
    };
    let mut updated = rules[index].clone();
    let key = match field {
        "host" => "host",
        "browser" => "browserBundleId",
        _ => return Ok(None),
    };
    let value = if field == "host" {
        normalize_host(value).ok_or("Enter an exact host such as github.com.")?
    } else {
        value.to_owned()
    };
    if updated[key] == value {
        return Ok(None);
    }
    updated[key] = json!(value);
    if rules.iter().enumerate().any(|(i, r)| {
        i != index
            && r["host"] == updated["host"]
            && r["browserBundleId"] == updated["browserBundleId"]
    }) {
        return Err("That browser and host already have a rule.");
    }
    let mut next = rules.to_vec();
    next[index] = updated;
    Ok(Some(next))
}
fn card(p: Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .border_1()
        .border_color(p.hairline_strong)
        .rounded(tokens::CARD_RADIUS)
        .bg(p.tint_raised)
        .shadow(p.card_shadow())
}
fn copy(title: impl Into<SharedString>, description: impl Into<SharedString>, p: Palette) -> Div {
    let title = title.into();
    let description = description.into();
    let title_id = SharedString::from(format!("mode-row-title-{title}"));
    let description_id = SharedString::from(format!("mode-row-description-{title}"));
    div()
        .flex_1()
        .min_w_0()
        .child(
            div()
                .font_weight(FontWeight::MEDIUM)
                .child(gpui::text!(id = title_id, title)),
        )
        .child(
            div()
                .mt(px(4.))
                .text_size(tokens::CAPTION_SIZE)
                .line_height(px(18.))
                .text_color(p.muted)
                .child(gpui::text!(id = description_id, description)),
        )
}
fn row(p: Palette) -> Div {
    div()
        .px(px(16.))
        .py(px(12.))
        .border_b_1()
        .border_color(p.hairline)
        .flex()
        .items_center()
        .gap(px(16.))
}
fn group(title: &'static str, content: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(
            div()
                .id(title)
                .role(Role::Heading)
                .aria_label(title)
                .px(px(16.))
                .text_size(px(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(content)
}
pub struct Modes {
    pub embedded: bool,
    _forms: Vec<Subscription>,
    pub controller: Controller<Snapshot, Action>,
    pub selected: String,
    pub editor: Entity<Form>,
    pub site: Entity<Form>,
    site_rows: std::collections::BTreeMap<String, Entity<Form>>,
    site_subscriptions: std::collections::BTreeMap<String, Vec<Subscription>>,
    site_sources: std::collections::BTreeMap<String, Value>,
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
            s.inline = true;
            s.set_placeholder("host", "github.com", w, cx);
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
            embedded: false,
            _forms: forms,
            controller: Controller::new(host),
            selected: "builtin.everyday".into(),
            editor,
            site,
            site_rows: Default::default(),
            site_subscriptions: Default::default(),
            site_sources: Default::default(),
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
        self.site_rows.clear();
        self.site_subscriptions.clear();
        self.site_sources.clear();
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
    fn save_site_row(&mut self, id: &str, field: &str, cx: &mut Context<Self>) {
        let Some(form) = self.site_rows.get(id) else {
            return;
        };
        let Some(snapshot) = self.controller.model.snapshot.as_ref() else {
            return;
        };
        let Some(rules) = snapshot.settings["browserSiteRules"].as_array() else {
            return;
        };
        let value = form.read(cx).value(field);
        let source_key = if field == "browser" {
            "browserBundleId"
        } else {
            "host"
        };
        if self
            .site_sources
            .get(id)
            .is_some_and(|source| source[source_key] == value)
        {
            return;
        }
        match edited_site_rules(rules, id, field, value) {
            Ok(Some(rules)) => self.patch(json!({"browserSiteRules":rules}), cx),
            Ok(None) => (),
            Err(notice) => {
                self.notice = Some(notice);
                cx.notify();
            }
        }
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
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
            .gap(px(32.))
            .when(!self.embedded, |d| {
                d.child(div().text_size(px(20.)).child(gpui::text!("Modes")))
            });
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
            let mut list = div().grid().grid_cols(3).gap(px(8.)).p(px(12.));
            for m in all {
                let id = m["id"].as_str().unwrap().to_string();
                let e = cx.entity();
                list = list.child(
                    Button::new(
                        SharedString::from(format!("mode-{id}")),
                        m["name"].as_str().unwrap_or("Untitled mode").to_string(),
                    )
                    .nav_row()
                    .palette(Palette {
                        tint_raised: p.surface.blend(p.accent.opacity(0.12)),
                        ..p
                    })
                    .padding_y(px(8.))
                    .bordered()
                    .description(if m["builtIn"] == true {
                        "Built-in"
                    } else if m["enabled"] == true {
                        "Enabled"
                    } else {
                        "Disabled"
                    })
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
            let mut library = card(p).child(list);
            let create = cx.entity();
            let duplicate = create.clone();
            let source = selected.clone();
            library = library
                .child(
                    row(p)
                        .child(copy(
                            "Custom mode",
                            "Start from your global settings, or duplicate a selected mode",
                            p,
                        ))
                        .child(
                            Button::new("mode-create", "Create mode")
                                .intent(ButtonIntent::Primary)
                                .with_scroll_parent(Some(&self.scroll))
                                .disabled(self.controller.model.pending)
                                .on_click(move |_, _, cx| {
                                    create.update(cx, |s, cx| s.create(None, cx))
                                }),
                        ),
                )
                .child(
                    row(p)
                        .child(copy(
                            "Import and export",
                            "Share custom modes with their app and site bindings as a JSON file",
                            p,
                        ))
                        .child(self.button(
                            "modes-import",
                            "Import modes…",
                            Action::InspectImport,
                            cx,
                        ))
                        .child(self.button(
                            "modes-export",
                            "Export custom modes…",
                            Action::Export,
                            cx,
                        )),
                );
            body = body.child(group("Library", library));
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
            let mut parts = vec![
                selected["writingStyle"]
                    .as_str()
                    .unwrap_or("global style")
                    .replace('_', " "),
            ];
            for field in ["modelId", "language"] {
                if let Some(value) = selected[field].as_str() {
                    parts.push(value.to_owned());
                }
            }
            if let Some(value) = selected["vocabularyPolicy"]
                .as_str()
                .filter(|s| *s != "inherit")
            {
                parts.push(format!("{value} vocabulary"));
            }
            if selected["contextPolicy"] == "project" {
                parts.push("project context".into());
            }
            if let Some(value) = selected["autoPaste"].as_bool() {
                parts.push(
                    if value {
                        "auto-paste"
                    } else {
                        "clipboard only"
                    }
                    .into(),
                );
            }
            let summary = parts.join(" · ");
            let summary = if custom {
                summary
            } else {
                format!("Built-in template · {summary}")
            };
            let mut selected_card = card(p).child(
                row(p)
                    .child(copy(
                        selected["name"]
                            .as_str()
                            .unwrap_or("Untitled mode")
                            .to_owned(),
                        summary,
                        p,
                    ))
                    .child(
                        Button::new("mode-duplicate", "Duplicate")
                            .size(crate::ButtonSize::Compact)
                            .with_scroll_parent(Some(&self.scroll))
                            .disabled(self.controller.model.pending)
                            .on_click(move |_, _, cx| {
                                duplicate.update(cx, |s, cx| s.create(Some(source.clone()), cx))
                            }),
                    ),
            );
            if custom {
                if self.editing.is_none() {
                    let e = cx.entity();
                    let m = selected.clone();
                    selected_card = selected_card.child(
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
                    selected_card = selected_card
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
                selected_card = selected_card.child(
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
                selected_card = selected_card.child(row(p).child(copy("Customize", "Built-in modes are read-only. Duplicate one to change its style, model, language, or vocabulary.", p)));
            }
            body = body.child(group("Selected mode", selected_card));
            let mut applies = card(p);
            let profiles = s.settings["appProfiles"].as_array().unwrap();
            if profiles.is_empty() {
                applies = applies.child(
                    row(p)
                        .child(copy(
                            "Bound applications",
                            "Add an app, then choose its Mode here.",
                            p,
                        ))
                        .child(self.button(
                            "mode-add-app",
                            "Add an app",
                            Action::AddApp {
                                mode_id: self.selected.clone(),
                            },
                            cx,
                        )),
                );
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
                applies = applies.child(self.button(
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
            let owner = cx.entity();
            applies=applies.child(row(p)
                .child(copy("Browser sites", "Reads only the current URL from an allowed browser, immediately discards everything except its exact host, and never reads page text or history. A host is saved only when you add a rule.", p))
                .child(crate::Toggle::new("site-activation","Use browser site mode rules",enabled).with_scroll_parent(&self.scroll).disabled(self.controller.model.pending).on_change(move |v,_,cx|owner.update(cx,|s,cx|s.patch(json!({"siteModeLookupEnabled":*v}),cx)))));
            if enabled {
                self.site
                    .update(cx, |f, _| f.scroll_parent = Some(self.scroll.clone()));
                let e = cx.entity();
                applies = applies.child(
                    div()
                        .px(px(16.))
                        .py(px(12.))
                        .border_b_1()
                        .border_color(p.hairline)
                        .child(copy(
                            "Add a site rule",
                            format!(
                                "Use {} on an exact host in one browser",
                                selected["name"].as_str().unwrap_or("this mode")
                            ),
                            p,
                        ))
                        .child(
                            div()
                                .mt(px(12.))
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .child(
                                    div()
                                        .flex_1()
                                        .max_w(px(440.))
                                        .min_w_0()
                                        .child(self.site.clone()),
                                )
                                .child(
                                    Button::new(
                                        "site-add",
                                        if self.site_edit.is_some() {
                                            "Save site rule"
                                        } else {
                                            "Add site rule"
                                        },
                                    )
                                    .caption(if self.site_edit.is_some() {
                                        "Save"
                                    } else {
                                        "Add"
                                    })
                                    .with_scroll_parent(Some(&self.scroll))
                                    .on_click(move |_, _, cx| e.update(cx, |s, cx| s.add_site(cx))),
                                ),
                        ),
                );
                let rules = s.settings["browserSiteRules"].as_array().unwrap();
                self.site_rows
                    .retain(|id, _| rules.iter().any(|r| r["id"] == *id));
                self.site_subscriptions
                    .retain(|id, _| self.site_rows.contains_key(id));
                self.site_sources
                    .retain(|id, _| self.site_rows.contains_key(id));
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
                    if !self.site_rows.contains_key(&id) {
                        let form = cx.new(|cx| {
                            Form::new(
                                vec![
                                    Spec::select("browser", "Site rule browser", &BROWSERS),
                                    Spec::text("host", "Site rule host"),
                                ],
                                w,
                                cx,
                            )
                        });
                        form.update(cx, |f, cx| {
                            f.inline = true;
                            f.set(
                                json!({"host":r["host"],"browser":r["browserBundleId"]}),
                                w,
                                cx,
                            );
                        });
                        let key = id.clone();
                        let change_subscription =
                            cx.subscribe(&form, move |s, _, event: &crate::form::Changed, cx| {
                                if event.0 == "browser" {
                                    s.save_site_row(&key, "browser", cx);
                                }
                            });
                        let focus = form.read(cx).control_focus("host", cx).unwrap();
                        let key = id.clone();
                        let blur_subscription = cx
                            .on_blur(&focus, w, move |s, _, cx| s.save_site_row(&key, "host", cx));
                        self.site_subscriptions
                            .insert(id.clone(), vec![change_subscription, blur_subscription]);
                        self.site_sources.insert(id.clone(), r.clone());
                        self.site_rows.insert(id.clone(), form);
                    }
                    let row_form = self.site_rows[&id].clone();
                    if self.site_sources.get(&id) != Some(r)
                        && !["host", "browser"]
                            .iter()
                            .filter_map(|key| row_form.read(cx).control_focus(key, cx))
                            .any(|f| f.is_focused(w))
                    {
                        row_form.update(cx, |f, cx| {
                            f.set(
                                json!({"host":r["host"],"browser":r["browserBundleId"]}),
                                w,
                                cx,
                            )
                        });
                        self.site_sources.insert(id.clone(), r.clone());
                    }
                    row_form.update(cx, |f, cx| {
                        f.set_label("host", format!("Host for {host}"), cx);
                        f.set_label("browser", format!("Browser for {host}"), cx);
                        f.scroll_parent = Some(self.scroll.clone());
                        f.disabled = self.controller.model.pending;
                    });
                    let toggle = cx.entity();
                    let remove = toggle.clone();
                    let removal_id = format!("site:{id}");
                    let armed = self.confirm.as_deref() == Some(&removal_id);
                    applies = applies.child(
                        div()
                            .id(SharedString::from(format!("site-row-{id}")))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .px(px(16.))
                            .py(px(12.))
                            .border_b_1()
                            .border_color(p.hairline)
                            .child(
                                crate::Toggle::new(
                                    SharedString::from(format!("toggle-site-{id}")),
                                    format!("Enable {host}"),
                                    r["enabled"] == true,
                                )
                                .with_scroll_parent(&self.scroll)
                                .disabled(self.controller.model.pending)
                                .on_change(move |_, _, cx| {
                                    toggle.update(cx, |s, cx| {
                                        s.patch(json!({"browserSiteRules":toggled}), cx)
                                    })
                                }),
                            )
                            .child(div().flex_1().min_w_0().child(row_form))
                            .child(
                                Button::new(
                                    SharedString::from(format!("remove-site-{id}")),
                                    format!(
                                        "{} {host}…",
                                        if armed { "Confirm remove" } else { "Remove" }
                                    ),
                                )
                                .caption(if armed {
                                    "Confirm remove…"
                                } else {
                                    "Remove…"
                                })
                                .destructive_text()
                                .bordered()
                                .with_scroll_parent(Some(&self.scroll))
                                .on_click(move |_, _, cx| {
                                    remove.update(cx, |s, cx| {
                                        if s.confirm.as_deref() == Some(&removal_id) {
                                            s.patch(json!({"browserSiteRules":removed}), cx)
                                        } else {
                                            s.confirm = Some(removal_id.clone());
                                            cx.notify();
                                        }
                                    })
                                }),
                            ),
                    );
                }
                applies = applies.child(
                    row(p)
                        .child(copy(
                            "Current site",
                            "Detects the frontmost browser host within five seconds",
                            p,
                        ))
                        .child(self.button("site-probe", "Test current site", Action::Probe, cx)),
                );
                if let Some(probe) = s.probe {
                    applies = applies.child(
                        div()
                            .id("mode-probe-result")
                            .role(Role::Status)
                            .child(gpui::text!(probe)),
                    );
                }
            }
            body = body.child(group("Where it applies", applies));
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
            .p(px(if self.embedded { 0. } else { 24. }))
            .bg(p.surface)
            .text_color(p.text)
            .text_size(tokens::BODY_SIZE)
            .line_height(tokens::BODY_LEADING)
            .child(body)
    }
}
