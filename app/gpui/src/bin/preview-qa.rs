//! Synthetic standalone host; no service, file, pasteboard or inference access.
use gpui::{prelude::*, *};
use murmur_core::{dictation::Status, overlay_geometry::*};
use murmur_ui::preview::{Host, Preview, model::*};
use std::{cell::Cell, rc::Rc};
struct Current(Cell<Option<u64>>);
impl Host for Current {
    fn is_current(&self, generation: u64) -> bool {
        self.0.get() == Some(generation)
    }
}
fn main() {
    murmur_ui::overlay::native::start_webkit_policy();
    if std::env::var_os("MURMUR_WEBKIT_BASELINE_ONLY").is_some() {
        murmur_ui::overlay::native::assert_webkit_policy("MURMUR_PREVIEW_INVENTORY");
        return;
    }
    let output = std::env::var_os("MURMUR_GOLDEN");
    let headless = output.is_some();
    let native_smoke = std::env::var_os("MURMUR_PREVIEW_NATIVE_SMOKE").is_some();
    assert!(
        !native_smoke || !headless,
        "native smoke needs a real window"
    );
    let app = if headless || native_smoke {
        gpui_platform::application().with_windowing(WindowingRequest::Headless)
    } else {
        gpui_platform::application()
    };
    app.run(move |cx| {
        let state = std::env::var("MURMUR_PREVIEW_STATE").unwrap_or_else(|_| "short".into());
        let above = std::env::var_os("MURMUR_PILL").is_some();
        let dark = std::env::var_os("MURMUR_DARK").is_some();
        murmur_ui::init_with_preferences(cx, headless.then_some(murmur_ui::preferences::DisplayPreferences {
            reduce_motion: true, reduce_transparency: true, increase_contrast: state == "contrast",
        }));
        murmur_ui::tokens::apply_appearance(dark, cx);
        let mut model = Model::default();
        model.event(Event::Anchor(AnchorSnapshot { revision: 1, anchor: murmur_ui::preview::model::Anchor {
            frame: OverlayFrame {x: 500., y: if above {700.} else {0.}, width: 240., height: 37.},
            above, lift: if state == "lift" {64.} else {0.},
            work: PopoverWorkArea {left: 0., right: 1440., top: 30., bottom: 900.},
        }}));
        model.event(Event::ConsentRead {epoch: 0, enabled: true});
        model.event(Event::Lifecycle(LifecycleSnapshot {revision: 1, generation: 1, status: Status::Recording}));
        let text = match state.as_str() {
            "long" => "Synthetic fixture words fill three lines and keep the newest synthetic ending in view. ".repeat(6),
            "unicode" => "Synthetic 日本語 😀 العربية עברית fixture".into(),
            "unbroken" => "x".repeat(320),
            _ => "Synthetic live words".into(),
        };
        if !native_smoke {
            model.event(Event::Partial {generation: 1, text});
        }
        if state == "stable" {
            model.event(Event::AuthoritativePartial {generation: 1,
                stable: "Synthetic authoritative words".into(), tentative: "synthetic provisional ending".into()});
        }
        let frame = model.anchor.unwrap().window(model.reserve).unwrap();
        let host = Rc::new(Current(Cell::new(Some(1))));
        let owner = host.clone();
        let mut entity = None;
        let handle = cx.open_window(WindowOptions {
            kind: WindowKind::PopUp, show: false, focus: false,
            window_background: WindowBackgroundAppearance::Transparent,
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(point(px(0.),px(0.)),size(px(frame.width as f32),px(frame.height as f32))))),
            ..Default::default()
        }, |w, cx| {
            let preview = cx.new(|_| {let mut p = Preview::new(host); p.model = model; p.fixture_dark = headless.then_some(dark); p});
            entity = Some(preview.clone());
            cx.new(|cx| gpui_base::Root::new(preview, w, cx))
        }).expect("synthetic preview window");
        let preview = entity.unwrap();
        handle.update(cx, |_, w, cx| preview.update(cx, |s, cx| {
            s.advance(w,cx); if !headless {s.follow(w,cx);}
        })).expect("initialize synthetic preview");
        if native_smoke {
            // No activation/HID, file reads, pasteboard or production services.
            // The observer may inspect/capture only this synthetic host's PID.
            cx.spawn(async move |cx| {
                for generation in 1..=4 {
                    owner.0.set(Some(generation));
                    cx.update_window(handle.into(), |_, w, cx| preview.update(cx, |s, cx| {
                        s.receive(Event::Lifecycle(LifecycleSnapshot {
                            revision: generation * 2, generation, status: Status::Recording,
                        }), w, cx);
                    })).expect("synthetic start");
                    println!("MURMUR_PREVIEW_STAGE=empty-{generation}");
                    cx.background_executor().timer(std::time::Duration::from_secs(2)).await;
                    if generation > 1 {
                        cx.update_window(handle.into(), |_, w, cx| preview.update(cx, |s, cx| {
                            s.receive(Event::Partial { generation, text: "Synthetic native fixture".into() }, w, cx);
                        })).expect("synthetic partial");
                        println!("MURMUR_PREVIEW_STAGE=visible-{generation}");
                        cx.background_executor().timer(std::time::Duration::from_secs(2)).await;
                    }
                    owner.0.set(None);
                    cx.update_window(handle.into(), |_, w, cx| preview.update(cx, |s, cx| {
                        s.receive(Event::Lifecycle(LifecycleSnapshot {
                            revision: generation * 2 + 1, generation, status: Status::Idle,
                        }), w, cx);
                    })).expect("synthetic stop");
                    println!("MURMUR_PREVIEW_STAGE=stopped-{generation}");
                    cx.background_executor().timer(std::time::Duration::from_secs(2)).await;
                }
                murmur_ui::overlay::native::assert_webkit_policy("MURMUR_PREVIEW_INVENTORY");
                cx.update(|cx| cx.quit());
            }).detach();
        }
        {
            if let Some(path) = output {
                cx.spawn(async move |cx| {
                    cx.background_executor().timer(std::time::Duration::from_millis(100)).await;
                    cx.update_window(handle.into(), |_, w, cx| {
                        w.draw(cx).clear(cx);
                        let image = w.render_to_image().expect("offscreen preview");
                        assert!(image.pixels().any(|p| *p != *image.get_pixel(0,0)), "blank synthetic preview");
                        murmur_ui::overlay::native::assert_webkit_policy("MURMUR_PREVIEW_INVENTORY");
                        image.save(path).expect("synthetic capture"); cx.quit();
                    }).expect("capture preview");
                }).detach();
            }
        }
    });
}
