# GPUI dictation preview contract (#1000)

Stack layer 11, based on `gpui/10-overlay` at
`033f1cc0b234ce0057065a7b3d0926f3ccea5fff` (PR #1028), part of #1011.
Draft only; do not merge independently. The [Phase 0 decision](../decisions/2026-11-05-gpui-phase-0-exit.md)
selects native parity first. This standalone screen does not switch the shipping
Tauri application; the native application host arrives in Phase 2 after Phase 3.
No React, bootstrap, hook or backend suite is retired.

## Ownership and host contract

`murmur_ui::preview::Preview` is an ephemeral presentation consumer, separate
from the overlay and the final transcript pipeline. It consumes existing core
`dictation::Status` and the shared `overlay_geometry` fitting function; it
reuses murmur-ui palettes, preferences, motion and the overlay's native panel
transaction. It has no capture, inference, settings writer, delivery, clipboard,
History, statistics, file export or diagnostic-text dependency. Transcript-bearing
models/events deliberately implement neither Debug nor serialization.

A native host creates an initially hidden, nonfocusing `WindowKind::PopUp`,
wraps Preview in `gpui_base::Root`, invokes `advance` after window creation and
`follow` once, and supplies the following adapters:

1. Subscribe to committed consent, generation, status, cancellation, partial
   and overlay anchor changes **before** the initial reads. Feed revisioned
   `LifecycleSnapshot`
   values with the current generation/status at attachment. Capture
   `Model::lifecycle_epoch()` before the asynchronous initial read and return
   `HydrateLifecycle { snapshot, observed_epoch }`; notifications use
   `Lifecycle(snapshot)`. Revisions must be positive and strictly increasing
   across reads and notifications, including same-generation status changes.
   A stale epoch rejects even a newer revision; a current epoch does not admit
   duplicate/older revisions. Subscribe and drain queued notifications before
   capturing the epoch, and retry rejected reads with a fresh epoch/revision.
   The initial model fails
   closed until durable consent and Recording ownership have been observed.
   Use the existing version-7 settings migration/default and owned-field writer;
   no duplicate settings parser or persistence owner is introduced here.
2. For each committed consent notification publish `ConsentChanged` immediately;
   `opted_out=true` clears before any asynchronous read. Capture
   `Model::consent_epoch()` after that change and tag the durable read result
   with it. Invalid/absent/unreadable settings produce `enabled=false`; a true
   notification or WebKit cache is never permission to enable. Serialize the
   epoch capture/read initiation in the UI turn, and deliver only typed results.
3. Publish worker results into the bounded `Inbox`. Rejected lifecycle messages
   must be retried; do not silently drop them or format their payloads. Only
   consecutive partials for the same generation coalesce. A lifecycle/consent
   boundary is never reordered. Observe lifecycle while disabled. Existing
   ticker cadence, single-flight decode, backend support, cancellation and
   generation checks remain with their existing core service owners.
4. Send today's whole-string decoder output as `Partial`: every word remains
   tentative. `AuthoritativePartial` is available only for a producer that
   actually supplies a committed boundary (currently synthetic fixtures).
   Never infer a stable prefix. UTF-16 input and visible-tail bounds match the
   React contracts, without splitting Unicode scalar values.
5. Implement `Host::is_current` as a synchronous main-thread read of the current
   generation, Recording status and cancellation state. The screen checks it
   at receive, after deferred first-frame work, and immediately before native
   presentation. It does not await a worker while mutating NSWindow. Ticket
   revisions prevent a stale show failure/callback from clearing successor text.
6. Forward the overlay's **acknowledged collapsed frame**, `above` and `lift`
   from `Host::preview_anchor`, together with the selected menu-bar display's
   logical work area, in revisioned
   `AnchorSnapshot` values through `Event::Anchor`. Capture
   `Model::anchor_epoch()` for the initial asynchronous read, then return
   `HydrateAnchor { snapshot, observed_epoch }`. The independent anchor stream
   uses the same positive, strictly increasing revision and epoch rules as
   lifecycle hydration. Hover notifications invalidate an outstanding anchor
   read; lifecycle/cancel/detach notifications invalidate lifecycle reads, and
   detach also invalidates anchor reads. Connect the existing display/Dock
   watcher; do not infer geometry from the main window's display. Register the
   preview before requesting the current anchor, so a missed hover event cannot
   strand its lift. Feed `Detached` on display loss/host detach. Primary-display
   polling in the overlay does not replace this applied-anchor contract.
7. When replacing the legacy window, consume its retained WKWebView with
   `murmur_webkit::PreviewRetirement::destroy` on main and require
   `remaining_webviews == 0`. It reuses the overlay's actual weak-deallocation
   barrier. Release every other owner. Hide/remove-from-superview is insufficient.
   Website-data-store retirement remains a separate final-host obligation.

The native window reserves 64pt of transparent hover room. A larger explicit
lift (up to the existing 136pt suggestion height) expands the reserve; that
reserve is retained through hover-off, so shrinking it cannot snap the card.
The shared fit clamps the window to the work area, with a 6pt gap below the
notch or above the pill. Only the card moves on hover. No app content, system
AX client, capture agent or pasteboard is read to position it.

Geometry and visibility change only in the main-thread update transaction;
render only refits the HUD clip to the painted card. Panel setup is applied once
per native instance so opacity/style setters cannot reenter a frame callback.
The first nonempty card is explicitly drawn and presented to its hidden backing
layer in a deferred main-thread effect cycle before generation-bound admission
orders it onto the screen. This does not wait for the macOS display link, which
is stopped while the native window is ordered out. Both the paint turn and
admission drain queued events and recheck the current host owner. Direct
`receive` drains earlier inbox entries before applying its own event.
Subsequent partials replace the visible
card without hiding it. Empty, stopped, cancelled, opted-out, detached or rejected
presentations clear the text allocation and remove AX content. Dropping the view closes and empties its inbox; retained producer clones reject
late events (`is_closed` is terminal). The host must also stop its subscriptions.
This screen never retains a
history of partials. Clearing an allocation is not a claim of secure RAM erasure.

## Parity and test map

The current contracts are [Live preview](../features/transcription.md#live-preview)
and [Live words](../features/overlay.md#live-words); matrix rows are O08, O09
and O10 in the [feature matrix](gpui-feature-matrix.md). Tests below live in
`app/gpui/src/preview/tests.rs` unless specified. All strings are synthetic.

| Current behavior / matrix row | Implementation | Automated coverage | Remaining native acceptance |
|---|---|---|---|
| O08 supported backend cadence, single-flight and final-only authority | Existing core/ticker owners; display-only events | Existing `recording_preview_tests.rs` and `useDictationPartial.test.tsx` retained | Connect real service; supported/unsupported models, delayed stop/cancel and exactly-once final delivery |
| O10 durable consent; default/v7 migration; true event cannot enable; failed reads close | Consent epoch and committed settings adapter contract; no cache read | `consent_is_durable_latest_read_only_and_opt_out_clears_before_reply`; existing settings/bootstrap suites retained | Host durable-settings adapter and upgrade/opt-out/relaunch flow |
| O10 stop/processing/recovery/cancel/new generation clears; disabled listeners preserve floor | `Model::event`, monotonic generation and stopped-through floor | `disabled_lifecycle_floor_survives_reenable_and_status_reordering`, `cancellation_and_new_generations_reject_old_text_and_presentation_callbacks`, `newer_partial_can_precede_generation_notice_but_not_stop_or_cancel_floor` | Integrated lifecycle ordering and recording configuration owner |
| O09 empty/silent recordings show no card; paint before show; main-turn show rechecks exact owner; rejection clears | Deferred first frame, revision ticket, `Host::is_current`, native transaction | `queued_show_rechecks_host_ownership_and_layout_shrink_wraps`; model ticket assertions | Real AppKit empty-flash, show failure and queued stop/new recording races |
| O09 trim/320 UTF-16 tail, whole-word boundary; 4096 UTF-16 payload bound | `visible_partial`, bounded Inbox and model | `empty_oversized_and_unicode_inputs_stay_bounded_and_current`, `ordered_bounded_inbox_never_coalesces_across_privacy_barriers` | Long multilingual text with real system shaping |
| O09 shrink-wrap short text; at most three lines; newest lines visible; no recording dot/control/title | System-font shaped card size; bottom-aligned overflow clip; no input handlers | Layout test and short/long/unbroken/Unicode goldens; actual AX test rejects activation | Retina text clipping, narrow work area, line endings and RTL |
| O09 wholly tentative strings; explicit stable-only/combined presentation | `Partial` vs `AuthoritativePartial`, styled spans | `stable_boundary_is_only_accepted_from_explicit_authority_and_cleared_with_content`; stable golden | Validate production adapter never invents a boundary |
| O09 menu-bar display/notch/pretend-notch/pill/nonzero origins/clamp/6pt gap | Existing `dictation_preview_fit_for_frame`; supplied applied anchor | `anchor_uses_shared_geometry_keeps_window_fixed_during_lift_and_clamps`; 32 inherited core geometry tests; notch/pill goldens | Physical/pretend notch, display changes/1×/2×, live Dock/calibration and primary-display selection |
| O09 synchronized hover lift, late subscription hydration, no hover-off snap | Shared Motion::Overlay transition, collapsed anchor, retained reserve | `overlay_applied_anchor_interface_drives_preview_lift_without_moving_window`; anchor invariance; lifted goldens differ from resting captures | Connect initial anchor read and observe overlay/card together during interrupted hover |
| O09 nonactivation/click-through/all Spaces/fullscreen | Shared never-key panel, force ignoresMouseEvents after every transaction | Compiled inert never-key/click-through/visibility and idempotent-setup tests; no focus/press action in AX adapter test | Click card into external editor, then type; Spaces/fullscreen/Stage Manager |
| O09 notch dark; pill system appearance/HUD clip; reduced motion/transparency/contrast | Shared palette/preferences/motion; per-card native HUD mask | Notch, light/dark pill, contrast and lift offscreen cases; existing preference/motion tests | Live system changes and material alignment while lifting/shrink-wrapping |
| O09 accessible text/status; no editing/copy/focus or press action | Emitted AccessKit status/text, shared production macOS adapter probe; dedicated preview announcement bridge matching the current polite text live region | `actual_preview_adapter_tree_is_informational_and_lifecycle_clears_text` checks actual text, no press dispatch, unchanged focus and immediate AX removal | VoiceOver speech/rotor and timing; the preview-only bridge sends words only after consent/current-generation admission, matching existing React aria-live |
| O08/O10 no logs/copy/paste/transforms/persistence/export/telemetry; clearing releases content | No such APIs; bounded ephemeral text; no Debug/serde on events/model | Source boundary validators, model clears, closed-inbox teardown, AX removal and synthetic-only captures | Confirm integrated adapter never duplicates partials into diagnostics or final pipeline |
| Webview destroyed, not hidden | `PreviewRetirement` aliases existing main-thread weak barrier; own-process dyld inventory | `murmur-webkit` real nonpersistent WKWebView extra-owner 1 → 0 test; every preview golden requires zero unattributed WebKit-family images | Phase 2 host must invoke barrier on actual replaced view; production removal is not claimed |

## Headless checks and native handoff

The existing `gpui-macos` CI job already runs strict fmt/check/Clippy,
`scripts/test_gpui.py`, compiled native shims, and
`scripts/check_gpui_goldens.py`. The golden runner includes the preview binary
and requires same-host byte-identical pairs, nonblank frames and the shared
[attributed WebKit policy](gpui-overlay.md#qa-webkit-attribution-policy) in each
preview process. Its separate launch-only baseline requires zero images before
AX exposure. It does not inspect other apps.
The offscreen fixture explicitly bypasses native panel mutation and announcements,
as the overlay QA host does; it never orders a window onto the GUI owner’s screen.
The standalone WebKit lifecycle test is already wired into `rust-macos`.
No dependency pin changes. The additive
`app/gpui/patches/gpui-hidden-presentation.patch` exposes the existing
`Window::present_if_needed` outside its profiler-test gate; apply it after the
existing GPUI patches. The recorded patch reproduces the vendor edit exactly.

The Mac mini GUI belongs to another worker. The review-correction task
additionally permits a task-owned background native
preview host, own-PID CGWindow visibility/alpha observations and noninteractive
own-window captures. `MURMUR_PREVIEW_NATIVE_SMOKE=1` runs an empty recording
followed by three partial/stop cycles at unchanged geometry, with content-free
stage markers. It neither activates a window nor drives HID. Other tests use
headless windows, offscreen Metal textures, inert Objective-C doubles and
nonpersistent windowless WKWebViews. No production profile/store, installed
Murmur, signing key, permission setting or clipboard is accessed.

The lead must build the exact head in an isolated synthetic host when the GUI
is available, then run the native column above, especially click-through and
click-then-type, initial nonempty display, live consent/lifecycle races,
overlay/card hover choreography, physical notch/Retina/multiple displays,
Spaces/fullscreen/Stage Manager, live appearance/accessibility preferences and
VoiceOver. The standalone QA executable shows synthetic words only; it does not
establish real dictation or a connected production settings adapter. Phase 2
must wire those adapters and prove actual production-view deallocation before
O08–O10 and #1000 can be marked complete.

## Verification receipt (Mac mini, checkout 45)

Headless Fleet route through opti, Rust 1.98.1, fresh
`~/Builds/gpui-checkout-45` and task-owned `gpui-checkout-45-target`:

- Native workspace locked check, strict all-target/gallery Clippy (`-D warnings`)
  and rustfmt pass. The AX transport runner passes **74 tests**, including
  production macOS adapter role/action checks, the actual overlay-to-preview
  anchor flow, stale first-frame admission and opt-out/new-generation races.
- Runner-source and compiled inert Objective-C shim suite: **8 passed**.
  Configuration is applied once across repeated native transactions; preview
  visibility and click-through remain independent of overlay hit testing.
- Offscreen goldens: **84 cases / 168 captures**, including **21 preview
  cases / 42 preview captures**. All pairs are byte-identical and nonblank;
  every preview capture inventories zero WebKit images in its own process.
  Light/dark pill fixtures and resting/lifted card bounds differ as expected.
  These are synthetic textures, not real-app screenshots or native acceptance.
- Capture helpers built before Tauri checks. Tauri locked check, rustfmt and
  workspace/all-target Clippy pass with `-D warnings` and the two requested
  newer-toolchain allowances. Serial Tauri tests: **1,552 passed / 16 ignored**,
  including windowless main-thread and shutdown harnesses.
- Core serial tests: **292 passed** (257 unit + 35 integration). An initial
  default-parallel invocation hit unrelated updater timing assertions; it was
  stopped and rerun serially as required, including those assertions passing.
- Standalone WebKit lifecycle target passes, with actual memory-only preview
  WKWebView listing **1 → 0 (deallocated)** and an extra-owner failure case.
  It also retains the overlay/import/removal/relaunch/store-retirement checks.
- Restart Clippy fixtures: **29 passed**. Policy/reference/Markdown/transport
  and restart suites: **118 passed**. Workflow, capture boundary, reference
  docs, Markdown links, core dependency and targeted-event validators pass.
- Reproduced **102 GPUI files**, **2,346 GPUI Kit files** and **13 AccessKit
  files** byte-for-byte from pinned sources plus recorded patches. Four
  supplementary license files, added by the earlier vendor layer and absent
  from the corresponding upstream archives, are preserved and excluded from
  that source reconstruction count. No vendor file or patch changes here.
- All **14 changed source/build/test files** in the Mac checkout match source
  commit `b86d27b569873618cc2457a76a2b0ee1f7129945`. The CI library-only manifest
  recipe is confined to the disposable Mac checkout.

Independent Sol review, the native handoff above, integrated production host
adapters and actual production-webview retirement remain open. The issue is
not claimed complete. No performance harness or benchmark was run; memory and
frame/CPU acceptance awaits George's explicit request. Checkout 45 and its
private targets are removed after the gates; protected directories, the GUI
owner's session and production application data remain untouched.

### Shared QA WebKit policy

Preview QA uses the overlay's [attributed WebKit policy](gpui-overlay.md#qa-webkit-attribution-policy)
at startup, offscreen capture, and native-smoke completion. The golden suite
also runs `MURMUR_WEBKIT_BASELINE_ONLY=1 preview-qa` before NSApplication/window
creation and requires zero images. Runtime inventories preserve numeric
`pid`/`webkitImages`, add `axAttributed`/`unattributed` plus startup/linkage counts,
and retain path/symbol load evidence. Apple accessibility bundle loads reached
through incoming AX Mach/runloop dispatch are allowed; app-owned or unexplained
loads fail. No accessibility suppression or production diagnostic overhead is
introduced. This policy concerns the GPUI process; legacy-view retirement remains
a separate lifetime check.
