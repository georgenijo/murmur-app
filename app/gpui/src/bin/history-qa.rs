//! Synthetic native/offscreen gallery. Never accesses application storage or OS clipboard.
//! Environment (all fixtures are gallery-only):
//! - MURMUR_HISTORY_QA_CONTROLS=1: Hide History / Reopen History on the SAME entity,
//!   Replace same row (owner notification), and AX-labelled cache/host readouts.
//! - MURMUR_HISTORY_CALENDAR=santiago|havana|tokyo: four rows around the earliest
//!   local day instant (-1s, 0s, +1s, +1h), fixed noon clock and named-zone dates.
//!   Today must include three; Show all includes four. No system TZ change.
//! - MURMUR_HISTORY_CACHE=1: enables QA controls; 3MB detail with retained original
//!   JSON. Every 500ms
//!   redraw prints scanned/shaped/boundary counters and transcript-sized allocation
//!   guard count to stdout (also AX). Replace same row proves invalidation.
//! - MURMUR_HISTORY_STATE=migration: Retry import receives a synthetic owner
//!   snapshot containing imported row 201. More history actions supports all
//!   copy/save formats and delete scopes; confirmation stays in the real view.
//!   Host counters expose saved outputs/clipboard bytes, revision, import state.
//! - MURMUR_HISTORY_CANCEL_SAVE=1: save cancellation through the same reply path.
//! - MURMUR_QA_SECONDS=N: process lifetime (default 30). No native file panels.
//!
//! Existing MURMUR_HISTORY_STATE and MURMUR_GOLDEN retain their gallery meanings.
use gpui::{prelude::*, *};
use murmur_ui::Button;
#[global_allocator]
static ALLOCATOR: fixtures::allocation_probe::RenderAllocator =
    fixtures::allocation_probe::RenderAllocator;
use murmur_ui::{
    history::{History, fixtures, model::*},
    preferences::DisplayPreferences,
};
use std::rc::Rc;
fn main() {
    murmur_ui::overlay::native::start_webkit_policy();
    if std::env::var_os("MURMUR_WEBKIT_BASELINE_ONLY").is_some() {
        murmur_ui::overlay::native::assert_webkit_policy("MURMUR_HISTORY_INVENTORY");
        return;
    }
    println!(
        "MURMUR_HISTORY_REVISION={}",
        option_env!("MURMUR_QA_REVISION").unwrap_or("development")
    );
    let output = std::env::var_os("MURMUR_GOLDEN");
    let headless = output.is_some();
    let app = if headless {
        gpui_platform::application().with_windowing(WindowingRequest::Headless)
    } else {
        gpui_platform::application()
    };
    app.run(move |cx| {
        if headless {
            murmur_ui::init_with_preferences(
                cx,
                Some(DisplayPreferences {
                    reduce_motion: true,
                    reduce_transparency: true,
                    increase_contrast: std::env::var_os("MURMUR_CONTRAST").is_some(),
                }),
            );
        } else {
            murmur_ui::init(cx);
        }
        murmur_ui::tokens::apply_appearance(std::env::var_os("MURMUR_DARK").is_some(), cx);
        let state = std::env::var("MURMUR_HISTORY_STATE").unwrap_or_else(|_| "list".into());
        let compact = state == "compact";
        let host = Rc::new(fixtures::FixtureHost::default());
        host.retry_import.set(state == "migration");
        host.cancel_save
            .set(std::env::var_os("MURMUR_HISTORY_CANCEL_SAVE").is_some());
        let calendar = std::env::var("MURMUR_HISTORY_CALENDAR").ok().map(|name| {
            let (zone, clock, rows) = fixtures::calendar(&name);
            *host.rows.borrow_mut() = rows;
            (zone, clock)
        });
        let cache = std::env::var_os("MURMUR_HISTORY_CACHE").is_some();
        if cache {
            let mut rows = host.rows.borrow_mut();
            let mut target = rows[199].target();
            target["text"] = serde_json::json!("synthetic 🐱\n".repeat(200_000));
            target["rawText"] = serde_json::json!("original JSON ".repeat(200_000));
            rows[199] = Entry::project(200, target).unwrap();
        }
        let controls = cache || std::env::var_os("MURMUR_HISTORY_QA_CONTROLS").is_some();
        let mut qa = None;
        let mut entity = None;
        let handle = cx
            .open_window(
                WindowOptions {
                    show: !headless,
                    focus: false,
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(0.), px(0.)),
                        if compact {
                            size(px(720.), px(560.))
                        } else {
                            size(px(880.), px(720.))
                        },
                    ))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Murmur History (synthetic)".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |w, cx| {
                    if !headless {
                        let apply = |w: &mut Window, cx: &mut App| {
                            murmur_ui::tokens::apply_appearance(
                                matches!(
                                    w.appearance(),
                                    WindowAppearance::Dark | WindowAppearance::VibrantDark
                                ),
                                cx,
                            );
                            cx.refresh_windows();
                        };
                        apply(w, cx);
                        w.observe_window_appearance(apply).detach();
                    }
                    let d = cx.new(|cx| History::new(host.clone(), w, cx));
                    entity = Some(d.clone());
                    let view = cx.new(|_| Qa {
                        history: d,
                        host: host.clone(),
                        visible: true,
                        controls,
                        allocations: 0,
                        guarded_redraws: 0,
                    });
                    qa = Some(view.clone());
                    cx.new(|cx| gpui_base::Root::new(view, w, cx))
                },
            )
            .expect("synthetic History window");
        let entity = entity.unwrap();
        handle
            .update(cx, |_, _, cx| {
                entity.update(cx, |s, cx| {
                    s.fixture = true;
                    s.fixture_clock = Some(calendar.map_or(fixtures::clock(), |(_, clock)| clock));
                    s.fixture_zone = calendar.map(|(zone, _)| zone);
                    fixtures::attach(s, cx);
                    match state.as_str() {
                        "detail" => {
                            s.model.detail = Some(200);
                        }
                        "detail-last-page" => {
                            s.model.detail = Some(200);
                            s.last_detail_page();
                        }
                        "search" => {
                            s.model.query.text = "violet release".into();
                            s.model.query.source = Source::Files;
                        }
                        "empty" => s.model.entries.clear(),
                        "no-matches" => s.model.query.text = "no synthetic match".into(),
                        "loading" => {
                            s.model.entries.clear();
                            s.model.loading = true;
                        }
                        "retention-off" => s.model.retain_history = false,
                        "migration" => s.model.retry_import = true,
                        "error" => s.model.problem = Some(Problem::Unavailable),
                        "confirm" => {
                            s.model.arm(DeleteScope::All, std::time::Instant::now());
                        }
                        _ => (),
                    }
                    if cache {
                        s.model.detail = Some(200);
                    }
                    s.prepare(cx);
                })
            })
            .expect("initialize synthetic History");
        if cache || controls {
            let qa = qa.unwrap();
            cx.spawn(async move |cx| {
                loop {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(500))
                        .await;
                    if cx
                        .update_window(handle.into(), |_, w, cx| {
                            entity.update(cx, |_, cx| cx.notify());
                            // Force the actual view renderer inside the guard, rather
                            // than counting a cached frame that shapes no detail text.
                            w.refresh();
                            let allocations = if cache {
                                fixtures::allocation_probe::count_large_render_allocations(
                                    1_000_000,
                                    || {
                                        w.draw(cx).clear(cx);
                                    },
                                )
                            } else {
                                0
                            };
                            qa.update(cx, |s, cx| {
                                s.allocations += allocations;
                                if cache {
                                    s.guarded_redraws += 1;
                                }
                                println!("MURMUR_HISTORY_QA={}", s.readout(cx));
                                cx.notify();
                            });
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .detach();
        }
        if let Some(path) = output {
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(if cache || controls {
                        750
                    } else {
                        100
                    }))
                    .await;
                cx.update_window(handle.into(), |_, w, cx| {
                    // Headless windows can inherit the shared desktop pointer.
                    // Keep golden hover state fixed using in-process GPUI input;
                    // this never moves the OS pointer or sends native HID input.
                    w.dispatch_event(
                        MouseMoveEvent {
                            position: point(px(-1.), px(-1.)),
                            pressed_button: None,
                            modifiers: Modifiers::default(),
                        }
                        .to_platform_input(),
                        cx,
                    );
                    w.draw(cx).clear(cx);
                    let image = w.render_to_image().expect("offscreen History");
                    assert!(
                        image.pixels().any(|p| *p != *image.get_pixel(0, 0)),
                        "blank History capture"
                    );
                    murmur_ui::overlay::native::assert_webkit_policy("MURMUR_HISTORY_INVENTORY");
                    image.save(path).expect("synthetic capture");
                    cx.quit();
                })
                .expect("History capture");
            })
            .detach();
        } else {
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(
                        std::env::var("MURMUR_QA_SECONDS")
                            .ok()
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(30),
                    ))
                    .await;
                murmur_ui::overlay::native::assert_webkit_policy("MURMUR_HISTORY_INVENTORY");
                cx.update(|cx| cx.quit());
            })
            .detach();
        }
    });
}

