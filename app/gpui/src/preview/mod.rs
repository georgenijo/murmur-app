//! Standalone Live Words screen; host services remain the sole capture,
//! consent, inference and final-delivery owners. No text enters diagnostics.
pub mod inbox;
pub mod model;
use crate::{motion::Motion, overlay::native, preferences, tokens};
use gpui::{prelude::*, *};
use model::*;
use std::rc::Rc;

/// Read synchronously on the native main turn, immediately before display.
/// Must include current recording ID, Recording status and cancellation state.
pub trait Host {
    fn is_current(&self, generation: u64) -> bool;
}

pub struct Preview {
    pub model: Model,
    pub inbox: inbox::Inbox,
    host: Rc<dyn Host>,
    /// Offscreen fixtures have no NSScreen appearance. Production uses NSWindow.
    #[cfg(feature = "gallery")]
    pub fixture_dark: Option<bool>,
    presented: Option<Ticket>,
    scheduled: Option<Ticket>,
    announced: Option<Ticket>,
    last_surface: Option<native::Surface>,
    painted_card: Option<Bounds<Pixels>>,
}
impl Preview {
    pub fn new(host: Rc<dyn Host>) -> Self {
        Self {
            model: Model::default(),
            inbox: inbox::Inbox::default(),
            host,
            #[cfg(feature = "gallery")]
            fixture_dark: None,
            presented: None,
            scheduled: None,
            announced: None,
            last_surface: None,
            painted_card: None,
        }
    }
    /// All worker callbacks publish into Inbox; only this main-thread turn
    /// changes the model or NSWindow. Subscribe before reading durable consent.
    pub fn receive(&mut self, event: Event, window: &mut Window, cx: &mut Context<Self>) {
        self.model.event(event);
        self.advance(window, cx);
    }
    pub fn advance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for event in self.inbox.drain() {
            self.model.event(event);
        }
        if let Some(ticket) = self.model.ticket()
            && !self.host.is_current(ticket.generation)
        {
            self.model.rejected(ticket);
        }
        if let (Some(old), Some(new)) = (self.presented, self.model.ticket()) {
            self.presented = (old.generation == new.generation).then_some(new);
        } else {
            self.presented = None;
        }
        if !self.model.visible() {
            self.scheduled = None;
            self.announced = None;
        }
        let prefs = preferences::current(cx);
        let ticket = self.model.ticket();
        let surface = self
            .model
            .anchor
            .and_then(|a| {
                a.window(self.model.reserve).map(|frame| {
                    let (_, height) = card_size(self.model.text(), frame.width as f32, window);
                    native::Surface {
                        frame,
                        floating: a.above,
                        hidden: ticket.is_none() || ticket != self.presented,
                        reduce_transparency: prefs.reduce_transparency,
                        dark: self.dark(window),
                        surface_height: height as f64,
                        radius: 14.,
                    }
                })
            })
            .or_else(|| {
                self.last_surface.map(|mut surface| {
                    surface.hidden = true;
                    surface
                })
            });
        if let Some(surface) = surface
            && self.last_surface != Some(surface)
        {
            // Preserve the last painted clip during the visibility transaction;
            // render refits it on the same presentation frame as text/lift.
            let card = self.painted_card.unwrap_or_default();
            if self.offscreen() || native::apply_preview(window, surface, card) {
                self.last_surface = Some(surface);
            } else {
                if let Some(ticket) = ticket {
                    self.model.rejected(ticket);
                }
                self.presented = None;
            }
        }
        if let Some(ticket) = self.model.ticket()
            && self.presented == Some(ticket)
            && self.announced != Some(ticket)
            && self.last_surface.is_some_and(|surface| !surface.hidden)
        {
            if !self.offscreen() {
                native::announce_preview_text(window, self.model.text());
            }
            self.announced = Some(ticket);
        }
        cx.notify();
    }
    fn offscreen(&self) -> bool {
        #[cfg(feature = "gallery")]
        {
            self.fixture_dark.is_some()
        }
        #[cfg(not(feature = "gallery"))]
        {
            false
        }
    }
    fn dark(&self, window: &Window) -> bool {
        let dark = matches!(
            window.appearance(),
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        );
        #[cfg(feature = "gallery")]
        let dark = self.fixture_dark.unwrap_or(dark);
        dark
    }

    fn admit_frame(&mut self, ticket: Ticket, window: &mut Window, cx: &mut Context<Self>) {
        // First-frame continuation revalidates every queued privacy boundary.
        for event in self.inbox.drain() {
            self.model.event(event);
        }
        if self.model.owns(ticket) {
            if self.host.is_current(ticket.generation) {
                self.presented = Some(ticket);
            } else {
                self.model.rejected(ticket);
            }
        }
        if self.scheduled == Some(ticket) {
            self.scheduled = None;
        }
        self.advance(window, cx);
    }
    /// The native host calls once. Weak ownership ends polling at teardown;
    /// each queued event is revalidated in advance, never at worker receipt.
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
                            let prefs = preferences::current(cx);
                            let restyle = s.last_surface.is_some_and(|surface| {
                                surface.reduce_transparency != prefs.reduce_transparency
                                    || surface.dark != s.dark(w)
                            });
                            if s.inbox.pending()
                                || restyle
                                || s.model
                                    .ticket()
                                    .is_some_and(|t| !s.host.is_current(t.generation))
                            {
                                s.advance(w, cx);
                            }
                        })
                    })
                    .is_ok_and(|r| r.is_ok())
                {
                    break;
                }
            }
        })
        .detach();
    }
}

