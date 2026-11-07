//! Remaining Customize sheets. Settings and repository owners remain authoritative.
#[cfg(any(test, feature = "gallery"))]
pub mod fixtures;
pub mod host;
pub mod model;
#[cfg(test)]
mod tests;
pub mod vocabulary;
use crate::{
    Button,
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
pub struct Back;
pub struct Customize {
    pub controller: Controller<Snapshot, Action>,
    pub page: Page,
    pub editor: Entity<Form>,
    pub tester: Entity<Form>,
    pub search: Entity<Form>,
    editing: Option<Value>,
    edit_source: Value,
    baseline: Value,
    saving: bool,
    discard: bool,
    confirm: Option<String>,
    scroll: ScrollHandle,
    terms_page: usize,
    notice: Option<&'static str>,
    return_focus: Option<FocusHandle>,
    pub return_to: Option<FocusHandle>,
    _subscriptions: Vec<Subscription>,
    editor_subscription: Subscription,
    #[cfg(feature = "production-host")]
    pending_mode: Option<String>,
    #[cfg(feature = "production-host")]
    return_mode: bool,
}
impl EventEmitter<Back> for Customize {}
fn specs(page: Page) -> Vec<Spec> {
    match page {
        Page::Spellings => vec![
            Spec::text("written", "Murmur types"),
            Spec::text("aliases", "Spoken aliases (comma separated)"),
            Spec::boolean("enabled", "Enable spelling"),
        ],
        Page::Commands => vec![
            Spec::select(
                "kind",
                "Voice command type",
                &[
                    ("text_replacement", "Text replacement"),
                    ("snippet", "Multiline snippet"),
                ],
            ),
            Spec::text("phrase", "Spoken phrase"),
            Spec {
                multiline: true,
                ..Spec::text("content", "Command content")
            },
            Spec::select(
                "scope",
                "Voice command scope",
                &[("global", "All apps"), ("app", "One configured app")],
            ),
            Spec::text("bundleId", "Configured app bundle ID"),
            Spec::boolean("enabled", "Enable command"),
            Spec::boolean("allowClipboard", "Allow this snippet to read clipboard"),
        ],
        Page::Apps => {
            let mut specs = vec![
                Spec::text("bundleId", "App bundle ID"),
                Spec::text("label", "App label"),
                Spec::select(
                    "writingStyle",
                    "App writing style",
                    &[
                        ("global", "Use global"),
                        ("conversational", "Conversational"),
                        ("polished", "Polished prose"),
                        ("code_technical", "Code / technical"),
                        ("verbatim", "Verbatim"),
                        ("notes", "Notes"),
                    ],
                ),
                Spec::boolean("ideContextEnabled", "Local IDE symbols & @file"),
            ];
            for (k, l) in [
                ("autoPasteOverride", "Auto-paste override"),
                ("cleanupOverride", "Cleanup override"),
                ("smartFormattingOverride", "Smart formatting override"),
                ("cliFormattingOverride", "Command formatting override"),
            ] {
                specs.push(Spec::select(
                    k,
                    l,
                    &[
                        ("global", "Use global setting"),
                        ("true", "Always"),
                        ("false", "Never"),
                    ],
                ));
            }
            specs
        }
        Page::Scan => vec![],
    }
}
fn edit_source(page: Page, s: &Snapshot) -> Value {
    match page {
        Page::Spellings => {
            json!({"vocabularyEntries":s.settings["vocabularyEntries"],"voiceCommands":s.settings["voiceCommands"]})
        }
        Page::Apps => s.settings["appProfiles"].clone(),
        Page::Commands => Value::Null,
        Page::Scan => Value::Null,
    }
}
impl Customize {
    #[cfg(feature = "production-host")]
    pub fn bind_app(&mut self, mode: String, w: &mut Window, cx: &mut Context<Self>) {
        self.set_page(Page::Apps, w, cx);
        self.pending_mode = Some(mode);
        self.return_mode = true;
        cx.notify();
    }
    pub fn new(host: Rc<host::Host>, w: &mut Window, cx: &mut Context<Self>) -> Self {
        let editor = cx.new(|cx| Form::new(specs(Page::Spellings), w, cx));
        let tester = cx.new(|cx| {
            Form::new(
                vec![
                    Spec {
                        multiline: true,
                        ..Spec::text("text", "Preview input")
                    },
                    Spec::boolean("cli", "Include final CLI formatting"),
                    Spec::boolean("readClipboard", "Allow clipboard reading for this preview"),
                ],
                w,
                cx,
            )
        });
        tester.update(cx, |f, cx| {
            f.set(
                json!({"text":"npm run Tori dev","cli":"true","readClipboard":"false"}),
                w,
                cx,
            )
        });
        let search = cx.new(|cx| {
            Form::new(
                vec![
                    Spec::text("query", "Search commands or terms"),
                    Spec::select(
                        "sort",
                        "Term sort",
                        &[("freq", "Frequency"), ("alpha", "A–Z")],
                    ),
                ],
                w,
                cx,
            )
        });
        search.update(cx, |f, cx| f.set(json!({"query":"","sort":"freq"}), w, cx));
        let editor_subscription = cx.observe(&editor, |_, _, cx| cx.notify());
        let subscriptions = vec![
            cx.observe(&tester, |_, _, cx| cx.notify()),
            cx.observe(&search, |s, _, cx| {
                s.terms_page = 0;
                cx.notify()
            }),
        ];
        Self {
            controller: Controller::new(host),
            page: Page::Spellings,
            editor,
            tester,
            search,
            editing: None,
            edit_source: Value::Null,
            baseline: Value::Null,
            saving: false,
            discard: false,
            confirm: None,
            scroll: ScrollHandle::new(),
            terms_page: 0,
            notice: None,
            return_focus: None,
            return_to: None,
            _subscriptions: subscriptions,
            editor_subscription,
            #[cfg(feature = "production-host")]
            pending_mode: None,
            #[cfg(feature = "production-host")]
            return_mode: false,
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
        self.confirm = None;
        cx.notify();
    }
    pub fn set_page(&mut self, page: Page, w: &mut Window, cx: &mut Context<Self>) {
        self.controller.detach();
        self.controller.attach(Action::Read);
        self.page = page;
        self.editing = None;
        self.confirm = None;
        self.notice = None;
        self.editor = cx.new(|cx| Form::new(specs(page), w, cx));
        self.editor_subscription = cx.observe(&self.editor, |_, _, cx| cx.notify());
        self.scroll.set_offset(point(px(0.), px(0.)));
        cx.notify();
    }
    fn request(&mut self, a: Action, cx: &mut Context<Self>) {
        self.controller.request(a);
        cx.notify();
    }
    fn button(
        &self,
        id: impl Into<String>,
        label: impl Into<String>,
        a: Action,
        cx: &mut Context<Self>,
    ) -> Button {
        let e = cx.entity();
        Button::new(SharedString::from(id.into()), label.into())
            .with_scroll_parent(Some(&self.scroll))
            .disabled(self.controller.model.pending)
            .on_click(move |_, _, cx| e.update(cx, |s, cx| s.request(a.clone(), cx)))
    }
    fn open(&mut self, entry: Value, w: &mut Window, cx: &mut Context<Self>) {
        self.return_focus = w.focused(cx);
        self.edit_source = self
            .controller
            .model
            .snapshot
            .as_ref()
            .map_or(Value::Null, |s| edit_source(self.page, s));
        self.discard = false;
        self.notice = None;
        let values = match self.page {
            Page::Spellings => {
                json!({"written":entry["written"].as_str().unwrap_or(""),"aliases":entry["aliases"].as_array().map_or(String::new(),|a|a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")),"enabled":entry["enabled"].as_bool().unwrap_or(true).to_string()})
            }
            Page::Commands => {
                json!({"kind":entry["voiceCommand"]["commandType"].as_str().unwrap_or("text_replacement"),"phrase":entry["payload"]["source"].as_str().or(entry["payload"]["trigger"].as_str()).unwrap_or(""),"content":entry["payload"]["replacement"].as_str().or(entry["payload"]["body"].as_str()).unwrap_or(""),"scope":if entry["scope"]["kind"]=="app"{"app"}else{"global"},"bundleId":entry["scope"]["bundleId"].as_str().unwrap_or(""),"enabled":entry["enabled"].as_bool().unwrap_or(true).to_string(),"allowClipboard":entry["voiceCommand"]["allowClipboardRead"].as_bool().unwrap_or(false).to_string()})
            }
            Page::Apps => {
                let mut v = entry.clone();
                if !v.is_object() {
                    v = profile("", "")
                }
                for (_, v) in v.as_object_mut().unwrap() {
                    if v.is_null() {
                        *v = json!("global")
                    } else if v.is_boolean() {
                        *v = json!(v.to_string())
                    }
                }
                v
            }
            Page::Scan => json!({}),
        };
        self.baseline = values.clone();
        self.editor.update(cx, |f, cx| {
            f.set(values, w, cx);
            if self.page == Page::Apps && entry["bundleId"].as_str().is_some_and(|s| !s.is_empty())
            {
                f.disabled_keys.insert("bundleId");
            }
            // A save that finished while the editor was closed leaves the
            // render-synced flag stale; sync it before seeding focus.
            f.disabled = self.controller.model.pending;
            f.focus_first(w, cx)
        });
        self.tester.update(cx, |f, cx| {
            f.disabled_keys.clear();
            let mut v = f.values.clone();
            v["readClipboard"] = json!("false");
            if self.page == Page::Commands {
                v["text"] = self.baseline["phrase"].clone();
            }
            f.set(v, w, cx)
        });
        self.editing = Some(entry);
        cx.notify();
    }
    fn cancel(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        if self.page == Page::Commands
            && !self.discard
            && self.editor.read(cx).values != self.baseline
        {
            self.discard = true;
            cx.notify();
            return;
        }
        self.editing = None;
        #[cfg(feature = "production-host")]
        if self.return_mode {
            self.return_mode = false;
            cx.emit(Back);
        }
        self.discard = false;
        self.controller.invalidate();
        if let Some(f) = self.return_focus.take() {
            w.focus(&f, cx)
        }
        cx.notify();
    }
    fn save(&mut self, cx: &mut Context<Self>) {
        let Some(entry) = self.editing.clone() else {
            return;
        };
        if self
            .controller
            .model
            .snapshot
            .as_ref()
            .is_some_and(|s| self.edit_source != edit_source(self.page, s))
        {
            self.notice =
                Some("State changed elsewhere. Cancel this edit and review the current value.");
            cx.notify();
            return;
        }
        let Some(snapshot) = self.controller.model.snapshot.clone() else {
            return;
        };
        let f = self.editor.read(cx);
        let action = match self.page {
            Page::Spellings => {
                let mut entries = snapshot.settings["vocabularyEntries"]
                    .as_array()
                    .unwrap()
                    .clone();
                let id = entry["id"].as_str().map(String::from).unwrap_or_else(|| {
                    format!(
                        "vocabulary-{}",
                        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
                    )
                });
                let next = json!({"id":id,"written":f.value("written"),"aliases":f.value("aliases").split(',').map(str::trim).filter(|s|!s.is_empty()).collect::<Vec<_>>(),"enabled":f.value("enabled")!="false","scope":entry.get("scope").cloned().unwrap_or(json!({"kind":"global"}))});
                if let Some(v) = entries.iter_mut().find(|v| v["id"] == id) {
                    *v = next
                } else {
                    entries.push(next)
                }
                if vocabulary::validate(
                    &entries,
                    snapshot.settings["voiceCommands"].as_array().unwrap(),
                )
                .is_err()
                {
                    self.notice = Some(
                        "Review spelling values, alias conflicts, and reserved command phrases.",
                    );
                    cx.notify();
                    return;
                }
                Action::Patch(json!({"vocabularyEntries":entries}))
            }
            Page::Commands => {
                let draft = match command_draft(&f.values, Some(&entry)) {
                    Ok(d) => d,
                    Err(_) => {
                        self.notice = Some(
                            "Enter a phrase, valid snippet variables, and explicit clipboard permission when required.",
                        );
                        cx.notify();
                        return;
                    }
                };
                if draft["scope"]["kind"] == "app"
                    && !snapshot.settings["appProfiles"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|p| p["bundleId"] == draft["scope"]["bundleId"])
                {
                    self.notice = Some("Choose a configured app profile.");
                    cx.notify();
                    return;
                }
                Action::Upsert(draft)
            }
            Page::Apps => {
                let mut next = entry.clone();
                if !next.is_object() {
                    next = profile(f.value("bundleId").trim(), f.value("label").trim())
                }
                let mut profiles = snapshot.settings["appProfiles"].as_array().unwrap().clone();
                let id = f.value("bundleId").trim();
                if id.is_empty()
                    || !id.contains('.')
                    || profiles.iter().any(|p| {
                        p["bundleId"]
                            .as_str()
                            .is_some_and(|b| b.eq_ignore_ascii_case(id))
                            && entry["bundleId"] != p["bundleId"]
                    })
                {
                    self.notice = Some("Enter a unique app bundle ID.");
                    cx.notify();
                    return;
                }
                next["bundleId"] = json!(id);
                next["label"] = json!(f.value("label"));
                for key in [
                    "autoPasteOverride",
                    "cleanupOverride",
                    "smartFormattingOverride",
                    "cliFormattingOverride",
                    "writingStyle",
                    "ideContextEnabled",
                ] {
                    next[key] = match f.value(key) {
                        "global" => Value::Null,
                        "true" => json!(true),
                        "false" => json!(false),
                        s => json!(s),
                    }
                }
                if let Some(p) = profiles.iter_mut().find(|p| p["bundleId"] == id) {
                    *p = next
                } else {
                    profiles.push(next)
                }
                Action::Patch(json!({"appProfiles":profiles}))
            }
            Page::Scan => return,
        };
        self.saving = true;
        self.request(action, cx);
    }
    fn preview(&mut self, cx: &mut Context<Self>) {
        let Some(snapshot) = &self.controller.model.snapshot else {
            return;
        };
        let t = self.tester.read(cx);
        let action = if self.page == Page::Commands {
            let Ok(draft) = command_draft(&self.editor.read(cx).values, self.editing.as_ref())
            else {
                self.notice = Some("Review the command before previewing.");
                cx.notify();
                return;
            };
            Action::PreviewCommand {
                draft,
                text: t.value("text").into(),
                read_clipboard: t.value("readClipboard") == "true",
            }
        } else {
            Action::PreviewAliases {
                entries: snapshot.settings["vocabularyEntries"].clone(),
                voice_commands: snapshot.settings["voiceCommands"].clone(),
                text: t.value("text").into(),
                cli: t.value("cli") == "true",
            }
        };
        self.request(action, cx);
    }
    fn delete(&mut self, key: String, a: Action, cx: &mut Context<Self>) {
        if self.confirm.as_ref() == Some(&key) {
            self.confirm = None;
            self.request(a, cx)
        } else {
            self.confirm = Some(key);
            cx.notify();
        }
    }
}
impl Render for Customize {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.controller.drain();
        if self.saving && !self.controller.model.pending {
            self.saving = false;
            if self.controller.model.problem.is_none() {
                self.editing = None;
                #[cfg(feature = "production-host")]
                if self.return_mode {
                    self.return_mode = false;
                    cx.emit(Back);
                }
                if let Some(f) = self.return_focus.take() {
                    w.focus(&f, cx)
                }
            }
        }
        #[cfg(feature = "production-host")]
        if self.controller.model.snapshot.is_some()
            && let Some(mode) = self.pending_mode.take()
        {
            let mut entry = profile("", "");
            entry["modeId"] = json!(mode);
            self.open(entry, w, cx);
        }
        let p = Palette::get(cx);
        let e = cx.entity();
        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                Button::new("customize-back", "Back to Knowledge")
                    .with_scroll_parent(Some(&self.scroll))
                    .on_click(move |_, w, cx| {
                        e.update(cx, |s, cx| {
                            s.hide(cx);
                            if let Some(f) = s.return_to.take() {
                                w.focus(&f, cx)
                            }
                            crate::events::publish(cx, Back)
                        })
                    }),
            )
            .child(
                div()
                    .text_size(px(20.))
                    .child(gpui::text!(self.page.label())),
            );
        let mut nav = div().flex().flex_wrap().gap(px(8.));
        for page in Page::ALL {
            let e = cx.entity();
            nav = nav.child(
                Button::new(
                    SharedString::from(format!("customize-{page:?}")),
                    page.label(),
                )
                .with_scroll_parent(Some(&self.scroll))
                .toggled(self.page == page)
                .on_click(move |_, w, cx| e.update(cx, |s, cx| s.set_page(page, w, cx))),
            );
        }
        body = body.child(nav);
        if let Some(problem) = self.controller.model.problem {
            body = body
                .child(
                    div()
                        .id("customize-error")
                        .role(Role::Alert)
                        .child(gpui::text!(problem.message())),
                )
                .child(self.button("customize-retry", "Retry", Action::Read, cx));
        }
        if let Some(n) = self.notice {
            body = body.child(
                div()
                    .id("customize-notice")
                    .role(Role::Alert)
                    .child(gpui::text!(n)),
            );
        }
        if let Some(snapshot) = self.controller.model.snapshot.clone() {
            if self.editing.is_some() {
                self.editor.update(cx, |f, _| {
                    f.scroll_parent = Some(self.scroll.clone());
                    f.disabled = self.controller.model.pending
                });
                body = body.child(
                    div()
                        .id("customize-editor")
                        .role(Role::Dialog)
                        .aria_label(format!("Edit {}", self.page.label()))
                        .child(self.editor.clone()),
                );
                if self.discard {
                    let e = cx.entity();
                    let keep = e.clone();
                    body = body
                        .child(gpui::text!("Discard changes?"))
                        .child(
                            Button::new("discard-command", "Discard changes")
                                .with_scroll_parent(Some(&self.scroll))
                                .on_click(move |_, w, cx| e.update(cx, |s, cx| s.cancel(w, cx))),
                        )
                        .child(
                            Button::new("keep-command", "Keep editing")
                                .with_scroll_parent(Some(&self.scroll))
                                .on_click(move |_, _, cx| {
                                    keep.update(cx, |s, cx| {
                                        s.discard = false;
                                        cx.notify()
                                    })
                                }),
                        );
                } else {
                    let save = cx.entity();
                    let cancel = save.clone();
                    body = body
                        .child(
                            Button::new("customize-save", "Save")
                                .disabled(self.controller.model.pending)
                                .with_scroll_parent(Some(&self.scroll))
                                .on_click(move |_, _, cx| save.update(cx, |s, cx| s.save(cx))),
                        )
                        .child(
                            Button::new("customize-cancel", "Cancel")
                                .with_scroll_parent(Some(&self.scroll))
                                .disabled(self.controller.model.pending)
                                .on_click(move |_, w, cx| {
                                    cancel.update(cx, |s, cx| s.cancel(w, cx))
                                }),
                        );
                    if self.page == Page::Commands {
                        let e = cx.entity();
                        self.tester
                            .update(cx, |f, _| f.scroll_parent = Some(self.scroll.clone()));
                        body=body.child(gpui::text!("Preview never writes to clipboard or pastes. Clipboard reading needs separate permission for this preview.")).child(self.tester.clone()).child(Button::new("command-preview","Preview command").with_scroll_parent(Some(&self.scroll)).disabled(self.controller.model.pending).on_click(move|_,_,cx|e.update(cx,|s,cx|s.preview(cx))));
                    }
                }
            } else {
                match self.page {
                    Page::Spellings => {
                        let e = cx.entity();
                        body=body.child(gpui::text!("For reliable replacement, enter the words Murmur actually hears on the left. Separate variants with commas.")).child(Button::new("spelling-add","Add spelling").with_scroll_parent(Some(&self.scroll)).on_click(move|_,w,cx|e.update(cx,|s,cx|s.open(Value::Null,w,cx)))).child(self.button("spellings-import","Import saved spellings…",Action::InspectSpellings,cx)).child(self.button("spellings-export","Export saved spellings…",Action::ExportSpellings,cx));
                        let entries = snapshot.settings["vocabularyEntries"].as_array().unwrap();
                        if entries.is_empty() {
                            body=body.child(gpui::text!("Everything spelled correctly? Add a name or unusual word when Murmur gets it wrong."));
                        }
                        for v in entries {
                            let id = v["id"].as_str().unwrap().to_string();
                            let edit = cx.entity();
                            let delete = edit.clone();
                            let entry = v.clone();
                            let remaining: Vec<_> =
                                entries.iter().filter(|p| p["id"] != id).cloned().collect();
                            let mut toggled = entries.clone();
                            for p in &mut toggled {
                                if p["id"] == id {
                                    p["enabled"] = json!(p["enabled"] != true)
                                }
                            }
                            let key = format!("spelling:{id}");
                            let armed = self.confirm.as_ref() == Some(&key);
                            body = body.child(
                                div()
                                    .id(SharedString::from(format!("spelling-row-{id}")))
                                    .child(gpui::text!(v["written"].as_str().unwrap().to_string()))
                                    .child(gpui::text!(v["aliases"].to_string()))
                                    .child(
                                        Button::new(
                                            SharedString::from(format!("spelling-edit-{id}")),
                                            "Edit spelling…",
                                        )
                                        .with_scroll_parent(Some(&self.scroll))
                                        .on_click(
                                            move |_, w, cx| {
                                                edit.update(cx, |s, cx| {
                                                    s.open(entry.clone(), w, cx)
                                                })
                                            },
                                        ),
                                    )
                                    .child(self.button(
                                        format!("spelling-enable-{id}"),
                                        if v["enabled"] == true {
                                            "Disable spelling"
                                        } else {
                                            "Enable spelling"
                                        },
                                        Action::Patch(json!({"vocabularyEntries":toggled})),
                                        cx,
                                    ))
                                    .child(
                                        Button::new(
                                            SharedString::from(format!("spelling-delete-{id}")),
                                            if armed {
                                                "Confirm remove spelling…"
                                            } else {
                                                "Remove spelling…"
                                            },
                                        )
                                        .with_scroll_parent(Some(&self.scroll))
                                        .on_click(
                                            move |_, _, cx| {
                                                delete.update(cx, |s, cx| {
                                                    s.delete(
                                                        key.clone(),
                                                        Action::Patch(
                                                            json!({"vocabularyEntries":remaining}),
                                                        ),
                                                        cx,
                                                    )
                                                })
                                            },
                                        ),
                                    ),
                            );
                        }
                        self.tester.update(cx, |f, _| {
                            f.disabled_keys.insert("readClipboard");
                            f.scroll_parent = Some(self.scroll.clone());
                        });
                        let e = cx.entity();
                        body = body.child(self.tester.clone()).child(
                            Button::new("alias-preview", "Preview spellings")
                                .with_scroll_parent(Some(&self.scroll))
                                .on_click(move |_, _, cx| e.update(cx, |s, cx| s.preview(cx))),
                        );
                        if let Some(preview) = &snapshot.import_preview {
                            let stale = vocabulary::commit(preview, &snapshot.settings).is_err();
                            body = body.child(
                                div()
                                    .id("spelling-import-preview")
                                    .role(Role::Group)
                                    .aria_label("Spelling import preview")
                                    .child(gpui::text!(format!(
                                        "{} new · {} duplicates · {} conflicts",
                                        preview.added, preview.duplicates, preview.conflicts
                                    )))
                                    .child(
                                        self.button(
                                            "spelling-import-confirm",
                                            "Confirm spelling import",
                                            Action::ImportSpellings(preview.clone()),
                                            cx,
                                        )
                                        .disabled(stale || self.controller.model.pending),
                                    )
                                    .child(self.button(
                                        "spelling-import-cancel",
                                        "Cancel import",
                                        Action::Cancel,
                                        cx,
                                    )),
                            );
                        }
                    }
                    Page::Commands => {
                        self.search
                            .update(cx, |f, _| f.scroll_parent = Some(self.scroll.clone()));
                        if snapshot.settings["voiceCommandsEnabled"] != true {
                            body=body.child(gpui::text!("Voice commands are saved but do not run while Voice commands & spoken formatting is off.")).child(self.button("voice-command-settings","Open voice command settings",Action::VoiceCommandSettings,cx));
                        }
                        let e = cx.entity();
                        body = body
                            .child(
                                Button::new("command-add", "Create voice command…")
                                    .with_scroll_parent(Some(&self.scroll))
                                    .on_click(move |_, w, cx| {
                                        e.update(cx, |s, cx| s.open(Value::Null, w, cx))
                                    }),
                            )
                            .child(self.search.clone())
                            .child(self.button(
                                "commands-search",
                                "Search voice commands",
                                Action::ListCommands {
                                    query: self.search.read(cx).value("query").into(),
                                    offset: 0,
                                },
                                cx,
                            ));
                        if snapshot.commands.is_empty() {
                            body = body.child(gpui::text!("No voice commands yet."));
                        }
                        for v in snapshot.commands {
                            let id = v["id"].as_str().unwrap().to_string();
                            let edit = cx.entity();
                            let duplicate = edit.clone();
                            let delete = edit.clone();
                            let entry = v.clone();
                            let copy = duplicate_command(&v);
                            let key = format!("command:{id}");
                            let armed = self.confirm.as_ref() == Some(&key);
                            body = body.child(
                                div()
                                    .id(SharedString::from(format!("command-row-{id}")))
                                    .child(gpui::text!(
                                        crate::knowledge::model::title(&v).to_string()
                                    ))
                                    .child(gpui::text!(crate::knowledge::model::detail(&v)))
                                    .child(
                                        Button::new(
                                            SharedString::from(format!("command-edit-{id}")),
                                            "Edit voice command…",
                                        )
                                        .with_scroll_parent(Some(&self.scroll))
                                        .on_click(
                                            move |_, w, cx| {
                                                edit.update(cx, |s, cx| {
                                                    s.open(entry.clone(), w, cx)
                                                })
                                            },
                                        ),
                                    )
                                    .child(
                                        Button::new(
                                            SharedString::from(format!("command-duplicate-{id}")),
                                            "Duplicate voice command",
                                        )
                                        .with_scroll_parent(Some(&self.scroll))
                                        .on_click(
                                            move |_, w, cx| {
                                                duplicate
                                                    .update(cx, |s, cx| s.open(copy.clone(), w, cx))
                                            },
                                        ),
                                    )
                                    .child(self.button(
                                        format!("command-enable-{id}"),
                                        if v["enabled"] == true {
                                            "Disable command"
                                        } else {
                                            "Enable command"
                                        },
                                        Action::Enable {
                                            entry: v.clone(),
                                            enabled: v["enabled"] != true,
                                        },
                                        cx,
                                    ))
                                    .child(
                                        Button::new(
                                            SharedString::from(format!("command-delete-{id}")),
                                            if armed {
                                                "Confirm delete command…"
                                            } else {
                                                "Delete command…"
                                            },
                                        )
                                        .with_scroll_parent(Some(&self.scroll))
                                        .on_click(
                                            move |_, _, cx| {
                                                delete.update(cx, |s, cx| {
                                                    s.delete(
                                                        key.clone(),
                                                        Action::Delete(v.clone()),
                                                        cx,
                                                    )
                                                })
                                            },
                                        ),
                                    ),
                            );
                        }
                        if let Some(offset) = snapshot.next_offset {
                            body = body.child(self.button(
                                "commands-next",
                                "Next commands",
                                Action::ListCommands {
                                    query: self.search.read(cx).value("query").into(),
                                    offset,
                                },
                                cx,
                            ));
                        }
                    }
                    Page::Apps => {
                        let e = cx.entity();
                        body = body
                            .child(
                                Button::new("app-add-manual", "Add app manually…")
                                    .with_scroll_parent(Some(&self.scroll))
                                    .on_click(move |_, w, cx| {
                                        e.update(cx, |s, cx| s.open(Value::Null, w, cx))
                                    }),
                            )
                            .child(self.button(
                                "apps-running",
                                "Choose running app…",
                                Action::RunningApps,
                                cx,
                            ));
                        for app in snapshot.running_apps {
                            let e = cx.entity();
                            let next = profile(
                                app["bundleId"].as_str().unwrap(),
                                app["name"].as_str().unwrap(),
                            );
                            body = body.child(
                                Button::new(
                                    SharedString::from(format!("running-{}", app["bundleId"])),
                                    format!("Add {}", app["name"].as_str().unwrap()),
                                )
                                .with_scroll_parent(Some(&self.scroll))
                                .on_click(move |_, w, cx| {
                                    e.update(cx, |s, cx| s.open(next.clone(), w, cx))
                                }),
                            );
                        }
                        let profiles = snapshot.settings["appProfiles"].as_array().unwrap();
                        for profile in profiles {
                            let bundle = profile["bundleId"].as_str().unwrap().to_string();
                            let edit = cx.entity();
                            let delete = edit.clone();
                            let entry = profile.clone();
                            let remaining: Vec<_> = profiles
                                .iter()
                                .filter(|p| p["bundleId"] != bundle)
                                .cloned()
                                .collect();
                            let key = format!("app:{bundle}");
                            let armed = self.confirm.as_ref() == Some(&key);
                            let mut row = div()
                                .id(SharedString::from(format!("app-row-{bundle}")))
                                .p(px(12.))
                                .border_1()
                                .border_color(p.border)
                                .rounded(tokens::CARD_RADIUS)
                                .child(gpui::text!(
                                    profile["label"]
                                        .as_str()
                                        .filter(|s| !s.is_empty())
                                        .unwrap_or(&bundle)
                                        .to_string()
                                ))
                                .child(gpui::text!(bundle.clone()))
                                .child(
                                    Button::new(
                                        SharedString::from(format!("app-edit-{bundle}")),
                                        "Edit app overrides…",
                                    )
                                    .with_scroll_parent(Some(&self.scroll))
                                    .on_click(
                                        move |_, w, cx| {
                                            edit.update(cx, |s, cx| s.open(entry.clone(), w, cx))
                                        },
                                    ),
                                )
                                .child(
                                    Button::new(
                                        SharedString::from(format!("app-delete-{bundle}")),
                                        if armed {
                                            "Confirm remove app…"
                                        } else {
                                            "Remove app…"
                                        },
                                    )
                                    .with_scroll_parent(Some(&self.scroll))
                                    .on_click(
                                        move |_, _, cx| {
                                            delete.update(cx, |s, cx| {
                                                s.delete(
                                                    key.clone(),
                                                    Action::Patch(json!({"appProfiles":remaining})),
                                                    cx,
                                                )
                                            })
                                        },
                                    ),
                                );
                            if profile["ideContextEnabled"] == true {
                                row=row.child(gpui::text!("Project symbols stay in memory. No screen, selection, or clipboard context is read.")).child(self.button(format!("root-add-{bundle}"),"Add project root…",Action::ChooseRoot(bundle.clone()),cx).disabled(profile["ideProjectRoots"].as_array().is_some_and(|a|a.len()>=4)||self.controller.model.pending)).child(self.button(format!("ide-refresh-{bundle}"),"Refresh index",Action::RefreshIde(bundle.clone()),cx)).child(self.button(format!("ide-clear-{bundle}"),"Clear index",Action::ClearIde(bundle.clone()),cx));
                                for root in
                                    profile["ideProjectRoots"].as_array().into_iter().flatten()
                                {
                                    let mut next = profiles.clone();
                                    for p in &mut next {
                                        if p["bundleId"] == bundle {
                                            p["ideProjectRoots"]
                                                .as_array_mut()
                                                .unwrap()
                                                .retain(|r| r != root)
                                        }
                                    }
                                    row = row
                                        .child(gpui::text!(
                                            id =
                                                SharedString::from(format!("root-{bundle}-{root}")),
                                            root.as_str().unwrap().to_string()
                                        ))
                                        .child(self.button(
                                            format!("remove-root-{bundle}-{root}"),
                                            "Remove root…",
                                            Action::Patch(json!({"appProfiles":next})),
                                            cx,
                                        ));
                                }
                                if let Some(status) = snapshot.ide.get(&bundle) {
                                    row = row.child(gpui::text!(format!(
                                        "{} · {} files · {} symbols",
                                        status["state"], status["files"], status["symbols"]
                                    )));
                                }
                            }
                            body = body.child(row);
                        }
                    }
                    Page::Scan => {
                        let folder = snapshot.settings["codeVocabFolder"]
                            .as_str()
                            .unwrap_or("")
                            .to_string();
                        let scanning = snapshot.scan["state"] == "scanning";
                        let scan_id = snapshot.scan["id"].as_str().unwrap_or("").to_string();
                        body=body.child(gpui::text!("Built-in dev terms active. Scan this folder to add your own identifiers.")).child(gpui::text!(folder.clone())).child(self.button("project-folder","Choose project folder…",Action::ChooseProject,cx)).child(self.button("project-scan",if scanning{"Scanning…"}else{"Scan folder"},Action::Scan{folder:folder.clone(),id:format!("scan-{}",chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default())},cx).disabled(scanning||folder.is_empty()||self.controller.model.pending));
                        if scanning {
                            body = body.child(self.button(
                                "project-scan-cancel",
                                "Cancel scan",
                                Action::CancelScan(scan_id),
                                cx,
                            ));
                        }
                        body = body.child(div().id("scan-status").role(Role::Status).child(
                            gpui::text!(format!(
                                "{} · {} files · {} terms",
                                snapshot.scan["state"],
                                snapshot.scan["filesRead"],
                                snapshot.scan["terms"]
                            )),
                        ));
                        if snapshot.scan["capped"] == true {
                            body=body.child(gpui::text!("The 1,000-file cap was reached. Choose a smaller folder for complete coverage."));
                        }
                        self.search
                            .update(cx, |f, _| f.scroll_parent = Some(self.scroll.clone()));
                        body = body.child(self.search.clone());
                        let q = self.search.read(cx).value("query").to_lowercase();
                        let mut terms: Vec<_> = snapshot.scan["rankedTerms"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter(|t| {
                                t["term"].as_str().unwrap_or("").to_lowercase().contains(&q)
                            })
                            .cloned()
                            .collect();
                        if self.search.read(cx).value("sort") == "alpha" {
                            terms.sort_by_key(|v| v["term"].as_str().unwrap_or("").to_lowercase());
                        }
                        let copy = terms
                            .iter()
                            .filter_map(|v| v["term"].as_str().map(String::from))
                            .collect();
                        body = body.child(self.button(
                            "copy-vocab-terms",
                            "Copy terms",
                            Action::CopyTerms(copy),
                            cx,
                        ));
                        for term in terms.iter().skip(self.terms_page * 50).take(50) {
                            body = body.child(gpui::text!(
                                id = SharedString::from(format!("term-{}", term["term"])),
                                format!(
                                    "{} · {}",
                                    term["term"].as_str().unwrap_or(""),
                                    term["frequency"]
                                )
                            ));
                        }
                        if self.terms_page > 0 {
                            let e = cx.entity();
                            body = body.child(
                                Button::new("terms-prev", "Previous terms")
                                    .with_scroll_parent(Some(&self.scroll))
                                    .on_click(move |_, _, cx| {
                                        e.update(cx, |s, cx| {
                                            s.terms_page = s.terms_page.saturating_sub(1);
                                            cx.notify()
                                        })
                                    }),
                            );
                        }
                        if (self.terms_page + 1) * 50 < terms.len() {
                            let e = cx.entity();
                            body = body.child(
                                Button::new("terms-next", "Next terms")
                                    .with_scroll_parent(Some(&self.scroll))
                                    .on_click(move |_, _, cx| {
                                        e.update(cx, |s, cx| {
                                            s.terms_page += 1;
                                            cx.notify()
                                        })
                                    }),
                            );
                        }
                    }
                }
            }
            if let Some(preview) = snapshot.preview {
                body = body.child(
                    div()
                        .id("customize-preview")
                        .role(Role::Group)
                        .aria_label("Local preview result")
                        .child(gpui::text!(preview)),
                );
            }
        } else {
            body = body.child(gpui::text!("Loading customization…"));
        }
        div()
            .id("customize")
            .capture_key_down(cx.listener(|s, e: &KeyDownEvent, w, cx| {
                if e.keystroke.key == "escape" && s.editing.is_some() {
                    s.cancel(w, cx);
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
