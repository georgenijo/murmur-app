//! Collision-safe placement of the collapsed floating overlay inside the
//! menu-bar band.
//!
//! The overlay window sits above the menu bar and takes mouse events, so it
//! hides and intercepts whatever menu or status item lies beneath it. Unlike a
//! physical notch, macOS does not reserve this space. The collapsed pill may
//! therefore enter the band only inside a gap that the frontmost application's
//! menu titles and the status items leave free. Occupancy that cannot be
//! determined (Accessibility failure, empty samples) or a band with no wide
//! enough gap yields `None`, and callers fall back to the standalone placement
//! below the menu bar. Only window/element frames are read, never titles or
//! content.

use std::time::{Duration, Instant};

/// A frame in global logical (top-left origin) coordinates, as reported by the
/// Accessibility API.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FrameRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// The menu-bar band of one display in global logical coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Band {
    pub left: f64,
    pub right: f64,
    pub top: f64,
    pub bottom: f64,
}

impl Band {
    fn is_valid(self) -> bool {
        [self.left, self.right, self.top, self.bottom]
            .iter()
            .all(|v| v.is_finite())
            && self.left < self.right
            && self.top < self.bottom
    }
}

/// Horizontal spans of `frames` that intersect `band`, clipped to its width.
/// `None` when the input is unusable (invalid band or non-finite frame), so a
/// caller can never mistake garbage for "nothing is occupied".
fn band_occupancy(band: Band, frames: &[FrameRect]) -> Option<Vec<(f64, f64)>> {
    if !band.is_valid() {
        return None;
    }
    let mut spans = Vec::new();
    for frame in frames {
        if ![frame.x, frame.y, frame.width, frame.height]
            .iter()
            .all(|v| v.is_finite())
        {
            return None;
        }
        if frame.width <= 0.0 || frame.height <= 0.0 {
            continue;
        }
        let (left, right) = (frame.x, frame.x + frame.width);
        let (top, bottom) = (frame.y, frame.y + frame.height);
        if bottom <= band.top || top >= band.bottom || right <= band.left || left >= band.right {
            continue;
        }
        spans.push((left.max(band.left), right.min(band.right)));
    }
    Some(spans)
}

/// Pick the centre of a `required_w`-wide slot inside `[left, right]` that
/// clears every occupied span by `margin`. The slot closest to the display
/// centre wins (exactly centred when the centre is free); ties go to the
/// leftmost. `None` when no gap is wide enough.
fn select_slot_center(
    left: f64,
    right: f64,
    occupied: &[(f64, f64)],
    margin: f64,
    required_w: f64,
) -> Option<f64> {
    if ![left, right, margin, required_w]
        .iter()
        .all(|v| v.is_finite())
        || left >= right
        || required_w <= 0.0
        || margin < 0.0
    {
        return None;
    }
    let mut spans: Vec<(f64, f64)> = occupied
        .iter()
        .map(|&(start, end)| ((start - margin).max(left), (end + margin).min(right)))
        .filter(|&(start, end)| start < end)
        .collect();
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));

    let preferred = (left + right) / 2.0;
    let half = required_w / 2.0;
    let mut best: Option<f64> = None;
    let mut consider = |gap_start: f64, gap_end: f64| {
        if gap_end - gap_start >= required_w {
            let center = preferred.clamp(gap_start + half, gap_end - half);
            if best.is_none_or(|b| (center - preferred).abs() < (b - preferred).abs()) {
                best = Some(center);
            }
        }
    };
    let mut cursor = left;
    for (start, end) in spans {
        consider(cursor, start);
        cursor = cursor.max(end);
    }
    consider(cursor, right);
    best
}

/// Resolve where the collapsed pill may sit in `band`, from the frontmost
/// application's menu-item frames and the status-item frames.
///
/// Either sample being unavailable or empty means occupancy is unknown (every
/// menu bar has an Apple menu and a clock), so the result is `None`.
///
/// `unanswered_status_apps` applications did not report their status items.
/// macOS packs status items into one run from the right edge of the bar, so
/// the seen items' positions already account for any unseen item to their
/// right; an unseen item can only extend the run leftward. Each such
/// application therefore reserves `UNANSWERED_STATUS_RESERVE` to the left of
/// the leftmost seen item.
pub(crate) fn resolve_band_center(
    band: Band,
    menu_items: Option<&[FrameRect]>,
    status_items: Option<&[FrameRect]>,
    unanswered_status_apps: usize,
    margin: f64,
    required_w: f64,
) -> Option<f64> {
    let menu_items = menu_items.filter(|frames| !frames.is_empty())?;
    let status_items = status_items.filter(|frames| !frames.is_empty())?;
    let mut occupied = band_occupancy(band, menu_items)?;
    let status_spans = band_occupancy(band, status_items)?;
    if unanswered_status_apps > 0 {
        let run_left = status_spans
            .iter()
            .map(|&(left, _)| left)
            .fold(band.right, f64::min);
        let reserve = unanswered_status_apps as f64 * UNANSWERED_STATUS_RESERVE;
        occupied.push(((run_left - reserve).max(band.left), run_left));
    }
    occupied.extend(status_spans);
    select_slot_center(band.left, band.right, &occupied, margin, required_w)
}

