//! Shared native editors for the remaining sections. Values are ephemeral drafts.
use crate::{
    Choice, ChoiceKind, Choices, SelectionChanged, TextField,
    field::{InputEvent, TextFieldState},
    tokens::Palette,
};
use gpui::{prelude::*, *};
use serde_json::{Value, json};
use std::collections::BTreeMap;
#[derive(Clone)]
pub struct Spec {
    pub key: &'static str,
    pub label: &'static str,
    pub choices: Vec<(String, String)>,
    pub multiline: bool,
}
impl Spec {
    pub fn text(key: &'static str, label: &'static str) -> Self {
        Self {
            key,
            label,
            choices: vec![],
            multiline: false,
        }
    }
    pub fn select(key: &'static str, label: &'static str, choices: &[(&str, &str)]) -> Self {
        Self {
            key,
            label,
            choices: choices
                .iter()
                .map(|(v, l)| (v.to_string(), l.to_string()))
                .collect(),
            multiline: false,
        }
    }
    pub fn boolean(key: &'static str, label: &'static str) -> Self {
        Self::select(key, label, &[("false", "Off"), ("true", "On")])
    }
}
#[derive(Clone)]
pub struct Changed(pub &'static str, pub Value);
pub struct Form {
    specs: Vec<Spec>,
    areas: BTreeMap<&'static str, Entity<gpui_component::input::TextareaState>>,
    texts: BTreeMap<&'static str, Entity<TextFieldState>>,
    choices: BTreeMap<&'static str, Entity<Choices>>,
    pub values: Value,
    pub disabled: bool,
    pub disabled_keys: std::collections::BTreeSet<&'static str>,
    pub scroll_parent: Option<ScrollHandle>,
    _subscriptions: Vec<Subscription>,
}
impl EventEmitter<Changed> for Form {}
impl Form {
    pub fn new(specs: Vec<Spec>, w: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut s = Self {
            specs,
            areas: BTreeMap::new(),
            texts: BTreeMap::new(),
            choices: BTreeMap::new(),
            values: json!({}),
            disabled: false,
            disabled_keys: Default::default(),
            scroll_parent: None,
            _subscriptions: vec![],
        };
        for spec in s.specs.clone() {
            let key = spec.key;
            if spec.multiline {
                let input = cx.new(|cx| gpui_component::input::TextareaState::new(w, cx));
                s._subscriptions
                    .push(cx.subscribe(&input, move |s, input, e, cx| {
                        if matches!(e, InputEvent::Change) {
                            let v = json!(input.read(cx).value().to_string());
                            if s.values[key] != v {
                                s.values[key] = v.clone();
                                crate::events::publish(cx, Changed(key, v));
                                cx.notify();
                            }
                        }
                    }));
                s.areas.insert(key, input);
            } else if spec.choices.is_empty() {
                let input = cx.new(|cx| TextFieldState::new(w, cx));
                s._subscriptions
                    .push(cx.subscribe(&input, move |s, input, e, cx| {
                        if matches!(e, InputEvent::Change) {
                            let v = json!(input.read(cx).value().to_string());
                            if s.values[key] != v {
                                s.values[key] = v.clone();
                                crate::events::publish(cx, Changed(key, v));
                                cx.notify();
                            }
                        }
                    }));
                s.texts.insert(key, input);
            } else {
                let picker = cx.new(|cx| {
                    Choices::new(
                        spec.label,
                        ChoiceKind::Dropdown,
                        spec.choices
                            .iter()
                            .map(|(v, l)| Choice::new(v.clone(), l.clone()))
                            .collect(),
                        w,
                        cx,
                    )
                });
                s._subscriptions.push(cx.subscribe(
                    &picker,
                    move |s, _, e: &SelectionChanged, cx| {
                        let v = json!(e.0.to_string());
                        if s.values[key] != v {
                            s.values[key] = v.clone();
                            crate::events::publish(cx, Changed(key, v));
                            cx.notify();
                        }
                    },
                ));
                s.choices.insert(key, picker);
            }
        }
        s
    }
    pub fn set(&mut self, values: Value, w: &mut Window, cx: &mut Context<Self>) {
        self.values = values.clone();
        for (k, e) in &self.areas {
            let v = values[*k].as_str().unwrap_or("");
            if e.read(cx).value().as_str() != v {
                e.update(cx, |s, cx| s.set_value(v, w, cx));
            }
        }
        for (k, e) in &self.texts {
            let v = values[*k].as_str().unwrap_or("");
            if e.read(cx).value().as_str() != v {
                e.update(cx, |s, cx| s.set_value(v, w, cx));
            }
        }
        for (k, e) in &self.choices {
            let v = values[*k]
                .as_str()
                .map(String::from)
                .unwrap_or_else(|| values[*k].to_string());
            e.update(cx, |s, cx| s.set_value(&v, cx));
        }
        cx.notify();
    }
    pub fn value(&self, key: &str) -> &str {
        self.values[key].as_str().unwrap_or("")
    }
    /// Dialog defaults must leave multiline editing and choice selection alone.
    pub fn single_line_focused(&self, w: &Window, cx: &App) -> bool {
        !self.disabled
            && self.texts.iter().any(|(key, input)| {
                !self.disabled_keys.contains(key) && input.read(cx).focus_handle(cx).is_focused(w)
            })
    }
    pub fn focus_first(&self, w: &mut Window, cx: &mut App) {
        if self.disabled {
            return;
        }
        for spec in &self.specs {
            if self.disabled_keys.contains(spec.key) {
                continue;
            }
            let focus = if let Some(e) = self.areas.get(spec.key) {
                e.read(cx).focus_handle(cx)
            } else if let Some(e) = self.texts.get(spec.key) {
                e.read(cx).focus_handle(cx)
            } else if let Some(e) = self.choices.get(spec.key) {
                e.read(cx).focus_handle(cx)
            } else {
                continue;
            };
            w.focus(&focus, cx);
            break;
        }
    }
}
impl Render for Form {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::get(cx);
        let mut body = div().flex().flex_col().gap(px(12.));
        for spec in &self.specs {
            let (focus, control) = if let Some(e) = self.areas.get(spec.key) {
                (
                    e.read(cx).focus_handle(cx),
                    gpui_component::input::Textarea::new(e)
                        .aria_label(spec.label)
                        .disabled(self.disabled || self.disabled_keys.contains(spec.key))
                        .h(px(100.))
                        .into_any_element(),
                )
            } else if let Some(e) = self.texts.get(spec.key) {
                (
                    e.read(cx).focus_handle(cx),
                    TextField::new(e, spec.label)
                        .disabled(self.disabled || self.disabled_keys.contains(spec.key))
                        .into_any_element(),
                )
            } else {
                let e = &self.choices[spec.key];
                e.update(cx, |s, cx| {
                    s.set_disabled(self.disabled || self.disabled_keys.contains(spec.key), cx)
                });
                (e.read(cx).focus_handle(cx), e.clone().into_any_element())
            };
            let reveal = crate::data_components::focus_reveal(
                focus,
                self.scroll_parent.clone(),
                None,
                w,
                cx,
            );
            body = body.child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(div().text_color(p.muted).child(spec.label))
                    .child(control)
                    .child(reveal),
            );
        }
        body
    }
}

