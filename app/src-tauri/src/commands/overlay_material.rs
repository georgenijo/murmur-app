//! Bezel material for the floating pill and its Live Words card.
//!
//! A transparent webview has nothing behind it to blur, so the material is
//! native: a behind-window `NSVisualEffectView` (`.hudWindow`, active,
//! appearance inherited from the system) sits below the WKWebView in the
//! overlay and Live Words windows, inside a container clipped to the visible
//! capsule, hover card, or Live Words card. A tint layer above the blur sets
//! the material floor and a hairline traces the edge. The webview draws only
//! the content on top.
//!
//! Rust owns the shape. The overlay webview reports semantic state (shown,
//! hover-expanded, content); the rect comes from the same geometry contract
//! that sizes the native window, bottom-centred like the CSS island. Shape
//! changes animate on the island's curve and duration; window resizes re-fit
//! the container in the same main-thread turn as the resize. A Dock-follow
//! move needs nothing: the container lives inside the window. The notch never
//! gets a material: switching placement removes it from both windows.

use super::overlay::{OverlayContent, OverlayGeometry};

pub(crate) const OVERLAY_LABEL: &str = "overlay";
pub(crate) const LIVE_WORDS_LABEL: &str = "dictation-preview";

/// The island's width/height transition (`OVERLAY_HEIGHT_MS`) and settling
/// curve (`OVERLAY_PILL_EASE`) in `src/lib/overlayMotion.ts`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const SHAPE_SECONDS: f64 = 0.36;
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const SHAPE_EASE: [f32; 4] = [0.32, 0.72, 0.0, 1.0];
/// CSS `ease-out`, the fade's curve.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const FADE_EASE: [f32; 4] = [0.0, 0.0, 0.58, 1.0];
/// The pill's opacity fade, and its Reduce Motion crossfade.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const FADE_SECONDS: f64 = 0.18;
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const REDUCED_FADE_SECONDS: f64 = 0.16;

/// Live Words card: its window's horizontal padding and corner radius
/// (`.live-words-window`, `.live-words-card` in `src/styles/overlay.css`).
const LIVE_WORDS_INSET_X: f64 = 12.0;
const LIVE_WORDS_RADIUS: f64 = 14.0;

/// Material tint in sRGB. 74–76% opaque, so 11pt labels keep at least 7:1
/// over pure white and pure black whatever the blur passes through. Mirrors
/// `BEZEL_PALETTE` in `src/lib/bezelPalette.ts`; Reduce Transparency draws the
/// same tone fully opaque.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Tint {
    pub(crate) fill: [f64; 4],
    pub(crate) hairline: [f64; 4],
}

pub(crate) const DARK_TINT: Tint = Tint {
    fill: [28.0 / 255.0, 28.0 / 255.0, 30.0 / 255.0, 0.74],
    hairline: [1.0, 1.0, 1.0, 0.16],
};
pub(crate) const LIGHT_TINT: Tint = Tint {
    fill: [246.0 / 255.0, 246.0 / 255.0, 248.0 / 255.0, 0.76],
    hairline: [0.0, 0.0, 0.0, 0.14],
};

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn tint_for(dark: bool, reduce_transparency: bool) -> Tint {
    let mut tint = if dark { DARK_TINT } else { LIGHT_TINT };
    if reduce_transparency {
        tint.fill[3] = 1.0;
    }
    tint
}

/// The visible surface the material covers: bottom-centred in its window,
/// raised by `lift`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MaterialShape {
    pub(crate) width: f64,
    pub(crate) height: f64,
    pub(crate) radius: f64,
    pub(crate) lift: f64,
    pub(crate) visible: bool,
}

/// The floating island's visible shape for `geometry` (resolved for the same
/// `expanded` state). Mirrors `OverlayIsland`: the capsule while collapsed,
/// the window-wide card while hover-expanded.
pub(crate) fn pill_shape(geometry: &OverlayGeometry, shown: bool, expanded: bool) -> MaterialShape {
    MaterialShape {
        width: if expanded {
            geometry.window_w
        } else {
            geometry.island.pill_w
        },
        height: geometry.collapsed_h + if expanded { geometry.dropdown_h } else { 0.0 },
        radius: if expanded {
            geometry.island.corner_active + 4.0
        } else {
            geometry.collapsed_h / 2.0
        },
        lift: 0.0,
        visible: shown || expanded,
    }
}

