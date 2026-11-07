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
    screen_poll: u64,
    surface_writer: SurfaceWriter,
    host: Rc<dyn Host>,
    epoch: Instant,
    now: u64,
    applied: Option<native::Surface>,
    last_indicator: Option<Indicator>,
    interested: bool,
    pointer: PointerPresence,
}
impl Overlay {
    pub fn new(snapshot: Snapshot, host: Rc<dyn Host>) -> Self {
        Self {
            model: Model::new(snapshot),
            inbox: inbox::Inbox::default(),
            screen_poll: 0,
            surface_writer: SurfaceWriter::default(),
            host,
            epoch: Instant::now(),
            now: 0,
            applied: None,
            last_indicator: None,
            interested: false,
            pointer: PointerPresence::default(),
        }
    }
    pub fn receive(&mut self, event: Event, window: &mut Window, cx: &mut Context<Self>) {
        self.now = self.epoch.elapsed().as_millis() as u64;
        self.model.event(event, self.now);
        self.advance(window, cx);
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
            self.model.close(self.now);
        } else {
            self.model.expand(self.now);
        }
        self.advance(window, cx);
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
            hidden: self.model.hidden(self.now),
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
        cx.notify();
    }
    /// Start only in the real host. Deterministic tests drive advance/events
    /// explicitly. The idle watchdog does no rendering unless state changes.
    pub fn follow(&self, window: &Window, cx: &mut Context<Self>) {
        let handle = window.window_handle();
        cx.spawn(async move |weak, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(33))
                    .await;
                if !cx
                    .update_window(handle, |_, w, cx| {
                        weak.update(cx, |s, cx| {
                            let before = (
                                s.model.indicator(s.now),
                                s.model.phase,
                                s.model.elapsed(s.now),
                            );
                            s.now = s.epoch.elapsed().as_millis() as u64;
                            let (events, level) = s.inbox.drain();
                            let prefs = preferences::current(cx);
                            let dark = s.model.snapshot.placement
                                != murmur_core::overlay_geometry::OverlayPlacement::FloatingPill
                                || matches!(
                                    w.appearance(),
                                    WindowAppearance::Dark | WindowAppearance::VibrantDark
                                );
                            let mut changed = s.applied.is_none()
                                || !events.is_empty()
                                || level.is_some()
                                || s.applied.is_some_and(|a| {
                                    a.reduce_transparency != prefs.reduce_transparency
                                        || a.increase_contrast != prefs.increase_contrast
                                        || a.dark != dark
                                });
                            for event in events {
                                s.model.event(event, s.now);
                            }
                            if let Some((generation, level)) = level {
                                s.model.sample(generation, level, s.now);
                            }
                            if let Some(inside) = native::pointer_inside(w)
                                && s.observe_pointer(inside, s.now)
                            {
                                changed = true;
                            }
                            if s.now >= s.screen_poll {
                                s.screen_poll = s.now + 500;
                                if let Some(mut screen) = native::screen() {
                                    if screen.bounds == s.model.screen.bounds
                                        && screen.scale_factor == s.model.screen.scale_factor
                                    {
                                        screen.dock = s.model.screen.dock;
                                    }
                                    if screen != s.model.screen {
                                        s.model.event(Event::Screen(screen), s.now);
                                        changed = true;
                                    }
                                }
                            }
                            let due = changed || s.model.needs_tick() || s.surface_writer.pending();
                            if before
                                != (
                                    s.model.indicator(s.now),
                                    s.model.phase,
                                    s.model.elapsed(s.now),
                                )
                                || due
                            {
                                s.advance(w, cx);
                            }
                        })
                    })
                    .is_ok_and(|result| result.is_ok())
                {
                    break;
                }
            }
        })
        .detach();
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
                    s.model.close(s.now);
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
        if self.model.hidden(now) {
            return root.into_any_element();
        }
        let policy = crate::motion::Motion::Overlay.resolve(cx.reduce_motion());
        let width = gpui_base::transition(
            "overlay-width",
            px(g.window_w as f32),
            gpui_base::Transition::new(policy.duration),
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
            gpui_base::Transition::new(policy.duration),
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
        let mut card = div()
            .id("overlay-surface")
            .debug_selector(|| "overlay-surface".into())
            .relative()
            .flex()
            .flex_col()
            .items_center()
            .w(width)
            .h(height)
            .rounded(px(radius))
            .overflow_hidden()
            .text_size(tokens::CAPTION_SIZE)
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
                    } else {
                        0.76
                    },
                )
            } else {
                tokens::overlay_surface()
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
            .on_hover(cx.listener(|s, inside, w, cx| {
                let now = s.epoch.elapsed().as_millis() as u64;
                // GPUI defers layout-derived hover callbacks. A centered
                // resize can invalidate their local hit-test snapshot while
                // the screen pointer remains over the painted surface. Query
                // the current native presentation before changing ownership;
                // a stale exit must not rearm an explicitly closed visit.
                let inside = native::pointer_inside(w).unwrap_or(*inside);
                if s.observe_pointer(inside, now) {
                    s.advance(w, cx);
                }
            }));
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
            .child(if expanded { "⌃" } else { "⌄" })
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
            .when(g.floating, |s| s.justify_center().gap(px(2.)).px(px(4.)))
            .on_click(cx.listener(|s, e: &ClickEvent, w, cx| {
                let action = if matches!(e, ClickEvent::Keyboard(_)) {
                    s.model.accessible_activate(s.now)
                } else {
                    s.model.click(e.click_count(), s.now)
                };
                s.send(action);
                s.advance(w, cx);
            }));
        let glyph = match indicator {
            Indicator::Recording => "●",
            Indicator::Done | Indicator::Delivery(Delivery::Pasted) => "✓",
            Indicator::Cancelled => "×",
            Indicator::TapMissed | Indicator::Failure(_) => "⚠",
            Indicator::Starting | Indicator::Slow => "◯",
            Indicator::Processing | Indicator::Recovering | Indicator::MeetingProcessing => "◌",
            // The notch leaves 8 pt beside its 20 pt disclosure. Use the
            // single copy symbol there; the pill has room for the shortcut.
            Indicator::Delivery(Delivery::Clipboard) if !g.floating => "⎘",
            Indicator::Delivery(Delivery::Clipboard) => "⌘V",
            Indicator::Delivery(Delivery::Retry | Delivery::Retrying) => "↻",
            Indicator::Delivery(Delivery::Nothing) => "−",
            _ => "·",
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
            .when(!g.floating, |s| s.w(px(8.)).flex_shrink_0())
            .text_color(if indicator == Indicator::Recording {
                p.danger
            } else {
                p.text
            })
            .when(indicator == Indicator::Idle, |s| {
                s.opacity(if self.model.snapshot.disabled {
                    0.15
                } else {
                    0.4
                })
            })
            .child(
                div()
                    .debug_selector(|| "overlay-state-symbol".into())
                    .flex_shrink_0()
                    .child(glyph),
            );
        let show_status = g.floating || expanded || indicator != Indicator::Idle;
        if show_status {
            if matches!(
                indicator,
                Indicator::Processing | Indicator::Recovering | Indicator::MeetingProcessing
            ) {
                leading = leading.child(
                    gpui_component::Icon::default()
                        .data(include_bytes!(
                            "../../vendor/gpui-component/crates/assets/assets/icons/loader.svg"
                        ))
                        .size(px(if g.floating { 14. } else { 8. }))
                        .text_color(p.text)
                        .with_animation(
                            "processing-spinner",
                            Animation::new(std::time::Duration::from_millis(900))
                                .repeat()
                                .with_max_fps(30.),
                            |s, delta| s.transform(Transformation::rotate(percentage(delta))),
                        ),
                );
            } else if matches!(
                indicator,
                Indicator::Starting | Indicator::Slow | Indicator::Recording
            ) {
                leading = leading.child(
                    state_glyph.with_animation(
                        "recording-pulse",
                        Animation::new(std::time::Duration::from_millis(1200))
                            .repeat()
                            .with_max_fps(30.),
                        |s, delta| {
                            s.opacity(0.65 + 0.35 * (delta * std::f32::consts::TAU).cos().abs())
                        },
                    ),
                );
            } else {
                leading = leading.child(state_glyph);
            }
        }
        leading = leading.child(disclosure);
        if g.floating
            && !matches!(indicator, Indicator::Recording)
            && !matches!(indicator, Indicator::Delivery(d) if d.retryable())
        {
            let word = match indicator {
                Indicator::Failure(_) => "Error",
                Indicator::Recovering => "Recovering",
                Indicator::Calibrating => "Position",
                Indicator::Meeting => "Meeting",
                Indicator::MeetingProcessing => "Finishing",
                Indicator::Delivery(Delivery::Retrying) => "Trying",
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
                            .bars(if g.floating { 18 } else { 7 })
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
                        cx.listener(|s, _, _, _| s.model.pause_delivery(s.now)),
                    )
                    .on_hover(cx.listener(|s, inside, _, _| {
                        if !inside {
                            s.model.resume_delivery(s.now)
                        }
                    }))
                    .child(
                        Button::new("retry-delivery", "Try again")
                            .caption(if g.floating { "Retry" } else { "↻" })
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
            .when(show_status, |s| s.child(trailing));
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
