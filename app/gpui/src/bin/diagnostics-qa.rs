//! Synthetic QA only; goldens use offscreen Metal, never the shared desktop.
use gpui::*;
use murmur_ui::{
    diagnostics::{Diagnostics, fixtures::FixtureHost, model::*},
    preferences::DisplayPreferences,
};
use std::rc::Rc;
fn main() {
    println!(
        "MURMUR_DIAGNOSTICS_REVISION={}",
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
        let state = std::env::var("MURMUR_DIAGNOSTICS_STATE").unwrap_or_else(|_| "events".into());
        let mut entity = None;
        let handle = cx
            .open_window(
                WindowOptions {
                    show: !headless,
                    focus: false,
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(0.), px(0.)),
                        size(px(1120.), px(820.)),
                    ))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Murmur Diagnostics (synthetic)".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |w, cx| {
                    if !headless {
                        let apply = |w: &mut Window, cx:&mut App| {
                            murmur_ui::tokens::apply_appearance(matches!(w.appearance(), WindowAppearance::Dark|WindowAppearance::VibrantDark),cx);
                            cx.refresh_windows();
                        };
                        apply(w,cx);
                        w.observe_window_appearance(apply).detach();
                    }
                    let d = cx.new(|cx| {
                        let mut d = Diagnostics::new(Rc::new(FixtureHost::default()), w, cx);
                        d.fixture = headless;
                        d
                    });
                    entity = Some(d.clone());
                    cx.new(|cx| gpui_base::Root::new(d, w, cx))
                },
            )
            .expect("synthetic diagnostics");
        let entity = entity.unwrap();
        handle
            .update(cx, |_, _, cx| {
                entity.update(cx, |s, cx| {
                    s.attach(cx);
                    // One UI turn drains fixture replies before choosing the requested
                    // state. The production host follows the same inbox admission path.
                    s.receive(
                        Event::Action {
                            ticket: Ticket {
                                attachment: 0,
                                lane: Lane::Route,
                                epoch: 0,
                                store_epoch: 0,
                            },
                            success: false,
                        },
                        cx,
                    );
                    s.model.tab = if state == "run-detail" {
                        s.model.selected_run = Some("synthetic-run".into());
                        Tab::Runs
                    } else if state == "private-detail" {
                        s.model.selected_capture = Some("synthetic-capture".into());
                        s.model.capture_detail = Some(murmur_ui::diagnostics::fixtures::capture(
                            "synthetic-capture".into(),
                        ));
                        Tab::Dictation
                    } else {
                        Tab::from_route(&state).unwrap_or(Tab::Events)
                    };
                    if state == "store-unavailable" {
                        s.model.tab = Tab::Runs;
                        s.model.health = Some(Health {
                            available: false,
                            recommended: Recovery::Reinitialize,
            problem:Some(StoreProblem::CorruptIntegrity),
                            skipped: 0,
                            recent_write_failed: false,
                            recovered: false,
                        });
                    }
                    if state=="empty" {s.model.events.clear();s.model.reports.clear();s.model.runs.clear();s.model.samples.clear();s.model.captures.clear();}
            if state=="loading" {s.model.events.clear();s.model.loading.insert(Lane::Events);}
            if state=="reports-blocked" {s.model.tab=Tab::Compare; s.model.reports[1]=murmur_ui::diagnostics::reports::Report::parse(include_bytes!("../../../../tests/fixtures/diagnostic-reports/evaluation-v1-deterministic.json"),0,murmur_ui::diagnostics::reports::Source::Imported).ok().unwrap();}
            s.prepare(cx);
            if state=="events-expanded" {s.events.update(cx,|s,cx|s.set_expanded(160,true,cx));}
                    if !headless {
                        s.follow(cx);
                    }
                })
            })
            .expect("initialize diagnostics");
        if !headless && std::env::var_os("MURMUR_DIAGNOSTICS_NATIVE_SMOKE").is_some() {
            cx.spawn(async move |cx| {
                cx.background_executor().timer(std::time::Duration::from_secs(20)).await;
                cx.update(|cx| cx.quit());
            }).detach();
        }
        if let Some(path) = output {
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(100))
                    .await;
                cx.update_window(handle.into(), |_, w, cx| {
                    w.draw(cx).clear(cx);
                    let image = w.render_to_image().expect("offscreen diagnostics");
                    assert!(
                        image.pixels().any(|p| *p != *image.get_pixel(0, 0)),
                        "blank synthetic diagnostics"
                    );
                    let images = murmur_ui::overlay::native::webkit_images();
                    assert_eq!(images, 0);
                    println!(
                        "MURMUR_DIAGNOSTICS_INVENTORY={{\"pid\":{},\"webkitImages\":{images}}}",
                        std::process::id()
                    );
                    image.save(path).expect("synthetic capture");
                    cx.quit();
                })
                .expect("capture diagnostics");
            })
            .detach();
        }
    });
}