/// One application's status items (`AXExtrasMenuBar` children), as read.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ExtrasRead {
    /// The application reports no status items.
    NoItems,
    Frames(Vec<FrameRect>),
    /// The application did not answer (timeout, unsupported, busy).
    Failed,
}

/// Every status item that answered, plus how many applications did not.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct StatusItems {
    pub frames: Vec<FrameRect>,
    /// Applications whose status items, if any, are unseen.
    pub unanswered: usize,
}

/// Width assumed for the unseen status items of each application that did not
/// answer (see `resolve_band_center`).
const UNANSWERED_STATUS_RESERVE: f64 = 128.0;
/// Upper bound for one scan of every application. Measured on macOS 27 with 53
/// applications: ~0.6s for the first scan (Accessibility connections being
/// established), ~4ms afterwards.
const STATUS_SCAN_BUDGET: Duration = Duration::from_millis(1500);

/// Read the status items of every application in `process_ids` with `read`.
/// macOS 27 no longer exposes status items as `kCGStatusWindowLevel` windows,
/// so each application's `AXExtrasMenuBar` is read instead, every time:
/// cached absence cannot prove that an application has not added an item
/// since. `None` when the budget is spent.
pub(crate) fn collect_status_items(
    process_ids: impl IntoIterator<Item = i32>,
    mut read: impl FnMut(i32) -> ExtrasRead,
) -> Option<StatusItems> {
    let started = Instant::now();
    let mut items = StatusItems::default();
    for process_id in process_ids {
        if started.elapsed() > STATUS_SCAN_BUDGET {
            return None;
        }
        match read(process_id) {
            ExtrasRead::Frames(frames) => items.frames.extend(frames),
            ExtrasRead::NoItems => {}
            ExtrasRead::Failed => items.unanswered += 1,
        }
    }
    Some(items)
}

#[cfg(target_os = "macos")]
pub(crate) use native::{frontmost_menu_item_frames, status_items};

#[cfg(target_os = "macos")]
mod native {
    use super::{collect_status_items, ExtrasRead, FrameRect, StatusItems};
    use std::ffi::{c_char, c_void, CString};
    use std::time::{Duration, Instant};

    type AXUIElementRef = *const c_void;
    type CFTypeRef = *const c_void;
    type CFIndex = isize;

    #[repr(C)]
    #[derive(Default)]
    struct CGPoint {
        x: f64,
        y: f64,
    }

