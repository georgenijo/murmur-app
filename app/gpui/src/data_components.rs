//! Accessible data surfaces shared by Diagnostics and later native screens.
//! Bounded rendering is explicit; missing observations never turn into zeros.
use crate::{
    Button,
    tokens::{self, Palette},
};
use gpui::{prelude::*, *};
use std::collections::BTreeSet;

#[derive(Clone)]
pub struct TabChanged(pub usize);
pub struct Tabs {
    focus: FocusHandle,
    label: SharedString,
    labels: Vec<SharedString>,
    selected: usize,
}
impl EventEmitter<TabChanged> for Tabs {}
impl Focusable for Tabs {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Tabs {
    pub fn new(
        label: impl Into<SharedString>,
        labels: Vec<SharedString>,
        cx: &mut Context<Self>,
    ) -> Self {
        let label = label.into();
        assert!(!label.trim().is_empty() && !labels.is_empty());
        assert!(labels.iter().all(|s| !s.trim().is_empty()));
        Self {
            focus: cx.focus_handle().tab_stop(true),
            label,
            labels,
            selected: 0,
        }
    }
    pub fn set_selected(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.labels.len() && self.selected != index {
            self.selected = index;
            cx.notify();
        }
    }
    pub fn selected(&self) -> usize {
        self.selected
    }
    pub fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.labels.len() && self.selected != index {
            self.selected = index;
            crate::events::publish(cx, TabChanged(index));
            cx.notify();
        }
    }
    fn activate(&mut self, index: usize, w: &mut Window, cx: &mut Context<Self>) {
        w.focus(&self.focus, cx);
        self.select(index, cx);
    }
}
impl Render for Tabs {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::get(cx);
        div()
            .id("tabs")
            .role(Role::TabList)
            .aria_label(self.label.clone())
            .track_focus(&self.focus)
            .flex()
            .flex_wrap()
            .gap(px(4.))
            .p(px(4.))
            .bg(p.raised)
            .rounded(tokens::CONTROL_RADIUS)
            .capture_key_down(cx.listener(|s, e: &KeyDownEvent, w, cx| {
                if e.keystroke.modifiers.number_of_modifiers() != 0 || e.prefer_character_input {
                    return;
                }
                let index = match e.keystroke.key.as_str() {
                    "left" | "up" => (s.selected + s.labels.len() - 1) % s.labels.len(),
                    "right" | "down" => (s.selected + 1) % s.labels.len(),
                    "home" => 0,
                    "end" => s.labels.len() - 1,
                    _ => return,
                };
                w.prevent_default();
                cx.stop_propagation();
                s.activate(index, w, cx);
            }))
            .children(self.labels.iter().enumerate().map(|(i, label)| {
                div()
                    .id(i)
                    .role(Role::Tab)
                    .aria_label(label.clone())
                    .aria_selected(self.selected == i)
                    .when(self.selected == i, |d| {
                        d.aria_active_descendant().bg(p.surface)
                    })
                    .px(px(12.))
                    .h(tokens::STANDARD_HEIGHT)
                    .flex()
                    .items_center()
                    .rounded(tokens::CONTROL_RADIUS)
                    .when(self.selected == i, |d| d.border_1().border_color(p.border))
                    .hover(|d| d.bg(p.surface))
                    .child(label.clone())
                    .on_click(cx.listener(move |s, _, w, cx| s.activate(i, w, cx)))
                    .on_a11y_action(AccessibleAction::Click, {
                        let entity = cx.entity();
                        move |_, w, cx| entity.update(cx, |s, cx| s.activate(i, w, cx))
                    })
            }))
            .focus_visible(|d| d.border_1().border_color(p.focus))
    }
}

