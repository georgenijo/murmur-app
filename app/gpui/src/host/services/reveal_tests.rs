use super::{adapters::voice_command_settings_route, backend::Route, root::reveal_setting};
use crate::settings::{Settings, fixtures::FixtureHost, schema::Page};
use gpui::{Entity, TestAppContext, VisualTestContext, prelude::*};
use std::rc::Rc;

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|w, cx| w.draw(cx).clear(cx));
}

fn attached(
    cx: &mut TestAppContext,
    hold_read: bool,
) -> (Entity<Settings>, Rc<FixtureHost>, &mut VisualTestContext) {
    cx.update(crate::init);
    let host = Rc::new(FixtureHost::default());
    host.delay_read.set(hold_read);
    let mut entity = None;
    let (_, cx) = cx.add_window_view(|w, cx| {
        let e = cx.new(|cx| Settings::new(host.clone(), w, cx));
        entity = Some(e.clone());
        gpui_base::Root::new(e, w, cx)
    });
    let settings = entity.unwrap();
    settings.update(cx, |s, cx| s.attach(cx));
    (settings, host, cx)
}

fn link_key() -> &'static str {
    // The production route handler resolves the link to its settings row.
    match voice_command_settings_route() {
        Route::SettingsReveal(key) => key,
        _ => "link must name a settings row",
    }
}

fn assert_revealed(settings: &Entity<Settings>, key: &'static str, cx: &mut VisualTestContext) {
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

/// The production sequence: root selects Settings (attach starts the async
/// Read) and reveals the row before that Read has resolved.
#[gpui::test]
fn reveal_before_the_first_read_resolves_expands_advanced_once_loaded(cx: &mut TestAppContext) {
    use crate::settings::host::Host;
    let (settings, host, cx) = attached(cx, true);
    draw(cx);
    assert!(
        !settings.read_with(cx, |s, _| s.loaded()),
        "the first Read must still be pending"
    );
    assert_eq!(host.delayed.borrow().len(), 1, "one held Read");
    assert_eq!(settings.read_with(cx, |s, _| s.model_page()), Page::General);
    let key = link_key();
    cx.update(|w, cx| reveal_setting(&settings, key, w, cx));
    draw(cx);
    assert_eq!(
        settings.read_with(cx, |s, _| s.model_page()),
        Page::Text,
        "the page opens immediately"
    );
    assert!(
        !settings.read_with(cx, |s, _| s.model_advanced()),
        "Advanced cannot expand without a snapshot"
    );
    // The held Read now resolves; the queued reveal applies and expands Advanced.
    host.delay_read.set(false);
    let (request, inbox) = host.delayed.borrow_mut().pop().unwrap();
    host.request(request, inbox);
    draw(cx);
    draw(cx);
    draw(cx);
    assert!(settings.read_with(cx, |s, _| s.loaded()));
    assert_revealed(&settings, key, cx);
}

#[gpui::test]
fn voice_commands_link_reveals_the_collapsed_advanced_toggle(cx: &mut TestAppContext) {
    let (settings, _host, cx) = attached(cx, false);
    draw(cx);
    assert_eq!(settings.read_with(cx, |s, _| s.model_page()), Page::General);
    assert!(!settings.read_with(cx, |s, _| s.model_advanced()));
    let key = link_key();
    cx.update(|w, cx| reveal_setting(&settings, key, w, cx));
    draw(cx);
    draw(cx);
    assert_revealed(&settings, key, cx);
}
