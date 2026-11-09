//! Shared updater presentation; the service owner supplies state and actions.
use crate::{
    Button,
    icons::{self, Glyph},
    tokens::{self, Palette},
};
use gpui::{prelude::*, *};
pub fn card(p: Palette) -> Stateful<Div> {
    div()
        .id("update-dialog")
        .role(Role::Dialog)
        .aria_label("Murmur update")
        .w(px(560.))
        .max_w_full()
        .h(px(620.))
        .max_h_full()
        .flex()
        .flex_col()
        .overflow_hidden()
        .bg(p.tint_raised)
        .border_1()
        .border_color(p.hairline_strong)
        .rounded(tokens::CARD_RADIUS)
        .shadow(vec![BoxShadow {
            color: black().opacity(0.24),
            offset: point(px(0.), px(16.)),
            blur_radius: px(48.),
            spread_radius: px(0.),
            inset: false,
        }])
}
pub fn header(
    title: impl Into<SharedString>,
    caption: impl Into<SharedString>,
    close: Option<Button>,
    p: Palette,
) -> Div {
    let title: SharedString = title.into();
    let caption: SharedString = caption.into();
    div()
        .flex()
        .items_start()
        .gap(px(12.))
        .px(px(24.))
        .py(px(16.))
        .border_b_1()
        .border_color(p.hairline)
        .flex_shrink_0()
        .child(
            div()
                .size(px(40.))
                .rounded(tokens::CARD_RADIUS)
                .bg(p.surface_container_low)
                .flex()
                .items_center()
                .justify_center()
                .child(icons::icon(Glyph::Download).size(px(20.))),
        )
        .child(
            div()
                .flex_1()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(
                    div()
                        .text_size(px(18.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .id("update-dialog-heading")
                        .role(Role::Heading)
                        .aria_label(title.clone())
                        .child(title),
                )
                .child(
                    div()
                        .text_size(tokens::LABEL_SIZE)
                        .text_color(p.muted)
                        .child(gpui::text!(caption)),
                ),
        )
        .children(close)
}
pub fn notes() -> Stateful<Div> {
    div()
        .id("release-notes")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .px(px(24.))
        .py(px(20.))
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(
            div()
                .text_size(px(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(gpui::text!("What's Changed")),
        )
}
pub fn footer(p: Palette) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .px(px(24.))
        .py(px(16.))
        .border_t_1()
        .border_color(p.hairline)
        .bg(p.surface_container_low)
        .flex_shrink_0()
}
pub fn scrim() -> Stateful<Div> {
    div()
        .id("update-dialog-scrim")
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(|_, _, cx| cx.stop_propagation())
        .absolute()
        .inset_0()
        .px(px(24.))
        .py(px(20.))
        .flex()
        .items_center()
        .justify_center()
        .bg(black().opacity(0.45))
}
/// An optional update can fail with a reinstall recovery (App Translocation); the
/// only fix is to quit and move the app, so the dialog must offer Quit.
pub fn is_reinstall(status: &murmur_core::updater::Status) -> bool {
    matches!(
        status,
        murmur_core::updater::Status::Error { recovery: Some(r), .. } if r == "reinstall"
    )
}
/// Forced updates and reinstall errors both offer Quit; optional ones keep Later too.
pub fn quit_offered(forced: bool, status: &murmur_core::updater::Status) -> bool {
    forced || is_reinstall(status)
}
/// Updates the user cannot dismiss: forced ones and any in-flight work.
pub fn update_required(status: &murmur_core::updater::Status) -> bool {
    use murmur_core::updater::Status;
    matches!(
        status,
        Status::Available {
            is_forced: true,
            ..
        } | Status::Ready {
            is_forced: true,
            ..
        } | Status::Error {
            is_forced: true,
            ..
        } | Status::Preparing { .. }
            | Status::Downloading { .. }
            | Status::Restarting { .. }
    )
}
/// The updater action that closes the dialog, or None when it cannot be closed.
/// Release notes for a finished update are always dismissible and must be
/// acknowledged with `dismissCompleted`; a plain `dismiss` leaves them pending, so
/// the notes would reappear on the next snapshot.
pub fn dismiss_action(
    snapshot: &murmur_core::updater::Snapshot,
    required: bool,
) -> Option<&'static str> {
    if snapshot.completed_update.is_some() {
        Some("dismissCompleted")
    } else if required || update_required(&snapshot.update_status) {
        None
    } else {
        Some("dismiss")
    }
}

#[cfg(test)]
mod tests {
    use super::{card, footer, scrim};
    use crate::{Button, tokens::Palette};
    use gpui::{Context, IntoElement, Modifiers, Render, TestAppContext, Window, div, prelude::*};
    use std::{cell::Cell, rc::Rc};
    struct Review {
        open: bool,
        background_clicks: Rc<Cell<usize>>,
    }
    impl Render for Review {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let clicks = self.background_clicks.clone();
            let p = Palette::get(cx);
            div()
                .size_full()
                .relative()
                .child(
                    div()
                        .id("workspace-hit-target")
                        .size_full()
                        .on_click(move |_, _, _| clicks.set(clicks.get() + 1)),
                )
                .when(self.open, |d| {
                    d.child(
                        scrim().child(card(p).child(div().flex_1()).child(footer(p).child(
                            div().debug_selector(|| "updater-test-later".into()).child(
                                Button::new("later", "Later").on_click(cx.listener(
                                    |s, _, _, cx| {
                                        s.open = false;
                                        cx.notify();
                                    },
                                )),
                            ),
                        ))),
                    )
                })
        }
    }
    #[gpui::test]
    fn dismiss_does_not_activate_the_workspace_underneath(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let clicks = Rc::new(Cell::new(0));
        let mut owner = None;
        let (_, cx) = cx.add_window_view(|w, cx| {
            let view = cx.new(|_| Review {
                open: true,
                background_clicks: clicks.clone(),
            });
            owner = Some(view.clone());
            gpui_base::Root::new(view, w, cx)
        });
        cx.run_until_parked();
        cx.update(|w, cx| w.draw(cx).clear(cx));
        let button = cx.debug_bounds("updater-test-later").unwrap();
        cx.simulate_click(button.center(), Modifiers::default());
        cx.run_until_parked();
        assert!(
            !owner.unwrap().read_with(cx, |s, _| s.open),
            "Later must dismiss the dialog"
        );
        assert_eq!(
            clicks.get(),
            0,
            "modal dismissal must not click the workspace"
        );
    }
    /// The host reads `updater_snapshot` as wire JSON, so exercise the same decode.
    fn wire(
        status: serde_json::Value,
        completed: Option<(&str, &str)>,
    ) -> murmur_core::updater::Snapshot {
        serde_json::from_value(serde_json::json!({
            "revision": 7,
            "updateStatus": status,
            "completedUpdate": completed.map(|(v, n)| serde_json::json!({"version": v, "notes": n})),
            "isUpdateDialogOpen": true,
        }))
        .unwrap()
    }
    fn error(forced: bool, recovery: Option<&str>) -> serde_json::Value {
        let mut v = serde_json::json!({"phase":"error","stage":"install","message":"Quit Murmur, then move it to Applications.","isForced":forced});
        if let Some(r) = recovery {
            v["recovery"] = r.into();
        }
        v
    }
    #[test]
    fn reinstall_error_offers_quit_whether_or_not_the_update_is_forced() {
        use super::{dismiss_action, is_reinstall, quit_offered};
        // Optional translocation failure: Quit and Later, as in the shipped dialog.
        let optional = wire(error(false, Some("reinstall")), None);
        assert!(is_reinstall(&optional.update_status));
        assert!(quit_offered(false, &optional.update_status));
        assert_eq!(dismiss_action(&optional, false), Some("dismiss"));
        // Forced translocation failure: Quit only.
        let forced = wire(error(true, Some("reinstall")), None);
        assert!(quit_offered(true, &forced.update_status));
        assert_eq!(dismiss_action(&forced, true), None);
        // Ordinary optional failure keeps Retry and Later, no Quit; forced adds Quit.
        let plain = wire(error(false, None), None);
        assert!(!is_reinstall(&plain.update_status));
        assert!(!quit_offered(false, &plain.update_status));
        assert!(quit_offered(
            true,
            &wire(error(true, None), None).update_status
        ));
        // Another recovery value is not a reinstall.
        assert!(!is_reinstall(
            &wire(error(false, Some("retry")), None).update_status
        ));
    }
    #[test]
    fn finished_update_notes_are_acknowledged_not_merely_dismissed() {
        use super::dismiss_action;
        let notes = Some(("1.2.0", "Synthetic release notes"));
        // Escape, the close button and Later all use this decision.
        let finished = wire(serde_json::json!({"phase":"idle"}), notes);
        assert_eq!(dismiss_action(&finished, false), Some("dismissCompleted"));
        // Release notes are always dismissible, even with a stale required flag.
        assert_eq!(dismiss_action(&finished, true), Some("dismissCompleted"));
        let plain = wire(serde_json::json!({"phase":"up-to-date"}), None);
        assert_eq!(dismiss_action(&plain, false), Some("dismiss"));
        // Forced and in-flight updates cannot be dismissed.
        let forced = wire(
            serde_json::json!({"phase":"available","version":"1.2.0","notes":"n","isForced":true}),
            None,
        );
        assert_eq!(dismiss_action(&forced, false), None);
        let downloading = wire(
            serde_json::json!({"phase":"downloading","version":"1.2.0","progress":42.0}),
            None,
        );
        assert_eq!(dismiss_action(&downloading, false), None);
        assert_eq!(dismiss_action(&plain, true), None);
    }
}
