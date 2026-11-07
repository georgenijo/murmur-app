use crate::*;
use gpui::{
    ClipboardItem, Context, Entity, FocusHandle, Focusable, IntoElement, KeyDownEvent, KeyUpEvent,
    Keystroke, Render, TestAppContext, VisualTestContext, Window, div, prelude::*, px,
};
use gpui_component::input::InputState;

struct Harness {
    button: FocusHandle,
    toggle: FocusHandle,
    after: FocusHandle,
    field: Entity<InputState>,
    choices: Entity<Choices>,
    disabled: bool,
    readonly: bool,
    default_confirm: bool,
    outer_deferred: bool,
    intent: ButtonIntent,
    clicks: usize,
    changes: usize,
    checked: bool,
}
impl Harness {
    fn new(w: &mut Window, cx: &mut Context<Self>, kind: ChoiceKind) -> Self {
        Self {
            button: cx.focus_handle(),
            toggle: cx.focus_handle(),
            after: cx.focus_handle(),
            field: cx.new(|cx| InputState::new(w, cx).default_value("A日👩‍💻e\u{301}Z")),
            choices: cx.new(|cx| {
                Choices::new(
                    "Examples",
                    kind,
                    vec![
                        Choice::new("a", "Alpha"),
                        Choice::new("b", "Unavailable").disabled(true),
                        Choice::new("c", "Charlie"),
                    ],
                    w,
                    cx,
                )
            }),
            disabled: false,
            readonly: false,
            default_confirm: false,
            outer_deferred: false,
            intent: ButtonIntent::Secondary,
            clicks: 0,
            changes: 0,
            checked: false,
        }
    }
}
impl Render for Harness {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = div()
            .id("test-root")
            .tab_group()
            .flex()
            .flex_col()
            .gap(px(8.))
            .size_full()
            .child(
                Button::new("button", "Save example")
                    .track_focus(&self.button)
                    .disabled(self.disabled)
                    .intent(self.intent)
                    .default_confirm(self.default_confirm)
                    .on_click(cx.listener(|s, _, _, _| s.clicks += 1)),
            )
            .child(
                Toggle::new("toggle", "Keep examples", self.checked)
                    .track_focus(&self.toggle)
                    .disabled(self.disabled)
                    .on_change(cx.listener(|s, checked, _, cx| {
                        s.changes += 1;
                        s.checked = *checked;
                        cx.notify();
                    })),
            )
            .child(
                TextField::new(&self.field, "Example text")
                    .disabled(self.disabled)
                    .readonly(self.readonly),
            )
            .child(self.choices.clone())
            .child(Button::new("after", "After composite").track_focus(&self.after));
        if self.outer_deferred {
            gpui::deferred(content).into_any_element()
        } else {
            content.into_any_element()
        }
    }
}
fn harness(cx: &mut TestAppContext, kind: ChoiceKind) -> (Entity<Harness>, &mut VisualTestContext) {
    cx.update(crate::init);
    let mut harness = None;
    let (_, cx) = cx.add_window_view(|w, cx| {
        let h = cx.new(|cx| Harness::new(w, cx, kind));
        harness = Some(h.clone());
        gpui_base::Root::new(h, w, cx)
    });
    draw(cx);
    (harness.unwrap(), cx)
}
fn keys(cx: &mut VisualTestContext, keys: &str) {
    for key in keys.split_whitespace() {
        let key = cx.update(|_, cx| Keystroke::parse_with_os(key, cx.platform_info().os).unwrap());
        cx.simulate_event(KeyDownEvent {
            keystroke: key.clone(),
            is_held: false,
            prefer_character_input: false,
        });
        cx.simulate_event(KeyUpEvent { keystroke: key });
        draw(cx);
    }
}
fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|w, cx| w.draw(cx).clear(cx));
}
fn focus(h: &Entity<Harness>, cx: &mut VisualTestContext, target: usize) {
    cx.update(|w, cx| {
        h.update(cx, |h, cx| {
            let handle = match target {
                0 => h.button.clone(),
                1 => h.toggle.clone(),
                2 => h.field.read(cx).focus_handle(cx),
                _ => h.choices.read(cx).focus_handle(cx),
            };
            w.focus(&handle, cx);
        })
    });
    draw(cx);
}
#[gpui::test]
fn button_space_and_return_policy(cx: &mut TestAppContext) {
    let (h, cx) = harness(cx, ChoiceKind::List);
    focus(&h, cx, 0);
    keys(cx, "enter space");
    assert_eq!(h.read_with(cx, |h, _| h.clicks), 1);
    h.update(cx, |h, cx| {
        h.default_confirm = true;
        cx.notify();
    });
    draw(cx);
    keys(cx, "enter");
    assert_eq!(h.read_with(cx, |h, _| h.clicks), 2);
}
#[gpui::test]
fn destructive_button_never_becomes_return_default(cx: &mut TestAppContext) {
    let (h, cx) = harness(cx, ChoiceKind::List);
    h.update(cx, |h, cx| {
        h.intent = ButtonIntent::Danger;
        h.default_confirm = true;
        cx.notify();
    });
    focus(&h, cx, 0);
    keys(cx, "enter space");
    assert_eq!(h.read_with(cx, |h, _| h.clicks), 1);
}
#[gpui::test]
fn repeated_or_composing_return_does_not_confirm(cx: &mut TestAppContext) {
    let (h, cx) = harness(cx, ChoiceKind::List);
    h.update(cx, |h, cx| {
        h.default_confirm = true;
        cx.notify();
    });
    focus(&h, cx, 0);
    for (is_held, prefer_character_input) in [(true, false), (false, true)] {
        let key =
            cx.update(|_, cx| Keystroke::parse_with_os("enter", cx.platform_info().os).unwrap());
        cx.simulate_event(KeyDownEvent {
            keystroke: key.clone(),
            is_held,
            prefer_character_input,
        });
        cx.simulate_event(KeyUpEvent { keystroke: key });
    }
    assert_eq!(h.read_with(cx, |h, _| h.clicks), 0);
}
#[gpui::test]
fn toggle_space_changes_once_and_disabled_is_inert(cx: &mut TestAppContext) {
    let (h, cx) = harness(cx, ChoiceKind::List);
    focus(&h, cx, 1);
    keys(cx, "space");
    draw(cx);
    assert_eq!(h.read_with(cx, |h, _| (h.checked, h.changes)), (true, 1));
    h.update(cx, |h, cx| {
        h.disabled = true;
        cx.notify();
    });
    draw(cx);
    keys(cx, "space enter");
    assert_eq!(h.read_with(cx, |h, _| h.changes), 1);
}
#[gpui::test]
fn disabled_button_keeps_traversal_but_rejects_activation(cx: &mut TestAppContext) {
    let (h, cx) = harness(cx, ChoiceKind::List);
    focus(&h, cx, 0);
    h.update(cx, |h, cx| {
        h.disabled = true;
        cx.notify();
    });
    draw(cx);
    cx.simulate_keystrokes("enter space tab");
    assert_eq!(h.read_with(cx, |h, _| h.clicks), 0);
    let expected = h.read_with(cx, |h, cx| h.choices.read(cx).focus_handle(cx));
    cx.update(|w, _| assert!(expected.is_focused(w)));
}
#[gpui::test]
fn disabled_focused_toggle_keeps_forward_and_reverse_traversal(cx: &mut TestAppContext) {
    let (h, cx) = harness(cx, ChoiceKind::List);
    for key in ["tab", "shift-tab"] {
        h.update(cx, |h, cx| {
            h.disabled = false;
            cx.notify();
        });
        focus(&h, cx, 1);
        h.update(cx, |h, cx| {
            h.disabled = true;
            cx.notify();
        });
        draw(cx);
        keys(cx, key);
        let expected = h.read_with(cx, |h, cx| {
            if key == "tab" {
                h.choices.read(cx).focus_handle(cx)
            } else {
                h.after.clone()
            }
        });
        cx.update(|w, _| assert!(expected.is_focused(w), "{key} must leave disabled toggle"));
    }
}
#[gpui::test]
fn placeholder_uses_preference_color_in_the_editor_layout(cx: &mut TestAppContext) {
    let (h, cx) = harness(cx, ChoiceKind::List);
    cx.update(|w, cx| {
        h.update(cx, |h, cx| {
            h.field.update(cx, |f, cx| {
                f.set_value("", w, cx);
                f.set_placeholder("Synthetic placeholder", w, cx);
            })
        })
    });
    for high_contrast in [false, true, false] {
        cx.update(|_, cx| {
            preferences::apply(
                preferences::DisplayPreferences {
                    increase_contrast: high_contrast,
                    ..Default::default()
                },
                cx,
            )
        });
        draw(cx);
        let actual = h.read_with(cx, |h, cx| h.field.read(cx).painted_placeholder_color());
        let expected = cx.update(|_, cx| tokens::Palette::get(cx).muted);
        assert_eq!(actual, Some(expected));
    }
}
#[gpui::test]
fn focus_order_and_reverse_traversal(cx: &mut TestAppContext) {
    let (h, cx) = harness(cx, ChoiceKind::List);
    focus(&h, cx, 0);
    for target in [1, 2, 3] {
        cx.simulate_keystrokes("tab");
        draw(cx);
        let expected = h.read_with(cx, |h, cx| match target {
            1 => h.toggle.clone(),
            2 => h.field.read(cx).focus_handle(cx),
            _ => h.choices.read(cx).focus_handle(cx),
        });
        cx.update(|w, _| assert!(expected.is_focused(w)));
    }
    cx.simulate_keystrokes("shift-tab");
    let expected = h.read_with(cx, |h, cx| h.field.read(cx).focus_handle(cx));
    cx.update(|w, _| assert!(expected.is_focused(w)));
}
fn composite_keys(cx: &mut TestAppContext, kind: ChoiceKind) {
    let (h, cx) = harness(cx, kind);
    focus(&h, cx, 3);
    let choices = h.read_with(cx, |h, _| h.choices.clone());
    if kind == ChoiceKind::Dropdown {
        keys(cx, "space");
    }
    cx.simulate_keystrokes("down enter");
    draw(cx);
    assert_eq!(
        choices.read_with(cx, |c, _| c.value().cloned()),
        Some("c".into())
    );
    if kind == ChoiceKind::Dropdown {
        keys(cx, "space");
    }
    cx.simulate_keystrokes("down space");
    draw(cx);
    assert_eq!(
        choices.read_with(cx, |c, _| c.value().cloned()),
        Some("a".into())
    );
    choices.update(cx, |c, cx| c.set_disabled(true, cx));
    draw(cx);
    cx.simulate_keystrokes("end enter space");
    assert_eq!(
        choices.read_with(cx, |c, _| c.value().cloned()),
        Some("a".into())
    );
}
#[gpui::test]
fn dropdown_keyboard_skips_disabled(cx: &mut TestAppContext) {
    composite_keys(cx, ChoiceKind::Dropdown);
}
#[gpui::test]
fn list_keyboard_skips_disabled(cx: &mut TestAppContext) {
    composite_keys(cx, ChoiceKind::List);
}
#[gpui::test]
fn navigation_keyboard_skips_disabled(cx: &mut TestAppContext) {
    composite_keys(cx, ChoiceKind::Navigation);
}
#[gpui::test]
fn dropdown_escape_and_tab_leave_selection_unchanged(cx: &mut TestAppContext) {
    let (h, cx) = harness(cx, ChoiceKind::Dropdown);
    focus(&h, cx, 3);
    cx.simulate_keystrokes("space down escape");
    draw(cx);
    assert!(h.read_with(cx, |h, cx| !h.choices.read(cx).is_open()
        && h.choices.read(cx).value().is_none()));
    cx.simulate_keystrokes("space tab");
    draw(cx);
    let after = h.read_with(cx, |h, _| h.after.clone());
    cx.update(|w, _| assert!(after.is_focused(w)));
    assert!(h.read_with(cx, |h, cx| !h.choices.read(cx).is_open()));
}
#[gpui::test]
fn dropdown_reopen_resets_active_after_every_cancellation(cx: &mut TestAppContext) {
    let (h, cx) = harness(cx, ChoiceKind::Dropdown);
    let choices = h.read_with(cx, |h, _| h.choices.clone());
    focus(&h, cx, 3);
    keys(cx, "space down escape space enter");
    assert_eq!(
        choices.read_with(cx, |c, _| c.value().cloned()),
        Some("a".into())
    );
    for cancel in ["tab", "shift-tab", "outside"] {
        focus(&h, cx, 3);
        keys(cx, "space down");
        if cancel == "outside" {
            cx.simulate_click(gpui::point(px(900.), px(600.)), gpui::Modifiers::default());
            draw(cx);
        } else {
            keys(cx, cancel);
        }
        assert!(!choices.read_with(cx, |c, _| c.is_open()));
        focus(&h, cx, 3);
        keys(cx, "space enter");
        assert_eq!(
            choices.read_with(cx, |c, _| c.value().cloned()),
            Some("a".into()),
            "{cancel}"
        );
    }
}
#[gpui::test]
fn composites_handle_empty_and_all_disabled(cx: &mut TestAppContext) {
    cx.update(crate::init);
    for choices in [
        vec![],
        vec![
            Choice::new("a", "Alpha").disabled(true),
            Choice::new("b", "Beta").disabled(true),
        ],
    ] {
        let (c, window) = cx.add_window_view(|w, cx| {
            Choices::new("Empty or disabled", ChoiceKind::List, choices, w, cx)
        });
        window.update(|w, cx| w.focus(&c.read(cx).focus_handle(cx), cx));
        draw(window);
        keys(window, "down up home end enter space");
        assert!(c.read_with(window, |c, _| c.value().is_none()));
    }
}
#[gpui::test]
fn field_keyboard_selection_edit_and_undo(cx: &mut TestAppContext) {
    let (h, cx) = harness(cx, ChoiceKind::List);
    focus(&h, cx, 2);
    cx.write_to_clipboard(ClipboardItem::new_string("Synthetic input".into()));
    cx.simulate_keystrokes("cmd-a cmd-v");
    draw(cx);
    assert_eq!(
        h.read_with(cx, |h, cx| h.field.read(cx).value()),
        "Synthetic input"
    );
    cx.simulate_keystrokes("cmd-z");
    draw(cx);
    assert_eq!(
        h.read_with(cx, |h, cx| h.field.read(cx).value()),
        "A日👩‍💻e\u{301}Z"
    );
}
#[gpui::test]
fn field_readonly_and_disabled_reject_typing(cx: &mut TestAppContext) {
    let (h, cx) = harness(cx, ChoiceKind::List);
    focus(&h, cx, 2);
    for readonly in [true, false] {
        h.update(cx, |h, cx| {
            h.readonly = readonly;
            h.disabled = !readonly;
            cx.notify();
        });
        draw(cx);
        cx.simulate_keystrokes("cmd-a");
        cx.simulate_input("Rejected");
        draw(cx);
        assert_eq!(
            h.read_with(cx, |h, cx| h.field.read(cx).value()),
            "A日👩‍💻e\u{301}Z"
        );
    }
}
#[test]
fn reduced_motion_removes_scale_travel_and_duration() {
    for motion in [
        motion::Motion::Feedback,
        motion::Motion::Toggle,
        motion::Motion::Popup,
        motion::Motion::SearchPulse,
    ] {
        let transition = motion.resolve(true);
        assert!(transition.duration.is_zero());
        assert_eq!(transition.scale, 1.);
        assert_eq!(transition.translate, 0.);
        assert!(!motion.resolve(false).duration.is_zero());
    }
}
#[gpui::test]
fn display_preferences_feed_the_actual_animation_engine(cx: &mut TestAppContext) {
    cx.update(crate::init);
    cx.update(|cx| {
        preferences::apply(
            preferences::DisplayPreferences {
                reduce_motion: true,
                increase_contrast: true,
                reduce_transparency: true,
            },
            cx,
        );
        assert!(cx.reduce_motion());
        let p = tokens::Palette::get(cx);
        assert_eq!(p.border, p.text);
        assert_eq!(p.popover(cx), p.surface);
        preferences::apply(preferences::DisplayPreferences::default(), cx);
        assert!(!cx.reduce_motion());
    });
}

