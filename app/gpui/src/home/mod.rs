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
            history
        });
        Self {
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
        let k = &e.keystroke;
        if k.modifiers.platform && k.modifiers.number_of_modifiers() == 1 && k.key == "," {
            let focus = w.focused(cx);
            self.open(Destination::General, "settings", cx);
            self.keyboard_return = focus;
            w.prevent_default();
            cx.stop_propagation();
        } else if k.modifiers.platform && k.modifiers.number_of_modifiers() == 1 && k.key == "k" {
            let focus = w.focused(cx);
            self.open(Destination::Palette, "palette", cx);
            self.keyboard_return = focus;
            w.prevent_default();
            cx.stop_propagation();
        } else if k.modifiers.platform && k.modifiers.number_of_modifiers() == 1 && k.key == "f" {
            self.history.update(cx, |h, cx| h.focus_search(w, cx));
            w.prevent_default();
            cx.stop_propagation();
        } else if k.key == "escape" {
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
            .h(px(40.))
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
                .caption("☰")
                .track_focus(&self.focus_for("sidebar-toggle", cx))
                .on_click(move |_, w, cx| e.update(cx, |s, cx| s.toggle(w, cx))),
            )
            .child(div().flex_1())
            .child(
                self.button("palette", "Command palette", Destination::Palette, cx)
                    .size(ButtonSize::Compact),
            )
            .child(
                self.button("settings", "Settings", Destination::General, cx)
                    .size(ButtonSize::Compact),
            );
        let mut nav = div()
            .id("home-sidebar")
            .role(Role::Navigation)
            .aria_label("Murmur navigation")
            .w(px(width))
            .h_full()
            .flex_shrink_0()
            .border_r_1()
            .border_color(p.border)
            .flex()
            .flex_col()
            .p(px(8.))
            .gap(px(8.));
        if !collapsed {
            nav = nav.child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(text!("Murmur")),
            );
        }
        for (id, label, short, d) in [
            ("nav-home", "Home", "H", Destination::Home),
            ("nav-meetings", "Notetaker", "N", Destination::Meetings),
            ("nav-insights", "Insights", "I", Destination::Insights),
        ] {
            let b = self
                .button(id, label, d, cx)
                .toggled(id == "nav-home")
                .intent(if id == "nav-home" {
                    ButtonIntent::Secondary
                } else {
                    ButtonIntent::Ghost
                });
            nav = nav.child(if collapsed { b.caption(short) } else { b });
        }
        nav = nav.child(div().flex_1());
        if !collapsed {
            nav = nav.child(
                div()
                    .text_color(p.muted)
                    .text_size(px(12.))
                    .child(text!("Dictation stays on this Mac.")),
            );
        }
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
            .w(px(6.))
            .h_full()
            .cursor_col_resize()
            .bg(p.border.opacity(0.4))
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
        let mut content = div()
            .id("home-content")
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .flex()
            .flex_col()
            .gap(px(12.))
            .p(px(if narrow { 16. } else { 24. }));
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
                        .bg(p.accent)
                }),
            );
            let talk = div()
                .id("home-talk")
                .role(Role::Group)
                .aria_label("Dictation controls")
                .border_1()
                .border_color(p.border)
                .rounded(tokens::CONTROL_RADIUS)
                .flex()
                .flex_col()
                .gap(px(8.))
                .p(px(8.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .child(
                            Button::new("home-record", name)
                                .caption(if s.capturing() { "Stop" } else { "Start" })
                                .track_focus(&self.focus_for("home-record", cx))
                                .intent(ButtonIntent::Primary)
                                .disabled(s.waiting() || self.model.pending)
                                .on_click(move |_, _, cx| e.update(cx, |s, cx| s.record(cx))),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .id("dictation-status")
                                        .role(Role::Status)
                                        .aria_label(title.clone())
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(title),
                                )
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .line_height(px(16.))
                                        .text_color(p.muted)
                                        .child(text!(hint)),
                                ),
                        ),
                )
                .when(s.status == Status::Recording, |d| d.child(waveform))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(8.))
                        .child(text!(format!(
                            "This month: {} words, {} {}",
                            s.words_this_month,
                            s.recordings_this_month,
                            if s.recordings_this_month == 1 {
                                "dictation"
                            } else {
                                "dictations"
                            }
                        )))
                        .child(
                            self.button("insights", "View insights", Destination::Insights, cx)
                                .size(ButtonSize::Compact)
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
                    .gap(px(8.))
                    .p(px(12.))
                    .rounded(tokens::CARD_RADIUS)
                    .border_1()
                    .border_color(p.border)
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(8.))
                            .child(text!(format!("Get more from Murmur · {count}/5")))
                            .child(
                                Button::new("dismiss-checklist", "Dismiss Get more from Murmur")
                                    .with_scroll_parent(Some(&self.scroll))
                                    .size(ButtonSize::Compact)
                                    .on_click(move |_, _, cx| {
                                        e.update(cx, |s, cx| {
                                            s.request(Action::DismissChecklist, cx)
                                        })
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(text!("Try what is ready when you are")),
                    );
                for (id, title, detail, action) in ITEMS {
                    let e = cx.entity();
                    let done = s.completed(id);
                    let focus = self.focus_for(id, cx);
                    checklist = checklist.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
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
                                            .child(format!(
                                                "{}{}",
                                                if done { "✓ " } else { "" },
                                                title
                                            )),
                                    )
                                    .child(div().text_size(px(12.)).text_color(p.muted).child(
                                        text!(
                                            id = SharedString::from(format!(
                                                "checklist-detail-{id}"
                                            )),
                                            detail
                                        ),
                                    )),
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
                                .track_focus(&focus)
                                .with_scroll_parent(Some(&self.scroll))
                                .size(ButtonSize::Compact)
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
                content = content.child(
                    div()
                        .id("discovery-scroll")
                        .max_h(px(if narrow { 180. } else { 260. }))
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
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .flex()
                    .child(nav)
                    .when(!collapsed, |d| d.child(separator))
                    .child(content),
            )
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
