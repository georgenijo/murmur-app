//! Live Dock tracking for the floating pill.
//!
//! A bottom Dock that auto-hides reserves almost no work area, and a magnified
//! Dock overflows the area it does reserve, so neither preferences nor the
//! work area say where the Dock actually is. The Dock's accessibility tree
//! does: its `AXList` slides with the reveal and its items report their
//! magnified frames. A worker samples that geometry and publishes a
//! [`DockLimit`]; `commands::overlay` rests the pill above it and glides the
//! window when it changes. When accessibility is unavailable the limit stays
//! [`DockLimit::Unknown`] and the overlay keeps its conservative static reserve.
//!
//! Only frames are read: no titles, no labels, no screen capture.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Room above the tallest Dock icon for the hover name label and its gap. The
/// label has no accessibility frame of its own, so this is an allowance.
const LABEL_ALLOWANCE: f64 = 40.0;
/// Pointer distance from the bottom edge that wakes fast sampling, so a reveal
/// is caught while the Dock is still sliding in.
const WAKE_BAND: f64 = 200.0;
const IDLE_INTERVAL: Duration = Duration::from_millis(500);
const FAST_INTERVAL: Duration = Duration::from_millis(33);
const FRAME_INTERVAL: Duration = Duration::from_millis(16);
const RISE: Duration = Duration::from_millis(120);
const FALL: Duration = Duration::from_millis(200);
/// Consecutive hidden samples before the pill drops. One stray sample while
/// the pointer crosses the Dock edge must not bounce it.
const HIDDEN_CONFIRMATIONS: u8 = 2;
/// Consecutive failed samples before falling back to the static reserve.
const FAILURES_BEFORE_FALLBACK: u8 = 3;
/// Never walk more Dock items than this in one sample.
const MAX_ITEMS: usize = 96;

/// Where the pill must stay above, in global logical y (top-left origin).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum DockLimit {
    /// No live reading: use the static preference reserve.
    Unknown,
    /// The Dock is hidden, elsewhere, or within the work area already.
    Clear,
    /// The revealed Dock (with magnification and label room) starts here.
    Above(f64),
}

static LIMIT: Mutex<DockLimit> = Mutex::new(DockLimit::Unknown);

pub(crate) fn current_limit() -> DockLimit {
    *LIMIT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn publish(limit: DockLimit) -> bool {
    let mut current = LIMIT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let changed = *current != limit;
    *current = limit;
    changed
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Rect {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) width: f64,
    pub(crate) height: f64,
}

impl Rect {
    fn finite(self) -> bool {
        [self.x, self.y, self.width, self.height]
            .iter()
            .all(|v| v.is_finite())
            && self.width >= 0.0
            && self.height >= 0.0
    }
}

/// The pill's display in global logical points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PillDisplay {
    pub(crate) left: f64,
    pub(crate) top: f64,
    pub(crate) right: f64,
    pub(crate) bottom: f64,
    /// Bottom of the work area: where the pill rests with the Dock clear.
    pub(crate) work_bottom: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum DockReading {
    Hidden,
    Exposed {
        top: f64,
    },
    /// A side Dock, or a Dock on another display: it never covers the pill.
    Elsewhere,
    Unavailable,
}

/// Classify one sample of the Dock list frame and its item frames.
pub(crate) fn classify(list: Rect, items: &[Rect], display: PillDisplay) -> DockReading {
    if !list.finite() || list.width <= 0.0 {
        return DockReading::Unavailable;
    }
    let centre = list.x + list.width / 2.0;
    if list.width <= list.height
        || centre < display.left
        || centre > display.right
        || list.y < display.top
    {
        return DockReading::Elsewhere;
    }
    if list.y >= display.bottom - 1.0 {
        return DockReading::Hidden;
    }
    let top = items
        .iter()
        .filter(|item| item.finite() && item.height > 0.0 && item.y >= display.top)
        .map(|item| item.y)
        .fold(list.y, f64::min);
    DockReading::Exposed {
        top: top - LABEL_ALLOWANCE,
    }
}

/// Turns readings into the limit the pill should settle on. It holds the
/// highest clearance of one reveal so magnification never makes the pill bob,
/// and drops only after the Dock is confirmed hidden.
#[derive(Debug, Default)]
pub(crate) struct DockTracker {
    envelope: Option<f64>,
    hidden: u8,
    failures: u8,
    last: Option<DockLimit>,
}

impl DockTracker {
    pub(crate) fn observe(&mut self, reading: DockReading) -> DockLimit {
        let limit = match reading {
            DockReading::Unavailable => {
                self.failures = self.failures.saturating_add(1);
                match self.last {
                    Some(last) if self.failures < FAILURES_BEFORE_FALLBACK => last,
                    _ => {
                        self.envelope = None;
                        DockLimit::Unknown
                    }
                }
            }
            DockReading::Elsewhere => {
                self.failures = 0;
                self.hidden = 0;
                self.envelope = None;
                DockLimit::Clear
            }
            DockReading::Hidden => {
                self.failures = 0;
                self.hidden = self.hidden.saturating_add(1);
                match (self.envelope, self.hidden >= HIDDEN_CONFIRMATIONS) {
                    (Some(top), false) => DockLimit::Above(top),
                    _ => {
                        self.envelope = None;
                        DockLimit::Clear
                    }
                }
            }
            DockReading::Exposed { top } => {
                self.failures = 0;
                self.hidden = 0;
                let top = self.envelope.map_or(top, |held| held.min(top));
                self.envelope = Some(top);
                DockLimit::Above(top)
            }
        };
        self.last = Some(limit);
        limit
    }
}

/// The y a limit puts the pill's bottom edge at, before the pill's own gap.
fn resting_bottom(limit: DockLimit, display: PillDisplay) -> Option<f64> {
    match limit {
        DockLimit::Unknown => None,
        DockLimit::Clear => Some(display.work_bottom),
        DockLimit::Above(top) => Some(display.work_bottom.min(top)),
    }
}

/// One ease-out glide of the pill's bottom edge toward a settled limit.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Glide {
    from: f64,
    to: f64,
    started: Instant,
    duration: Duration,
    target: DockLimit,
}

