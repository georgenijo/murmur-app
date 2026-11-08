//! Native overlay screen. Hosts supply committed snapshots and typed events;
//! no Tauri, WebKit, persistence, microphone, pasteboard or inference dependency.
pub mod inbox;
pub mod model;
pub mod native;

use crate::{Button, ButtonSize, Toggle, preferences, tokens};
use gpui::{prelude::*, *};
use model::*;
use murmur_core::overlay_geometry::OverlayFrame;
use std::{rc::Rc, time::Instant};

fn size_easing(floating: bool) -> gpui_base::Easing {
    if floating {
        gpui_base::Easing::cubic_bezier(0.32, 0.72, 0., 1.)
    } else {
        gpui_base::Easing::cubic_bezier(0.34, 1.56, 0.64, 1.)
    }
    .expect("CSS overlay curve")
}

/// Nonblocking intents go to the existing service owners. Implementations must
/// recheck generation/admission and use the established owned-field settings API.
/// A UI must never run capture, completion publication or shutdown itself.
pub trait Host {
    fn submit(&self, intent: Intent);
    /// Same named interest as the targeted-event router; hidden/priority-cue
    /// overlays release their waveform interest without stopping native control.
    fn waveform_interest(&self, consumer: murmur_core::events::WaveformConsumer, active: bool);
    /// 3.3 preview owner consumes the applied collapsed anchor and hover lift.
    /// No provisional text, clipboard data or logs travel through this method.
    fn preview_anchor(&self, frame: OverlayFrame, above: bool, lift: f64);
}

