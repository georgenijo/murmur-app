//! Home composition reuses the complete native History workspace.
#[cfg(any(test, feature = "gallery"))]
pub mod fixtures;
pub mod host;
pub mod inbox;
pub mod model;
#[cfg(test)]
mod tests;
use crate::{
    Button, ButtonIntent, ButtonSize,
    history::History,
    icons::{Glyph, icon},
    onboarding::model::{Access, Permission},
    tokens::{self, Palette},
    workspace::Authority,
};
use gpui::{prelude::*, *};
use host::{Action, Host, Request};
use inbox::Inbox;
use model::*;
use std::{collections::BTreeMap, rc::Rc, time::Duration};
pub struct Home {
    pub workspace: Option<(Destination, AnyView)>,
    pub model: Session<Snapshot>,
    history: Entity<History>,
    host: Rc<dyn Host>,
    inbox: Inbox,
    authority: Authority,
    subscription: Option<Box<dyn host::Subscription>>,
    focus: BTreeMap<String, FocusHandle>,
    pub desktop_collapsed: bool,
    pub expanded_width: f32,
    narrow: bool,
    narrow_expanded: bool,
    drag: Option<(f32, f32)>,
    warning_dismissed: bool,
    more: bool,
    scroll: ScrollHandle,
    return_to: Option<String>,
    keyboard_return: Option<FocusHandle>,
    last_permission: Option<Access>,
}
impl Home {
    pub fn new(
        host: Rc<dyn Host>,
        history_host: Rc<dyn crate::history::host::Host>,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let history = cx.new(|cx| {
            let mut history = History::new(history_host, w, cx);
            history.title = "Your dictations";
            history.set_embedded();
            history
        });
        Self {
            workspace: None,
            model: Session::default(),
            history,
            host,
            inbox: Inbox::default(),
            authority: Authority::default(),
            subscription: None,
            focus: BTreeMap::new(),
            desktop_collapsed: false,
            expanded_width: 256.,
            narrow: false,
            narrow_expanded: false,
            drag: None,
            warning_dismissed: false,
            more: false,
            scroll: ScrollHandle::new(),
            return_to: None,
            keyboard_return: None,
            last_permission: None,
        }
    }
    fn focus_for(&mut self, id: &str, cx: &mut Context<Self>) -> FocusHandle {
        self.focus
            .entry(id.into())
            .or_insert_with(|| cx.focus_handle())
            .clone()
    }
    pub fn attach(&mut self, cx: &mut Context<Self>) {
        if self.model.attached {
            return;
        }
        self.model.attach();
        self.inbox = Inbox::default();
        self.authority.sync(&self.model);
        self.subscription = Some(self.host.subscribe(
            murmur_core::events::Consumer::Main,
            self.model.attachment,
            self.inbox.clone(),
        ));
        self.request(Action::Read, cx);
        self.history.update(cx, |h, cx| {
            h.attach(cx);
            h.drain(cx);
        });
    }
    pub fn hide(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        self.detach();
        self.history.update(cx, |h, cx| h.hide(w, cx));
        cx.notify();
    }
    fn detach(&mut self) {
        self.authority.revoke();
        self.inbox.close();
        if let Some(mut s) = self.subscription.take() {
            s.stop();
        }
        self.model.detach();
        self.return_to = None;
        self.keyboard_return = None;
        self.more = false;
        self.warning_dismissed = false;
        self.drag = None;
        self.last_permission = None;
    }
    pub fn follow(&self, cx: &mut Context<Self>) {
        self.history.update(cx, |h, cx| h.follow(cx));
        let attachment = self.model.attachment;
        cx.spawn(async move |weak, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(32))
                    .await;
                if !weak
                    .update(cx, |s, cx| {
                        if !s.model.attached
                            || s.model.attachment != attachment
                            || s.inbox.is_closed()
                        {
                            return false;
                        }
                        s.drain(cx);
                        true
                    })
                    .unwrap_or(false)
                {
                    break;
                }
            }
        })
        .detach();
    }
    pub fn drain(&mut self, cx: &mut Context<Self>) {
        for e in self.inbox.drain() {
            if self.model.event(e) {
                self.authority.sync(&self.model);
                if let Some(s) = &self.model.snapshot {
                    if s.permissions[2] == Access::Denied
                        && self.last_permission != Some(Access::Denied)
                    {
                        self.warning_dismissed = false;
                    }
                    self.last_permission = Some(s.permissions[2]);
                }
                cx.notify();
            }
        }
    }
    pub fn request(&mut self, action: Action, cx: &mut Context<Self>) {
        let Some(ticket) = self.model.begin() else {
            return;
        };
        self.authority.sync(&self.model);
        self.host.request(
            Request {
                ticket,
                lease: self.authority.lease(ticket),
                action,
            },
            self.inbox.clone(),
        );
        cx.notify();
    }
    fn open(&mut self, d: Destination, id: &str, cx: &mut Context<Self>) {
        self.return_to = Some(id.into());
        self.keyboard_return = None;
        self.request(Action::Open(d), cx);
    }
    pub fn destination_closed(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        if let Some(focus) = self.keyboard_return.take() {
            self.return_to = None;
            w.focus(&focus, cx);
        } else if let Some(id) = self.return_to.take() {
            w.focus(&self.focus_for(&id, cx), cx);
        }
    }
    fn collapsed(&self) -> bool {
        if self.narrow {
            !self.narrow_expanded
        } else {
            self.desktop_collapsed
        }
    }
    pub fn toggle(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        self.cancel_drag();
        if self.narrow {
            self.narrow_expanded = !self.narrow_expanded;
        } else {
            self.desktop_collapsed = !self.desktop_collapsed;
        }
        w.focus(&self.focus_for("sidebar-toggle", cx), cx);
        cx.notify();
    }
    fn cancel_drag(&mut self) {
        if let Some((_, width)) = self.drag.take() {
            self.expanded_width = width;
        }
    }
    fn record(&mut self, cx: &mut Context<Self>) {
        let Some(s) = &self.model.snapshot else {
            return;
        };
        if s.waiting() {
            return;
        }
        let action = if s.capturing() {
            let Some(generation) = s.recording_generation else {
                return;
            };
            Action::Stop { generation }
        } else {
            Action::Start
        };
        self.request(action, cx);
    }
    fn key(&mut self, e: &KeyDownEvent, w: &mut Window, cx: &mut Context<Self>) {
        if e.prefer_character_input || e.is_held {
            return;
        }
        use crate::palette::model::Shortcut;
        let shortcut = crate::palette::main_shortcut(e, w);
        let destination = match shortcut {
            Some(Shortcut::Settings) => Some((Destination::General, "settings")),
            Some(Shortcut::Palette) => Some((Destination::Palette, "palette")),
            Some(Shortcut::Logs) => Some((Destination::Performance, "settings")),
            _ => None,
        };
        if let Some((destination, id)) = destination {
            let focus = w.focused(cx);
            self.open(destination, id, cx);
            self.keyboard_return = focus;
            w.prevent_default();
            cx.stop_propagation();
        } else if shortcut == Some(Shortcut::Search) && self.workspace.is_none() {
            self.history.update(cx, |h, cx| h.focus_search(w, cx));
            w.prevent_default();
            cx.stop_propagation();
        } else if e.keystroke.key == "escape" {
            if self.drag.is_some() {
                self.cancel_drag();
            } else if self.more {
                self.more = false;
                w.focus(&self.focus_for("permission-more", cx), cx);
            } else {
                return;
            }
            cx.notify();
            w.prevent_default();
            cx.stop_propagation();
        }
    }
    fn button(&mut self, id: &str, label: &str, d: Destination, cx: &mut Context<Self>) -> Button {
        let e = cx.entity();
        let key = id.to_string();
        Button::new(SharedString::from(id.to_string()), label.to_string())
            .track_focus(&self.focus_for(id, cx))
            .on_click(move |_, _, cx| e.update(cx, |s, cx| s.open(d.clone(), &key, cx)))
    }
}
impl Drop for Home {
    fn drop(&mut self) {
        self.detach();
    }
}
impl Render for Home {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.drain(cx);
        let p = Palette::get(cx);
        let narrow = w.viewport_size().width < px(768.);
        if narrow != self.narrow {
            self.cancel_drag();
            self.narrow = narrow;
            self.narrow_expanded = false;
            if self
                .focus
                .get("sidebar-width")
                .is_some_and(|f| f.is_focused(w))
            {
                w.focus(&self.focus_for("sidebar-toggle", cx), cx);
            }
        }
        if !w.is_window_active() {
            self.cancel_drag();
        }
        let collapsed = self.collapsed();
        let width = if collapsed { 48. } else { self.expanded_width };
        let e = cx.entity();
        let header = div()
            .relative()
            .h(px(40.))
            .flex_shrink_0()
            .when(!collapsed, |d| {
                d.child(
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .h_full()
                        .w(px(width))
                        .bg(p.surface_container_low),
                )
            })
            .flex()
            .items_center()
            .gap(px(8.))
            .pl(px(80.))
            .pr(px(16.))
            .child(
                Button::new(
                    "sidebar-toggle",
                    if collapsed {
                        "Expand sidebar"
                    } else {
                        "Collapse sidebar"
                    },
                )
                .icon_only(Glyph::PanelLeft)
                .intent(ButtonIntent::Ghost)
                .track_focus(&self.focus_for("sidebar-toggle", cx))
                .on_click(move |_, w, cx| e.update(cx, |s, cx| s.toggle(w, cx))),
            )
            .when(self.workspace.is_some(), |d| {
                let label = self
                    .model
                    .snapshot
                    .as_ref()
                    .map(|s| match s.status {
                        Status::Idle => {
                            if s.initialized {
                                "Ready"
                            } else {
                                "Initializing"
                            }
                        }
                        Status::Starting => "Connecting",
                        Status::Recording => "Recording",
                        Status::Processing => "Processing",
                        Status::Recovering => "Recovering",
                    })
                    .unwrap_or("Ready");
                d.child(
                    div()
                        .id("workspace-status")
                        .role(Role::Status)
                        .aria_label(label)
                        .rounded_full()
                        .px(px(8.))
                        .py(px(3.))
                        .text_size(tokens::CAPTION_SIZE)
                        .text_color(p.muted)
                        .bg(p.tint_raised)
                        .border_1()
                        .border_color(p.hairline_strong)
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(div().size(px(7.)).rounded_full().bg(if label == "Ready" {
                            p.success
                        } else {
                            p.accent
                        }))
                        .child(label),
                )
            })
            .child(div().flex_1())
            .child(
                self.button("settings", "Settings", Destination::General, cx)
                    .icon_only(Glyph::Settings)
                    .intent(ButtonIntent::Ghost)
                    .size(ButtonSize::Compact),
            );
        let mut nav = div()
            .id("home-sidebar")
            .role(Role::Navigation)
            .aria_label("Murmur navigation")
            .w(px(width))
            .h_full()
            .flex_shrink_0()
            .bg(p.surface_container_low)
            .border_r_1()
            .border_color(p.hairline)
            .flex()
            .flex_col()
            .p(px(8.))
            .gap(px(2.));
        nav =
            nav.child(
                div()
                    .h(px(if collapsed { 10. } else { 48. }))
                    .pb(px(if collapsed { 0. } else { 6. }))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .px(px(6.))
                    .gap(px(8.))
                    .when(!collapsed, |d| {
                        d.child(
                            div()
                                .size(px(26.))
                                .flex_shrink_0()
                                .rounded(px(9.))
                                .bg(p.accent)
                                .shadow(p.card_shadow())
                                .flex()
                                .items_center()
                                .justify_center()
                                .gap(px(2.))
                                .children([7., 14., 17., 7.].map(|h| {
                                    div().w(px(2.)).h(px(h)).rounded(px(1.)).bg(p.on_accent)
                                })),
                        )
                        .child(div().font_weight(FontWeight::SEMIBOLD).child("Murmur"))
                    }),
            );
        for (id, label, glyph, destination) in [
            ("nav-home", "Home", Glyph::House, Destination::Home),
            (
                "nav-meetings",
                "Notetaker",
                Glyph::Calendar,
                Destination::Meetings,
            ),
            (
                "nav-insights",
                "Insights",
                Glyph::Chart,
                Destination::Insights,
            ),
        ] {
            let b = self
                .button(id, label, destination.clone(), cx)
                .nav_row()
                .icon(glyph)
                .toggled(
                    self.workspace
                        .as_ref()
                        .map_or(id == "nav-home", |(selected, _)| selected == &destination),
                );
            nav = nav.child(if collapsed { b.icon_only(glyph) } else { b });
        }
        nav = nav.child(div().flex_1()).child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .p(px(if collapsed { 8. } else { 10. }))
                .rounded(tokens::CARD_RADIUS)
                .border_1()
                .border_color(p.hairline)
                .bg(p.tint_raised)
                .child(
                    div()
                        .size(px(6.))
                        .flex_shrink_0()
                        .rounded_full()
                        .bg(p.success),
                )
                .when(!collapsed, |d| {
                    d.child(
                        div()
                            .text_color(p.muted)
                            .text_size(tokens::MICRO_SIZE)
                            .line_height(tokens::CAPTION_LEADING)
                            .child("Dictation stays on this Mac."),
                    )
                }),
        );
        let inc = cx.entity();
        let dec = inc.clone();
        let separator = div()
            .id("sidebar-width")
            .role(Role::Splitter)
            .aria_label("Sidebar width")
            .on_a11y_action(AccessibleAction::Increment, move |_, _, cx| {
                inc.update(cx, |s, cx| {
                    s.expanded_width = (s.expanded_width + 16.).min(384.);
                    cx.notify();
                })
            })
            .on_a11y_action(AccessibleAction::Decrement, move |_, _, cx| {
                dec.update(cx, |s, cx| {
                    s.expanded_width = (s.expanded_width - 16.).max(256.);
                    cx.notify();
                })
            })
            .aria_numeric_value(width as f64)
            .aria_min_numeric_value(256.)
            .aria_max_numeric_value(384.)
            .track_focus(&self.focus_for("sidebar-width", cx).tab_stop(true))
            .w(px(8.))
            .h_full()
            .cursor_col_resize()
            .relative()
            .bg(transparent_black())
            .child(
                div()
                    .absolute()
                    .left(px(3.))
                    .top_0()
                    .w(px(1.))
                    .h_full()
                    .bg(p.hairline),
            )
            .border_1()
            .border_color(gpui::transparent_black())
            .focus_visible(|s| {
                s.border_color(p.focus).shadow(vec![BoxShadow {
                    inset: false,
                    color: p.focus.opacity(0.24),
                    offset: point(px(0.), px(0.)),
                    blur_radius: px(0.),
                    spread_radius: tokens::FOCUS_WIDTH,
                }])
            })
            .on_key_down(cx.listener(|s, e: &KeyDownEvent, w, cx| {
                let step = if e.keystroke.modifiers.shift {
                    32.
                } else {
                    16.
                };
                let v = match e.keystroke.key.as_str() {
                    "left" => s.expanded_width - step,
                    "right" => s.expanded_width + step,
                    "home" => 256.,
                    "end" => 384.,
                    _ => return,
                };
                s.expanded_width = v.clamp(256., 384.);
                w.prevent_default();
                cx.stop_propagation();
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|s, e: &MouseDownEvent, w, cx| {
                    s.drag = Some((e.position.x.into(), s.expanded_width));
                    w.focus(&s.focus_for("sidebar-width", cx), cx);
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|s, e: &MouseMoveEvent, _, cx| {
                if let Some((start, width)) = s.drag {
                    if e.pressed_button == Some(MouseButton::Left) {
                        s.expanded_width =
                            (width + f32::from(e.position.x) - start).clamp(256., 384.);
                        cx.notify();
                    } else {
                        s.cancel_drag();
                    }
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|s, _, _, cx| {
                    s.drag = None;
                    cx.notify();
                }),
            );
        let nav = nav.relative().when(!collapsed, |d| {
            d.child(separator.absolute().right(px(-4.)).top_0())
        });
        let mut content = div()
            .id("home-content")
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .flex()
            .flex_col()
            .gap(px(14.))
            .p(px(16.))
            .pt(px(18.));
        if let Some(problem) = self.model.problem {
            let e = cx.entity();
            content = content
                .child(
                    div()
                        .id("home-error")
                        .role(Role::Alert)
                        .aria_label(problem.message())
                        .child(problem.message()),
                )
                .child(
                    Button::new("home-reload", "Retry").on_click(move |_, _, cx| {
                        e.update(cx, |s, cx| s.request(Action::Read, cx))
                    }),
                );
        }
        if let Some(s) = self.model.snapshot.clone() {
            if !self.warning_dismissed
                && let Some((message, permission)) = s.warning()
            {
                let e = cx.entity();
                let e2 = e.clone();
                let e3 = e.clone();
                let mut warning = div()
                    .id("permission-warning")
                    .role(Role::Status)
                    .aria_label("Permission warning")
                    .flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .child(text!(message))
                    .child(
                        Button::new("permission-settings", "Open System Settings…")
                            .disabled(self.model.pending)
                            .on_click(move |_, _, cx| {
                                e.update(cx, |s, cx| {
                                    s.request(Action::PermissionSettings(permission), cx)
                                })
                            }),
                    )
                    .child(
                        Button::new("permission-more", "More")
                            .track_focus(&self.focus_for("permission-more", cx))
                            .on_click(move |_, _, cx| {
                                e2.update(cx, |s, cx| {
                                    s.more = !s.more;
                                    cx.notify();
                                })
                            }),
                    )
                    .child(
                        Button::new("dismiss-warning", "Dismiss").on_click(move |_, _, cx| {
                            e3.update(cx, |s, cx| {
                                s.warning_dismissed = true;
                                s.more = false;
                                cx.notify();
                            })
                        }),
                    );
                if self.more {
                    let e = cx.entity();
                    warning = warning.child(
                        Button::new("recheck", "Re-check permissions").on_click(move |_, _, cx| {
                            e.update(cx, |s, cx| s.request(Action::RecheckPermissions, cx))
                        }),
                    );
                    for (p, label, visible) in [
                        (
                            Permission::Microphone,
                            "Reset microphone permission…",
                            s.permissions[0] == Access::Denied,
                        ),
                        (
                            Permission::Accessibility,
                            "Reset Accessibility permission…",
                            s.permissions[1] != Access::Granted,
                        ),
                    ] {
                        if visible {
                            let e = cx.entity();
                            warning = warning.child(
                                Button::new(SharedString::from(format!("reset-{p:?}")), label)
                                    .disabled(self.model.pending)
                                    .on_click(move |_, _, cx| {
                                        e.update(cx, |s, cx| {
                                            s.request(Action::ResetPermission(p), cx)
                                        })
                                    }),
                            );
                        }
                    }
                }
                content = content.child(warning);
            }
            let e = cx.entity();
            let title = s.title();
            let hint = s.hint();
            let name = if s.status == Status::Recording {
                format!(
                    "Click to stop recording, {}:{:02}",
                    s.recording_seconds / 60,
                    s.recording_seconds % 60
                )
            } else if s.status == Status::Starting {
                "Click to cancel recording".into()
            } else {
                title.clone()
            };
            let level = (s.audio_level * 16.).clamp(0., 1.);
            let waveform = div().flex().items_center().gap(px(2.)).children(
                [0.52, 0.78, 1., 0.78, 0.52].into_iter().map(|envelope| {
                    div()
                        .w(px(2.))
                        .h(px(((0.15 + level * envelope) * 16.).round().max(3.)))
                        .rounded(px(1.))
                        .bg(p.danger)
                }),
            );
            let record_focus = self.focus_for("home-record", cx);
            let waiting = s.waiting() || self.model.pending;
            let tile = div()
                .size(px(32.))
                .flex_shrink_0()
                .rounded(tokens::CONTROL_RADIUS)
                .flex()
                .items_center()
                .justify_center()
                .bg(if s.capturing() {
                    p.danger
                } else if waiting {
                    p.surface_container_low
                } else {
                    p.accent
                })
                .text_color(if waiting { p.muted } else { p.on_accent })
                .child(
                    if waiting && crate::preferences::current(cx).reduce_motion {
                        icon(Glyph::Spinner).into_any_element()
                    } else if waiting {
                        icon(Glyph::Spinner)
                            .with_animation(
                                "talk-spinner",
                                Animation::new(Duration::from_millis(900)).repeat(),
                                |i, delta| i.transform(Transformation::rotate(percentage(delta))),
                            )
                            .into_any_element()
                    } else if s.capturing() {
                        div()
                            .size(px(10.))
                            .rounded(px(1.))
                            .bg(p.on_accent)
                            .into_any_element()
                    } else {
                        icon(Glyph::Mic).into_any_element()
                    },
                );
            let hint_view = div()
                .flex()
                .items_center()
                .gap(px(4.))
                .text_size(tokens::LABEL_SIZE)
                .line_height(tokens::BODY_LEADING)
                .text_color(p.muted);
            let hint_view = if s.status == Status::Idle && !s.waiting() {
                // Keep the configured gesture/key from the shared model, with a technical key cap.
                let key = s.settings["doubleTapKey"]
                    .as_str()
                    .unwrap_or("ControlRight");
                let label = crate::settings::schema::ROWS
                    .iter()
                    .find(|r| r.key == "doubleTapKey")
                    .and_then(|r| {
                        if let crate::settings::schema::Kind::Choice(o) = r.kind {
                            o.iter().find(|(v, _)| *v == key).map(|(_, l)| *l)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(key);
                let gesture = match s.settings["recordingMode"].as_str() {
                    Some("double_tap") => "or double-tap",
                    Some("both") => "or hold or double-tap",
                    _ => "or hold",
                };
                hint_view
                    .child(gesture)
                    .child(
                        div()
                            .font_family(tokens::MONO_FONT)
                            .text_size(tokens::MICRO_SIZE)
                            .line_height(px(14.))
                            .px(px(4.))
                            .rounded(px(3.))
                            .border_1()
                            .border_color(p.hairline_strong)
                            .bg(p.surface_container_low)
                            .child(if key.contains("Shift") {
                                format!("⇧ {label}")
                            } else {
                                label.to_owned()
                            }),
                    )
                    .child("in any app")
            } else {
                hint_view.child(hint)
            };
            let talk = div()
                .id("home-talk")
                .role(Role::Group)
                .aria_label("Dictation controls")
                .flex_shrink_0()
                .border_1()
                .border_color(if s.capturing() {
                    p.danger.opacity(0.4)
                } else {
                    p.hairline_strong
                })
                .rounded(tokens::CONTROL_RADIUS)
                .bg(p.tint_raised)
                .overflow_hidden()
                .flex()
                .flex_col()
                .shadow(p.card_shadow())
                .child(
                    gpui_base::Button::new("home-record")
                        .accessibility_label(name)
                        .track_focus(&record_focus)
                        .disabled(waiting)
                        .focus_visible(|s| {
                            s.shadow(vec![BoxShadow {
                                inset: true,
                                color: p.focus,
                                offset: point(px(0.), px(0.)),
                                blur_radius: px(0.),
                                spread_radius: tokens::FOCUS_WIDTH,
                            }])
                        })
                        .capture_key_down(|e, w, cx| {
                            if e.keystroke.key == "enter" {
                                w.prevent_default();
                                cx.stop_propagation();
                            }
                        })
                        .h(px(54.))
                        .w_full()
                        .px(px(12.))
                        .hover(|d| d.bg(p.text.opacity(0.025)))
                        .on_click(move |_, _, cx| e.update(cx, |s, cx| s.record(cx)))
                        .child(
                            div()
                                .w_full()
                                .flex()
                                .items_center()
                                .gap(px(12.))
                                .child(tile)
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .flex()
                                        .flex_col()
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap(px(8.))
                                                .child(
                                                    div()
                                                        .id("dictation-status")
                                                        .role(Role::Status)
                                                        .aria_label(title.clone())
                                                        .font_weight(FontWeight::MEDIUM)
                                                        .child(title),
                                                )
                                                .when(s.status == Status::Recording, |d| {
                                                    d.child(waveform)
                                                }),
                                        )
                                        .child(hint_view),
                                ),
                        ),
                )
                .child(
                    div()
                        .h(px(28.))
                        .border_t_1()
                        .border_color(p.hairline)
                        .bg(p.surface_container_low)
                        .px(px(12.))
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .text_size(tokens::CAPTION_SIZE)
                        .text_color(p.muted)
                        .child(
                            div()
                                .id("monthly-statistics")
                                .role(Role::Label)
                                .aria_value(format!(
                                    "This month {} words · {} dictations",
                                    s.words_this_month, s.recordings_this_month
                                ))
                                .flex()
                                .items_center()
                                .gap(px(4.))
                                .child("This month")
                                .child(
                                    div()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(p.text)
                                        .font_features(tokens::tabular_numbers())
                                        .child(s.words_this_month.to_string()),
                                )
                                .child("words ·")
                                .child(
                                    div()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(p.text)
                                        .font_features(tokens::tabular_numbers())
                                        .child(s.recordings_this_month.to_string()),
                                )
                                .child(if s.recordings_this_month == 1 {
                                    "dictation"
                                } else {
                                    "dictations"
                                }),
                        )
                        .child(div().flex_1())
                        .child(
                            self.button("insights", "View insights", Destination::Insights, cx)
                                .caption("View insights")
                                .trailing_icon(crate::icons::Glyph::ChevronRight)
                                .size(ButtonSize::ExtraSmall)
                                .intent(ButtonIntent::Ghost),
                        ),
                );
            content = content.child(talk);
            if let Some(id) = s.discovery["pendingHints"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|v| v.as_str())
                && let Some(&(id, title, detail, action)) =
                    HINTS.iter().find(|(key, _, _, _)| *key == id)
            {
                let e = cx.entity();
                let e2 = e.clone();
                content = content.child(
                    div()
                        .id("discovery-hint")
                        .role(Role::Status)
                        .aria_label(title)
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .p(px(12.))
                        .border_1()
                        .border_color(p.border)
                        .rounded(tokens::CARD_RADIUS)
                        .child(title)
                        .child(text!(detail))
                        .child(
                            Button::new("hint-action", action)
                                .disabled(self.model.pending)
                                .on_click(move |_, _, cx| {
                                    e.update(cx, |s, cx| {
                                        s.request(Action::Hint { id, open: true }, cx)
                                    })
                                }),
                        )
                        .child(
                            Button::new("hint-dismiss", format!("Dismiss {title}"))
                                .disabled(self.model.pending)
                                .on_click(move |_, _, cx| {
                                    e2.update(cx, |s, cx| {
                                        s.request(Action::Hint { id, open: false }, cx)
                                    })
                                }),
                        ),
                );
            }
            if s.checklist() {
                let e = cx.entity();
                let count = ITEMS.iter().filter(|(id, _, _, _)| s.completed(id)).count();
                let mut checklist = div()
                    .id("discovery")
                    .flex()
                    .flex_col()
                    .flex_shrink_0()
                    .bg(p.tint_raised)
                    .rounded(tokens::CONTROL_RADIUS)
                    .border_1()
                    .border_color(p.hairline_strong)
                    .child(
                        div()
                            .h(px(40.))
                            .px(px(12.))
                            .border_b_1()
                            .border_color(p.hairline)
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .child(
                                div()
                                    .text_size(tokens::MICRO_SIZE)
                                    .text_color(p.muted)
                                    .id("discovery-progress")
                                    .role(Role::Label)
                                    .aria_value(format!("Get more from Murmur · {count}/5"))
                                    .child("Get more from Murmur"),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(text!("Try what is ready when you are")),
                            )
                            .child(
                                div()
                                    .w(px(48.))
                                    .h(px(4.))
                                    .rounded_full()
                                    .bg(p.raised)
                                    .child(
                                        div()
                                            .h_full()
                                            .w(px(count as f32 / 5. * 48.))
                                            .rounded_full()
                                            .bg(p.accent),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(tokens::CAPTION_SIZE)
                                    .text_color(p.muted)
                                    .child(format!("{count}/5")),
                            )
                            .child(
                                Button::new("dismiss-checklist", "Dismiss Get more from Murmur")
                                    .icon_only(Glyph::Close)
                                    .intent(ButtonIntent::Ghost)
                                    .size(ButtonSize::ExtraSmall)
                                    .with_scroll_parent(Some(&self.scroll))
                                    .on_click(move |_, _, cx| {
                                        e.update(cx, |s, cx| {
                                            s.request(Action::DismissChecklist, cx)
                                        })
                                    }),
                            ),
                    );
                // CSS auto-fit minmax(min(100%, 19rem), 1fr), including its insets.
                let available = f32::from(w.viewport_size().width) - width - 32. - 24.;
                let columns = ((available + 12.) / (304. + 12.)).floor().max(1.) as u16;
                let mut items = div()
                    .grid()
                    .grid_cols(columns)
                    .gap_x(px(12.))
                    .px(px(12.))
                    .py(px(6.));
                for (id, title, detail, action) in ITEMS {
                    let e = cx.entity();
                    let done = s.completed(id);
                    let focus = self.focus_for(id, cx);
                    items = items.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .py(px(6.))
                            .child(
                                div()
                                    .size(px(14.))
                                    .flex_shrink_0()
                                    .rounded_full()
                                    .border_1()
                                    .border_color(if done { p.accent } else { p.border })
                                    .when(done, |d| {
                                        d.bg(p.accent).child(
                                            icon(Glyph::Check)
                                                .size(px(12.))
                                                .text_color(p.on_accent),
                                        )
                                    }),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .child(
                                        div()
                                            .id(SharedString::from(format!("checklist-title-{id}")))
                                            .role(Role::Label)
                                            .aria_value(format!(
                                                "{title}: {}",
                                                if done { "Complete" } else { "Not completed" }
                                            ))
                                            .text_size(tokens::CAPTION_SIZE)
                                            .line_height(tokens::CAPTION_LEADING)
                                            .font_weight(if done {
                                                FontWeight::MEDIUM
                                            } else {
                                                FontWeight::SEMIBOLD
                                            })
                                            .text_color(if done { p.muted } else { p.text })
                                            .child(title),
                                    )
                                    .child(
                                        div()
                                            .text_size(tokens::MICRO_SIZE)
                                            .line_height(tokens::CAPTION_LEADING)
                                            .text_color(p.muted)
                                            .child(text!(
                                                id = SharedString::from(format!(
                                                    "checklist-detail-{id}"
                                                )),
                                                detail
                                            )),
                                    ),
                            )
                            .child(
                                Button::new(
                                    id,
                                    format!(
                                        "{title}: {}",
                                        if done { "Open again" } else { action }
                                    ),
                                )
                                .caption(if done { "Open again" } else { action })
                                .intent(if done {
                                    ButtonIntent::Ghost
                                } else {
                                    ButtonIntent::Secondary
                                })
                                .track_focus(&focus)
                                .with_scroll_parent(Some(&self.scroll))
                                .size(ButtonSize::ExtraSmall)
                                .disabled(self.model.pending)
                                .on_click(move |_, _, cx| {
                                    e.update(cx, |s, cx| {
                                        s.return_to = Some(id.into());
                                        s.request(Action::Checklist(id), cx);
                                    })
                                }),
                            ),
                    );
                }
                checklist = checklist.child(items);
                content = content.child(
                    div()
                        .id("discovery-scroll")
                        .max_h(px(if narrow { 180. } else { 272. }))
                        .flex_shrink_0()
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll)
                        .child(checklist),
                );
            }
        } else {
            content = content.child(text!("Getting ready…"));
        }
        content = content.child(div().flex_1().min_h(px(120.)).child(self.history.clone()));
        div()
            .id("home")
            .capture_key_down(cx.listener(Self::key))
            .on_mouse_move(cx.listener(|s, e: &MouseMoveEvent, _, cx| {
                if let Some((start, width)) = s.drag {
                    if e.pressed_button == Some(MouseButton::Left) {
                        s.expanded_width =
                            (width + f32::from(e.position.x) - start).clamp(256., 384.);
                        cx.notify();
                    } else {
                        s.cancel_drag();
                    }
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|s, _, _, cx| {
                    if s.drag.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|s, _, _, cx| {
                    if s.drag.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .size_full()
            .flex()
            .flex_col()
            .bg(p.surface)
            .text_color(p.text)
            .text_size(tokens::BODY_SIZE)
            .line_height(tokens::BODY_LEADING)
            .child(header)
            .child(div().flex_1().min_h(px(0.)).flex().child(nav).child(
                if let Some((_, view)) = &self.workspace {
                    div()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .child(view.clone())
                        .into_any_element()
                } else {
                    content.into_any_element()
                },
            ))
    }
}

#[cfg(feature = "production-host")]
impl Home {
    pub(crate) fn production_history_ready(&self, cx: &App) -> bool {
        let h = self.history.read(cx);
        !h.model.loading && h.model.problem.is_none()
    }
    pub(crate) fn production_history_action(
        &mut self,
        action: &str,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.history
            .update(cx, |h, cx| h.production_action(action, w, cx));
    }
}