/// Immutable table semantics with column names on every cell. Interactive rows
/// are ordinary named Buttons, so Tab, Space, pointer and AX share one action.
#[derive(IntoElement)]
pub struct DataTable {
    id: ElementId,
    label: SharedString,
    headers: Vec<SharedString>,
    rows: Vec<Vec<SharedString>>,
    actions: Vec<Button>,
}
impl DataTable {
    pub fn new(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        headers: &[&str],
        rows: Vec<Vec<SharedString>>,
    ) -> Self {
        let label = label.into();
        assert!(!label.trim().is_empty() && !headers.is_empty());
        assert!(rows.iter().all(|r| r.len() == headers.len()));
        Self {
            id: id.into(),
            label,
            headers: headers.iter().map(|s| (*s).into()).collect(),
            rows,
            actions: vec![],
        }
    }
    pub fn with_row_actions(mut self, actions: Vec<Button>) -> Self {
        assert_eq!(actions.len(), self.rows.len());
        self.actions = actions;
        self
    }
}
struct TableNavigation {
    focus: FocusHandle,
    row: usize,
    column: usize,
}
impl RenderOnce for DataTable {
    fn render(self, w: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = Palette::get(cx);
        let cols = self.headers.len();
        let navigation = w.use_keyed_state(self.id.clone(), cx, |_, cx| TableNavigation {
            focus: cx.focus_handle().tab_stop(true),
            row: 0,
            column: 0,
        });
        let focus = navigation.read(cx).focus.clone();
        let table_focused = focus.is_focused(w);
        let row_count = self.rows.len() + 1;
        navigation.update(cx, |s, _| {
            s.row = s.row.min(row_count - 1);
            s.column = s.column.min(cols - 1);
        });
        let active = (navigation.read(cx).row, navigation.read(cx).column);
        let mut actions = self.actions.into_iter();
        let header = div()
            .id("headers")
            .role(Role::Row)
            .flex()
            .bg(p.raised)
            .a11y_synthetic_children(|b| b.parent_node().set_row_index(0))
            .children(self.headers.iter().enumerate().map(|(i, h)| {
                div()
                    .id(i)
                    .role(Role::ColumnHeader)
                    .when(active == (0, i), |d| {
                        d.aria_active_descendant()
                            .when(table_focused, |d| d.bg(p.focus.opacity(0.15)))
                    })
                    .aria_label(h.clone())
                    .flex_1()
                    .min_w(px(90.))
                    .p(px(8.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .a11y_synthetic_children(move |b| {
                        b.parent_node().set_column_index(i);
                        b.parent_node().set_row_index(0);
                    })
                    .child(h.clone())
            }));
        div()
            .id(self.id)
            .track_focus(&focus)
            .focus_visible(|d| d.border_color(p.focus))
            .capture_key_down(move |e, w, cx| {
                if !focus.is_focused(w)
                    || e.prefer_character_input
                    || e.keystroke.modifiers.number_of_modifiers() != 0
                {
                    return;
                }
                let key = e.keystroke.key.as_str();
                if !matches!(key, "up" | "down" | "left" | "right" | "home" | "end") {
                    return;
                }
                navigation.update(cx, |s, cx| {
                    match key {
                        "up" => s.row = s.row.saturating_sub(1),
                        "down" => s.row = (s.row + 1).min(row_count - 1),
                        "left" => s.column = s.column.saturating_sub(1),
                        "right" => s.column = (s.column + 1).min(cols - 1),
                        "home" => {
                            s.row = 0;
                            s.column = 0;
                        }
                        "end" => {
                            s.row = row_count - 1;
                            s.column = cols - 1;
                        }
                        _ => unreachable!(),
                    }
                    cx.notify();
                });
                w.prevent_default();
                cx.stop_propagation();
                w.refresh();
            })
            .role(Role::Table)
            .overflow_x_scroll()
            .flex_shrink_0()
            .aria_label(self.label)
            .border_1()
            .border_color(p.border)
            .rounded(tokens::CONTROL_RADIUS)
            .a11y_synthetic_children(move |b| {
                b.parent_node().set_column_count(cols);
                b.parent_node().set_row_count(row_count);
            })
            .text_size(tokens::CAPTION_SIZE)
            .line_height(tokens::CAPTION_LEADING)
            .child(header)
            .children(self.rows.into_iter().enumerate().map(|(r, cells)| {
                let mut action = actions.next();
                div()
                    .id(r)
                    .role(Role::Row)
                    .flex()
                    .border_t_1()
                    .border_color(p.border)
                    .a11y_synthetic_children(move |b| b.parent_node().set_row_index(r + 1))
                    .children(cells.into_iter().enumerate().map(|(i, c)| {
                        div()
                            .id(i)
                            .role(Role::Cell)
                            .when(active == (r + 1, i), |d| {
                                d.aria_active_descendant()
                                    .when(table_focused, |d| d.bg(p.focus.opacity(0.15)))
                            })
                            .aria_label(self.headers[i].clone())
                            .aria_value(c.clone())
                            .a11y_synthetic_children(move |b| {
                                b.parent_node().set_column_index(i);
                                b.parent_node().set_row_index(r + 1);
                            })
                            .flex_1()
                            .min_w(px(90.))
                            .p(px(8.))
                            .child(if i == 0 {
                                if let Some(action) = action.take() {
                                    div()
                                        .flex()
                                        .flex_col()
                                        .items_start()
                                        .gap(px(4.))
                                        .child(c)
                                        .child(action)
                                        .into_any_element()
                                } else {
                                    c.into_any_element()
                                }
                            } else {
                                c.into_any_element()
                            })
                    }))
            }))
    }
}

#[derive(Clone, PartialEq)]
pub struct DetailRow {
    pub id: u64,
    pub label: SharedString,
    pub detail: SharedString,
}
/// Bounded accessible list with one keyboard stop and independently expandable
/// monospace rows. Producers own the retained history; this paints its newest N.
pub struct DetailList {
    label: SharedString,
    rows: Vec<DetailRow>,
    total: usize,
    limit: usize,
    expanded: BTreeSet<u64>,
    active: usize,
    focus: FocusHandle,
    pub scroll: ScrollHandle,
    pub following: bool,
}
impl Focusable for DetailList {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl DetailList {
    pub fn new(label: impl Into<SharedString>, limit: usize, cx: &mut Context<Self>) -> Self {
        let label = label.into();
        assert!(!label.trim().is_empty() && limit > 0);
        Self {
            label,
            rows: vec![],
            total: 0,
            limit,
            expanded: BTreeSet::new(),
            active: 0,
            focus: cx.focus_handle().tab_stop(true),
            scroll: ScrollHandle::new(),
            following: true,
        }
    }
    pub fn set_rows(&mut self, rows: Vec<DetailRow>, cx: &mut Context<Self>) {
        self.total = rows.len();
        let rows: Vec<_> = rows
            .into_iter()
            .rev()
            .take(self.limit)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        if self.rows == rows {
            return;
        }
        let old = self.rows.get(self.active).map(|r| r.id);
        self.expanded.retain(|id| rows.iter().any(|r| r.id == *id));
        self.rows = rows;
        self.active = old
            .and_then(|id| self.rows.iter().position(|r| r.id == id))
            .unwrap_or(0);
        if self.following {
            self.scroll.scroll_to_bottom();
        }
        cx.notify();
    }
    pub fn resume(&mut self, cx: &mut Context<Self>) {
        self.following = true;
        self.scroll.scroll_to_bottom();
        cx.notify();
    }
    pub fn rendered_len(&self) -> usize {
        self.rows.len()
    }
    pub fn set_expanded(&mut self, id: u64, expanded: bool, cx: &mut Context<Self>) {
        if self.rows.iter().any(|r| r.id == id && !r.detail.is_empty()) {
            if expanded {
                self.expanded.insert(id);
            } else {
                self.expanded.remove(&id);
            }
            cx.notify();
        }
    }
    pub fn is_expanded(&self, id: u64) -> bool {
        self.expanded.contains(&id)
    }
    fn toggle(&mut self, i: usize, w: &mut Window, cx: &mut Context<Self>) {
        if let Some(row) = self.rows.get(i) {
            self.active = i;
            if !row.detail.is_empty() && !self.expanded.remove(&row.id) {
                self.expanded.insert(row.id);
            }
            w.focus(&self.focus, cx);
            cx.notify();
        }
    }
}
impl Render for DetailList {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::get(cx);
        div()
            .id("bounded-detail-list")
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .child(
                div()
                    .id("list-status")
                    .role(Role::Status)
                    .p(px(8.))
                    .text_color(p.muted)
                    .child(format!(
                        "Showing newest {} of {} · {}",
                        self.rows.len(),
                        self.total,
                        if self.following {
                            "Following"
                        } else {
                            "Auto-scroll paused"
                        }
                    )),
            )
            .when(!self.following, |d| {
                d.child(
                    Button::new("resume", "Resume auto-scroll")
                        .on_click(cx.listener(|s, _, _, cx| s.resume(cx))),
                )
            })
            .child(
                div()
                    .id("rows")
                    .role(Role::List)
                    .aria_label(self.label.clone())
                    .track_focus(&self.focus)
                    .min_h_0()
                    .flex_1()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .on_scroll(cx.listener(|s, _, _, cx| {
                        // Scroll listeners run after GPUI has applied the wheel offset.
                        s.following = s.scroll.max_offset().y + s.scroll.offset().y > -px(40.)
                            && (s.scroll.max_offset().y + s.scroll.offset().y).abs() < px(40.);
                        cx.notify();
                    }))
                    .focus_visible(|d| d.border_1().border_color(p.focus))
                    .capture_key_down(cx.listener(|s, e: &KeyDownEvent, w, cx| {
                        if s.rows.is_empty()
                            || e.prefer_character_input
                            || e.keystroke.modifiers.number_of_modifiers() != 0
                        {
                            return;
                        }
                        match e.keystroke.key.as_str() {
                            "down" => s.active = (s.active + 1).min(s.rows.len() - 1),
                            "up" => s.active = s.active.saturating_sub(1),
                            "home" => s.active = 0,
                            "end" => s.active = s.rows.len() - 1,
                            "enter" | "space" if !e.is_held => s.toggle(s.active, w, cx),
                            "escape" => {
                                s.expanded.clear();
                            }
                            _ => return,
                        }
                        s.following = false;
                        s.scroll.scroll_to_item(s.active);
                        w.prevent_default();
                        cx.stop_propagation();
                        cx.notify();
                    }))
                    .children(self.rows.iter().enumerate().map(|(i, r)| {
                        let open = self.expanded.contains(&r.id);
                        div()
                            .id(r.id)
                            .role(Role::ListItem)
                            .border_b_1()
                            .border_color(p.border)
                            .font_family("Menlo")
                            .text_size(tokens::CAPTION_SIZE)
                            .line_height(tokens::CAPTION_LEADING)
                            .child(
                                div()
                                    .id(("summary", r.id))
                                    .role(Role::Button)
                                    .aria_label(r.label.clone())
                                    .aria_expanded(open)
                                    .when(i == self.active, |d| {
                                        d.aria_active_descendant()
                                            .border_l_2()
                                            .border_color(p.focus)
                                    })
                                    .p(px(8.))
                                    .hover(|d| d.bg(p.raised))
                                    .child(format!("{} {}", if open { "▾" } else { "▸" }, r.label))
                                    .on_click(cx.listener(move |s, _, w, cx| s.toggle(i, w, cx)))
                                    .on_a11y_action(AccessibleAction::Click, {
                                        let entity = cx.entity();
                                        move |_, w, cx| {
                                            entity.update(cx, |s, cx| s.toggle(i, w, cx))
                                        }
                                    }),
                            )
                            .when(open, |d| {
                                d.child(
                                    div()
                                        .id("event-data")
                                        .role(Role::Label)
                                        .aria_label("Event data")
                                        .aria_value(r.detail.clone())
                                        .p(px(12.))
                                        .bg(p.raised)
                                        .child(r.detail.clone()),
                                )
                            })
                    })),
            )
    }
}

#[derive(IntoElement)]
pub struct Progress {
    label: SharedString,
    value: Option<f64>,
}
impl Progress {
    pub fn new(label: impl Into<SharedString>, value: Option<f64>) -> Self {
        let label = label.into();
        assert!(!label.trim().is_empty());
        Self {
            label,
            value: value.filter(|v| v.is_finite()).map(|v| v.clamp(0., 1.)),
        }
    }
}
impl RenderOnce for Progress {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = Palette::get(cx);
        div()
            .id("progress")
            .role(Role::ProgressIndicator)
            .aria_label(self.label.clone())
            .aria_value(
                self.value
                    .map(|v| format!("{:.0}%", v * 100.))
                    .unwrap_or_else(|| "In progress".into()),
            )
            .when_some(self.value, |d, v| {
                d.aria_numeric_value(v * 100.)
                    .aria_min_numeric_value(0.)
                    .aria_max_numeric_value(100.)
            })
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(self.label)
            .child(
                div().h(px(4.)).bg(p.raised).rounded(px(2.)).child(
                    div()
                        .h_full()
                        .w(relative(self.value.unwrap_or(0.25) as f32))
                        .bg(p.accent),
                ),
            )
    }
}

/// Bounded exact-value chart. Values and units remain available as text and AX
/// data independently of color. One composite keyboard stop avoids 600 Tab stops.
#[derive(Clone, PartialEq)]
pub struct ChartPoint {
    pub label: SharedString,
    pub value: Option<f64>,
}
pub struct Chart {
    label: SharedString,
    unit: SharedString,
    points: Vec<ChartPoint>,
    active: usize,
    focus: FocusHandle,
    shown: bool,
    _blur: Subscription,
}
impl Focusable for Chart {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Chart {
    pub fn new(
        label: impl Into<SharedString>,
        unit: impl Into<SharedString>,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle().tab_stop(true);
        let blur = cx.on_blur(&focus, w, |s, _, cx| {
            s.shown = false;
            cx.notify();
        });
        Self {
            label: label.into(),
            unit: unit.into(),
            points: vec![],
            active: 0,
            focus,
            shown: false,
            _blur: blur,
        }
    }
    pub fn set_points(&mut self, points: Vec<ChartPoint>, cx: &mut Context<Self>) {
        let points: Vec<_> = points
            .into_iter()
            .rev()
            .take(600)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        if points == self.points {
            return;
        }
        self.points = points;
        self.active = self.active.min(self.points.len().saturating_sub(1));
        cx.notify();
    }
    fn value(&self, i: usize) -> String {
        let point = &self.points[i];
        format!(
            "{}: {}",
            point.label,
            point
                .value
                .filter(|v| v.is_finite())
                .map(|v| format!("{v:.2} {}", self.unit))
                .unwrap_or_else(|| "Unavailable".into())
        )
    }
}
impl Render for Chart {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::get(cx);
        let max = self
            .points
            .iter()
            .filter_map(|p| p.value)
            .filter(|v| v.is_finite())
            .fold(1., f64::max);
        div()
            .id("chart")
            .role(Role::Group)
            .aria_label(self.label.clone())
            .track_focus(&self.focus)
            .flex_shrink_0()
            .p(px(12.))
            .border_1()
            .border_color(p.border)
            .rounded(tokens::CARD_RADIUS)
            .on_hover(cx.listener(|s, hovered: &bool, w, cx| {
                if !*hovered && !s.focus.is_focused(w) {
                    s.shown = false;
                    cx.notify();
                }
            }))
            .focus_visible(|d| d.border_color(p.focus))
            .capture_key_down(cx.listener(|s, e: &KeyDownEvent, w, cx| {
                if s.points.is_empty()
                    || e.prefer_character_input
                    || e.keystroke.modifiers.number_of_modifiers() != 0
                {
                    return;
                }
                if e.keystroke.key == "escape" {
                    s.shown = false;
                    w.prevent_default();
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                s.shown = true;
                match e.keystroke.key.as_str() {
                    "right" | "down" => s.active = (s.active + 1).min(s.points.len() - 1),
                    "left" | "up" => s.active = s.active.saturating_sub(1),
                    "home" => s.active = 0,
                    "end" => s.active = s.points.len() - 1,
                    _ => return,
                }
                w.prevent_default();
                cx.stop_propagation();
                cx.notify();
            }))
            .child(self.label.clone())
            .child(
                div()
                    .h(px(24.))
                    .text_color(p.muted)
                    .child(if self.points.is_empty() {
                        "No samples".into()
                    } else if self.shown {
                        self.value(self.active)
                    } else {
                        "Arrow keys or hover to inspect exact values".into()
                    }),
            )
            .child(div().h(px(100.)).flex().items_end().gap(px(1.)).children(
                self.points.iter().enumerate().map(|(i, point)| {
                    let measured = point.value.filter(|v| v.is_finite());
                    div()
                        .id(i)
                        .role(Role::Button)
                        .aria_label(self.value(i))
                        .when(i == self.active, |d| {
                            d.aria_active_descendant().border_1().border_color(p.focus)
                        })
                        .flex_1()
                        .h(px(measured
                            .map(|v| (v.max(0.) / max * 96.).max(2.) as f32)
                            .unwrap_or(2.)))
                        .bg(if measured.is_some() {
                            p.accent
                        } else {
                            p.muted
                        })
                        .on_mouse_move(cx.listener(move |s, _, _, cx| {
                            s.active = i;
                            s.shown = true;
                            cx.notify();
                        }))
                        .on_click(cx.listener(move |s, _, w, cx| {
                            s.active = i;
                            s.shown = true;
                            w.focus(&s.focus, cx);
                            cx.notify();
                        }))
                        .on_a11y_action(AccessibleAction::Click, {
                            let entity = cx.entity();
                            move |_, w, cx| {
                                entity.update(cx, |s, cx| {
                                    s.active = i;
                                    s.shown = true;
                                    w.focus(&s.focus, cx);
                                    cx.notify();
                                })
                            }
                        })
                }),
            ))
    }
}

#[cfg(test)]
mod tests {
    use super::Progress;
    #[test]
    fn progress_bounds_finite_values_and_preserves_indeterminate_state() {
        assert_eq!(
            Progress::new("Synthetic progress", Some(1.5)).value,
            Some(1.)
        );
        assert_eq!(
            Progress::new("Synthetic progress", Some(-0.5)).value,
            Some(0.)
        );
        assert_eq!(
            Progress::new("Synthetic progress", Some(f64::NAN)).value,
            None
        );
        assert_eq!(Progress::new("Synthetic progress", None).value, None);
    }
}