    #[repr(C)]
    #[derive(Default)]
    struct CGSize {
        width: f64,
        height: f64,
    }

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
        fn AXUIElementCopyAttributeValue(
            element: AXUIElementRef,
            attribute: CFTypeRef,
            value: *mut CFTypeRef,
        ) -> i32;
        fn AXUIElementSetMessagingTimeout(element: AXUIElementRef, timeout: f32) -> i32;
        fn AXValueGetValue(value: CFTypeRef, value_type: u32, value_ptr: *mut c_void) -> bool;
        fn CFStringCreateWithCString(
            allocator: CFTypeRef,
            string: *const c_char,
            encoding: u32,
        ) -> CFTypeRef;
        fn CFArrayGetCount(array: CFTypeRef) -> CFIndex;
        fn CFArrayGetValueAtIndex(array: CFTypeRef, index: CFIndex) -> CFTypeRef;
        fn CFGetTypeID(value: CFTypeRef) -> usize;
        fn CFArrayGetTypeID() -> usize;
        fn CFRelease(value: CFTypeRef);
    }

    struct CFGuard(CFTypeRef);
    impl Drop for CFGuard {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CFRelease(self.0) };
            }
        }
    }

    const AX_SUCCESS: i32 = 0;
    /// `kAXErrorAttributeUnsupported` / `kAXErrorNoValue`: the application has
    /// no status items, as opposed to failing to answer.
    const AX_ERROR_ATTRIBUTE_UNSUPPORTED: i32 = -25205;
    const AX_ERROR_NO_VALUE: i32 = -25212;
    const AX_VALUE_CG_POINT: u32 = 1;
    const AX_VALUE_CG_SIZE: u32 = 2;
    const UTF8_ENCODING: u32 = 0x0800_0100;
    /// Per-element Accessibility timeout. A hung application yields `None`
    /// (standalone fallback) rather than a long wait.
    const AX_QUERY_TIMEOUT_SECONDS: f32 = 0.05;
    /// Upper bound for the whole menu-bar read.
    const MENU_READ_BUDGET: Duration = Duration::from_millis(300);
    /// Menu bars hold a few dozen items at most; more means something is off.
    const MAX_MENU_BAR_ITEMS: CFIndex = 64;

    fn cf_string(name: &str) -> Option<CFGuard> {
        let name = CString::new(name).ok()?;
        let value =
            unsafe { CFStringCreateWithCString(std::ptr::null(), name.as_ptr(), UTF8_ENCODING) };
        (!value.is_null()).then(|| CFGuard(value))
    }

    fn copy_attribute(element: AXUIElementRef, name: &str) -> Option<CFGuard> {
        let attribute = cf_string(name)?;
        let mut value: CFTypeRef = std::ptr::null();
        let status = unsafe { AXUIElementCopyAttributeValue(element, attribute.0, &mut value) };
        if status != AX_SUCCESS || value.is_null() {
            if !value.is_null() {
                unsafe { CFRelease(value) };
            }
            return None;
        }
        Some(CFGuard(value))
    }

    fn item_frame(item: AXUIElementRef) -> Option<FrameRect> {
        if unsafe { AXUIElementSetMessagingTimeout(item, AX_QUERY_TIMEOUT_SECONDS) } != AX_SUCCESS {
            return None;
        }
        let position = copy_attribute(item, "AXPosition")?;
        let size = copy_attribute(item, "AXSize")?;
        let mut point = CGPoint::default();
        let mut extent = CGSize::default();
        let have_point = unsafe {
            AXValueGetValue(
                position.0,
                AX_VALUE_CG_POINT,
                (&mut point as *mut CGPoint).cast(),
            )
        };
        let have_size = unsafe {
            AXValueGetValue(
                size.0,
                AX_VALUE_CG_SIZE,
                (&mut extent as *mut CGSize).cast(),
            )
        };
        (have_point && have_size).then_some(FrameRect {
            x: point.x,
            y: point.y,
            width: extent.width,
            height: extent.height,
        })
    }

    /// Frames of the menu titles (Apple menu included) the given application
    /// shows in the menu bar. `None` on any failure or timeout.
    fn menu_bar_item_frames(process_id: i32) -> Option<Vec<FrameRect>> {
        let started = Instant::now();
        let application = unsafe { AXUIElementCreateApplication(process_id) };
        if application.is_null() {
            return None;
        }
        let application = CFGuard(application);
        if unsafe { AXUIElementSetMessagingTimeout(application.0, AX_QUERY_TIMEOUT_SECONDS) }
            != AX_SUCCESS
        {
            return None;
        }
        let menu_bar = copy_attribute(application.0, "AXMenuBar")?;
        let frames = children_frames(&menu_bar, started)?;
        (!frames.is_empty()).then_some(frames)
    }

    /// Frames of a menu bar's children. `None` on any failure, on more than
    /// `MAX_MENU_BAR_ITEMS` children, or once `MENU_READ_BUDGET` since
    /// `started` is spent.
    fn children_frames(bar: &CFGuard, started: Instant) -> Option<Vec<FrameRect>> {
        if unsafe { AXUIElementSetMessagingTimeout(bar.0, AX_QUERY_TIMEOUT_SECONDS) } != AX_SUCCESS
        {
            return None;
        }
        let children = copy_attribute(bar.0, "AXChildren")?;
        if unsafe { CFGetTypeID(children.0) != CFArrayGetTypeID() } {
            return None;
        }
        let count = unsafe { CFArrayGetCount(children.0) };
        if !(0..=MAX_MENU_BAR_ITEMS).contains(&count) {
            return None;
        }
        let mut frames = Vec::with_capacity(count as usize);
        for index in 0..count {
            if started.elapsed() > MENU_READ_BUDGET {
                return None;
            }
            // Borrowed from `children`; released with the array.
            let item = unsafe { CFArrayGetValueAtIndex(children.0, index) };
            if item.is_null() {
                return None;
            }
            frames.push(item_frame(item)?);
        }
        Some(frames)
    }

    /// Menu-title frames of the frontmost application. Blocks for the
    /// Accessibility round trips, so never call this on the main thread.
    pub(crate) fn frontmost_menu_item_frames() -> Option<Vec<FrameRect>> {
        use objc2_app_kit::NSWorkspace;
        let application = NSWorkspace::sharedWorkspace().frontmostApplication()?;
        menu_bar_item_frames(application.processIdentifier())
    }

    /// One application's status-item frames from its `AXExtrasMenuBar`.
    fn extras_frames(process_id: i32) -> ExtrasRead {
        let started = Instant::now();
        let application = unsafe { AXUIElementCreateApplication(process_id) };
        if application.is_null() {
            return ExtrasRead::Failed;
        }
        let application = CFGuard(application);
        if unsafe { AXUIElementSetMessagingTimeout(application.0, AX_QUERY_TIMEOUT_SECONDS) }
            != AX_SUCCESS
        {
            return ExtrasRead::Failed;
        }
        let Some(attribute) = cf_string("AXExtrasMenuBar") else {
            return ExtrasRead::Failed;
        };
        let mut value: CFTypeRef = std::ptr::null();
        let status =
            unsafe { AXUIElementCopyAttributeValue(application.0, attribute.0, &mut value) };
        if !value.is_null() && status != AX_SUCCESS {
            unsafe { CFRelease(value) };
        }
        match status {
            AX_SUCCESS if !value.is_null() => {
                let bar = CFGuard(value);
                children_frames(&bar, started).map_or(ExtrasRead::Failed, ExtrasRead::Frames)
            }
            AX_ERROR_NO_VALUE | AX_ERROR_ATTRIBUTE_UNSUPPORTED => ExtrasRead::NoItems,
            _ => ExtrasRead::Failed,
        }
    }

    /// Every status item (status items, Control Center, the clock) on every
    /// display, read through Accessibility. Hidden items report frames off the
    /// bar, which the band filter ignores. `None` when the sample is unknown.
    /// Blocks for Accessibility round trips, so never call this on the main
    /// thread.
    pub(crate) fn status_items() -> Option<StatusItems> {
        use objc2_app_kit::{NSApplicationActivationPolicy, NSWorkspace};
        let process_ids: Vec<i32> = objc2::rc::autoreleasepool(|_| {
            NSWorkspace::sharedWorkspace()
                .runningApplications()
                .iter()
                .filter(|application| {
                    // Background-only processes cannot show status items, and
                    // WebKit's helper processes never answer Accessibility.
                    application.activationPolicy() != NSApplicationActivationPolicy::Prohibited
                        && !application.bundleIdentifier().is_some_and(|identifier| {
                            identifier.to_string().starts_with("com.apple.WebKit.")
                        })
                })
                .map(|application| application.processIdentifier())
                .collect()
        });
        collect_status_items(process_ids, extras_frames)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MARGIN: f64 = 8.0;
    const WIDTH: f64 = 320.0;

    fn bar(left: f64, right: f64) -> Band {
        Band {
            left,
            right,
            top: 0.0,
            bottom: 24.0,
        }
    }

    fn item(x: f64, width: f64) -> FrameRect {
        FrameRect {
            x,
            y: 0.0,
            width,
            height: 24.0,
        }
    }

    fn slot(left: f64, right: f64, occupied: &[(f64, f64)]) -> Option<f64> {
        select_slot_center(left, right, occupied, MARGIN, WIDTH)
    }

    #[test]
    fn centre_free_centres_exactly() {
        assert_eq!(
            slot(0.0, 2560.0, &[(0.0, 600.0), (2200.0, 2560.0)]),
            Some(1280.0)
        );
        assert_eq!(slot(0.0, 2560.0, &[]), Some(1280.0));
    }

    #[test]
    fn occupied_centre_moves_to_the_nearest_gap() {
        // A long menu reaches past the centre; the nearest fit abuts it + margin.
        let center = slot(0.0, 2560.0, &[(0.0, 1400.0), (2300.0, 2560.0)]).unwrap();
        assert_eq!(center, 1400.0 + MARGIN + WIDTH / 2.0);
        // Occupied on the right of centre instead: mirror image.
        let center = slot(0.0, 2560.0, &[(0.0, 200.0), (1200.0, 2560.0)]).unwrap();
        assert_eq!(center, 1200.0 - MARGIN - WIDTH / 2.0);
    }

    #[test]
    fn picks_the_closest_of_several_gaps() {
        // Gaps 100..900 and 1500..2400: the right one is nearer the centre (1280).
        let occupied = [(0.0, 100.0), (900.0, 1500.0), (2400.0, 2560.0)];
        let center = slot(0.0, 2560.0, &occupied).unwrap();
        assert_eq!(center, 1500.0 + MARGIN + WIDTH / 2.0);
        // Gaps that are too narrow for the pill are ignored.
        let occupied = [(0.0, 1000.0), (1100.0, 1300.0), (1600.0, 2560.0)];
        assert_eq!(slot(0.0, 2560.0, &occupied), None);
        // A usable far gap wins over unusable near ones.
        let occupied = [(0.0, 1000.0), (1100.0, 1300.0), (1700.0, 2560.0)];
        assert_eq!(
            slot(0.0, 2560.0, &occupied),
            Some(1300.0 + MARGIN + WIDTH / 2.0)
        );
    }

    #[test]
    fn no_gap_wide_enough_returns_none() {
        assert_eq!(slot(0.0, 800.0, &[(0.0, 300.0), (400.0, 800.0)]), None);
        // Exactly one pill wide *without* margins is still too narrow.
        assert_eq!(slot(0.0, 1000.0, &[(0.0, 340.0), (660.0, 1000.0)]), None);
        // A gap of exactly required + both margins fits.
        let occupied = [(0.0, 340.0), (340.0 + 2.0 * MARGIN + WIDTH, 1000.0)];
        assert_eq!(
            slot(0.0, 1000.0, &occupied),
            Some(340.0 + MARGIN + WIDTH / 2.0)
        );
    }

    #[test]
    fn overlapping_and_unsorted_spans_merge() {
        let occupied = [
            (1200.0, 2560.0),
            (0.0, 500.0),
            (400.0, 900.0),
            (850.0, 1300.0),
        ];
        assert_eq!(slot(0.0, 2560.0, &occupied), None);
        let occupied = [(900.0, 1100.0), (0.0, 500.0), (400.0, 950.0)];
        let center = slot(0.0, 2560.0, &occupied).unwrap();
        assert_eq!(center, 1280.0, "centre is free of the merged 0..1100 run");
        let center = slot(0.0, 1500.0, &occupied).unwrap();
        assert_eq!(center, 1100.0 + MARGIN + WIDTH / 2.0);
    }

    #[test]
    fn invalid_inputs_never_produce_a_slot() {
        assert_eq!(select_slot_center(0.0, 0.0, &[], MARGIN, WIDTH), None);
        assert_eq!(
            select_slot_center(f64::NAN, 100.0, &[], MARGIN, WIDTH),
            None
        );
        assert_eq!(select_slot_center(0.0, 2560.0, &[], -1.0, WIDTH), None);
        assert_eq!(select_slot_center(0.0, 2560.0, &[], MARGIN, 0.0), None);
    }

    #[test]
    fn unknown_occupancy_falls_back() {
        let menu = [item(0.0, 400.0)];
        let status = [item(2300.0, 260.0)];
        let band = bar(0.0, 2560.0);
        assert!(resolve_band_center(band, Some(&menu), Some(&status), 0, MARGIN, WIDTH).is_some());
        // AX failure / timeout.
        assert_eq!(
            resolve_band_center(band, None, Some(&status), 0, MARGIN, WIDTH),
            None
        );
        // Window list failure.
        assert_eq!(
            resolve_band_center(band, Some(&menu), None, 0, MARGIN, WIDTH),
            None
        );
        // A menu bar always has an Apple menu and a clock: empty means unreadable.
        assert_eq!(
            resolve_band_center(band, Some(&[]), Some(&status), 0, MARGIN, WIDTH),
            None
        );
        assert_eq!(
            resolve_band_center(band, Some(&menu), Some(&[]), 0, MARGIN, WIDTH),
            None
        );
        // Garbage frames and degenerate bands are not "free space".
        let bad = [FrameRect {
            x: f64::NAN,
            ..item(0.0, 10.0)
        }];
        assert_eq!(
            resolve_band_center(band, Some(&bad), Some(&status), 0, MARGIN, WIDTH),
            None
        );
        let flat = Band {
            bottom: 0.0,
            ..band
        };
        assert_eq!(
            resolve_band_center(flat, Some(&menu), Some(&status), 0, MARGIN, WIDTH),
            None
        );
    }

    #[test]
    fn only_frames_inside_the_band_count() {
        let band = bar(0.0, 2560.0);
        let status = [item(2300.0, 260.0)];
        // A window under the bar, one above the display, and a zero-size item.
        let menu = [
            item(0.0, 300.0),
            FrameRect {
                y: 24.0,
                ..item(1000.0, 600.0)
            },
            FrameRect {
                y: -40.0,
                ..item(1000.0, 600.0)
            },
            item(1200.0, 0.0),
        ];
        assert_eq!(
            resolve_band_center(band, Some(&menu), Some(&status), 0, MARGIN, WIDTH),
            Some(1280.0)
        );
        // The same frame inside the band pushes the pill aside.
        let menu = [item(0.0, 300.0), item(1000.0, 600.0)];
        assert_eq!(
            resolve_band_center(band, Some(&menu), Some(&status), 0, MARGIN, WIDTH),
            Some(1000.0 - MARGIN - WIDTH / 2.0)
        );
    }

    #[test]
    fn status_items_and_menus_both_block() {
        let band = bar(0.0, 1440.0);
        let menu = [item(0.0, 420.0)];
        // Status cluster reaches far to the left on a narrow display: 428..972 fits.
        let status = [item(980.0, 460.0)];
        assert_eq!(
            resolve_band_center(band, Some(&menu), Some(&status), 0, MARGIN, WIDTH),
            Some(720.0)
        );
        // Starting at 700 leaves only 428..692: too narrow, so no slot.
        let status = [item(700.0, 740.0)];
        assert_eq!(
            resolve_band_center(band, Some(&menu), Some(&status), 0, MARGIN, WIDTH),
            None
        );
    }

    #[test]
    fn multi_display_origins_use_global_coordinates() {
        let status = |x| [item(x, 200.0)];
        // Display left of the primary (negative origin).
        let band = bar(-2560.0, 0.0);
        let menu = [item(-2560.0, 500.0)];
        assert_eq!(
            resolve_band_center(band, Some(&menu), Some(&status(-200.0)), 0, MARGIN, WIDTH),
            Some(-1280.0)
        );
        // Display right of the primary; items on the neighbouring display are ignored.
        let band = bar(2560.0, 5120.0);
        let menu = [item(2560.0, 500.0), item(100.0, 900.0)];
        assert_eq!(
            resolve_band_center(band, Some(&menu), Some(&status(4920.0)), 0, MARGIN, WIDTH),
            Some(3840.0)
        );
        // Centre blocked on a secondary display: nearest gap still inside it.
        let menu = [item(2560.0, 1400.0)];
        let center =
            resolve_band_center(band, Some(&menu), Some(&status(4920.0)), 0, MARGIN, WIDTH)
                .unwrap();
        assert_eq!(center, 2560.0 + 1400.0 + MARGIN + WIDTH / 2.0);
        assert!(center - WIDTH / 2.0 >= band.left && center + WIDTH / 2.0 <= band.right);
    }

    #[test]
    fn status_collection_counts_unanswered_apps_without_caching() {
        let clock = item(2400.0, 129.0);
        let read = |pid| match pid {
            1 => ExtrasRead::Frames(vec![clock]),
            2 => ExtrasRead::NoItems,
            _ => ExtrasRead::Failed,
        };
        assert_eq!(
            collect_status_items([1, 2, 3, 4], read),
            Some(StatusItems {
                frames: vec![clock],
                unanswered: 2,
            })
        );
    }

    #[test]
    fn unanswered_apps_reserve_space_left_of_the_status_run() {
        let band = bar(0.0, 2560.0);
        let menu = [item(0.0, 300.0)];
        let status = [item(2100.0, 460.0)];
        let resolve = |unanswered| {
            resolve_band_center(band, Some(&menu), Some(&status), unanswered, MARGIN, WIDTH)
        };
        // Plenty of room: the reserve stays clear of the centre.
        assert_eq!(resolve(0), Some(1280.0));
        assert_eq!(resolve(2), Some(1280.0));
        // The reserve pushes the slot left of an otherwise free centre.
        let shifted = resolve(6).unwrap();
        assert!(shifted + WIDTH / 2.0 + MARGIN <= 2100.0 - 6.0 * UNANSWERED_STATUS_RESERVE);
        // Enough unanswered applications leave no slot at all.
        assert_eq!(resolve(14), None);
    }
}