/// The Live Words card: measured by the webview (its width follows the words),
/// bounded by the window, and lifted clear of the hover card by Rust.
pub(crate) fn live_words_shape(card: (f64, f64), window: (f64, f64), lift: f64) -> MaterialShape {
    let finite = |v: f64| if v.is_finite() { v.max(0.0) } else { 0.0 };
    let width = finite(card.0).min((window.0 - 2.0 * LIVE_WORDS_INSET_X).max(0.0));
    let height = finite(card.1).min(window.1);
    MaterialShape {
        width,
        height,
        radius: LIVE_WORDS_RADIUS.min(height / 2.0),
        lift: finite(lift),
        visible: width >= 1.0 && height >= 1.0,
    }
}

/// The hover card's height, which the Live Words card rises by.
pub(crate) fn live_words_lift(geometry: &OverlayGeometry, expanded: bool) -> f64 {
    if expanded {
        geometry.dropdown_h
    } else {
        0.0
    }
}

/// `shape`'s frame inside a superview of `bounds`, in that view's coordinates.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn frame_in(bounds: (f64, f64), shape: MaterialShape, flipped: bool) -> [f64; 4] {
    let (w, h) = bounds;
    let x = (w - shape.width) / 2.0;
    let y = if flipped {
        h - shape.height - shape.lift
    } else {
        shape.lift
    };
    [x, y, shape.width, shape.height]
}

/// Content the pill reports with its state. Absent means the controls.
pub(crate) fn content_or_default(content: Option<OverlayContent>) -> OverlayContent {
    content.unwrap_or_default()
}

#[cfg(target_os = "macos")]
mod native {
    use super::*;
    use objc2::rc::Retained;
    use objc2::runtime::NSObject;
    use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{
        NSAnimatablePropertyContainer, NSAnimationContext, NSAppearanceCustomization,
        NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSAutoresizingMaskOptions, NSResponder,
        NSView, NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState,
        NSVisualEffectView, NSWindow, NSWindowOrderingMode, NSWorkspace,
    };
    use objc2_core_graphics::CGColor;
    use objc2_foundation::{NSArray, NSPoint, NSRect, NSSize};
    use objc2_quartz_core::CAMediaTimingFunction;
    use std::cell::{OnceCell, RefCell};
    use std::collections::HashMap;

    pub(crate) struct Ivars {
        tint: OnceCell<Retained<NSView>>,
    }

    define_class!(
        /// The clipped container. It restyles its tint and hairline when the
        /// system appearance changes; the blur follows on its own.
        #[unsafe(super(NSView, NSResponder, NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "MurmurBezelMaterialView"]
        #[ivars = Ivars]
        pub(crate) struct BezelMaterialView;

        impl BezelMaterialView {
            #[unsafe(method(viewDidChangeEffectiveAppearance))]
            fn view_did_change_effective_appearance(&self) {
                let _: () = unsafe { msg_send![super(self), viewDidChangeEffectiveAppearance] };
                self.restyle();
            }
        }
    );