impl Glide {
    /// Rising clears the Dock quickly; falling back down is gentler.
    pub(crate) fn new(from: f64, to: f64, target: DockLimit, now: Instant) -> Self {
        Self {
            from,
            to,
            started: now,
            duration: if to < from { RISE } else { FALL },
            target,
        }
    }

    /// The limit to publish at `now`, and whether the glide has finished.
    pub(crate) fn at(&self, now: Instant) -> (DockLimit, bool) {
        let t =
            now.saturating_duration_since(self.started).as_secs_f64() / self.duration.as_secs_f64();
        if t >= 1.0 {
            return (self.target, true);
        }
        let eased = 1.0 - (1.0 - t).powi(3);
        (
            DockLimit::Above(self.from + (self.to - self.from) * eased),
            false,
        )
    }
}

/// Owns the published limit: settled targets either jump (no previous
/// position, static fallback, reduced motion) or glide.
#[derive(Debug, Default)]
pub(crate) struct LimitAnimator {
    shown: Option<f64>,
    glide: Option<Glide>,
    settled: Option<DockLimit>,
}

impl LimitAnimator {
    /// Retarget toward `target`. Returns the limit to publish now.
    pub(crate) fn retarget(
        &mut self,
        target: DockLimit,
        display: PillDisplay,
        reduce_motion: bool,
        now: Instant,
    ) -> DockLimit {
        if self.settled == Some(target) {
            return self.step(now).unwrap_or(target);
        }
        self.settled = Some(target);
        let to = resting_bottom(target, display);
        match (self.current(now), to) {
            (Some(from), Some(to)) if !reduce_motion && (from - to).abs() > 0.5 => {
                self.glide = Some(Glide::new(from, to, target, now));
                self.step(now).unwrap_or(target)
            }
            _ => {
                self.glide = None;
                self.shown = to;
                target
            }
        }
    }

    /// Advance an active glide; `None` when there is nothing to animate.
    pub(crate) fn step(&mut self, now: Instant) -> Option<DockLimit> {
        let glide = self.glide?;
        let (limit, done) = glide.at(now);
        if done {
            self.glide = None;
            self.shown = Some(glide.to);
        } else if let DockLimit::Above(y) = limit {
            self.shown = Some(y);
        }
        Some(limit)
    }

    pub(crate) fn animating(&self) -> bool {
        self.glide.is_some()
    }

    fn current(&self, now: Instant) -> Option<f64> {
        match self.glide {
            Some(glide) => match glide.at(now) {
                (DockLimit::Above(y), false) => Some(y),
                _ => Some(glide.to),
            },
            None => self.shown,
        }
    }
}

/// What the worker learns from the overlay each tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct WatchContext {
    pub(crate) display: PillDisplay,
    /// The pill is on screen (recording or processing), so motion matters.
    pub(crate) visible: bool,
}

