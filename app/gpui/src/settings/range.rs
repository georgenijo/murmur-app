//! Labelled ranges use the shared pointer transport and one keyboard/AX owner.
use super::*;
use gpui_base::slider::{SliderEvent, SliderState};
pub struct Committed(pub i64);
pub struct Range {
    pub state: Entity<SliderState>,
    pub draft: Rc<std::cell::Cell<Option<i64>>>,
    focus: FocusHandle,
    pub(super) scroll_parent: Option<ScrollHandle>,
    label: &'static str,
    unit: &'static str,
    disabled: bool,
    pub(crate) scroll_parent: Option<ScrollHandle>,
    _subscriptions: Vec<Subscription>,
}
impl EventEmitter<Committed> for Range {}
impl Focusable for Range {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Range {
    pub fn new(
        label: &'static str,
        unit: &'static str,
        min: i64,
        max: i64,
        step: i64,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let state = cx.new(|_| {
            SliderState::new()
                .min(min as f32)
                .max(max as f32)
                .step(step as f32)
        });
        let focus = cx.focus_handle();
        let draft = Rc::new(std::cell::Cell::new(None));
        let change = cx.subscribe(&state, |s, _, e: &SliderEvent, cx| {
            s.draft.set(Some(s.value(cx)));
            match e {
                SliderEvent::Change(v) if !s.state.read(cx).is_dragging() => {
                    crate::events::publish(cx, Committed(v.end() as i64))
                }
                SliderEvent::Release(v) => crate::events::publish(cx, Committed(v.end() as i64)),
                _ => (),
            }
            cx.notify();
        });
        let blur = cx.on_blur(&focus, w, |s, _, cx| s.finish(cx));
        let activation = cx.observe_window_activation(w, |s, w, cx| {
            if !w.is_window_active() {
                s.finish(cx);
            }
        });
        Self {
            state,
            draft,
            focus,
            scroll_parent: None,
            label,
            unit,
            disabled: false,
            scroll_parent: None,
            _subscriptions: vec![change, blur, activation],
        }
    }
    /// Change timeline bounds only when the source duration changes.
    pub fn set_max(&mut self, max: i64, cx: &mut Context<Self>) {
        if self.state.read(cx).max_value() != max.max(1) as f32 {
            self.state.update(cx, |s, _| {
                *s = SliderState::new().min(0.).max(max.max(1) as f32).step(100.)
            });
        }
    }
    pub fn value(&self, cx: &App) -> i64 {
        self.state.read(cx).value().end() as i64
    }
    pub fn finish(&self, cx: &mut Context<Self>) {
        self.state.update(cx, |s, cx| s.handle_release(cx));
    }
    pub fn set_disabled(&mut self, disabled: bool, cx: &mut Context<Self>) {
        if self.disabled != disabled {
            self.disabled = disabled;
            cx.notify();
        }
    }
    /// Progress may update while the pointer owns a draft seek. Release must
    /// commit that draft, rather than the latest playback position.
    pub fn set_playback_position(
        &mut self,
        value: i64,
        disabled: bool,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_disabled(disabled, cx);
        if !self.state.read(cx).is_dragging() {
            self.set(value, disabled, w, cx);
        }
    }
    pub fn set(&mut self, value: i64, disabled: bool, w: &mut Window, cx: &mut Context<Self>) {
        self.disabled = disabled;
        self.draft.set(None);
        self.state
            .update(cx, |s, cx| s.set_value(value as f32, w, cx));
        cx.notify();
    }
}
impl Render for Range {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::get(cx);
        let v = self.state.read(cx).value().end();
        let min = self.state.read(cx).min_value();
        let max = self.state.read(cx).max_value();
        let reveal = crate::data_components::focus_reveal(
            self.focus.clone(),
            self.scroll_parent.clone(),
            w,
            cx,
        );
        div()
            .relative()
            .child(reveal)
            .w_full()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(
                gpui_base::Slider::new(&self.state)
                    .accessibility_label(self.label)
                    .track_focus(&self.focus)
                    .disabled(self.disabled)
                    .h(tokens::STANDARD_HEIGHT)
                    .border_1()
                    .border_color(if self.focus.is_focused(w) {
                        p.focus
                    } else {
                        p.border
                    })
                    .rounded(tokens::CONTROL_RADIUS)
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .child(
                        gpui_base::SliderTrack::new(&self.state)
                            .disabled(self.disabled)
                            .w_full()
                            .h(px(4.))
                            .bg(p.raised)
                            .rounded_full()
                            .relative()
                            .child(
                                gpui_base::SliderIndicator::new(&self.state)
                                    .w_full()
                                    .h_full()
                                    .relative()
                                    .child(
                                        gpui_base::SliderThumb::new(&self.state)
                                            .disabled(self.disabled)
                                            .absolute()
                                            .left(relative((v - min) / (max - min)))
                                            .ml(px(-7.))
                                            .top(px(-5.))
                                            .size(px(14.))
                                            .bg(p.accent)
                                            .rounded_full(),
                                    ),
                            ),
                    ),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .line_height(px(18.))
                    .child(format!("{} {}", v as i64, self.unit)),
            )
    }
}