    impl BezelMaterialView {
        fn new(mtm: MainThreadMarker) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(Ivars {
                tint: OnceCell::new(),
            });
            let zero = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0));
            let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: zero] };
            this.setWantsLayer(true);
            if let Some(layer) = this.layer() {
                layer.setMasksToBounds(true);
                layer.setBorderWidth(1.0);
            }
            let fill = |view: &NSView| {
                view.setAutoresizingMask(
                    NSAutoresizingMaskOptions::ViewWidthSizable
                        | NSAutoresizingMaskOptions::ViewHeightSizable,
                );
                this.addSubview(view);
            };
            let blur = NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm), zero);
            blur.setMaterial(NSVisualEffectMaterial::HUDWindow);
            blur.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
            blur.setState(NSVisualEffectState::Active);
            fill(&blur);
            let tint = NSView::initWithFrame(NSView::alloc(mtm), zero);
            tint.setWantsLayer(true);
            fill(&tint);
            let _ = this.ivars().tint.set(tint);
            this.setAlphaValue(0.0);
            this.restyle();
            this
        }

        fn restyle(&self) {
            let appearance = self.effectiveAppearance();
            let names = NSArray::from_slice(&[unsafe { NSAppearanceNameAqua }, unsafe {
                NSAppearanceNameDarkAqua
            }]);
            let dark = appearance
                .bestMatchFromAppearancesWithNames(&names)
                .is_some_and(|name| &*name == unsafe { NSAppearanceNameDarkAqua });
            let reduce =
                NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceTransparency();
            let tint = tint_for(dark, reduce);
            let color = |c: [f64; 4]| CGColor::new_srgb(c[0], c[1], c[2], c[3]);
            if let Some(layer) = self.layer() {
                layer.setBorderColor(Some(&color(tint.hairline)));
            }
            if let Some(layer) = self.ivars().tint.get().and_then(|view| view.layer()) {
                layer.setBackgroundColor(Some(&color(tint.fill)));
            }
        }
    }

    struct Installed {
        view: Retained<BezelMaterialView>,
        shape: Option<MaterialShape>,
    }

    thread_local! {
        static INSTALLED: RefCell<HashMap<&'static str, Installed>> = RefCell::new(HashMap::new());
        /// The Live Words card as last measured, and the lift the pill set.
        static LIVE_WORDS: RefCell<((f64, f64), f64)> = const { RefCell::new(((0.0, 0.0), 0.0)) };
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    pub(crate) enum Motion {
        /// Follow the island: shape changes on its curve, visibility on its fade.
        Island,
        /// Track the webview without animation (Live Words words and resizes).
        Instant,
    }

    fn content_view(window: &tauri::WebviewWindow) -> Option<Retained<NSView>> {
        let ptr = window.ns_window().ok()?;
        let ns_window: &NSWindow = unsafe { &*(ptr.cast()) };
        ns_window.contentView()
    }

    fn rect(frame: [f64; 4]) -> NSRect {
        NSRect::new(
            NSPoint::new(frame[0], frame[1]),
            NSSize::new(frame[2], frame[3]),
        )
    }

    fn same(a: NSRect, b: NSRect) -> bool {
        const EPSILON: f64 = 0.01;
        (a.origin.x - b.origin.x).abs() < EPSILON
            && (a.origin.y - b.origin.y).abs() < EPSILON
            && (a.size.width - b.size.width).abs() < EPSILON
            && (a.size.height - b.size.height).abs() < EPSILON
    }

    fn animate(seconds: f64, ease: [f32; 4], changes: impl Fn() + 'static) {
        let block = block2::RcBlock::new(move |context: std::ptr::NonNull<NSAnimationContext>| {
            let context = unsafe { context.as_ref() };
            context.setDuration(seconds);
            let [a, b, c, d] = ease;
            let timing = CAMediaTimingFunction::functionWithControlPoints(a, b, c, d);
            context.setTimingFunction(Some(&timing));
            context.setAllowsImplicitAnimation(true);
            changes();
        });
        NSAnimationContext::runAnimationGroup(&block);
    }

    /// Install (once) and move the material in `label`'s window to `shape`.
    /// `None` removes it. Must run on the main thread.
    pub(crate) fn apply(
        app: &tauri::AppHandle,
        label: &'static str,
        shape: Option<MaterialShape>,
        motion: Motion,
    ) {
        use tauri::Manager;
        let Some(mtm) = MainThreadMarker::new() else {
            debug_assert!(false, "NSView mutation must run on the main thread");
            return;
        };
        let Some(shape) = shape else {
            remove(label);
            return;
        };
        let Some(content) = app
            .get_webview_window(label)
            .and_then(|window| content_view(&window))
        else {
            return;
        };
        INSTALLED.with_borrow_mut(|installed| {
            let entry = installed.entry(label).or_insert_with(|| {
                let view = BezelMaterialView::new(mtm);
                // Pin to the bottom edge, centred: a window resize the fit
                // below has not caught yet still keeps the island's anchor.
                let bottom = if content.isFlipped() {
                    NSAutoresizingMaskOptions::ViewMinYMargin
                } else {
                    NSAutoresizingMaskOptions::ViewMaxYMargin
                };
                view.setAutoresizingMask(
                    NSAutoresizingMaskOptions::ViewMinXMargin
                        | NSAutoresizingMaskOptions::ViewMaxXMargin
                        | bottom,
                );
                content.addSubview_positioned_relativeTo(&view, NSWindowOrderingMode::Below, None);
                Installed { view, shape: None }
            });
            let previous = entry.shape.replace(shape);
            let view = entry.view.clone();
            let bounds = content.bounds().size;
            let target = rect(frame_in(
                (bounds.width, bounds.height),
                shape,
                content.isFlipped(),
            ));
            let corner = |view: &BezelMaterialView| {
                if let Some(layer) = view.layer() {
                    layer.setCornerRadius(shape.radius);
                }
            };
            let alpha = if shape.visible { 1.0 } else { 0.0 };
            let was_visible = previous.is_some_and(|p| p.visible) && view.alphaValue() > 0.0;
            let reduce_motion =
                NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion();
            if motion == Motion::Instant || !shape.visible || !was_visible {
                // Appearing takes the current shape at once and fades in;
                // hiding fades out in place, as the island does.
                if shape.visible || motion == Motion::Instant {
                    if !same(view.frame(), target) {
                        view.setFrame(target);
                    }
                    corner(&view);
                }
                if (view.alphaValue() - alpha).abs() > f64::EPSILON {
                    if motion == Motion::Instant {
                        view.setAlphaValue(alpha);
                    } else {
                        let seconds = if reduce_motion {
                            REDUCED_FADE_SECONDS
                        } else {
                            FADE_SECONDS
                        };
                        let faded = view.clone();
                        animate(seconds, FADE_EASE, move || {
                            faded.animator().setAlphaValue(alpha)
                        });
                    }
                }
                return;
            }
            corner(&view);
            if same(view.frame(), target) {
                return;
            }
            if reduce_motion {
                view.setFrame(target);
            } else {
                let moved = view.clone();
                animate(SHAPE_SECONDS, SHAPE_EASE, move || {
                    moved.animator().setFrame(target)
                });
            }
        });
    }

    /// Re-fit `label`'s material to its window after a native resize, in the
    /// same main-thread turn, so it never trails the window by a frame.
    pub(crate) fn refit_now(app: &tauri::AppHandle, label: &'static str) {
        if label == LIVE_WORDS_LABEL {
            apply_live_words(app, None, None, Motion::Instant);
            return;
        }
        let shape = INSTALLED.with_borrow(|installed| installed.get(label).and_then(|i| i.shape));
        if let Some(shape) = shape {
            apply(app, label, Some(shape), Motion::Instant);
        }
    }

    pub(crate) fn installed(label: &'static str) -> bool {
        INSTALLED.with_borrow(|installed| installed.contains_key(label))
    }

    pub(crate) fn remove(label: &'static str) {
        if let Some(installed) = INSTALLED.with_borrow_mut(|installed| installed.remove(label)) {
            installed.view.removeFromSuperview();
        }
    }

    pub(crate) fn remove_all() {
        remove(OVERLAY_LABEL);
        remove(LIVE_WORDS_LABEL);
        LIVE_WORDS.set(((0.0, 0.0), 0.0));
    }

    fn live_words_window_size(app: &tauri::AppHandle) -> Option<(f64, f64)> {
        use tauri::Manager;
        let window = app.get_webview_window(LIVE_WORDS_LABEL)?;
        let content = content_view(&window)?;
        let size = content.bounds().size;
        Some((size.width, size.height))
    }

    /// Move the Live Words card. `card` keeps the last measurement when absent;
    /// `lift` keeps the last lift when absent.
    pub(crate) fn apply_live_words(
        app: &tauri::AppHandle,
        card: Option<(f64, f64)>,
        lift: Option<f64>,
        motion: Motion,
    ) {
        let (card, lift) = LIVE_WORDS.with_borrow_mut(|state| {
            if let Some(card) = card {
                state.0 = card;
            }
            if let Some(lift) = lift {
                state.1 = lift;
            }
            *state
        });
        if !installed(LIVE_WORDS_LABEL) && card == (0.0, 0.0) {
            return;
        }
        let Some(window) = live_words_window_size(app) else {
            return;
        };
        apply(
            app,
            LIVE_WORDS_LABEL,
            Some(live_words_shape(card, window, lift)),
            motion,
        );
    }

    /// Restyle every installed material after an accessibility display change
    /// (Reduce Transparency). Appearance changes reach the views directly.
    pub(crate) fn restyle_all() {
        INSTALLED.with_borrow(|installed| {
            for entry in installed.values() {
                entry.view.restyle();
            }
        });
    }

    /// Follow Reduce Transparency for the app's lifetime.
    pub(crate) fn register_accessibility_observer() {
        use objc2_app_kit::NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification;
        use objc2_foundation::{NSNotification, NSOperationQueue};
        let block = block2::RcBlock::new(|_notification: std::ptr::NonNull<NSNotification>| {
            restyle_all();
        });
        let center = NSWorkspace::sharedWorkspace().notificationCenter();
        let observer = unsafe {
            center.addObserverForName_object_queue_usingBlock(
                Some(NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification),
                None,
                Some(&NSOperationQueue::mainQueue()),
                &block,
            )
        };
        // App-lifetime observer — intentionally leak to avoid premature deallocation
        std::mem::forget(observer);
    }
}