#[gpui::test]
fn native_appearance_snapshot_updates_theme_and_invalidates_only_on_change(
    cx: &mut TestAppContext,
) {
    use gpui_component::{ActiveTheme, ThemeMode};
    cx.update(crate::init);
    cx.update(|cx| {
        assert!(preferences::apply_snapshot(0, cx));
        let light = tokens::Palette::get(cx);
        assert_eq!(cx.theme().mode, ThemeMode::Light);
        assert!(!preferences::apply_snapshot(0, cx));
        assert!(preferences::apply_snapshot(8, cx));
        let dark = tokens::Palette::get(cx);
        assert_eq!(cx.theme().mode, ThemeMode::Dark);
        assert_ne!(light.surface, dark.surface);
        assert_ne!(light.text, dark.text);
        assert_ne!(light.raised, dark.raised);
        assert_ne!(light.accent, dark.accent);
        assert!(!preferences::apply_snapshot(8, cx));
        // Accessibility changes keep the selected appearance and still redraw.
        assert!(preferences::apply_snapshot(8 | 7, cx));
        assert_eq!(tokens::Palette::get(cx).surface, dark.surface);
        assert_eq!(tokens::Palette::get(cx).border, dark.text);
        assert!(cx.reduce_motion());
        assert!(!preferences::apply_snapshot(8 | 7, cx));
        assert!(preferences::apply_snapshot(0, cx));
        assert_eq!(cx.theme().mode, ThemeMode::Light);
        assert_eq!(tokens::Palette::get(cx).surface, light.surface);
        assert!(!cx.reduce_motion());
    });
}

