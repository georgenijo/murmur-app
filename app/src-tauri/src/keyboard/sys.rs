// Manual FFI declarations for CGEventTap (CoreGraphics) and CFRunLoop / CFMachPort
// (CoreFoundation). Using hand-rolled FFI keeps CGEventTap* coverage stable
// across macOS SDK versions and avoids the partial coverage risk of the
// `core-graphics` crate. This mirrors the pattern in injector.rs for AX APIs.

#![allow(dead_code)]

use std::os::raw::c_void;

// ── Types ─────────────────────────────────────────────────────────────────────

pub type CGEventTapProxy = *mut c_void;
pub type CGEventRef = *mut c_void;
pub type CGEventType = u32;
pub type CGEventMask = u64;
pub type CGEventFlags = u64;
pub type CFMachPortRef = *mut c_void;
pub type CFRunLoopRef = *mut c_void;
pub type CFRunLoopSourceRef = *mut c_void;
pub type CFAllocatorRef = *mut c_void;

pub type CGEventTapCallBack = unsafe extern "C" fn(
    proxy: CGEventTapProxy,
    etype: CGEventType,
    event: CGEventRef,
    user_info: *mut c_void,
) -> CGEventRef;

// ── CGEventTap location (CGEventTapLocation) ─────────────────────────────────

/// Events injected at the HID level, before session and application routing.
/// Delivered regardless of which app (if any) is focused.
pub const K_CG_HID_EVENT_TAP: u32 = 0;
/// Events at the session level; may not be delivered when no window is focused.
pub const K_CG_SESSION_EVENT_TAP: u32 = 1;

// ── CGEventTap placement (CGEventTapPlacement) ────────────────────────────────

pub const K_CG_HEAD_INSERT_EVENT_TAP: u32 = 0;

// ── CGEventTap options ────────────────────────────────────────────────────────

/// Passive tap — not subject to macOS event-tap timeout penalty.
pub const K_CG_EVENT_TAP_OPTION_LISTEN_ONLY: u32 = 1;

// ── CGEventType constants ─────────────────────────────────────────────────────

pub const K_CG_EVENT_KEY_DOWN: u32 = 10;
pub const K_CG_EVENT_KEY_UP: u32 = 11;
/// Fires when a modifier key (Shift, Option, Control, Command) is pressed or
/// released. Modifier keys do NOT generate KeyDown/KeyUp events on macOS.
pub const K_CG_EVENT_FLAGS_CHANGED: u32 = 12;
/// Synthesized event delivered to the tap when macOS disables it due to timeout.
pub const K_CG_EVENT_TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFFFFFE;
/// Synthesized event delivered when the tap is disabled by user input throttling.
pub const K_CG_EVENT_TAP_DISABLED_BY_USER_INPUT: u32 = 0xFFFFFFFF;

// ── CGEventField constants ────────────────────────────────────────────────────

pub const K_CG_KEYBOARD_EVENT_KEYCODE: u32 = 9;

// ── Device-specific modifier flag masks (from IOKit/hid/IOLLEvent.h) ─────────
//
// These bits appear in CGEventFlags (returned by CGEventGetFlags) and identify
// which hand's modifier key was pressed, enabling Left/Right disambiguation
// for Shift, Option, Control, and Command.

pub const NX_DEVICELCTLKEYMASK: u64 = 0x0000_0001;
pub const NX_DEVICELSHIFTKEYMASK: u64 = 0x0000_0002;
pub const NX_DEVICERSHIFTKEYMASK: u64 = 0x0000_0004;
pub const NX_DEVICELCMDKEYMASK: u64 = 0x0000_0008;
pub const NX_DEVICERCMDKEYMASK: u64 = 0x0000_0010;
pub const NX_DEVICELALTKEYMASK: u64 = 0x0000_0020;
pub const NX_DEVICERALTKEYMASK: u64 = 0x0000_0040;
pub const NX_DEVICERCTLKEYMASK: u64 = 0x0000_2000;

// ── Helper ────────────────────────────────────────────────────────────────────

/// Compute the event mask bit for a given CGEventType value.
pub const fn cg_event_mask_bit(t: u32) -> u64 {
    1u64 << (t as u64)
}

// ── CoreGraphics FFI ──────────────────────────────────────────────────────────

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    pub fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events_of_interest: CGEventMask,
        callback: CGEventTapCallBack,
        user_info: *mut c_void,
    ) -> CFMachPortRef;

    pub fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);

    pub fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;

    pub fn CGEventGetFlags(event: CGEventRef) -> CGEventFlags;
}

// ── CoreFoundation FFI ────────────────────────────────────────────────────────

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    pub fn CFRetain(cf: *const c_void) -> *const c_void;
    pub fn CFRelease(cf: *const c_void);

    pub fn CFMachPortCreateRunLoopSource(
        allocator: CFAllocatorRef,
        port: CFMachPortRef,
        order: isize,
    ) -> CFRunLoopSourceRef;

    pub fn CFRunLoopGetCurrent() -> CFRunLoopRef;

    pub fn CFRunLoopAddSource(rl: CFRunLoopRef, source: CFRunLoopSourceRef, mode: *const c_void);

    pub fn CFRunLoopRun();

    pub static kCFRunLoopCommonModes: *const c_void;
}