pub struct Overlay {
    pub model: Model,
    pub inbox: inbox::Inbox,
    deadline: Option<(u64, Task<()>)>,
    follower: Option<Task<()>>,
    screen_watch: Option<native::ScreenWatch>,
    surface_writer: SurfaceWriter,
    host: Rc<dyn Host>,
    epoch: Instant,
    now: u64,
    applied: Option<native::Surface>,
    last_indicator: Option<Indicator>,
    last_hidden: Option<bool>,
    visibility: Option<VisibilityMotion>,
    interested: bool,
    pointer: PointerPresence,
    closed_pointer: Option<Point<Pixels>>,
}
impl Overlay {
    pub fn new(snapshot: Snapshot, host: Rc<dyn Host>) -> Self {
        Self {
            model: Model::new(snapshot),
            inbox: inbox::Inbox::default(),
            deadline: None,
            follower: None,
            screen_watch: None,
            surface_writer: SurfaceWriter::default(),
            host,
            epoch: Instant::now(),
            now: 0,
            applied: None,
            last_indicator: None,
            last_hidden: None,
            visibility: None,
            interested: false,
            pointer: PointerPresence::default(),
            closed_pointer: None,
        }
    }
    pub fn receive(&mut self, event: Event, window: &mut Window, cx: &mut Context<Self>) {
        self.now = self.epoch.elapsed().as_millis() as u64;
        self.model.event(event, self.now);
        self.advance(window, cx);
    }
    /// Settings calibration must commit geometry through the same writer as
    /// lifecycle updates; rendering alone never moves an AppKit panel.
    pub fn calibrate(
        &mut self,
        offset: Option<f64>,
        finish: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(offset) = offset {
            self.model.snapshot.offset = offset;
        }
        self.receive(Event::Calibration(!finish), window, cx);
    }
    pub fn audio_level(
        &mut self,
        generation: u64,
        level: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.model.sample(generation, level, self.now);
        if self.model.waveform(self.now) {
            window.request_animation_frame();
            cx.notify();
        }
    }
    fn send(&self, intent: Option<Intent>) {
        if let Some(intent) = intent {
            self.host.submit(intent);
        }
    }
    fn toggle_quick_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.now = self.epoch.elapsed().as_millis() as u64;
        if self.model.phase == Phase::Open {
            self.close_quick_settings(window);
        } else {
            self.closed_pointer = None;
            self.model.expand(self.now);
        }
        self.advance(window, cx);
    }
    fn close_quick_settings(&mut self, window: &Window) {
        // AX/keyboard activation may close before any tracking event or sample.
        // Only the visit actually present at close belongs to this action.
        let inside = native::pointer_inside(window).unwrap_or(self.pointer.inside());
        self.pointer.observe(inside);
        self.closed_pointer = inside.then(|| self.pointer_position(window));
        self.model.close(self.now, inside);
    }
    fn pointer_position(&self, window: &Window) -> Point<Pixels> {
        native::pointer_position(window).unwrap_or(window.bounds().origin + window.mouse_position())
    }
    fn observe_pointer_at(&mut self, inside: bool, position: Point<Pixels>, now: u64) -> bool {
        if !self.model.hover_suppressed() {
            self.closed_pointer = None;
        }
        if let Some(previous) = self.closed_pointer {
            // Resizing/tracking-area reconciliation can report an exit without
            // cursor movement. It cannot end an explicitly closed pointer visit.
            if !inside && position == previous {
                return false;
            }
            self.closed_pointer = inside.then_some(position);
        }
        self.observe_pointer(inside, now)
    }
    /// Tracking events and watchdog samples share transition ownership. A
    /// delivered exit cancels dwell immediately, even between two inside
    /// samples; repeated inside observations never move the deadline.
    fn observe_pointer(&mut self, inside: bool, now: u64) -> bool {
        if !inside {
            self.model.rearm_hover();
        }
        if self.pointer.observe(inside) {
            self.model.hover(inside, now);
            true
        } else {
            false
        }
    }
    pub fn advance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.now = self.epoch.elapsed().as_millis() as u64;
        let prefs = preferences::current(cx);
        let dark = !self.model.geometry(false, self.now).floating
            || matches!(
                window.appearance(),
                WindowAppearance::Dark | WindowAppearance::VibrantDark
            );
        let floating = self.model.geometry(false, self.now).floating;
        let hidden = self.model.hidden(self.now);
        if !floating || prefs.reduce_motion {
            self.visibility = None;
        } else if self.last_hidden != Some(hidden) {
            let from = self.visibility.map(|fade| fade.value(self.now)).unwrap_or(
                if self.last_hidden.unwrap_or(true) {
                    0.
                } else {
                    1.
                },
            );
            let to = if hidden { 0. } else { 1. };
            self.visibility = (from != to).then_some(VisibilityMotion {
                from,
                to,
                started: self.now,
            });
        }
        self.last_hidden = Some(hidden);
        let fading = self.visibility.is_some_and(|fade| fade.running(self.now));
        let was_closing = self.model.phase == Phase::Closing;
        let intent = self
            .model
            .tick(self.now, prefs.reduce_motion, |frame, hidden| {
                native::apply(
                    window,
                    native::Surface {
                        frame,
                        floating,
                        hidden,
                        reduce_transparency: prefs.reduce_transparency,
                        increase_contrast: prefs.increase_contrast,
                        dark,
                        surface_height: frame.height,
                        radius: if frame.height <= 34. { 17. } else { 12. },
                    },
                    cx,
                )
            });
        self.send(intent);
        let expanded = matches!(self.model.phase, Phase::Open | Phase::Closing);
        let frame = self.surface_writer.resolve(
            self.model.frame(expanded, self.now),
            self.applied.map(|a| a.frame),
            self.now,
            floating,
            !prefs.reduce_motion && !was_closing,
        );
        let visual = self.model.frame(self.model.phase == Phase::Open, self.now);
        let surface = native::Surface {
            frame,
            floating,
            hidden: hidden && !fading,
            reduce_transparency: prefs.reduce_transparency,
            increase_contrast: prefs.increase_contrast,
            dark,
            surface_height: visual.height,
            radius: if self.model.phase == Phase::Open {
                12.
            } else {
                17.
            },
        };
        if self.applied != Some(surface) && native::apply(window, surface, cx) {
            self.applied = Some(surface);
            self.host.preview_anchor(
                self.model.frame(false, self.now),
                floating,
                if self.model.phase == Phase::Open {
                    self.model.geometry(true, self.now).dropdown_h
                } else {
                    0.
                },
            );
        }
        // Fading-out pixels remain for 180 ms, but input stops immediately.
        native::pointer_enabled(window, !hidden);
        let interest = self.model.waveform(self.now);
        if interest != self.interested {
            self.interested = interest;
            self.host
                .waveform_interest(murmur_core::events::WaveformConsumer::Overlay, interest);
        }
        let indicator = self.model.indicator(self.now);
        if self.last_indicator != Some(indicator) && self.model.visible {
            native::announce(window, indicator.label());
        }
        self.last_indicator = Some(indicator);
        self.schedule(window, cx);
        cx.notify();
    }
    fn schedule(&mut self, window: &Window, cx: &mut Context<Self>) {
        let deadline = self
            .model
            .next_deadline(self.now)
            .into_iter()
            .chain(self.surface_writer.deadline())
            .chain(
                self.visibility
                    .filter(|fade| fade.running(self.now))
                    .map(|fade| fade.started + PILL_FADE_MS),
            )
            .min();
        if self.deadline.as_ref().map(|(at, _)| *at) == deadline {
            return;
        }
        self.deadline = deadline.map(|at| {
            let handle = window.window_handle();
            let delay = std::time::Duration::from_millis(at.saturating_sub(self.now));
            let task = cx.spawn(async move |weak, cx| {
                cx.background_executor().timer(delay).await;
                let _ = cx.update_window(handle, |_, w, cx| {
                    let _ = weak.update(cx, |s, cx| {
                        s.deadline = None;
                        s.advance(w, cx);
                    });
                });
            });
            (at, task)
        });
    }
    /// Publishers wake a parked task. Neither an idle watchdog nor a pointer
    /// poller is necessary: tracking events own hover; AppKit owns topology
    /// notifications; semantic deadlines get a cancellable one-shot timer.
    pub fn follow(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if cx
            .background_executor()
            .scheduler_executor()
            .scheduler()
            .as_test()
            .is_none()
        {
            self.screen_watch = native::watch_screen(self.inbox.clone());
        }
        let inbox = self.inbox.clone();
        let handle = window.window_handle();
        self.follower = Some(cx.spawn(async move |weak, cx| {
            loop {
                inbox.ready().await;
                if !cx
                    .update_window(handle, |_, w, cx| {
                        weak.update(cx, |s, cx| {
                            let (events, level) = s.inbox.drain();
                            let mut changed = false;
                            for event in events {
                                match event {
                                    Event::Dock(limit) if limit == s.model.screen.dock => continue,
                                    Event::RefreshScreen | Event::RefreshDock => {
                                        let dock_changed = matches!(event, Event::RefreshDock);
                                        if let Some(mut screen) = native::screen() {
                                            if !dock_changed
                                                && screen.bounds == s.model.screen.bounds
                                                && screen.scale_factor
                                                    == s.model.screen.scale_factor
                                            {
                                                screen.dock = s.model.screen.dock;
                                            }
                                            s.model.event(Event::Screen(screen), s.now);
                                        }
                                    }
                                    event => {
                                        s.model.event(event, s.epoch.elapsed().as_millis() as u64)
                                    }
                                }
                                changed = true;
                            }
                            if let Some((generation, level)) = level {
                                s.model.sample(generation, level, s.now);
                                changed |= s.model.waveform(s.now);
                            }
                            if changed {
                                s.advance(w, cx);
                            }
                        })
                    })
                    .is_ok_and(|result| result.is_ok())
                {
                    break;
                }
            }
        }));
        cx.observe_global::<preferences::DisplayPreferences>(|s, cx| {
            let _ = s.inbox.publish(Event::RefreshScreen);
            cx.notify();
        })
        .detach();
        cx.observe_window_appearance(window, |s, w, cx| s.advance(w, cx))
            .detach();
        self.schedule(window, cx);
    }
}
impl Drop for Overlay {
    fn drop(&mut self) {
        if self.interested {
            self.host
                .waveform_interest(murmur_core::events::WaveformConsumer::Overlay, false);
        }
    }
}
impl Render for Overlay {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = self.now;
        let fading = self.visibility.is_some_and(|fade| fade.running(now));
        if (self.model.waveform(now) || fading) && !cx.reduce_motion() {
            let weak = cx.entity().downgrade();
            window.on_next_frame(move |_, cx| {
                let _ = weak.update(cx, |s, cx| {
                    s.now = s.epoch.elapsed().as_millis() as u64;
                    cx.notify();
                });
            });
        }
        let indicator = self.model.indicator(now);
        let expanded = self.model.phase == Phase::Open;
        let g = self.model.geometry(expanded, now);
        let p = tokens::Palette::overlay(
            g.floating,
            matches!(
                window.appearance(),
                WindowAppearance::Dark | WindowAppearance::VibrantDark
            ),
            cx,
        );
        let root = div()
            .id("overlay-root")
            .tab_group()
            .on_key_down(cx.listener(|s, e: &KeyDownEvent, w, cx| {
                if e.keystroke.key == "escape" {
                    s.now = s.epoch.elapsed().as_millis() as u64;
                    s.close_quick_settings(w);
                    cx.stop_propagation();
                    w.prevent_default();
                    s.advance(w, cx);
                }
            }))
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .when(g.floating, |s| s.justify_end())
            .when(!g.floating, |s| s.justify_start());
        if self.model.hidden(now) && !fading {
            return root.into_any_element();
        }
        let policy = crate::motion::Motion::Overlay.resolve(cx.reduce_motion());
        let timing = gpui_base::Transition::new(policy.duration).easing(size_easing(g.floating));
        let width = gpui_base::transition(
            "overlay-width",
            px(g.window_w as f32),
            timing.clone(),
            window,
            cx,
        );
        let height = gpui_base::transition(
            "overlay-height",
            px(if expanded {
                g.expanded_h as f32
            } else {
                g.collapsed_h as f32
            }),
            timing.clone(),
            window,
            cx,
        );
        let radius = if g.floating && !expanded {
            17.
        } else if indicator == Indicator::Idle && !expanded {
            8.
        } else {
            12.
        };
        native::refit_material(window, width, height, radius, g.floating);
        let opacity = self.visibility.map_or(1., |fade| fade.value(now));
        native::material_opacity(window, opacity);
        let mut card = div()
            .id("overlay-surface")
            .opacity(opacity)
            .flex_shrink_0()
            .debug_selector(|| "overlay-surface".into())
            .relative()
            .flex()
            .flex_col()
            .items_center()
            .w(width)
            .h(height)
            .when(g.floating, |s| s.rounded(px(radius)))
            .when(!g.floating, |s| s.rounded_b(px(radius)))
            .overflow_hidden()
            .text_size(tokens::MICRO_SIZE)
            .line_height(tokens::CAPTION_LEADING)
            .text_color(p.text)
            // Tint belongs above the AppKit effect in the same Metal scene as
            // the foreground. NSVisualEffectView's backing color is not a tint.
            .bg(if g.floating {
                p.surface.opacity(
                    if preferences::current(cx).reduce_transparency
                        || preferences::current(cx).increase_contrast
                    {
                        1.
                    } else if matches!(
                        window.appearance(),
                        WindowAppearance::Dark | WindowAppearance::VibrantDark
                    ) {
                        0.74
                    } else {
                        0.76
                    },
                )
            } else {
                tokens::overlay_surface().opacity(
                    if preferences::current(cx).reduce_transparency
                        || preferences::current(cx).increase_contrast
                    {
                        1.
                    } else {
                        0.92
                    },
                )
            })
            .when(g.floating, |s| {
                s.child(
                    div()
                        .absolute()
                        .inset_0()
                        .rounded(px(radius))
                        .border_1()
                        .border_color(p.border),
                )
            })
            .hover_listener_mode(HoverListenerMode::InputModalityIndependent)
            .on_hover_with_source({
                let listener =
                    cx.listener(|s, (inside, source): &(bool, HoverEventSource), w, cx| {
                        let now = s.epoch.elapsed().as_millis() as u64;
                        // Recheck deferred layout snapshots, but retain delivered
                        // pointer history: a real exit releases close ownership even
                        // when the pointer is already back inside at dispatch time.
                        let (inside, position) = match source {
                            HoverEventSource::Layout => (
                                native::pointer_inside(w).unwrap_or(*inside),
                                s.pointer_position(w),
                            ),
                            HoverEventSource::Pointer(position) => {
                                (*inside, w.bounds().origin + *position)
                            }
                        };
                        if s.observe_pointer_at(inside, position, now) {
                            s.advance(w, cx);
                        }
                    });
                move |inside, source, w, cx| listener(&(*inside, source), w, cx)
            });
        // A separate bounded affordance keeps recording/status clicks and
        // child quick controls independent. AX and pointer input share its
        // exact laid-out rectangle and action, without taking key focus.
        let disclosure_owner = cx.entity().downgrade();
        let disclosure = div()
            .id("overlay-quick-settings-disclosure")
            .debug_selector(|| "overlay-quick-settings-disclosure".into())
            .role(Role::Button)
            .aria_label(if expanded {
                "Quick settings, expanded"
            } else {
                "Quick settings, collapsed"
            })
            .w(px(20.))
            .h(px(24.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .text_size(px(8.))
            .absolute()
            .left_0()
            .when(g.floating, |s| s.bottom(px(5.)))
            .when(!g.floating, |s| {
                s.top(px((g.collapsed_h as f32 - 24.) / 2.))
            })
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|s, _, w, cx| {
                cx.stop_propagation();
                s.toggle_quick_settings(w, cx);
            }))
            .on_a11y_action(AccessibleAction::Click, move |_, w, cx| {
                let _ = disclosure_owner.update(cx, |s, cx| s.toggle_quick_settings(w, cx));
            });
        let accessible = cx.entity().downgrade();
        let mut status = div()
            .id("overlay-status")
            .debug_selector(|| "overlay-status".into())
            .role(Role::Status)
            .aria_label(indicator.label())
            .aria_description("Start locked dictation, or stop the current recording")
            .on_a11y_action(AccessibleAction::Click, move |_, w, cx| {
                let _ = accessible.update(cx, |s, cx| {
                    let action = s.model.accessible_activate(s.now);
                    s.send(action);
                    s.advance(w, cx);
                });
            })
            .w_full()
            .h(px(g.collapsed_h as f32))
            // Keep the status row within the painted card at every transition
            // frame. Quick controls retain their minimum content height while
            // growing, so they must never participate in this row's placement.
            .absolute()
            .left_0()
            .when(g.floating, |s| s.bottom_0())
            .when(!g.floating, |s| s.top_0())
            .flex()
            .items_center()
            .flex_shrink_0()
            // Reserve the disclosure inside the existing fixed-width pill;
            // its width replaces padding/gap, never the waveform or retry.
            .when(g.floating, |s| {
                let plain_recording = indicator == Indicator::Recording && !expanded;
                s.justify_center()
                    .gap(px(if plain_recording { 8. } else { 6. }))
                    .px(px(12.))
            })
            .on_click(cx.listener(|s, e: &ClickEvent, w, cx| {
                let action = if matches!(e, ClickEvent::Keyboard(_)) {
                    s.model.accessible_activate(s.now)
                } else {
                    s.model.click(e.click_count(), s.now)
                };
                s.send(action);
                s.advance(w, cx);
            }));
        let glyph: &[u8] = match indicator {
            Indicator::Done if g.floating => {
                include_bytes!("../../vendor/gpui-component/crates/assets/assets/icons/check.svg")
            }
            Indicator::Delivery(Delivery::Pasted | Delivery::Clipboard) => {
                include_bytes!("../../vendor/gpui-component/crates/assets/assets/icons/check.svg")
            }
            Indicator::Cancelled | Indicator::Delivery(Delivery::Nothing) => {
                include_bytes!("../../vendor/gpui-component/crates/assets/assets/icons/x.svg")
            }
            Indicator::Failure(Failure::Permission | Failure::ChooseInput) if !g.floating => {
                include_bytes!("../../vendor/gpui-component/crates/assets/assets/icons/mic-off.svg")
            }
            Indicator::TapMissed | Indicator::Failure(_) if !g.floating => include_bytes!(
                "../../vendor/gpui-component/crates/assets/assets/icons/circle-alert.svg"
            ),
            Indicator::TapMissed | Indicator::Failure(_) => include_bytes!(
                "../../vendor/gpui-component/crates/assets/assets/icons/triangle-alert.svg"
            ),
            Indicator::Starting | Indicator::Slow => {
                include_bytes!("../../vendor/gpui-component/crates/assets/assets/icons/circle.svg")
            }
            Indicator::Calibrating => include_bytes!(
                "../../vendor/gpui-component/crates/assets/assets/icons/arrow-up-down.svg"
            ),
            Indicator::Delivery(Delivery::Retry | Delivery::Retrying) => include_bytes!(
                "../../vendor/gpui-component/crates/assets/assets/icons/rotate-cw.svg"
            ),
            _ => include_bytes!("../../vendor/gpui-component/crates/assets/assets/icons/mic.svg"),
        };
        let mut leading = div()
            .id("overlay-leading")
            .debug_selector(|| "overlay-leading".into())
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .when(!g.floating, |s| s.w(px(g.wing_w as f32)));
        let mut trailing = div()
            .id("overlay-trailing")
            .debug_selector(|| "overlay-trailing".into())
            .flex()
            .flex_shrink_0()
            .whitespace_nowrap()
            .items_center()
            .justify_center()
            .when(!g.floating, |s| s.w(px(g.wing_w as f32)));
        let state_glyph = div()
            .debug_selector(|| "overlay-state-glyph".into())
            .flex()
            .whitespace_nowrap()
            .flex_shrink_0()
            .text_color(match indicator {
                Indicator::Recording => p.danger,
                Indicator::Starting if !g.floating => rgb(0x38bdf8).into(),
                Indicator::Slow | Indicator::Recovering | Indicator::TapMissed if !g.floating => {
                    rgb(0xfbbf24).into()
                }
                Indicator::Cancelled | Indicator::Failure(_) if !g.floating => rgb(0xef4444).into(),
                _ => p.text,
            })
            .when(
                indicator == Indicator::Idle || (indicator == Indicator::Done && !g.floating),
                |s| {
                    s.opacity(if self.model.snapshot.disabled {
                        0.15
                    } else {
                        0.4
                    })
                },
            )
            .when(indicator == Indicator::Recording, |s| {
                s.size(px(10.)).rounded_full().bg(p.danger)
            })
            .when(indicator == Indicator::Recording, |s| {
                s.child(
                    div()
                        .debug_selector(|| "overlay-state-symbol".into())
                        .size(px(10.))
                        .rounded_full()
                        .bg(p.danger),
                )
            })
            .when(
                matches!(indicator, Indicator::Starting | Indicator::Slow) && !g.floating,
                |s| {
                    s.size(px(10.))
                        .rounded_full()
                        .bg(if indicator == Indicator::Slow {
                            rgb(0xfbbf24)
                        } else {
                            rgb(0x38bdf8)
                        })
                },
            )
            .when(indicator == Indicator::Meeting, |s| {
                s.child(
                    div()
                        .debug_selector(|| "overlay-state-symbol".into())
                        .flex()
                        .gap(px(2.))
                        .child(
                            div()
                                .size(px(if g.floating { 10. } else { 8. }))
                                .rounded_full()
                                .bg(if g.floating {
                                    p.danger
                                } else {
                                    rgb(0x67e8f9).into()
                                }),
                        )
                        .when(!g.floating, |s| {
                            s.child(div().size(px(8.)).rounded_full().bg(rgb(0xc4b5fd)))
                        }),
                )
            })
            .when(
                !matches!(indicator, Indicator::Recording | Indicator::Meeting)
                    && (g.floating || !matches!(indicator, Indicator::Starting | Indicator::Slow)),
                |s| {
                    s.child(
                        div()
                            .debug_selector(|| "overlay-state-symbol".into())
                            .flex_shrink_0()
                            .child(gpui_component::Icon::default().data(glyph).size(px(12.))),
                    )
                },
            );
        if matches!(
            indicator,
            Indicator::Processing | Indicator::Recovering | Indicator::MeetingProcessing
        ) {
            leading = leading.child(
                gpui_component::Icon::default()
                    .data(include_bytes!(
                        "../../vendor/gpui-component/crates/assets/assets/icons/loader.svg"
                    ))
                    .size(px(12.))
                    .text_color(if !g.floating && indicator == Indicator::Recovering {
                        rgb(0xfcd34d).into()
                    } else if !g.floating && indicator == Indicator::MeetingProcessing {
                        rgb(0x67e8f9).into()
                    } else {
                        p.text
                    })
                    .with_animation(
                        "processing-spinner",
                        Animation::new(std::time::Duration::from_millis(1000)).repeat(),
                        |s, delta| s.transform(Transformation::rotate(percentage(delta))),
                    ),
            );
        } else if matches!(
            indicator,
            Indicator::Starting | Indicator::Slow | Indicator::Recording | Indicator::Meeting
        ) {
            leading = leading.child(
                state_glyph.with_animation(
                    "recording-pulse",
                    Animation::new(std::time::Duration::from_millis(
                        if indicator == Indicator::Recording {
                            800
                        } else if indicator == Indicator::Meeting {
                            2000
                        } else {
                            1000
                        },
                    ))
                    .repeat(),
                    |s, delta| {
                        let half = if delta <= 0.5 {
                            delta * 2.
                        } else {
                            (1. - delta) * 2.
                        };
                        let eased = gpui_base::Easing::EaseInOut.sample(half);
                        s.opacity(1. - 0.5 * eased)
                    },
                ),
            );
        } else {
            leading = leading.child(state_glyph);
        }
        status = status.child(disclosure);
        if g.floating
            && !matches!(indicator, Indicator::Recording)
            && !matches!(indicator, Indicator::Delivery(d) if d.retryable())
        {
            let word = match indicator {
                Indicator::Failure(Failure::Retry) => "Mic failed",
                Indicator::Failure(Failure::Permission) => "Mic denied",
                Indicator::Failure(Failure::ChooseInput) => "No mic",
                Indicator::Failure(Failure::Partial) => "Mic lost",
                Indicator::Starting | Indicator::Slow => "Connecting",
                Indicator::Processing => "Transcribing…",
                Indicator::Recovering => "Recovering",
                Indicator::Calibrating => "Calibrating",
                Indicator::Meeting => "Meeting",
                Indicator::MeetingProcessing => "Finishing",
                Indicator::Delivery(Delivery::Retrying) => "Trying…",
                Indicator::Delivery(Delivery::Nothing) => "Nothing",
                Indicator::Delivery(Delivery::Pasted) => "Pasted",
                _ => indicator.label(),
            };
            trailing = trailing.child(word);
        }
        if self.model.waveform(now) {
            trailing = trailing.child(
                div()
                    .id("overlay-waveform")
                    .debug_selector(|| "overlay-waveform".into())
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .gap(px(1.5))
                    .children(
                        self.model
                            .bars_at(
                                if g.floating { 18 } else { 7 },
                                if cx.reduce_motion() { 0 } else { now },
                            )
                            .into_iter()
                            .map(|height| {
                                div()
                                    .w(px(2.))
                                    .h(px(if g.floating { 16. } else { 14. } * height))
                                    .rounded_full()
                                    .bg(p.text)
                            }),
                    ),
            );
        }
        if matches!(indicator,Indicator::Delivery(d) if d.retryable()) {
            trailing = trailing.child(
                div()
                    .id("delivery-action")
                    .debug_selector(|| "delivery-action".into())
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|s, _, w, cx| {
                            s.model.pause_delivery(s.now);
                            s.advance(w, cx);
                        }),
                    )
                    .on_hover(cx.listener(|s, inside: &bool, w, cx| {
                        if !inside {
                            s.model
                                .resume_delivery(s.epoch.elapsed().as_millis() as u64);
                            s.advance(w, cx);
                        }
                    }))
                    .child(
                        Button::new("retry-delivery", "Try again")
                            .caption(if g.floating {
                                if matches!(indicator, Indicator::Delivery(Delivery::Clipboard)) {
                                    "Paste"
                                } else {
                                    "Try again"
                                }
                            } else {
                                "↻"
                            })
                            .palette(p)
                            .size(ButtonSize::ExtraSmall)
                            .on_click(cx.listener(|s, _, w, cx| {
                                cx.stop_propagation();
                                let action = s.model.retry(s.now);
                                s.send(action);
                                s.advance(w, cx);
                            })),
                    ),
            );
        }
        // The shared geometry reserves a fixed wing on each side of the
        // physical (or pretend) notch. Never pad a full-width row: even a few
        // points would push the waveform behind the hardware cutout.
        status = status
            .child(leading)
            .when(!g.floating, |s| s.child(div().flex_1()))
            .child(trailing);
        let controls = if expanded {
            let mut group = div()
                .id("overlay-quick-settings")
                .role(Role::Group)
                .aria_label("Quick settings")
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(4.))
                .w_full()
                .h(px(g.dropdown_h as f32))
                .absolute()
                .left_0()
                .when(g.floating, |s| s.bottom(px(g.collapsed_h as f32)))
                .when(!g.floating, |s| s.top(px(g.collapsed_h as f32)))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
            if let Some(suggestion) = &self.model.suggestion {
                let accept = suggestion.token.clone();
                let dismiss = accept.clone();
                group = group
                    .child(format!("{} started. Start Notetaker?", suggestion.title))
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(
                                Button::new("accept-suggestion", "Accept")
                                    .disabled(suggestion.busy)
                                    .palette(p)
                                    .size(ButtonSize::Compact)
                                    .on_click(cx.listener(move |s, _, _, cx| {
                                        let action = s.model.suggestion_action(&accept, true);
                                        s.send(action);
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("dismiss-suggestion", "Dismiss")
                                    .disabled(suggestion.busy)
                                    .palette(p)
                                    .size(ButtonSize::Compact)
                                    .on_click(cx.listener(move |s, _, _, cx| {
                                        let action = s.model.suggestion_action(&dismiss, false);
                                        s.send(action);
                                        cx.notify();
                                    })),
                            ),
                    );
                if suggestion.failed {
                    group = group.child("Notetaker could not start. Try again.");
                }
            } else {
                let mode = self.model.snapshot.next_mode.as_ref().map_or_else(
                    || self.model.snapshot.mode.clone(),
                    |s| format!("Next: {s}"),
                );
                let paste_label = if self.model.snapshot.file_output {
                    "Auto-paste paused while saving files"
                } else {
                    "Auto-paste"
                };
                group = group.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(
                            Button::new("cycle-mode", format!("Mode: {mode}. Click to cycle"))
                                .caption(mode.chars().take(14).collect::<String>())
                                .palette(p)
                                .size(ButtonSize::ExtraSmall)
                                .on_click(
                                    cx.listener(|s, _, _, _| s.host.submit(Intent::CycleMode)),
                                ),
                        )
                        .child(
                            Button::new(
                                "power",
                                if self.model.snapshot.disabled {
                                    "Enable Murmur"
                                } else {
                                    "Disable Murmur"
                                },
                            )
                            .caption("⏻")
                            .palette(p)
                            .size(ButtonSize::ExtraSmall)
                            .on_click(cx.listener(|s, _, _, _| {
                                s.host.submit(Intent::Disabled(!s.model.snapshot.disabled))
                            })),
                        )
                        .child(
                            Toggle::new(
                                "auto-paste",
                                paste_label,
                                self.model.snapshot.auto_paste && !self.model.snapshot.file_output,
                            )
                            .palette(p)
                            .on_change(cx.listener(|s, _, _, _| {
                                s.host
                                    .submit(Intent::AutoPaste(!s.model.snapshot.auto_paste))
                            })),
                        )
                        .child(
                            Button::new("settings", "Open settings")
                                .caption("⚙")
                                .palette(p)
                                .size(ButtonSize::ExtraSmall)
                                .on_click(
                                    cx.listener(|s, _, _, _| s.host.submit(Intent::OpenSettings)),
                                ),
                        ),
                );
                let mut detail =
                    if self.model.snapshot.status == murmur_core::dictation::Status::Recording {
                        self.model.elapsed(now)
                    } else {
                        indicator.label().into()
                    };
                if let Some(destination) = &self.model.snapshot.destination {
                    detail.push_str(" · ");
                    detail.push_str(destination);
                }
                if let Some(summary) = &self.model.snapshot.smart_auto {
                    detail.push_str(" · ");
                    detail.push_str(summary);
                }
                group = group.child(div().overflow_hidden().child(detail));
            }
            Some(group)
        } else {
            None
        };
        // AX traversal follows child order, independently of top/bottom visual
        // anchoring. Encounter disclosure before the controls it reveals.
        card = card.child(status).children(controls);
        root.child(card).into_any_element()
    }
}
#[cfg(test)]
mod tests;
