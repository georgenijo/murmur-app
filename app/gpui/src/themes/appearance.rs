//! Single main-thread appearance owner; no persistence and no network here.
use super::compile;
use gpui::{App, Global, WindowAppearance};
use serde_json::Value;
#[derive(Clone)]
pub struct Appearance {
    pub document: Value,
    system_dark: bool,
    compiled: compile::Compiled,
}
impl Global for Appearance {}
fn dark(doc: &Value, system: bool) -> bool {
    match doc["mode"].as_str() {
        Some("dark") => true,
        Some("light") => false,
        _ => system,
    }
}
pub fn install(document: Value, cx: &mut App) {
    let system = cx.try_global::<Appearance>().map_or(
        matches!(
            cx.window_appearance(),
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        ),
        |a| a.system_dark,
    );
    let compiled = compile::compile(&document["theme"]);
    let is_dark = dark(&document, system);
    cx.set_window_appearance(match document["mode"].as_str() {
        Some("dark") => Some(WindowAppearance::Dark),
        Some("light") => Some(WindowAppearance::Light),
        _ => None,
    });
    compile::apply(
        if is_dark {
            &compiled.dark
        } else {
            &compiled.light
        },
        is_dark,
        cx,
    );
    cx.set_global(Appearance {
        document,
        system_dark: system,
        compiled,
    });
    cx.refresh_windows();
}
pub fn system_changed(system_dark: bool, cx: &mut App) {
    if let Some(a) = cx.try_global::<Appearance>().cloned() {
        let is_dark = dark(&a.document, system_dark);
        compile::apply(
            if is_dark {
                &a.compiled.dark
            } else {
                &a.compiled.light
            },
            is_dark,
            cx,
        );
        cx.set_global(Appearance { system_dark, ..a });
    } else {
        crate::tokens::apply_appearance(system_dark, cx);
    }
}