#[cfg(target_os = "macos")]
pub(crate) use native::register_accessibility_observer;

/// Run `f` on the main thread: at once when already there, so a refit lands
/// in the same turn as the resize that caused it.
#[cfg(target_os = "macos")]
fn on_main(app: &tauri::AppHandle, f: impl FnOnce(&tauri::AppHandle) + Send + 'static) {
    if objc2::MainThreadMarker::new().is_some() {
        f(app);
        return;
    }
    let handle = app.clone();
    if let Err(error) = app.run_on_main_thread(move || f(&handle)) {
        tracing::warn!(target: "system", "overlay material: main-thread dispatch failed: {}", error);
    }
}

/// Re-fit `label`'s material to its window after a native resize.
#[cfg(target_os = "macos")]
pub(crate) fn refit(app: &tauri::AppHandle, label: &'static str) {
    on_main(app, move |app| native::refit_now(app, label));
}

/// Remove the material from both windows (notch placement).
#[cfg(target_os = "macos")]
pub(crate) fn remove_all() {
    if objc2::MainThreadMarker::new().is_some() {
        native::remove_all();
    } else {
        debug_assert!(false, "material removal must run on the main thread");
    }
}

/// Run a material change on the main thread and wait for it.
#[cfg(target_os = "macos")]
async fn on_main_acked(
    app: &tauri::AppHandle,
    f: impl FnOnce(&tauri::AppHandle) -> bool + Send + 'static,
) -> Result<bool, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let _ = tx.send(f(&handle));
    })
    .map_err(|error| error.to_string())?;
    rx.await
        .map_err(|_| "overlay material update was dropped".to_string())
}

