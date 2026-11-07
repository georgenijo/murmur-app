//! Windowless platform-input tests. No OS keyboard/mouse input or pasteboard.
use gpui::{
    Context, IntoElement, MouseButton, MouseDownEvent, MouseUpEvent, PlatformInput, Render, Role,
    TestAppContext, Window, div, point, prelude::*, px, size,
};
struct View(usize);
impl Render for View {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(
            div()
                .id("zoom-target")
                .role(Role::Button)
                .aria_label("Synthetic zoom target")
                .w(px(40.))
                .h(px(30.))
                .on_click(cx.listener(|s, _, _, cx| {
                    s.0 += 1;
                    cx.notify();
                })),
        )
    }
}
#[gpui::test]
fn whole_window_zoom_preserves_geometry_and_native_hit_testing(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, _| View(0));
    cx.update(|w, cx| {
        w.draw(cx).clear(cx);
    });
    let (bounds, viewport, scale) =
        cx.update(|w, _| (w.bounds(), w.viewport_size(), w.scale_factor()));
    cx.update(|w, cx| {
        assert!(w.set_ui_zoom(2., cx));
        w.draw(cx).clear(cx);
    });
    cx.update(|w, _| {
        assert_eq!(w.bounds(), bounds);
        assert_eq!(w.viewport_size(), viewport / 2.);
        assert_eq!(w.scale_factor(), scale * 2.);
    });
    // Layout target ends at x=40. Native point x=60 must map inside it at 2x.
    cx.update(|w, cx| {
        w.dispatch_platform_event(
            PlatformInput::MouseDown(MouseDownEvent {
                position: point(px(60.), px(20.)),
                button: MouseButton::Left,
                click_count: 1,
                ..Default::default()
            }),
            cx,
        );
        w.dispatch_platform_event(
            PlatformInput::MouseUp(MouseUpEvent {
                position: point(px(60.), px(20.)),
                button: MouseButton::Left,
                click_count: 1,
                ..Default::default()
            }),
            cx,
        );
    });
    assert_eq!(view.read_with(cx, |s, _| s.0), 1);
    cx.simulate_scale_factor_change(1.);
    cx.update(|w, cx| {
        w.bounds_changed(cx);
        assert_eq!(w.ui_zoom(), 2.);
        assert_eq!(w.scale_factor(), 2.);
    });
    cx.simulate_resize(size(px(800.), px(600.)));
    cx.update(|w, cx| {
        w.bounds_changed(cx);
        assert_eq!(w.ui_zoom(), 2.);
        assert_eq!(w.viewport_size(), size(px(400.), px(300.)));
        assert!(!w.set_ui_zoom(f32::NAN, cx));
        assert!(!w.set_ui_zoom(0., cx));
        assert!(w.set_ui_zoom(1., cx));
    });
}
#[cfg(feature = "a11y-tests")]
#[gpui::test]
fn zoom_scales_ax_bounds_and_ax_click_is_not_scaled_twice(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, _| View(0));
    let handle = cx.update(|w, _| w.window_handle());
    cx.activate_accessibility(handle);
    cx.update(|w, cx| {
        w.draw(cx).clear(cx);
    });
    let tree = cx.accessibility_tree(handle);
    let before = tree
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Synthetic zoom target"))
        .unwrap()
        .1
        .bounds()
        .unwrap();
    cx.update(|w, cx| {
        w.set_ui_zoom(2., cx);
        w.draw(cx).clear(cx);
    });
    let tree = cx.accessibility_tree(handle);
    let (id, node) = tree
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Synthetic zoom target"))
        .unwrap();
    let after = node.bounds().unwrap();
    assert_eq!(after.width(), before.width() * 2.);
    assert_eq!(after.height(), before.height() * 2.);
    cx.dispatch_accessibility_action(
        handle,
        gpui::accesskit::ActionRequest {
            action: gpui::accesskit::Action::Click,
            target_tree: gpui::accesskit::TreeId::ROOT,
            target_node: *id,
            data: None,
        },
    );
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |s, _| s.0), 1);
}