#[cfg(feature = "a11y-tests")]
mod accessibility {
    use super::*;
    use gpui::accesskit::{
        ActionData, ActionRequest, Node, NodeId, TextPosition, TextSelection, TreeId, TreeUpdate,
    };
    use gpui::{AccessibleAction, Role, Toggled};
    #[test]
    fn native_heading_exposes_the_standard_ax_role() {
        let id = NodeId(1);
        let mut heading = Node::new(Role::Heading);
        heading.set_label("Synthetic heading");
        let update = TreeUpdate {
            nodes: vec![(id, heading)],
            tree: Some(gpui::accesskit::Tree::new(id)),
            tree_id: TreeId::ROOT,
            focus: id,
        };
        let (role, _, pressable, _) = accesskit_macos::test_press(update, id);
        assert_eq!(role, "AXHeading");
        assert!(!pressable);
    }

    fn tree(cx: &mut VisualTestContext) -> TreeUpdate {
        let w = cx.update(|w, _| w.window_handle());
        cx.activate_accessibility(w);
        draw(cx);
        cx.accessibility_tree(w)
    }
    fn named<'a>(tree: &'a TreeUpdate, label: &str) -> (NodeId, &'a Node) {
        tree.nodes
            .iter()
            .find(|(_, n)| n.label() == Some(label))
            .map(|(id, n)| (*id, n))
            .expect("named node in rendered AX tree")
    }
    fn action(
        cx: &mut VisualTestContext,
        target_node: NodeId,
        action: AccessibleAction,
        data: Option<ActionData>,
    ) {
        let w = cx.update(|w, _| w.window_handle());
        cx.dispatch_accessibility_action(
            w,
            ActionRequest {
                action,
                target_tree: TreeId::ROOT,
                target_node,
                data,
            },
        );
        draw(cx);
    }
    #[gpui::test]
    fn button_role_name_action_focus_and_disabled(cx: &mut TestAppContext) {
        let (h, cx) = harness(cx, ChoiceKind::List);
        focus(&h, cx, 2);
        let t = tree(cx);
        let (id, _) = named(&t, "Save example");
        action(cx, id, AccessibleAction::Focus, None);
        let t = tree(cx);
        let (id, n) = named(&t, "Save example");
        assert_eq!(n.role(), Role::Button);
        assert!(n.supports_action(AccessibleAction::Click));
        assert_eq!(t.focus, id);
        action(cx, id, AccessibleAction::Click, None);
        assert_eq!(h.read_with(cx, |h, _| h.clicks), 1);
        h.update(cx, |h, cx| {
            h.disabled = true;
            cx.notify();
        });
        let t = tree(cx);
        let (id, n) = named(&t, "Save example");
        assert!(n.is_disabled());
        assert!(!n.supports_action(AccessibleAction::Click));
        action(cx, id, AccessibleAction::Click, None);
        assert_eq!(h.read_with(cx, |h, _| h.clicks), 1);
    }
    #[gpui::test]
    fn switch_role_name_value_action_and_focus(cx: &mut TestAppContext) {
        let (h, cx) = harness(cx, ChoiceKind::List);
        focus(&h, cx, 1);
        let t = tree(cx);
        let (id, n) = named(&t, "Keep examples");
        assert_eq!(n.role(), Role::Switch);
        assert_eq!(n.toggled(), Some(Toggled::False));
        assert_eq!(t.focus, id);
        action(cx, id, AccessibleAction::Click, None);
        let t = tree(cx);
        assert_eq!(named(&t, "Keep examples").1.toggled(), Some(Toggled::True));
        assert_eq!(h.read_with(cx, |h, _| h.changes), 1);
        h.update(cx, |h, cx| {
            h.disabled = true;
            cx.notify();
        });
        let t = tree(cx);
        let (id, n) = named(&t, "Keep examples");
        assert!(n.is_disabled());
        assert!(!n.supports_action(AccessibleAction::Click));
        action(cx, id, AccessibleAction::Click, None);
        assert_eq!(h.read_with(cx, |h, _| h.changes), 1);
    }
    fn composite_tree(cx: &mut TestAppContext, kind: ChoiceKind, role: Role, child_role: Role) {
        let (h, cx) = harness(cx, kind);
        focus(&h, cx, 3);
        if kind == ChoiceKind::Dropdown {
            keys(cx, "space");
        }
        let t = tree(cx);
        assert!(
            t.nodes
                .iter()
                .any(|(_, n)| n.label() == Some("Examples") && n.role() == role)
        );
        let owner = t
            .nodes
            .iter()
            .find(|(_, n)| n.role() == role && n.label() == Some("Examples"))
            .unwrap();
        if kind == ChoiceKind::Dropdown {
            assert_eq!(owner.1.is_expanded(), Some(true));
        }
        let alpha = named(&t, "Alpha").0;
        assert_eq!(t.focus, alpha);
        fn descendant(t: &TreeUpdate, parent: NodeId, child: NodeId) -> bool {
            t.nodes
                .iter()
                .find(|(id, _)| *id == parent)
                .unwrap()
                .1
                .children()
                .iter()
                .any(|id| *id == child || descendant(t, *id, child))
        }
        assert!(descendant(&t, owner.0, alpha));
        keys(cx, "down");
        let t = tree(cx);
        let (id, n) = named(&t, "Charlie");
        assert!(descendant(&t, owner.0, id));
        assert_eq!(t.focus, id);
        assert_eq!(n.role(), child_role);
        assert!(n.supports_action(AccessibleAction::Click));
        let (_, n) = named(&t, "Unavailable");
        assert!(n.is_disabled());
        assert!(!n.supports_action(AccessibleAction::Click));
        action(cx, id, AccessibleAction::Click, None);
        assert_eq!(
            h.read_with(cx, |h, cx| h.choices.read(cx).value().cloned()),
            Some("c".into())
        );
        let t = tree(cx);
        if kind == ChoiceKind::Dropdown {
            let (_, n) = named(&t, "Examples");
            assert_eq!(n.value(), Some("Charlie"));
            assert_eq!(n.is_expanded(), Some(false));
        } else {
            assert_eq!(named(&t, "Charlie").1.is_selected(), Some(true));
        }
    }
    #[gpui::test]
    fn dropdown_tree_and_actions(cx: &mut TestAppContext) {
        composite_tree(
            cx,
            ChoiceKind::Dropdown,
            Role::ComboBox,
            Role::ListBoxOption,
        );
    }
    #[gpui::test]
    fn nested_deferred_popup_retains_ancestry_and_active_focus(cx: &mut TestAppContext) {
        let (h, cx) = harness(cx, ChoiceKind::Dropdown);
        h.update(cx, |h, cx| {
            h.outer_deferred = true;
            cx.notify();
        });
        focus(&h, cx, 3);
        keys(cx, "space");
        for label in ["Alpha", "Charlie", "Alpha"] {
            let t = tree(cx);
            let (option, _) = named(&t, label);
            assert_eq!(t.focus, option);
            let combo = t
                .nodes
                .iter()
                .find(|(_, n)| n.role() == Role::ComboBox)
                .unwrap();
            let list = t
                .nodes
                .iter()
                .find(|(_, n)| n.role() == Role::ListBox)
                .unwrap();
            assert!(combo.1.children().contains(&list.0));
            assert!(list.1.children().contains(&option));
            let consumer = accesskit_consumer::Tree::new(t, true);
            assert!(consumer.state().focus().unwrap().is_focused());
            keys(cx, "down");
        }
    }
    #[gpui::test]
    fn list_tree_and_actions(cx: &mut TestAppContext) {
        composite_tree(cx, ChoiceKind::List, Role::ListBox, Role::ListBoxOption);
    }
    #[gpui::test]
    fn navigation_tree_and_actions(cx: &mut TestAppContext) {
        composite_tree(cx, ChoiceKind::Navigation, Role::TabList, Role::Tab);
    }
    #[gpui::test]
    fn pointer_activation_and_popup_selection_use_real_hitboxes(cx: &mut TestAppContext) {
        fn click_named(cx: &mut VisualTestContext, label: &str) {
            let t = tree(cx);
            let bounds = named(&t, label)
                .1
                .bounds()
                .expect("painted node has bounds");
            let scale = cx.update(|w, _| w.scale_factor()) as f64;
            cx.simulate_click(
                gpui::point(
                    px(((bounds.x0 + bounds.x1) / (2. * scale)) as f32),
                    px(((bounds.y0 + bounds.y1) / (2. * scale)) as f32),
                ),
                gpui::Modifiers::default(),
            );
            draw(cx);
        }
        let (h, cx) = harness(cx, ChoiceKind::Dropdown);
        click_named(cx, "Save example");
        assert_eq!(h.read_with(cx, |h, _| h.clicks), 1);
        click_named(cx, "Keep examples");
        assert_eq!(h.read_with(cx, |h, _| h.changes), 1);
        click_named(cx, "Examples");
        assert!(h.read_with(cx, |h, cx| h.choices.read(cx).is_open()));
        click_named(cx, "Charlie");
        assert_eq!(
            h.read_with(cx, |h, cx| h.choices.read(cx).value().cloned()),
            Some("c".into())
        );
        click_named(cx, "Examples");
        cx.simulate_click(gpui::point(px(900.), px(600.)), gpui::Modifiers::default());
        draw(cx);
        assert!(!h.read_with(cx, |h, cx| h.choices.read(cx).is_open()));
    }
    #[gpui::test]
    fn field_tree_unicode_selection_and_replace_selected_text(cx: &mut TestAppContext) {
        let (h, cx) = harness(cx, ChoiceKind::List);
        focus(&h, cx, 2);
        let t = tree(cx);
        let (id, n) = named(&t, "Example text");
        assert_eq!(n.role(), Role::TextInput);
        assert_eq!(n.value(), Some("A日👩‍💻e\u{301}Z"));
        assert_eq!(t.focus, id);
        for a in [
            AccessibleAction::SetValue,
            AccessibleAction::SetTextSelection,
            AccessibleAction::ReplaceSelectedText,
        ] {
            assert!(n.supports_action(a));
        }
        let selection = n.text_selection().expect("selection attributes");
        let run = t
            .nodes
            .iter()
            .find(|(id, _)| *id == selection.anchor.node)
            .unwrap();
        assert_eq!(run.1.role(), Role::TextRun);
        assert_eq!(run.1.character_lengths(), &[1, 3, 11, 3, 1]);
        let selection = TextSelection {
            anchor: TextPosition {
                node: run.0,
                character_index: 3,
            },
            focus: TextPosition {
                node: run.0,
                character_index: 1,
            },
        };
        action(
            cx,
            id,
            AccessibleAction::SetTextSelection,
            Some(ActionData::SetTextSelection(selection)),
        );
        assert_eq!(
            h.read_with(cx, |h, cx| h.field.read(cx).selected_range()),
            1..15
        );
        let t = tree(cx);
        let s = named(&t, "Example text").1.text_selection().unwrap();
        assert_eq!((s.anchor.character_index, s.focus.character_index), (3, 1));
        for (node, index) in [(run.0, usize::MAX), (NodeId(u64::MAX), 0)] {
            action(
                cx,
                id,
                AccessibleAction::SetTextSelection,
                Some(ActionData::SetTextSelection(TextSelection {
                    anchor: TextPosition {
                        node,
                        character_index: index,
                    },
                    focus: TextPosition {
                        node,
                        character_index: 0,
                    },
                })),
            );
            assert_eq!(
                h.read_with(cx, |h, cx| h.field.read(cx).selected_range()),
                1..15
            );
        }
        action(
            cx,
            id,
            AccessibleAction::ReplaceSelectedText,
            Some(ActionData::Value("替".into())),
        );
        assert_eq!(
            h.read_with(cx, |h, cx| h.field.read(cx).value()),
            "A替e\u{301}Z"
        );
        action(
            cx,
            id,
            AccessibleAction::SetValue,
            Some(ActionData::Value("New synthetic value".into())),
        );
        assert_eq!(
            h.read_with(cx, |h, cx| h.field.read(cx).value()),
            "New synthetic value"
        );
    }
    #[gpui::test]
    fn field_range_bounds_follow_shaping_and_horizontal_scroll(cx: &mut TestAppContext) {
        let (h, cx) = harness(cx, ChoiceKind::List);
        focus(&h, cx, 2);
        let sample = "A日👩‍💻e\u{301}Z".repeat(80);
        cx.update(|w, cx| {
            h.update(cx, |h, cx| {
                h.field
                    .update(cx, |f, cx| f.set_value(sample.clone(), w, cx))
            })
        });
        let mut boxes = Vec::new();
        let mut scrolls = Vec::new();
        for key in ["cmd-left", "cmd-right"] {
            keys(cx, key);
            let t = tree(cx);
            let (id, _) = named(&t, "Example text");
            let consumer = accesskit_consumer::Tree::new(t, true);
            let node = consumer
                .state()
                .node_by_tree_local_id(id, TreeId::ROOT)
                .unwrap();
            let mut selected = node.document_range();
            selected.set_start(node.text_position_from_global_utf16_index(1).unwrap());
            selected.set_end(node.text_position_from_global_utf16_index(7).unwrap());
            let caret = node.text_selection().unwrap().bounding_boxes();
            let painted = h.read_with(cx, |h, cx| h.field.read(cx).cursor_layout().unwrap().0);
            let scale = cx.update(|w, _| w.scale_factor());
            assert_eq!(caret.len(), 1);
            assert!((caret[0].x0 - f64::from(f32::from(painted.origin.x) * scale)).abs() < 1.);
            assert_eq!(selected.text(), "日👩‍💻");
            let rects = selected.bounding_boxes();
            assert_eq!(
                rects.len(),
                1,
                "AXFrameForRange requires usable consumer geometry"
            );
            assert!(rects[0].width() > 0. && rects[0].height() > 0.);
            boxes.push(rects[0]);
            scrolls.push(h.read_with(cx, |h, cx| h.field.read(cx).scroll_offset().x));
        }
        let scale = cx.update(|w, _| w.scale_factor());
        assert!(scrolls[1] < scrolls[0], "long field must actually scroll");
        let expected = f32::from(scrolls[1] - scrolls[0]) * scale;
        assert!(((boxes[1].x0 - boxes[0].x0) - f64::from(expected)).abs() < 1.);
        assert!((boxes[1].width() - boxes[0].width()).abs() < 1.);
    }
    #[cfg(target_os = "macos")]
    #[gpui::test]
    fn rtl_value_does_not_advertise_range_geometry(cx: &mut TestAppContext) {
        assert_unsupported_direction(cx, &["אבג", "مرحبا"]);
    }
    #[cfg(target_os = "macos")]
    #[gpui::test]
    fn mixed_direction_value_does_not_advertise_range_geometry(cx: &mut TestAppContext) {
        assert_unsupported_direction(cx, &["abc אבג def", "abc مرحبا 日", "אבג abc"]);
    }
    #[cfg(target_os = "macos")]
    #[gpui::test]
    fn arabic_ligature_does_not_advertise_range_geometry(cx: &mut TestAppContext) {
        // CoreText can shape lam-alef as a single glyph. Its byte indices are
        // trivially sorted, so glyph order cannot establish text direction.
        assert_unsupported_direction(cx, &["لا", "abc لا def"]);
    }
    #[cfg(target_os = "macos")]
    #[gpui::test]
    fn bidi_controls_do_not_advertise_range_geometry(cx: &mut TestAppContext) {
        assert_unsupported_direction(
            cx,
            &[
                "abc\u{200f}def", // RLM
                "abc\u{061c}def", // ALM
                "abc\u{202a}def", // LRE
                "abc\u{202b}def", // RLE
                "abc\u{202c}def", // PDF
                "abc\u{202d}def", // LRO
                "abc\u{202e}def", // RLO
                "abc\u{2066}def", // LRI
                "abc\u{2067}def", // RLI
                "abc\u{2068}def", // FSI
                "abc\u{2069}def", // PDI
            ],
        );
    }
    #[cfg(target_os = "macos")]
    fn shaped_context(cx: &TestAppContext) -> TestAppContext {
        // The default NoopTextSystem never produces RTL glyph order. Use the
        // production shaper directly, without constructing a native platform.
        TestAppContext::build_with_text_system(
            cx.dispatcher.clone(),
            None,
            std::sync::Arc::new(gpui_apple::AppleTextSystem::new()),
        )
    }
    #[cfg(target_os = "macos")]
    fn assert_unsupported_direction(cx: &mut TestAppContext, samples: &[&str]) {
        let mut shaped = shaped_context(cx);
        let (h, cx) = harness(&mut shaped, ChoiceKind::List);
        focus(&h, cx, 2);
        for &sample in samples {
            cx.update(|w, cx| {
                h.update(cx, |h, cx| {
                    h.field.update(cx, |f, cx| f.set_value(sample, w, cx))
                })
            });
            let t = tree(cx);
            let (id, n) = named(&t, "Example text");
            assert_eq!(n.value(), Some(sample));
            assert!(n.supports_action(AccessibleAction::SetValue));
            assert!(n.text_selection().is_none(), "{sample}");
            assert!(!n.supports_action(AccessibleAction::SetTextSelection));
            assert!(!n.supports_action(AccessibleAction::ReplaceSelectedText));
            assert!(!t.nodes.iter().any(|(_, n)| n.role() == Role::TextRun));
            let consumer = accesskit_consumer::Tree::new(t, true);
            assert!(
                !consumer
                    .state()
                    .node_by_tree_local_id(id, TreeId::ROOT)
                    .unwrap()
                    .supports_text_ranges(),
                "{sample}"
            );
        }
        // A later supported value must regain ranges on its first layout.
        cx.update(|w, cx| {
            h.update(cx, |h, cx| {
                h.field.update(cx, |f, cx| f.set_value("abc", w, cx))
            })
        });
        assert!(
            named(&tree(cx), "Example text")
                .1
                .text_selection()
                .is_some()
        );
    }
    #[cfg(target_os = "macos")]
    #[gpui::test]
    fn ltr_character_bounds_match_painted_carets(cx: &mut TestAppContext) {
        let mut shaped = shaped_context(cx);
        let (h, cx) = harness(&mut shaped, ChoiceKind::List);
        focus(&h, cx, 2);
        for sample in ["ASCII", "é日🙂", "e\u{301}👩‍💻Z"] {
            cx.update(|w, cx| {
                h.update(cx, |h, cx| {
                    h.field.update(cx, |f, cx| f.set_value(sample, w, cx))
                })
            });
            keys(cx, "cmd-left");
            let t = tree(cx);
            let (id, n) = named(&t, "Example text");
            assert!(n.supports_action(AccessibleAction::SetTextSelection));
            assert!(n.supports_action(AccessibleAction::ReplaceSelectedText));
            let run_id = n.text_selection().unwrap().anchor.node;
            let run = &t.nodes.iter().find(|(id, _)| *id == run_id).unwrap().1;
            assert_eq!(
                run.text_direction(),
                Some(gpui::accesskit::TextDirection::LeftToRight)
            );
            let lengths = run.character_lengths().to_vec();
            let consumer = accesskit_consumer::Tree::new(t, true);
            let node = consumer
                .state()
                .node_by_tree_local_id(id, TreeId::ROOT)
                .unwrap();
            assert!(node.supports_text_ranges());
            let scale = cx.update(|w, _| f64::from(w.scale_factor()));
            let caret_x = |cx: &VisualTestContext| {
                h.read_with(cx, |h, cx| {
                    f64::from(f32::from(
                        h.field.read(cx).cursor_layout().unwrap().0.origin.x,
                    )) * scale
                })
            };
            let mut start_x = caret_x(cx);
            let (mut byte, mut utf16) = (0, 0);
            for (index, length) in lengths.into_iter().enumerate() {
                let end_byte = byte + usize::from(length);
                let character = &sample[byte..end_byte];
                let end_utf16 = utf16 + character.encode_utf16().count();
                let mut range = node.document_range();
                range.set_start(node.text_position_from_global_utf16_index(utf16).unwrap());
                range.set_end(
                    node.text_position_from_global_utf16_index(end_utf16)
                        .unwrap(),
                );
                assert_eq!(range.text(), character);
                let rects = range.bounding_boxes();
                assert_eq!(rects.len(), 1);
                assert!(
                    rects[0].width() > 0. && rects[0].height() > 0.,
                    "{character}"
                );
                let end = TextPosition {
                    node: run_id,
                    character_index: index + 1,
                };
                action(
                    cx,
                    id,
                    AccessibleAction::SetTextSelection,
                    Some(ActionData::SetTextSelection(TextSelection {
                        anchor: end,
                        focus: end,
                    })),
                );
                let end_x = caret_x(cx);
                assert!((rects[0].x0 - start_x).abs() < 1., "{character}");
                assert!((rects[0].x1 - end_x).abs() < 1., "{character}");
                start_x = end_x;
                byte = end_byte;
                utf16 = end_utf16;
            }
            assert_eq!(byte, sample.len());
        }
    }
    #[gpui::test]
    fn multiline_value_does_not_advertise_unimplemented_range_geometry(cx: &mut TestAppContext) {
        struct Multiline {
            field: Entity<gpui_component::input::TextareaState>,
        }
        impl Render for Multiline {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                gpui_component::input::Textarea::new(&self.field).aria_label("Multiline example")
            }
        }
        cx.update(crate::init);
        let (_, cx) = cx.add_window_view(|w, cx| {
            let field = cx.new(|cx| {
                gpui_component::input::TextareaState::new(w, cx).default_value("First\nSecond")
            });
            let content = cx.new(|_| Multiline { field });
            gpui_base::Root::new(content, w, cx)
        });
        let t = tree(cx);
        let (id, n) = named(&t, "Multiline example");
        assert_eq!(n.value(), Some("First\nSecond"));
        assert!(n.text_selection().is_none());
        assert!(!n.supports_action(AccessibleAction::SetTextSelection));
        assert!(!n.supports_action(AccessibleAction::ReplaceSelectedText));
        let consumer = accesskit_consumer::Tree::new(t, true);
        assert!(
            !consumer
                .state()
                .node_by_tree_local_id(id, TreeId::ROOT)
                .unwrap()
                .supports_text_ranges()
        );
    }
    #[gpui::test]
    fn field_readonly_disabled_and_masked_keep_ax_boundaries(cx: &mut TestAppContext) {
        let (h, cx) = harness(cx, ChoiceKind::List);
        focus(&h, cx, 2);
        h.update(cx, |h, cx| {
            h.readonly = true;
            cx.notify();
        });
        let t = tree(cx);
        let (id, n) = named(&t, "Example text");
        assert!(n.is_read_only());
        assert!(n.supports_action(AccessibleAction::SetTextSelection));
        let run = n.text_selection().unwrap().anchor.node;
        action(
            cx,
            id,
            AccessibleAction::SetTextSelection,
            Some(ActionData::SetTextSelection(TextSelection {
                anchor: TextPosition {
                    node: run,
                    character_index: 1,
                },
                focus: TextPosition {
                    node: run,
                    character_index: 3,
                },
            })),
        );
        assert_eq!(
            h.read_with(cx, |h, cx| h.field.read(cx).selected_range()),
            1..15
        );
        assert!(!n.supports_action(AccessibleAction::SetValue));
        assert!(!n.supports_action(AccessibleAction::ReplaceSelectedText));
        action(
            cx,
            id,
            AccessibleAction::SetValue,
            Some(ActionData::Value("Rejected".into())),
        );
        assert_eq!(
            h.read_with(cx, |h, cx| h.field.read(cx).value()),
            "A日👩‍💻e\u{301}Z"
        );
        h.update(cx, |h, cx| {
            h.readonly = false;
            h.disabled = true;
            cx.notify();
        });
        let t = tree(cx);
        assert!(named(&t, "Example text").1.is_disabled());
        assert!(
            !named(&t, "Example text")
                .1
                .supports_action(AccessibleAction::SetValue)
        );
        cx.update(|w, cx| {
            h.update(cx, |h, cx| {
                h.field.update(cx, |f, cx| f.set_masked(true, w, cx))
            })
        });
        let t = tree(cx);
        let (_, n) = named(&t, "Example text");
        assert_eq!(n.value(), None);
        assert_eq!(n.text_selection(), None);
    }
    #[gpui::test]
    fn huge_combining_cluster_has_lossless_text_run_lengths(cx: &mut TestAppContext) {
        let (h, cx) = harness(cx, ChoiceKind::List);
        let text = format!("a{}z", "\u{301}".repeat(200));
        cx.update(|w, cx| {
            h.update(cx, |h, cx| {
                h.field.update(cx, |f, cx| f.set_value(text.clone(), w, cx))
            })
        });
        let t = tree(cx);
        let (_, n) = named(&t, "Example text");
        let run_id = n.text_selection().unwrap().anchor.node;
        let run = &t.nodes.iter().find(|(id, _)| *id == run_id).unwrap().1;
        assert_eq!(
            run.character_lengths()
                .iter()
                .map(|n| usize::from(*n))
                .sum::<usize>(),
            text.len()
        );
    }
}

