# GPUI Phase 0 exit: GPUI, native parity first, one stacked branch

Date: 2026-11-05  
Status: active  
Issues: #983, #978, #979, #980, #981, #982, #1008, #1012  
Plan: [GPUI rewrite plan](../draft/gpui-rewrite-plan.md), revision 13

## Decision

Murmur's UI moves to GPUI at Zed rev
`654efae441bc8eb117d0042bb28cd7cf3570bdc9` with Rust 1.98.1. We are not
taking the SwiftUI/AppKit fallback.

The migration route is **native parity first**, the fourth row of the plan's
route table. Every window is built natively in a GPUI build developed beside
today's Tauri app. Nothing lands on `main` during the rewrite. Each plan issue
becomes one PR, stacked on the previous one. The top of the stack is built
and tested by hand on George's MacBook. Only a branch that works 100% is ever
merged.

Because nothing ships part-way, the hybrid machinery that exists only to keep
`main` releasable mid-migration is out of scope:

- 0c (#981), embedding GPUI in the Tauri process;
- 0d (#982), React inside a GPUI window;
- 2.2 (#995), the React bridge;
- 1.7 (#991), the frontend host adapter;
- 1.9 (#993), the lazy Tauri diagnostics window.

The order is Phase 1 (core extraction), then Phase 3 (screens), then Phase 2
(packaging, host replacements, identity and updates), then Phase 4 (removal).

## Stage A results

All prototypes are scratch code on the Mac mini under `~/Builds/gpui-phase0`,
with per-criterion reports.

| Prototype | Result |
|---|---|
| 0e build, CI, quit | Pass. Frozen pinned build: 55 s cold locally, about 4 min on hosted `macos-latest` (macOS 26) and `xcode-27` runners. Offscreen Metal goldens are identical across runs. Menu Quit, ⌘Q, Dock Quit (real HID click), the plain quit Apple event, simulated logout, and relaunch each held a 6 s teardown with the UI updating. GPUI quit handlers start 2.8–7.6 ms after the reply. The quit hook is 78 lines. Rust 1.96.0 fails (`Option::map_or_default`). |
| 0a overlay | Focus pass: 21 click-then-type trials, never key. Placement pass on all Spaces, over full-screen apps, across Dock changes, with Live Words click-through. HUD material, Reduce Transparency, and Reduce Motion pass live. Idle 25.7 MiB. Renders at GPUI's 30 fps inactive throttle; CPU 7.5% of one core while recording. A WindowServer capture shows 33.3 ms median and 50 ms p95 frame intervals. |
| 0b Settings and History | Keyboard, AX roles, and text editing pass. The app-level `AXFocusedUIElement` failure is fixed by an 89-line patch (hook, input, AccessKit), verified through the AX API. Idle 44.7 MiB; the History scroll peak is 97.3 MiB, mostly transient graphics-driver memory, settling back to 44.1 MiB. |

Not verified on the Mac mini, and left for George's MacBook pass:

- VoiceOver speech, Japanese IME, real dictation, and Low Power Mode;
- the physical notch, a second display, and Retina (2×) memory;
- a matched baseline of today's overlay.

## Frozen numbers

These numbers are provisional until the MacBook pass measures them on Retina.

- Overlay: at most 60 MiB idle. CPU while recording at most 10% of one core.
  Frame interval p95 at most 50 ms at the 30 fps throttle.
- Main window (Settings and History): at most 90 MiB idle, and at most
  130 MiB during a History-scroll peak.
- Whole app idle, core plus UI: at most 200 MiB. Today it is about 490 MB.
- Supported macOS: 14 through 27.

## Controls

Use `gpui-component` (Apache-2.0), ported to the pinned rev. The port was
proven in scratch: 21 errors fixed by dependency rewiring and one macro line.
On top of it go Murmur's accessibility fixes for focused-element reporting and
text selection attributes. Never use Zed's GPL crates (`ui`, `editor`,
`ui_input`).

## Main-window port strategy

Port section by section in the order of plan 3.5: History, Settings,
onboarding and home, meetings, then the rest. Every section must reach full
parity before the window flips. There is no mixed native and webview main
window.