#[cfg(test)]
mod tests {
    use super::{Form, Spec};
    use gpui::{AppContext, Focusable, TestAppContext};
    #[gpui::test]
    fn focus_first_skips_disabled_and_missing_controls(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let mut form = None;
        let (_, cx) = cx.add_window_view(|w, cx| {
            let e = cx.new(|cx| {
                Form::new(
                    vec![
                        Spec::boolean("kind", "Disabled choice"),
                        Spec::text("missing", "No focusable control"),
                        Spec {
                            multiline: true,
                            ..Spec::text("body", "Disabled multiline")
                        },
                        Spec::text("title", "Enabled title"),
                    ],
                    w,
                    cx,
                )
            });
            form = Some(e.clone());
            gpui_base::Root::new(e, w, cx)
        });
        let form = form.unwrap();
        cx.update(|w, cx| {
            form.update(cx, |s, cx| {
                s.disabled_keys.extend(["kind", "body"]);
                let missing = s.texts.remove("missing").unwrap();
                s.focus_first(w, cx);
                assert!(s.texts["title"].read(cx).focus_handle(cx).is_focused(w));
                assert!(!s.choices["kind"].read(cx).focus_handle(cx).is_focused(w));
                s.disabled = true;
                w.blur(cx);
                s.focus_first(w, cx);
                assert!(w.focused(cx).is_none());
                s.disabled = false;
                s.disabled_keys.insert("title");
                s.focus_first(w, cx);
                assert!(w.focused(cx).is_none());
                s.texts.insert("missing", missing);
            })
        });
    }
}
