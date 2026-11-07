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
            intent: ButtonIntent::Secondary,
            clicks: 0,
            changes: 0,
            checked: false,
        }
    }
}
impl Render for Harness {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
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
            .child(Button::new("after", "After composite").track_focus(&self.after))
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
fn composites_handle_empty_and_all_disabled(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let (c, cx) =
        cx.add_window_view(|w, cx| Choices::new("Empty", ChoiceKind::List, vec![], w, cx));
    cx.update(|w, cx| w.focus(&c.read(cx).focus_handle(cx), cx));
    draw(cx);
    cx.simulate_keystrokes("down up home end enter space");
    assert!(c.read_with(cx, |c, _| c.value().is_none()));
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

#[cfg(feature = "a11y-tests")]
mod accessibility {
    use super::*;
    use gpui::accesskit::{
        ActionData, ActionRequest, Node, NodeId, TextPosition, TextSelection, TreeId, TreeUpdate,
    };
    use gpui::{AccessibleAction, Role, Toggled};
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
        focus(&h, cx, 0);
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
        let (id, n) = named(&t, "Charlie");
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