fn next_interval(context: WatchContext, pointer_y: Option<f64>, limit: DockLimit) -> Duration {
    let near_bottom = pointer_y.is_some_and(|y| y >= context.display.bottom - WAKE_BAND);
    if context.visible && (near_bottom || matches!(limit, DockLimit::Above(_))) {
        FAST_INTERVAL
    } else {
        IDLE_INTERVAL
    }
}

/// Start the Dock worker. `context` returns `None` while the pill placement is
/// off; `reposition` moves the pill after the published limit changes.
#[cfg(target_os = "macos")]
pub(crate) fn start(
    context: impl Fn() -> Option<WatchContext> + Send + 'static,
    reposition: impl Fn() + Send + 'static,
) {
    let spawned = std::thread::Builder::new()
        .name("murmur-dock-watch".to_string())
        .spawn(move || {
            let mut sampler = macos::DockSampler::default();
            let mut tracker = DockTracker::default();
            let mut animator = LimitAnimator::default();
            loop {
                let Some(context) = context() else {
                    publish(DockLimit::Unknown);
                    sampler = macos::DockSampler::default();
                    tracker = DockTracker::default();
                    animator = LimitAnimator::default();
                    std::thread::sleep(IDLE_INTERVAL);
                    continue;
                };
                let reading = objc2::rc::autoreleasepool(|_| sampler.sample(context.display));
                let target = tracker.observe(reading);
                let now = Instant::now();
                // A hidden pill jumps: nobody sees it, and it must already be
                // in place when it appears.
                let reduce = !context.visible || macos::reduce_motion();
                if publish(animator.retarget(target, context.display, reduce, now)) {
                    reposition();
                }
                let interval = next_interval(context, macos::pointer_y(), target);
                let deadline = Instant::now() + interval;
                // Glide frames run between samples; a new sample retargets.
                while animator.animating() && Instant::now() < deadline {
                    std::thread::sleep(FRAME_INTERVAL);
                    if animator.step(Instant::now()).is_some_and(publish) {
                        reposition();
                    }
                }
                if let Some(rest) = deadline.checked_duration_since(Instant::now()) {
                    std::thread::sleep(rest);
                }
            }
        });
    if let Err(error) = spawned {
        tracing::warn!(target: "system", "dock watch worker failed to start: {}", error);
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::{classify, DockReading, PillDisplay, Rect, MAX_ITEMS};
    use core_foundation::base::TCFType;
    use core_foundation::string::{CFString, CFStringRef};
    use std::ffi::c_void;

    type AXUIElementRef = *const c_void;
    type CFTypeRef = *const c_void;
    type CFIndex = isize;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
        fn AXUIElementCopyAttributeValue(
            element: AXUIElementRef,
            attribute: CFTypeRef,
            value: *mut CFTypeRef,
        ) -> i32;
        fn AXUIElementSetMessagingTimeout(element: AXUIElementRef, timeout: f32) -> i32;
        fn AXValueGetType(value: CFTypeRef) -> u32;
        fn AXValueGetValue(value: CFTypeRef, value_type: u32, value_ptr: *mut c_void) -> bool;
        fn CFArrayGetCount(array: CFTypeRef) -> CFIndex;
        fn CFArrayGetValueAtIndex(array: CFTypeRef, index: CFIndex) -> CFTypeRef;
        fn CFArrayGetTypeID() -> usize;
        fn CFStringGetTypeID() -> usize;
        fn CFGetTypeID(value: CFTypeRef) -> usize;
        fn CFRetain(value: CFTypeRef) -> CFTypeRef;
        fn CFRelease(value: CFTypeRef);
    }

    const AX_SUCCESS: i32 = 0;
    const AX_TIMEOUT_SECONDS: f32 = 0.050;
    // kAXValueCGPointType / kAXValueCGSizeType (HIServices AXValue.h).
    const AX_VALUE_CGPOINT_TYPE: u32 = 1;
    const AX_VALUE_CGSIZE_TYPE: u32 = 2;

    /// Owned CF reference released on drop.
    struct Owned(CFTypeRef);
    // SAFETY: CF/AX references are thread-safe to retain, release and message;
    // the sampler lives on one worker thread.
    unsafe impl Send for Owned {}
    impl Drop for Owned {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CFRelease(self.0) };
            }
        }
    }

    fn copy(element: CFTypeRef, name: &'static str) -> Option<Owned> {
        let name = CFString::from_static_string(name);
        let mut value: CFTypeRef = std::ptr::null();
        // SAFETY: `element` is a live AX element and `name` a live CFString;
        // the copy rule hands us ownership of a non-null result.
        let status = unsafe {
            AXUIElementCopyAttributeValue(element, name.as_concrete_TypeRef().cast(), &mut value)
        };
        let value = Owned(value);
        (status == AX_SUCCESS && !value.0.is_null()).then_some(value)
    }

    fn decode<T: Default>(value: &Owned, kind: u32) -> Option<T> {
        let mut out = T::default();
        // SAFETY: the type tag is checked first, and `T` is the matching
        // repr(C) CoreGraphics value for that tag.
        unsafe {
            (AXValueGetType(value.0) == kind
                && AXValueGetValue(value.0, kind, &mut out as *mut T as *mut c_void))
            .then_some(out)
        }
    }

    #[repr(C)]
    #[derive(Default)]
    struct Pair(f64, f64);

    fn frame(element: CFTypeRef) -> Option<Rect> {
        let Pair(x, y) = decode(&copy(element, "AXPosition")?, AX_VALUE_CGPOINT_TYPE)?;
        let Pair(width, height) = decode(&copy(element, "AXSize")?, AX_VALUE_CGSIZE_TYPE)?;
        Some(Rect {
            x,
            y,
            width,
            height,
        })
    }

    /// Retained elements of an `AXChildren` array, at most `MAX_ITEMS`.
    fn children(element: CFTypeRef) -> Option<Vec<Owned>> {
        let array = copy(element, "AXChildren")?;
        // SAFETY: the array's type is checked before it is read; each element
        // is retained before the array is released.
        unsafe {
            if CFGetTypeID(array.0) != CFArrayGetTypeID() {
                return None;
            }
            let count = CFArrayGetCount(array.0).clamp(0, MAX_ITEMS as CFIndex);
            Some(
                (0..count)
                    .map(|index| CFArrayGetValueAtIndex(array.0, index))
                    .filter(|child| !child.is_null())
                    .map(|child| {
                        let child = Owned(CFRetain(child));
                        AXUIElementSetMessagingTimeout(child.0, AX_TIMEOUT_SECONDS);
                        child
                    })
                    .collect(),
            )
        }
    }

    fn role_is_list(element: CFTypeRef) -> bool {
        copy(element, "AXRole").is_some_and(|role| {
            // SAFETY: the type is checked before wrapping under the get rule.
            unsafe {
                CFGetTypeID(role.0) == CFStringGetTypeID()
                    && CFString::wrap_under_get_rule(role.0 as CFStringRef) == "AXList"
            }
        })
    }

    fn dock_pid() -> Option<i32> {
        use objc2_app_kit::NSRunningApplication;
        use objc2_foundation::NSString;
        let apps = NSRunningApplication::runningApplicationsWithBundleIdentifier(
            &NSString::from_str("com.apple.dock"),
        );
        let pid = apps.firstObject()?.processIdentifier();
        (pid > 0).then_some(pid)
    }

    /// Cached Dock application and list elements, rebound after any failure
    /// so a Dock restart is picked up on the next sample.
    #[derive(Default)]
    pub(super) struct DockSampler {
        list: Option<(Owned, Owned)>,
    }

    impl DockSampler {
        fn bind() -> Option<(Owned, Owned)> {
            let pid = dock_pid()?;
            // SAFETY: creates an owned AX element for a live process id.
            let app = Owned(unsafe { AXUIElementCreateApplication(pid) });
            if app.0.is_null()
                || unsafe { AXUIElementSetMessagingTimeout(app.0, AX_TIMEOUT_SECONDS) }
                    != AX_SUCCESS
            {
                return None;
            }
            let list = children(app.0)?
                .into_iter()
                .find(|child| role_is_list(child.0))?;
            Some((app, list))
        }

        pub(super) fn sample(&mut self, display: PillDisplay) -> DockReading {
            if self.list.is_none() {
                self.list = Self::bind();
            }
            let Some((_, list)) = &self.list else {
                return DockReading::Unavailable;
            };
            let Some(list_frame) = frame(list.0) else {
                self.list = None;
                return DockReading::Unavailable;
            };
            // Item frames only matter while the Dock shows; skip the IPC
            // otherwise.
            let mut items = Vec::new();
            if let reading @ DockReading::Exposed { .. } = classify(list_frame, &[], display) {
                let Some(children) = children(list.0) else {
                    self.list = None;
                    return DockReading::Unavailable;
                };
                for child in &children {
                    match frame(child.0) {
                        Some(rect) => items.push(rect),
                        // A failed item stops the sample rather than
                        // accumulating per-item timeouts.
                        None => return reading,
                    }
                }
            }
            classify(list_frame, &items, display)
        }
    }

    pub(super) fn pointer_y() -> Option<f64> {
        use core_graphics::event::CGEvent;
        use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
        let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState).ok()?;
        Some(CGEvent::new(source).ok()?.location().y)
    }

    pub(super) fn reduce_motion() -> bool {
        objc2_app_kit::NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Manual probe against the real Dock: `cargo test --lib live_dock_probe
    /// -- --ignored --nocapture` while revealing the Dock. Needs Accessibility.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore]
    fn live_dock_probe() {
        let (width, height) = {
            let bounds = core_graphics::display::CGDisplay::main().bounds();
            (bounds.size.width, bounds.size.height)
        };
        let display = PillDisplay {
            left: 0.0,
            top: 0.0,
            right: width,
            bottom: height,
            work_bottom: height - 4.0,
        };
        let mut sampler = macos::DockSampler::default();
        let mut tracker = DockTracker::default();
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(6) {
            let reading = objc2::rc::autoreleasepool(|_| sampler.sample(display));
            println!(
                "{:>5}ms {:?} -> {:?}",
                started.elapsed().as_millis(),
                reading,
                tracker.observe(reading)
            );
            std::thread::sleep(FAST_INTERVAL);
        }
    }

    // Measured on a 1920×1080 display: hidden, then revealed and magnified.
    const DISPLAY: PillDisplay = PillDisplay {
        left: 0.0,
        top: 0.0,
        right: 1920.0,
        bottom: 1080.0,
        work_bottom: 1076.0,
    };
    const HIDDEN: Rect = Rect {
        x: 367.0,
        y: 1080.0,
        width: 1186.0,
        height: 103.0,
    };
    const REVEALED: Rect = Rect {
        x: 280.0,
        y: 967.0,
        width: 1360.0,
        height: 103.0,
    };

    fn item(y: f64, height: f64) -> Rect {
        Rect {
            x: 900.0,
            y,
            width: height,
            height,
        }
    }

    #[test]
    fn classifies_hidden_revealed_and_foreign_docks() {
        assert_eq!(classify(HIDDEN, &[], DISPLAY), DockReading::Hidden);
        // Unmagnified items sit inside the list frame.
        assert_eq!(
            classify(REVEALED, &[item(975.0, 83.0)], DISPLAY),
            DockReading::Exposed {
                top: 967.0 - LABEL_ALLOWANCE
            }
        );
        // A magnified icon overflows the list: it sets the clearance.
        assert_eq!(
            classify(REVEALED, &[item(975.0, 83.0), item(927.1, 147.9)], DISPLAY),
            DockReading::Exposed {
                top: 927.1 - LABEL_ALLOWANCE
            }
        );
        // Mid-reveal the list is partly on screen.
        assert!(matches!(
            classify(
                Rect {
                    y: 1010.0,
                    ..REVEALED
                },
                &[],
                DISPLAY
            ),
            DockReading::Exposed { .. }
        ));
        // Side Dock and a Dock on a display to the right.
        let side = Rect {
            x: 0.0,
            y: 200.0,
            width: 90.0,
            height: 700.0,
        };
        assert_eq!(classify(side, &[], DISPLAY), DockReading::Elsewhere);
        let other = Rect {
            x: 2200.0,
            ..REVEALED
        };
        assert_eq!(classify(other, &[], DISPLAY), DockReading::Elsewhere);
        let broken = Rect {
            y: f64::NAN,
            ..REVEALED
        };
        assert_eq!(classify(broken, &[], DISPLAY), DockReading::Unavailable);
    }

    #[test]
    fn tracker_holds_the_highest_clearance_until_the_dock_is_confirmed_hidden() {
        let mut tracker = DockTracker::default();
        assert_eq!(tracker.observe(DockReading::Hidden), DockLimit::Clear);
        assert_eq!(
            tracker.observe(DockReading::Exposed { top: 900.0 }),
            DockLimit::Above(900.0)
        );
        // Magnification settling lower must not drop the pill.
        assert_eq!(
            tracker.observe(DockReading::Exposed { top: 930.0 }),
            DockLimit::Above(900.0)
        );
        assert_eq!(
            tracker.observe(DockReading::Exposed { top: 880.0 }),
            DockLimit::Above(880.0)
        );
        // One hidden sample is not enough; the second confirms.
        assert_eq!(
            tracker.observe(DockReading::Hidden),
            DockLimit::Above(880.0)
        );
        assert_eq!(tracker.observe(DockReading::Hidden), DockLimit::Clear);
        // The next reveal starts a fresh envelope.
        assert_eq!(
            tracker.observe(DockReading::Exposed { top: 950.0 }),
            DockLimit::Above(950.0)
        );
        assert_eq!(tracker.observe(DockReading::Elsewhere), DockLimit::Clear);
    }

    #[test]
    fn tracker_rides_out_transient_failures_then_falls_back() {
        let mut tracker = DockTracker::default();
        tracker.observe(DockReading::Exposed { top: 900.0 });
        assert_eq!(
            tracker.observe(DockReading::Unavailable),
            DockLimit::Above(900.0)
        );
        assert_eq!(
            tracker.observe(DockReading::Unavailable),
            DockLimit::Above(900.0)
        );
        assert_eq!(
            tracker.observe(DockReading::Unavailable),
            DockLimit::Unknown
        );
        // Never sampled successfully: fall back at once.
        assert_eq!(
            DockTracker::default().observe(DockReading::Unavailable),
            DockLimit::Unknown
        );
    }

    #[test]
    fn animator_glides_up_fast_and_down_gently() {
        let start = Instant::now();
        let mut animator = LimitAnimator::default();
        // First placement jumps.
        assert_eq!(
            animator.retarget(DockLimit::Clear, DISPLAY, false, start),
            DockLimit::Clear
        );
        assert!(!animator.animating());
        // Rise: starts at the resting bottom and reaches the Dock in RISE.
        let first = animator.retarget(DockLimit::Above(900.0), DISPLAY, false, start);
        assert_eq!(first, DockLimit::Above(1076.0));
        let DockLimit::Above(mid) = animator.step(start + RISE / 2).unwrap() else {
            panic!("mid-glide limit");
        };
        assert!(mid < 1076.0 && mid > 900.0, "{mid}");
        assert_eq!(animator.step(start + RISE), Some(DockLimit::Above(900.0)));
        assert!(!animator.animating());
        // Fall: lands on Clear itself, not a numeric copy of the work area.
        let later = start + Duration::from_secs(1);
        animator.retarget(DockLimit::Clear, DISPLAY, false, later);
        assert!(animator
            .step(later + RISE)
            .is_some_and(|l| l != DockLimit::Clear));
        assert_eq!(animator.step(later + FALL), Some(DockLimit::Clear));
        // Reduced motion and the static fallback jump.
        assert_eq!(
            animator.retarget(DockLimit::Above(900.0), DISPLAY, true, later),
            DockLimit::Above(900.0)
        );
        assert_eq!(
            animator.retarget(DockLimit::Unknown, DISPLAY, false, later),
            DockLimit::Unknown
        );
        assert!(!animator.animating());
    }

    #[test]
    fn animator_retargets_mid_glide_from_where_the_pill_is() {
        let start = Instant::now();
        let mut animator = LimitAnimator::default();
        animator.retarget(DockLimit::Clear, DISPLAY, false, start);
        animator.retarget(DockLimit::Above(900.0), DISPLAY, false, start);
        let DockLimit::Above(mid) = animator.step(start + RISE / 2).unwrap() else {
            panic!("mid-glide limit");
        };
        // The Dock hides again mid-rise: the fall starts from the pill.
        assert_eq!(
            animator.retarget(DockLimit::Clear, DISPLAY, false, start + RISE / 2),
            DockLimit::Above(mid)
        );
    }

    #[test]
    fn sampling_is_fast_only_while_the_pill_shows_and_the_dock_could_matter() {
        let visible = WatchContext {
            display: DISPLAY,
            visible: true,
        };
        let hidden = WatchContext {
            visible: false,
            ..visible
        };
        assert_eq!(
            next_interval(visible, Some(1000.0), DockLimit::Clear),
            FAST_INTERVAL
        );
        assert_eq!(
            next_interval(visible, Some(200.0), DockLimit::Above(900.0)),
            FAST_INTERVAL
        );
        assert_eq!(
            next_interval(visible, Some(200.0), DockLimit::Clear),
            IDLE_INTERVAL
        );
        assert_eq!(
            next_interval(hidden, Some(1070.0), DockLimit::Above(900.0)),
            IDLE_INTERVAL
        );
    }
}