struct Qa {
    history: Entity<History>,
    host: Rc<fixtures::FixtureHost>,
    visible: bool,
    controls: bool,
    allocations: usize,
    guarded_redraws: usize,
}
impl Qa {
    fn readout(&self, cx: &App) -> String {
        let (scanned, shaped, boundaries) = self.history.read(cx).cache_counters();
        format!(
            "cache scanned={scanned} shaped={shaped} boundaries={boundaries} allocations={} guarded_redraws={} host revision={} rows={} saved={} clipboard_bytes={} retry_import={} visible={}",
            self.allocations,
            self.guarded_redraws,
            self.host.revision(),
            self.host.rows.borrow().len(),
            self.host.saved.borrow().len(),
            self.host.clipboard.borrow().len(),
            self.host.retry_import.get(),
            self.visible
        )
    }
}
impl Render for Qa {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .when(self.controls, |d| {
                d.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(px(8.))
                        .child(
                            Button::new(
                                "qa-visibility",
                                if self.visible {
                                    "Hide History"
                                } else {
                                    "Reopen History"
                                },
                            )
                            .on_click(cx.listener(|s, _, w, cx| {
                                if s.visible {
                                    s.history.update(cx, |h, cx| h.hide(w, cx));
                                } else {
                                    s.history.update(cx, fixtures::attach);
                                }
                                s.visible = !s.visible;
                                cx.notify();
                            })),
                        )
                        .child(
                            Button::new("qa-replace", "Replace same row")
                                .disabled(
                                    !self.visible
                                        || !self.host.rows.borrow().iter().any(|e| e.key == 200),
                                )
                                .on_click(cx.listener(|s, _, _, _| {
                                    s.host.replace_row(
                                        200,
                                        "Synthetic same-row replacement; cache must invalidate.",
                                    );
                                })),
                        )
                        .child(
                            div()
                                .id("qa-readout")
                                .role(Role::Status)
                                .aria_label(self.readout(cx))
                                .child(self.readout(cx)),
                        ),
                )
            })
            .when(self.visible, |d| {
                d.child(div().flex_1().min_h_0().child(self.history.clone()))
            })
    }
}