/// Shape the floating pill's material from the island's semantic state:
/// `shown` (not faded out for idle), `expanded` (hover card open, as rendered
/// by CSS), and the card's `content`. `floating: false` removes the material
/// from both windows. Resolves `true` while a native material backs the pill,
/// so the webview draws no surface of its own.
#[tauri::command]
pub async fn set_overlay_material(
    app: tauri::AppHandle,
    shown: bool,
    expanded: bool,
    content: Option<OverlayContent>,
    floating: bool,
) -> Result<bool, String> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, shown, expanded, content, floating);
        Ok(false)
    }
    #[cfg(target_os = "macos")]
    {
        let content = content_or_default(content);
        on_main_acked(&app, move |app| {
            if !floating {
                native::remove_all();
                return false;
            }
            let geometry = super::overlay::floating_pill_geometry(app, content, expanded);
            native::apply(
                app,
                OVERLAY_LABEL,
                Some(pill_shape(&geometry, shown, expanded)),
                native::Motion::Island,
            );
            native::apply_live_words(
                app,
                None,
                Some(live_words_lift(&geometry, expanded)),
                native::Motion::Island,
            );
            native::installed(OVERLAY_LABEL)
        })
        .await
    }
}

/// The Live Words card's laid-out size (untransformed), measured by its
/// webview. Rust positions it bottom-centred, raised by the pill's lift.
/// Resolves `true` while a native material backs the card.
#[tauri::command]
pub async fn set_live_words_material(
    app: tauri::AppHandle,
    width: f64,
    height: f64,
) -> Result<bool, String> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, width, height);
        Ok(false)
    }
    #[cfg(target_os = "macos")]
    {
        on_main_acked(&app, move |app| {
            if !super::overlay::floating_pill_placed(app) {
                native::remove(LIVE_WORDS_LABEL);
                return false;
            }
            native::apply_live_words(app, Some((width, height)), None, native::Motion::Instant);
            native::installed(LIVE_WORDS_LABEL)
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn relative_luminance(c: [f64; 3]) -> f64 {
        let lin = |v: f64| {
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2])
    }

    fn over(top: [f64; 4], bottom: [f64; 3]) -> [f64; 3] {
        let a = top[3];
        [0, 1, 2].map(|i| top[i] * a + bottom[i] * (1.0 - a))
    }

    fn contrast(a: [f64; 3], b: [f64; 3]) -> f64 {
        let (la, lb) = (relative_luminance(a), relative_luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    #[test]
    fn material_floor_keeps_labels_at_seven_to_one_over_any_backdrop() {
        // Labels: solid white on dark, 87% black on light (bezelPalette.ts).
        for (tint, label) in [
            (DARK_TINT, [1.0, 1.0, 1.0, 1.0]),
            (LIGHT_TINT, [0.0, 0.0, 0.0, 0.87]),
        ] {
            assert!((0.74..=0.76).contains(&tint.fill[3]));
            for backdrop in [[1.0, 1.0, 1.0], [0.0, 0.0, 0.0]] {
                let material = over(tint.fill, backdrop);
                let ratio = contrast(over(label, material), material);
                assert!(ratio >= 7.0, "{tint:?} over {backdrop:?}: {ratio:.2}");
            }
        }
    }

    #[test]
    fn reduce_transparency_keeps_the_tone_and_drops_the_translucency() {
        for dark in [true, false] {
            let solid = tint_for(dark, true);
            let normal = tint_for(dark, false);
            assert_eq!(solid.fill[..3], normal.fill[..3]);
            assert_eq!(solid.fill[3], 1.0);
            assert_eq!(solid.hairline, normal.hairline);
        }
    }

    #[test]
    fn frames_are_bottom_centred_in_either_coordinate_system() {
        let shape = MaterialShape {
            width: 120.0,
            height: 34.0,
            radius: 17.0,
            lift: 0.0,
            visible: true,
        };
        assert_eq!(
            frame_in((241.0, 98.0), shape, false),
            [60.5, 0.0, 120.0, 34.0]
        );
        assert_eq!(
            frame_in((241.0, 98.0), shape, true),
            [60.5, 64.0, 120.0, 34.0]
        );
        let lifted = MaterialShape {
            lift: 64.0,
            ..shape
        };
        assert_eq!(
            frame_in((460.0, 168.0), lifted, false),
            [170.0, 64.0, 120.0, 34.0]
        );
        assert_eq!(
            frame_in((460.0, 168.0), lifted, true),
            [170.0, 70.0, 120.0, 34.0]
        );
    }

    #[test]
    fn live_words_card_is_bounded_by_its_window() {
        let shape = live_words_shape((900.0, 400.0), (460.0, 168.0), 64.0);
        assert_eq!(
            (shape.width, shape.height, shape.lift),
            (436.0, 168.0, 64.0)
        );
        assert!(shape.visible);
        let empty = live_words_shape((0.0, 0.0), (460.0, 168.0), 0.0);
        assert!(!empty.visible);
        let junk = live_words_shape((f64::NAN, -3.0), (460.0, 168.0), f64::INFINITY);
        assert_eq!((junk.width, junk.height, junk.lift), (0.0, 0.0, 0.0));
        assert_eq!(
            live_words_shape((200.0, 20.0), (460.0, 168.0), 0.0).radius,
            10.0
        );
    }
}
