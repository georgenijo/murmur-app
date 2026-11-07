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
    pub embedded: bool,
    _forms: Vec<Subscription>,
    pub controller: Controller<Snapshot, Action>,
    pub editor: Entity<Form>,
    pub search: Entity<Form>,
    dialog: Option<Dialog>,
    scroll: ScrollHandle,
    applied: Option<Value>,
    base_theme: Value,
    editor_document: Value,
    confirming: bool,
    notice: Option<&'static str>,
    restore: Option<FocusHandle>,
    import_focus: FocusHandle,
}
/// React's updateTheme drops compiled overrides/library ownership before editing.
fn editable_theme(document: &Value, dark: bool) -> Value {
    let theme = &document["theme"];
    let compiled = theme.get("light").is_some() || theme.get("dark").is_some();
    let library_owned = ["light", "dark"]
        .iter()
        .any(|mode| document["selection"][mode] != "custom");
    if !compiled && !library_owned {
        return theme.clone();
    }
    let rendered = compile::value(&compile::resolve(theme, dark));
    let mut base = json!({"version":1,"presetId":"custom","accent":rendered["primary"],"background":rendered["background"],"foreground":rendered["on-surface"]});
    if let Some(contrast) = theme.get("contrast") {
        base["contrast"] = contrast.clone();
    }
    base
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
            embedded: false,
            _forms: forms,
            controller: Controller::new(host),
            editor,
            search,
            dialog: None,
            scroll: ScrollHandle::new(),
            applied: None,
            base_theme: Value::Null,
            editor_document: Value::Null,
            confirming: false,
            notice: None,
            restore: None,
            import_focus: cx.focus_handle(),
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
            let document = self
                .controller
                .model
                .snapshot
                .as_ref()
                .map(|s| s.appearance.clone())
                .unwrap_or(json!({"theme":{"version":1,"presetId":"sonic"}}));
            let theme = editable_theme(&document, appearance::is_dark(&document, cx));
            self.base_theme = theme.clone();
            self.editor_document = document;
            self.editor.update(cx,|f,cx|f.set(json!({"label":if *create{"New theme"}else{"Custom"},"accent":theme["accent"].as_str().unwrap_or(""),"background":theme["background"].as_str().unwrap_or(""),"foreground":theme["foreground"].as_str().unwrap_or(""),"contrast":theme["contrast"].as_i64().unwrap_or(0).to_string()}),w,cx));
            self.editor.update(cx, |f, cx| f.focus_first(w, cx));
        }
        if matches!(d, Dialog::Community) {
            self.search.update(cx, |f, cx| f.focus_first(w, cx));
        }
        if matches!(d, Dialog::Import) {
            w.focus(&self.import_focus, cx);
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
        if self
            .controller
            .model
            .snapshot
            .as_ref()
            .is_none_or(|s| s.appearance != self.editor_document)
        {
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
            .when(!self.embedded, |d| {
                d.child(div().text_size(px(20.)).child(gpui::text!("Appearance")))
            });
        if let Some(problem) = self.controller.model.problem {
            body = body
                .child(
                    div()
                        .id("themes-error")
                        .role(Role::Alert)
                        .child(gpui::text!(problem.message())),
                )
                .child(self.button("themes-retry".into(), "Retry".into(), Action::Read, cx));
        }
        if let Some(n) = self.notice {
            body = body.child(
                div()
                    .id("theme-validation")
                    .role(Role::Alert)
                    .child(gpui::text!(n)),
            );
        }
        if let Some(n) = snapshot.as_ref().and_then(|s| s.notice.clone()) {
            body = body.child(
                div()
                    .id("themes-notice")
                    .role(Role::Status)
                    .child(gpui::text!(n)),
            );
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
                        .child(
                            self.button(
                                "theme-local-import".into(),
                                "Choose local file…".into(),
                                Action::InspectLocal,
                                cx,
                            )
                            .track_focus(&self.import_focus),
                        )
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
                    content=content.child(gpui::text!("Search sends your query and ordinary connection metadata directly to https://open-vsx.org. Downloads occur only when you choose Add or Update. No account, proxy, background polling, or automatic updates.")).child(self.search.clone()).child(self.button("community-search".into(),"Search Open VSX".into(),Action::Search{query,sort},cx));
                    if let Some(s) = &snapshot {
                        if s.results.is_empty() {
                            content = content.child(gpui::text!("No community themes to show."));
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
                                .id(SharedString::from(format!("community-{}", e.id)))
                                .child(gpui::text!(e.label.clone()))
                                .child(gpui::text!(format!(
                                    "{} · {} · {}",
                                    e.publisher, e.version, e.license
                                )));
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
                    content = content.child(self.editor.clone()).child(gpui::text!("Colors resolve to accessible semantic roles. Leave a color empty to use Sonic."));
                    if let Some(snapshot) = &snapshot {
                        let mut reset = snapshot.appearance.clone();
                        reset["theme"] = json!({"version":1,"presetId":"sonic"});
                        reset["selection"] = json!({"light":"sonic","dark":"sonic"});
                        reset["revision"] = json!(library::next_revision(
                            reset["revision"].as_u64().unwrap_or(0)
                        ));
                        let reset_owner = cx.entity();
                        content = content.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .pt(px(16.))
                                .border_t_1()
                                .border_color(p.hairline)
                                .child(
                                    Button::new("theme-reset", "Reset to Sonic")
                                        .intent(ButtonIntent::Ghost)
                                        .with_scroll_parent(Some(&self.scroll))
                                        .disabled(self.controller.model.pending)
                                        .on_click(move |_, _, cx| {
                                            reset_owner.update(cx, |s, cx| {
                                                s.confirming = true;
                                                s.request(Action::Appearance(reset.clone()), cx);
                                            })
                                        }),
                                )
                                .child(div().flex_1())
                                .child(
                                    self.button(
                                        "theme-export-current".into(),
                                        "Export current…".into(),
                                        Action::ExportCurrent,
                                        cx,
                                    )
                                    .caption("Export")
                                    .pill(),
                                )
                                .child(
                                    Button::new("theme-save", "Save theme")
                                        .with_scroll_parent(Some(&self.scroll))
                                        .on_click(move |_, _, cx| {
                                            e.update(cx, |s, cx| s.save_editor(cx))
                                        }),
                                ),
                        );
                    }
                }
                Dialog::Preview(entries) => {
                    for entry in &entries {
                        let c = compile::compile(&entry["theme"]);
                        content = content.child(gpui::text!(
                            id = SharedString::from(format!("theme-preview-{}", entry["id"])),
                            format!(
                                "{} · {} contrast adjustments",
                                entry["label"].as_str().unwrap_or("Imported theme"),
                                c.adjustments
                            )
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
                    content = content
                        .child(gpui::text!("Remove this saved theme?"))
                        .child(
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
                    content=content.child(gpui::text!("Replace the installed collection with this version? Local changes since this preview refuse the update.")).child(Button::new("theme-update-confirm","Confirm update").with_scroll_parent(Some(&self.scroll)).on_click(move|_,_,cx|e.update(cx,|s,cx|{s.dialog=None;s.request(Action::Update{extension:(*extension).clone(),expected:expected.clone()},cx)})));
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
            let mut mode_buttons = div()
                .flex()
                .gap(px(2.))
                .p(px(2.))
                .rounded(tokens::CONTROL_RADIUS);
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
                    .intent(crate::ButtonIntent::Ghost)
                    .caption(format!(
                        "{}{}",
                        if s.appearance["mode"] == mode {
                            "✓ "
                        } else {
                            ""
                        },
                        label
                    ))
                    .toggled(s.appearance["mode"] == mode),
                );
            }
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child("Color scheme")
                    .child(mode_buttons),
            );
            let create = cx.entity();
            let import = create.clone();
            body = body.child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .items_center()
                    .child(div().flex_1().child("Themes"))
                    .child(
                        Button::new("theme-create", "Create theme…")
                            .caption("Create theme")
                            .size(crate::ButtonSize::Standard)
                            .text_size(tokens::BODY_SIZE)
                            .palette(Palette {
                                hairline_strong: p.border,
                                ..p
                            })
                            .bordered()
                            .intent(crate::ButtonIntent::Secondary)
                            .icon(crate::icons::Glyph::Plus)
                            .with_scroll_parent(Some(&self.scroll))
                            .on_click(move |_, w, cx| {
                                create.update(cx, |s, cx| {
                                    s.dialog(Dialog::Edit { create: true }, w, cx)
                                })
                            }),
                    )
                    .child(
                        Button::new("theme-import", "Import theme…")
                            .caption("Import theme")
                            .size(crate::ButtonSize::Standard)
                            .text_size(tokens::BODY_SIZE)
                            .palette(Palette {
                                hairline_strong: p.border,
                                ..p
                            })
                            .bordered()
                            .intent(crate::ButtonIntent::Secondary)
                            .icon(crate::icons::Glyph::Download)
                            .with_scroll_parent(Some(&self.scroll))
                            .on_click(move |_, w, cx| {
                                import.update(cx, |s, cx| s.dialog(Dialog::Import, w, cx))
                            }),
                    ),
            );
            if !s.library_available {
                body = body.child(div().id("theme-library-error").role(Role::Alert).child(
                    gpui::text!(
                        "Theme library storage is unavailable. Your active appearance is kept."
                    ),
                ));
            }
            let mut grid = div().flex().flex_wrap().gap(px(8.));
            for entry in library::display_entries(&s.appearance, &s.library) {
                let id = entry["id"].as_str().unwrap().to_string();
                let label = entry["label"].as_str().unwrap_or("Theme");
                let selected = s.appearance["selection"][if appearance::is_dark(&s.appearance, cx) {
                    "dark"
                } else {
                    "light"
                }] == id;
                let mut orbs = div()
                    .flex()
                    .items_start()
                    .justify_center()
                    .gap(px(28.))
                    .pt(px(4.))
                    .h(px(68.));
                for mode in entry["modes"].as_array().unwrap() {
                    let mode = mode.as_str().unwrap();
                    if let Ok(doc) = library::selection(&s.appearance, &s.library, &id, Some(mode))
                    {
                        let colors = compile::resolve(&entry["theme"], mode == "dark");
                        let role =
                            |key: &str| color::hex(*colors.get(key).unwrap_or(&[0., 0., 0.]));
                        // Same background and layered radial fills as React ThemeOrb.
                        let svg = format!(
                            r#"<svg xmlns="http://www.w3.org/2000/svg" width="72" height="72" viewBox="0 0 36 36"><defs><radialGradient id="accent" cx="35%" cy="70%" r="62%"><stop stop-color="{}"/><stop offset="1" stop-color="{}" stop-opacity="0"/></radialGradient><radialGradient id="surface" cx="75%" cy="25%" r="58%"><stop stop-color="{}"/><stop offset="1" stop-color="{}" stop-opacity="0"/></radialGradient></defs><circle cx="18" cy="18" r="18" fill="{}"/><circle cx="18" cy="18" r="18" fill="url(#surface)"/><circle cx="18" cy="18" r="18" fill="url(#accent)"/></svg>"#,
                            role("primary"),
                            role("primary"),
                            role("surface-container-highest"),
                            role("surface-container-highest"),
                            role("background")
                        );
                        let orb = div()
                            .relative()
                            .size(px(36.))
                            .rounded_full()
                            .border_1()
                            .border_color(p.hairline_strong)
                            .child(
                                img(std::sync::Arc::new(Image::from_bytes(
                                    ImageFormat::Svg,
                                    svg.into_bytes(),
                                )))
                                .size(px(36.)),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .right(px(-4.))
                                    .bottom(px(-4.))
                                    .size(px(16.))
                                    .rounded_full()
                                    .bg(p.tint_raised)
                                    .border_1()
                                    .border_color(p.hairline)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(
                                        crate::icons::icon(if mode == "dark" {
                                            crate::icons::Glyph::Moon
                                        } else {
                                            crate::icons::Glyph::Sun
                                        })
                                        .size(px(10.))
                                        .text_color(p.muted),
                                    ),
                            );
                        orbs = orbs.child(
                            self.button(
                                format!("theme-{id}-{mode}"),
                                format!("{label} {mode} variant"),
                                Action::Appearance(doc),
                                cx,
                            )
                            .intent(crate::ButtonIntent::Ghost)
                            .padding_x(px(0.))
                            .padding_y(px(0.))
                            .content(
                                div()
                                    .size(px(44.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(orb),
                            ),
                        );
                    }
                }
                let mut footer = div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .h(px(36.))
                    .px(px(10.))
                    .pt(px(4.))
                    .pb(px(8.));
                if let Ok(doc) = library::selection(&s.appearance, &s.library, &id, None) {
                    footer = footer.child(
                        div().flex_1().min_w_0().child(
                            self.button(
                                format!("theme-{id}-pair"),
                                format!("Use {label} theme"),
                                Action::Appearance(doc),
                                cx,
                            )
                            .size(crate::ButtonSize::ExtraSmall)
                            .text_size(tokens::BODY_SIZE)
                            .padding_x(px(0.))
                            .padding_y(px(0.))
                            .intent(crate::ButtonIntent::Ghost)
                            .content(
                                div()
                                    .w_full()
                                    .truncate()
                                    .text_size(px(16.))
                                    .font_weight(FontWeight::NORMAL)
                                    .child(label.to_string()),
                            ),
                        ),
                    );
                }
                let mut actions = div().flex().flex_shrink_0().items_center().gap(px(2.));
                if id == "sonic" || id == "custom" {
                    let edit = cx.entity();
                    let sonic = id == "sonic";
                    actions = actions.child(
                        Button::new(
                            if sonic {
                                "theme-sonic-create"
                            } else {
                                "theme-edit"
                            },
                            if sonic {
                                "Create theme from Sonic"
                            } else {
                                "Edit colors…"
                            },
                        )
                        .icon_only(if sonic {
                            crate::icons::Glyph::Copy
                        } else {
                            crate::icons::Glyph::Palette
                        })
                        .palette(Palette { text: p.muted, ..p })
                        .size(crate::ButtonSize::ExtraSmall)
                        .intent(crate::ButtonIntent::Ghost)
                        .with_scroll_parent(Some(&self.scroll))
                        .on_click(move |_, w, cx| {
                            edit.update(cx, |s, cx| s.dialog(Dialog::Edit { create: false }, w, cx))
                        }),
                    );
                } else {
                    actions = actions
                        .child(
                            self.button(
                                format!("theme-{id}-export"),
                                format!("Export {label}…"),
                                Action::ExportEntry(id.clone()),
                                cx,
                            )
                            .icon_only(crate::icons::Glyph::Upload)
                            .palette(Palette { text: p.muted, ..p })
                            .size(crate::ButtonSize::ExtraSmall)
                            .intent(crate::ButtonIntent::Ghost),
                        )
                        .child(
                            self.button(
                                format!("theme-{id}-duplicate"),
                                format!("Duplicate {label}"),
                                Action::Duplicate(id.clone()),
                                cx,
                            )
                            .icon_only(crate::icons::Glyph::Copy)
                            .palette(Palette { text: p.muted, ..p })
                            .size(crate::ButtonSize::ExtraSmall)
                            .intent(crate::ButtonIntent::Ghost),
                        );
                }
                if let Some(url) = entry["source"]["sourceUrl"].as_str() {
                    actions = actions.child(
                        self.button(
                            format!("source-{id}"),
                            format!("Open {label} source…"),
                            Action::OpenSource(url.into()),
                            cx,
                        )
                        .icon_only(crate::icons::Glyph::File)
                        .palette(Palette { text: p.muted, ..p })
                        .size(crate::ButtonSize::ExtraSmall)
                        .intent(crate::ButtonIntent::Ghost),
                    );
                }
                if !["sonic", "opencode", "custom"].contains(&id.as_str()) {
                    let e = cx.entity();
                    let remove_id = id.clone();
                    actions = actions.child(
                        Button::new(
                            SharedString::from(format!("theme-{id}-remove")),
                            format!("Remove {label}…"),
                        )
                        .icon_only(crate::icons::Glyph::Trash)
                        .palette(Palette { text: p.muted, ..p })
                        .size(crate::ButtonSize::ExtraSmall)
                        .intent(crate::ButtonIntent::Ghost)
                        .with_scroll_parent(Some(&self.scroll))
                        .on_click(move |_, w, cx| {
                            e.update(cx, |s, cx| {
                                s.dialog(Dialog::Remove(remove_id.clone()), w, cx)
                            })
                        }),
                    );
                }
                footer = footer.child(actions);
                let resolved = compile::resolve(
                    &s.appearance["theme"],
                    appearance::is_dark(&s.appearance, cx),
                );
                let [r, g, b] = resolved["surface-container-lowest"];
                let card_surface =
                    Hsla::from(rgb(((r as u32) << 16) | ((g as u32) << 8) | b as u32));
                grid = grid.child(
                    div()
                        .id(SharedString::from(format!("theme-{id}")))
                        .w(px(208.))
                        .border_1()
                        .border_color(if selected {
                            p.accent
                        } else {
                            p.hairline_strong
                        })
                        .bg(card_surface)
                        .rounded(tokens::CARD_RADIUS)
                        .shadow(if selected {
                            vec![BoxShadow {
                                color: p.accent.opacity(0.35),
                                offset: point(px(0.), px(0.)),
                                blur_radius: px(0.),
                                spread_radius: px(2.),
                                inset: false,
                            }]
                        } else {
                            p.card_shadow()
                        })
                        .child(orbs)
                        .child(footer),
                );
            }
            body = body.child(grid);
        } else {
            body = body.child(gpui::text!("Loading appearance…"));
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
            .p(px(if self.embedded { 0. } else { 24. }))
            .bg(p.surface)
            .text_color(p.text)
            .text_size(tokens::BODY_SIZE)
            .line_height(tokens::BODY_LEADING)
            .child(body)
    }
}