#[gpui::test]
fn actual_transition_finishes_immediately_when_motion_is_reduced(cx: &mut TestAppContext) {
    struct Probe {
        target: f32,
        observed: std::rc::Rc<std::cell::Cell<f32>>,
    }
    impl Render for Probe {
        fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let policy = motion::Motion::Toggle.resolve(cx.reduce_motion());
            let value = gpui_base::transition(
                "switch-travel",
                self.target,
                gpui_base::Transition::new(policy.duration),
                w,
                cx,
            );
            self.observed.set(value);
            div().size(px(20.))
        }
    }
    cx.update(crate::init);
    let observed = std::rc::Rc::new(std::cell::Cell::new(0.));
    let result = observed.clone();
    let (probe, cx) = cx.add_window_view(|_, _| Probe {
        target: 0.,
        observed,
    });
    draw(cx);
    probe.update(cx, |p, cx| {
        p.target = 12.;
        cx.notify();
    });
    draw(cx);
    assert!(
        result.get() < 12.,
        "ordinary motion must have an intermediate state"
    );
    cx.update(|_, cx| {
        preferences::apply(
            preferences::DisplayPreferences {
                reduce_motion: true,
                ..Default::default()
            },
            cx,
        )
    });
    draw(cx);
    assert_eq!(result.get(), 12.);
}

#[gpui::test]
fn app_clipboard_provider_never_falls_back_to_platform(cx: &mut TestAppContext) {
    use std::{cell::RefCell, rc::Rc};
    struct Isolated(RefCell<Option<String>>);
    impl gpui::ClipboardProvider for Isolated {
        fn read(&self) -> Option<ClipboardItem> {
            self.0.borrow().clone().map(ClipboardItem::new_string)
        }
        fn write(&self, item: ClipboardItem) {
            *self.0.borrow_mut() = item.text();
        }
    }
    cx.update(|cx| {
        cx.write_to_clipboard(ClipboardItem::new_string("synthetic-platform-value".into()));
        cx.set_clipboard_provider(Rc::new(Isolated(RefCell::new(None))));
        assert!(cx.read_from_clipboard().is_none());
        assert!(
            futures::executor::block_on(cx.read_from_clipboard_async())
                .unwrap()
                .is_none()
        );
        cx.write_to_clipboard(ClipboardItem::new_string("synthetic-private-value".into()));
        assert_eq!(
            cx.read_from_clipboard().and_then(|v| v.text()).as_deref(),
            Some("synthetic-private-value")
        );
    });
}