impl Drop for Preview {
    fn drop(&mut self) {
        self.inbox.close();
        self.model.clear();
    }
}

/// Shape the same system font as the view. Keep all newest lines (bottom
/// clipping), rather than a line-clamp that discards the newest words.
fn card_size(text: &str, available: f32, window: &Window) -> (f32, f32) {
    let run = TextRun {
        len: text.len(),
        font: font(".SystemUIFont"),
        color: Hsla::default(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let width = (available - 52.).max(1.);
    let Ok(lines) = window.text_system().shape_text(
        text.to_owned().into(),
        px(13.),
        &[run],
        Some(px(width)),
        None,
    ) else {
        return ((available - 24.).max(1.), 85.);
    };
    let natural = lines
        .iter()
        .map(|l| f32::from(l.width()))
        .fold(0., f32::max);
    let count: usize = lines.iter().map(|l| l.wrap_boundaries.len() + 1).sum();
    (
        (natural.ceil() + 30.).min((available - 24.).max(1.)),
        (count as f32 * 20.8).min(63.) + 22.,
    )
}
impl Render for Preview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let prefs = preferences::current(cx);
        let root = div().id("live-words-window").size_full().overflow_hidden();
        if !self.model.visible() {
            return root.into_any_element();
        }
        let anchor = self.model.anchor.expect("visible anchor");
        let frame = anchor.window(self.model.reserve).expect("visible geometry");
        let dark = self.dark(window);
        let palette = tokens::Palette::overlay(anchor.above, dark, cx);
        let ticket = self.model.ticket();
        let hidden = ticket.is_none() || ticket != self.presented;
        let (width, height) = card_size(self.model.text(), frame.width as f32, window);
        let lift = gpui_base::transition(
            "preview-lift",
            px(anchor.lift as f32),
            gpui_base::Transition::new(Motion::Overlay.resolve(prefs.reduce_motion).duration),
            window,
            cx,
        );
        let y = if anchor.above {
            frame.height as f32 - height - f32::from(lift)
        } else {
            f32::from(lift)
        };
        let card = Bounds::new(
            point(px((frame.width as f32 - width) / 2.), px(y)),
            size(px(width), px(height)),
        );
        self.painted_card = Some(card);
        if !self.offscreen() {
            native::refit_preview_material(window, card);
        }
        let Some(ticket) = ticket else {
            return root.into_any_element();
        };
        if hidden && self.scheduled != Some(ticket) {
            self.scheduled = Some(ticket);
            let weak = cx.entity().downgrade();
            window.on_next_frame(move |w, cx| {
                let _ = weak.update(cx, |s, cx| {
                    s.admit_frame(ticket, w, cx);
                });
            });
        }
        let stable = self.model.stable_bytes();
        let tentative = if prefs.increase_contrast {
            palette.text
        } else {
            palette.muted.opacity(0.74)
        };
        let text = StyledText::new(self.model.text().to_owned()).with_highlights([
            (
                0..stable,
                HighlightStyle {
                    color: Some(palette.text),
                    ..Default::default()
                },
            ),
            (
                stable..self.model.text().len(),
                HighlightStyle {
                    color: Some(tentative),
                    ..Default::default()
                },
            ),
        ]);
        root.child(
            div()
                .id("live-words-card")
                .debug_selector(|| "live-words-card".into())
                .absolute()
                .left(card.origin.x)
                .top(card.origin.y)
                .w(card.size.width)
                .h(card.size.height)
                .rounded(px(14.))
                .border_1()
                .border_color(palette.border)
                .when(
                    !anchor.above
                        || self.offscreen()
                        || !native::has_native_window(window)
                        || prefs.reduce_transparency,
                    |s| {
                        s.bg(if anchor.above {
                            palette.surface
                        } else {
                            tokens::preview_surface(prefs.reduce_transparency)
                        })
                    },
                )
                .px(px(14.))
                .py(px(10.))
                .overflow_hidden()
                .child(
                    div()
                        .id("live-words-text")
                        .role(Role::Status)
                        .aria_label("Words recognized so far")
                        .flex()
                        .flex_col()
                        .justify_end()
                        .h_full()
                        .overflow_hidden()
                        .text_size(px(13.))
                        .line_height(px(20.8))
                        .text_color(if prefs.increase_contrast {
                            palette.text
                        } else {
                            palette.muted.opacity(0.74)
                        })
                        .child(
                            div()
                                .id("live-words-content")
                                .role(Role::Label)
                                .aria_value(self.model.text().to_owned())
                                .flex_shrink_0()
                                .child(text),
                        ),
                ),
        )
        .into_any_element()
    }
}
#[cfg(test)]
mod tests;
