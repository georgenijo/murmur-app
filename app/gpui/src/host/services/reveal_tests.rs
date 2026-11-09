use super::{adapters::voice_command_settings_route, backend::Route, root::reveal_setting};
use crate::settings::{Settings, fixtures::FixtureHost, schema::Page};
use gpui::{TestAppContext, VisualTestContext, prelude::*};
use std::rc::Rc;

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|w, cx| w.draw(cx).clear(cx));
}

#[gpui::test]
fn voice_commands_link_reveals_the_collapsed_advanced_toggle(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let host = Rc::new(FixtureHost::default());
    let mut entity = None;
    let (_, cx) = cx.add_window_view(|w, cx| {
        let e = cx.new(|cx| Settings::new(host.clone(), w, cx));
        entity = Some(e.clone());
        gpui_base::Root::new(e, w, cx)
    });
    let settings = entity.unwrap();
    settings.update(cx, |s, cx| s.attach(cx));
    draw(cx);
    assert_eq!(settings.read_with(cx, |s, _| s.model_page()), Page::General);
    assert!(!settings.read_with(cx, |s, _| s.model_advanced()));
    // The production route handler resolves the link to its settings row.
    let key = match voice_command_settings_route() {
        Route::SettingsReveal(key) => key,
        _ => "link must name a settings row",
    };
    cx.update(|w, cx| reveal_setting(&settings, key, w, cx));
    draw(cx);
    draw(cx);
    let (page, advanced, revealed, focused) = cx.update(|w, cx| {
        settings.read_with(cx, |s, _| {
            (
                s.model_page(),
                s.model_advanced(),
                s.revealed(),
                s.row_focused(key, w),
            )
        })
    });
    assert_eq!(page, Page::Text);
    assert!(advanced, "Advanced disclosure must be expanded");
    assert_eq!(revealed, Some("voiceCommandsEnabled"));
    assert!(focused, "the toggle must hold focus once rendered");
}
