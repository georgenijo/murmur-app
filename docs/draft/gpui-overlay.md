# GPUI overlay contract (#999)

Layer 10 of the draft stack, based on `gpui/09-accessible-components` at
`0ad8c3afb1c1f1cab930ac88c3cd8d34a172a43c` (PR #1027). **Do not merge.**
The [Phase 0 decision](../decisions/2026-11-05-gpui-phase-0-exit.md) selects
native parity first: the native screen is built beside the shipping Tauri app;
its host switch belongs to Phase 2. This layer does not remove the shipping
React overlay or retire its tests.

## Ownership and integration

`murmur-ui::overlay` owns presentation, click recognition, hover intent, transient
cues, native panel treatment and its accessible control tree. Its one-way core
dependency supplies `dictation::Status`/`Input`, named waveform interest and the
existing overlay geometry. No Tauri, WebKit, capture, text injection, settings
writer, completion actor, updater or shutdown runtime is added to the UI.

The original pure algorithms and 32 tests move from `commands/overlay.rs` to
`murmur-core::overlay_geometry`. The Tauri adapter re-exports the public types
and calls the same functions. Screen geometry, calibration intent, Dock limit,
fixed capsule dimensions and preview anchoring therefore have one owner.
The Rust fixture now lives in `tests/fixtures/overlay-geometry.json`; the
frontend fixture remains until that renderer's tests retire.

A native application host creates `Overlay::new(snapshot, host)`, wraps it in
`gpui_base::Root`, calls `advance` in that main-thread window turn, and calls `follow` once. It retains the host's existing
services and Tokio runtime through shutdown. The host must:

1. Register its Overlay consumer before requesting the initial read. Clone the
   view's `Inbox` for worker callbacks. Capture `Model::hydration_epoch` at the
   start of an asynchronous read and deliver `Event::Hydrate` with that epoch;
   an intervening lifecycle/visibility/geometry event rejects the old read.
   Authoritative full snapshots carry increasing presentation revisions and
   the current recording generation. Only a strictly greater revision with a
   non-stale generation is accepted;
   the constructor snapshot counts as already applied. Initial asynchronous
   hydration must have a newer revision, and a current hydration epoch does
   not make a duplicate revision new. An epoch mismatch rejects even a newer
   revision. Every accepted recording snapshot rebases
   the local timer at receipt: `elapsed_ms` is the authoritative base, including
   a refresh with unchanged status/generation. Rejected snapshots change neither
   the elapsed base nor its local clock anchor.
2. Forward typed lifecycle/cancellation/failure/delivery/stall events, committed
   settings and Mode/meeting/Smart Auto read models. Keep generation IDs as u64.
   The bounded queue returns rejected messages to the producer; retry or refresh
   an authoritative snapshot on backpressure. Levels occupy one replaceable slot
   and cannot displace lifecycle messages. Do not serialize payloads into logs.
3. Route `Intent::Control` through the existing dictation controller/QueueGuard
   and recheck its expected generation immediately before native execution.
   `Control::input` maps lock, stop and starting-cancel to existing core inputs.
   Retry uses the existing secure delivery service and returns its content-free
   result with both generation and attempt. The UI never writes a clipboard,
   infers paste success or submits a completion/History/statistics update.
4. Merge only the requested Disabled/Auto-paste field through the existing
   owned-field writer, then publish the committed snapshot. Cycle Mode through
   its runtime owner. Revalidate meeting suggestion tokens, consent and busy
   state in that service; UI single-flight is an additional presentation guard.
5. Apply `WaveformConsumer::Overlay` interest to the existing targeted router.
   Levels do not drive auto-stop here. Forward the applied collapsed anchor,
   placement and lift to the separate Live Words/preview owner. Provisional text
   is never part of this screen's event protocol or diagnostics.
6. Feed `Event::Dock` from the existing Dock watcher. A Dock-only update retains
   hover ownership; topology changes reset it. The native panel also polls the
   primary NSScreen and static hidden-Dock reserve every 500 ms, including scale,
   origin, safe area, work area and magnification preferences. Live Dock limits
   remain authoritative until the display changes. No new AX client reads the
   Dock, app names, window titles, clipboard or secure fields in this layer.
7. Route Settings opening and all termination/relaunch intents to the existing
   host owners. Dropping/replacing the view releases waveform interest and ends
   its weak-entity watchdog; it does not quit a runtime or stop capture itself.

The library is the screen integration boundary. `overlay-qa` is an explicitly
synthetic host: its controls exercise the core controller with fake readiness,
Mode and owned-field snapshots, and never record, paste or open user data.
It cannot establish real dictation, Settings-window integration or meeting
capture parity. Those service adapters must be connected in the native host.

## Native treatment and webview destruction

The #978/Phase 0a approach is recorded in the Phase 0 decision: a PopUp NSPanel,
never key, prevents activation, level 25/all Spaces/full-screen auxiliary and a
behind-window HUD material. The protected scratch prototype is not modified or
included in this repository.

`native/overlay.m` applies an instance-local subclass that returns NO from both
key/main-window queries; other GPUI panels retain their behavior. It guards
`_setPreventsActivation:`, includes Stationary/IgnoresCycle/CanJoinAllApplications,
and refits the `.hudWindow` material and tint inside the main-thread frame
transaction. GPUI's 360 ms value transitions refit the material mask on the same
presentation frame. Reduced Motion uses zero-duration transitions and static
pulses/spinners; Reduce Transparency uses opaque tint. The notch remains dark;
the pill follows system window appearance, independently of the main theme.
Shared buttons/toggles receive window-local semantic roles and keep their AX
names when their visible captions are compact glyphs.

The native UI links no WebKit. Every offscreen overlay capture inventories its
own loaded images using dyld and requires zero WebKit images. It records only
its PID and that count. This proves the native process's dependency/runtime
absence, not destruction of another process's Tauri window.

`murmur-webkit::OverlayRetirement::destroy` separately consumes the legacy
host's retained view on main, stops navigation, disconnects delegates, removes
it from its superview and releases it. Its weak-reference listing remains one
if any other owner survives; only ObjC deallocation produces zero. The existing
windowless `synthetic_store` integration target exercises an actual memory-only
WKWebView and the extra-owner failure case, printing before=1/after=0.
The native host must require this zero barrier when replacing an existing
view. Never count hide/detach as destruction. This does not retire the website
data store: unported windows and the existing migration importer retain their
v1/v2/v3 and final no-webview barrier contracts. No production store is touched.

## Parity and test map

Tests named below are in `overlay/tests.rs`, except `overlay_geometry::*` in the
core, `inbox::tests`, the Objective-C tests and the WebKit integration target.
All fixtures and rendered captures are synthetic. Native acceptance is separate.

| Current behavior / matrix row | GPUI implementation | Automated coverage and remaining acceptance |
|---|---|---|
| O01 notch, pretend notch, 1×/2×, nonzero origins, fixed pill | Shared geometry, `Model::frame`, primary NSScreen snapshot; fixed 28 pt wings with a notch spacer, centered pill row | 32 moved geometry tests; `both_placements_use_the_shared_geometry_and_live_dock_limit`; `painted_status_content_stays_in_notch_wings_and_centers_in_the_pill` checks actual painted waveform/retry bounds; physical/pretend-notch/pill goldens; native physical notch/Retina/display pass remains |
| O01 non-activation, all Spaces, fullscreen, Stage Manager | Instance-local never-key panel and guarded prevents-activation | `test_overlay_never_key_hook_is_instance_local_and_idempotent` uses inert NSObject doubles; native click-then-type/Space/fullscreen checks remain |
| O01 Dock tracking, calibration intent, bottom anchoring | Shared Dock clearance; separate `Event::Dock`; static preference fallback | Shared live/unknown Dock and work-area tests; native magnification/reveal/side-Dock checks and live watcher connection remain |
| O02 Idle, Starting/stalled, Recording, Recovering, Processing, meeting, Done | Core Status read model, priority derivation, bounded Done | `cue_priority_and_expiry_match_current_overlay`, `elapsed_starts_at_recording_not_hover_and_done_is_preemptible`; state goldens |
| O02 7/18 bars; coalesced 16 ms levels; no idle animation | Single latest-level slot, generation/visibility check, one GPUI draw stream | `waveform_is_bounded_and_only_the_current_visible_owner_can_supply_samples`, inbox backpressure/coalescing test; no performance claim |
| O02 HUD/tints, Reduce Motion/Transparency, appearance | Same-turn AppKit HUD and window-local palettes; shared motion engine | Inert shim compilation; deterministic offscreen goldens and existing motion/preference tests; live material and OS changes remain |
| O03 150 ms dwell, acknowledged growth, 300 ms leave, delayed shrink | One main-thread surface writer and phase reducer; disabled state retains hover; shrink retains a pending re-entry dwell | `hover_dwell_ack_leave_shrink_and_failed_grow`, `disabled_hover_can_reopen_quick_settings_but_cannot_start_recording`, `shrink_preserves_a_pending_dwell_in_the_persistent_top_band` (1000 ms enter → 1080 ms shrink → 1150 ms open), `state_contractions_retain_native_bounds_then_release_without_anchor_drift`, `hidden_and_topology_reset_do_not_reopen_transparent_hit_area`; native clipping/hit-area check remains |
| O03 Mode/Next, Power, Auto-paste/file-output pause, Settings | Shared Button/Toggle; typed owned-field/runtime intents; a dedicated bounded disclosure Button is present in the status row of each visible surface; its AX title conveys collapsed/expanded and AXPress toggles disclosure without key focus or dictation activation | `actual_overlay_tree_controls_and_actions` and `disabled_collapsed_overlay_opens_and_closes_through_macos_adapter` pass emitted nodes through the vendored macOS adapter role/title mapping and shared production AXPress handler, then transport only its generated actions; collapsed/disabled → open → Enable Murmur → close and unchanged focus/intents are asserted; native host field commits/Settings opening remain |
| O04 250 ms click, double-click lock, disabled/processing/open guards | Click recognizer sends generation-bound core inputs | `click_delay_double_click_and_generation_ownership`, `starting_click_cancels_and_disabled_recording_remains_stoppable`; real capture/delivery remains host acceptance |
| O05 Cancelled 800 ms, opt-in tap miss 500 ms, failure 5 s | Generation-scoped timed cues, fixed content-free announcements | Priority/expiry and stale-generation tests; actual VoiceOver speech remains |
| O05 Clipboard-only/retry, duplicate/stale outcomes | Secure retry intent, attempt/generation guard, bounded result | `delivery_duplicates_never_extend_cue_and_only_confirmed_clipboard_installs_it`, `retry_is_single_flight_and_uses_a_fresh_bounded_result`, stale retry test; no clipboard access in test host |
| O06 calibration cue and offset, no pointer dragging | Shared finite ±12 pt intent; pill zero travel; pretend-top clamp | Moved calibration tests and state priority test; Settings Save/Cancel remains its owner and a native acceptance flow |
| O07 reattach during recording, subscribe-before-read | Snapshot revision + hydration epoch + current elapsed read model | `hydration_cannot_roll_back_a_same_generation_event_and_restores_elapsed`, `accepted_recording_snapshots_rebase_elapsed_even_without_lifecycle_change`, `duplicate_snapshot_and_hydration_do_not_rewind_the_recording_timer`, `initial_hydration_and_current_epoch_refresh_accept_newer_revisions`; actual host subscription ordering must follow the integration contract |
| O08/O09 Live Words consent, generation, click-through and hover lift | Existing preview owner retained; anchor/lift interface only | Moved preview fitting/collapsed-anchor tests; text/rendering port belongs to #1000, with native click-through and consent checks retained |
| M01/M13 meeting states/suggestions, accept/dismiss | Idle-only prompt, token recheck and single-flight action | `suggestion_requires_idle_and_retains_expanded_bounds_until_dismissal`, `suggestion_actions_recheck_current_token_and_are_single_flight`; live capture/token authority stays in meeting service |
| A01 accessible names, roles, state changes, inactive announcements | Shared controls/AccessKit plus native content-free announcement notification | Emitted tree/actions test and component suite; VoiceOver spoken output cannot be established headlessly |
| Destroy overlay webview; no replacement WebContent process | Per-view weak destruction barrier and native own-process inventory | Actual nonpersistent WKWebView lifecycle fixture; every overlay Metal capture requires zero WebKit images; production host replacement remains pending |

The shipping `useOverlayExpansion` and its disabled-hover/re-entry tests define
pointer timing, including keeping the close shrink armed until a re-entry dwell
commits. `OverlayPill` and its tests reserve independent notch wings and center
the floating row. Native status uses those same content slots: seven bars occupy
23 pt centered in the right 28 pt wing; eighteen bars occupy 61.5 pt in the pill.
The idle, collapsed notch has no exposed wings and omits the recording status
content. A 20×24 pt disclosure target with a small chevron in the leading status
slot toggles Quick settings; the active state glyph retains its separate
recording/status click behavior.
The notch reserves an 8 pt state slot (including an 8 pt spinner) beside the
20 pt disclosure inside its fixed 28 pt wing. Clipboard-only delivery uses the
single copy symbol “⎘” in that slot; the floating pill keeps “⌘V”. Both retain
the status AX label “Copied. Paste manually or try again”. State symbols and
floating status text do not wrap. The pill's compact “Recovering” caption keeps
the full “Audio recovering” AX label. A CoreText-backed layout test audits all
indicators in physical/pretend notches at the 24 pt minimum row height and in
floating pills, collapsed and expanded, including intrinsic symbol bounds,
card containment, disclosure separation and meaningful status AX labels.
The floating pill retains its
120×34 pt collapsed geometry and its waveform/retry room. Its compact retry
caption is “Retry”, retaining the accessible name “Try again”. Disabled
state gates recording starts, never access to the Enable control.

The status row is absolutely anchored to the card's bottom edge for pills and
top edge for notches; controls occupy the opposite side of that row. Their
content minimum height cannot displace the disclosure during growth/shrink.
The status row precedes the controls in AX child order regardless of visual
position, so the disclosure is encountered before Mode, power, Auto-paste and
Settings. The disclosure's AX frame is its actual pointer hitbox; it does not
cover the status row or quick controls. Hovering any part of its enlarged
target counts as inside the painted card and uses the same dwell/leave reducer.
Pointer clicks and AXPress use the same toggle and acknowledged resize path,
with no focus owner. Its adapter-exported title
is “Quick settings, collapsed” or “Quick settings, expanded”. Mouse-down and
click propagation stop at the chevron so it cannot start/stop dictation. The
status retains its recording activation and the panel remains never-key.

`disclosure_ax_frame_is_its_pointer_target_in_both_placements` compares the emitted
AX frame to the laid-out hitbox, clicks its center and near-corners, and reverses
each click through the macOS adapter's AXPress route. It covers both placements,
disabled state, repeated growth/shrink, unchanged focus, no delayed recording
intent, the adjacent recording action and hidden-surface exclusion. Removing the
pointer handler makes the regression fail while AXPress remains available.
`disclosure_stays_hittable_and_precedes_controls_during_motion` enables real
360 ms transitions and samples both endpoints plus 16/60/120/180/240/270 ms
frames in both placements and disabled states. It checks full target containment
in the card clip, the 20×24 pt AX/hit bounds, near-corner and center clicks,
AXPress reversal, unchanged focus/intents and traversal of actual AX child
links before all four quick controls.
The windowless adapter probe shares the production press target resolution and
dispatch function; it creates no NSView/NSWindow and makes no AX client calls.
It establishes adapter mapping/action handling, not VoiceOver spoken behavior.

No React assertion is retired. This table describes replacement implementation
and executable tests; it does not mark the feature matrix's native rows passed.
The native host/service connection, live Dock watcher integration, and production
view replacement remain integration gaps until the host exists. The synthetic
QA executable must not be presented as an end-to-end dictation implementation.

## Checks and lead handoff

Use Rust 1.98.1 and the isolated target directory. The existing `gpui-macos`
job checks both gallery-gated binaries, strict all-target Clippy, rustfmt,
`scripts/test_gpui.py` (real emitted AX tree/actions without native windows),
Objective-C inert shims, and `scripts/check_gpui_goldens.py`. The latter now
renders 35 overlay cases twice in addition to the 30 component cases: 65 cases,
130 captures. Physical and pretend notches each cover 13 states, including
clipboard delivery at the minimum 24 pt row height and
collapsed/expanded disabled states; the pill covers nine visible states. It
rejects blank/non-deterministic captures and missing/nonzero own-process WebKit
inventories. The native workspace and core remain within
the existing source/transport/restart dependency guards. The WebKit lifecycle
target already runs under the existing `rust-macos` job; no new CI job is needed.

The implementation worker runs headless checks only, never drives a real app,
requests OS permissions, opens a user profile, or uses production signing keys.
The lead must verify the integrated revision in an isolated native build:

- Click every quick control and retry, single/double click each permitted and
  refused state, then type in a synthetic target editor. The panel must never
  become key, activate Murmur, reopen main unexpectedly or redirect delivery.
- Graze/dwell/leave/re-enter during growth and shrink; disable while open;
  hide/show and change display/scale while opening. Verify acknowledged reveal,
  no clipping, restored collapsed hit area and background click-through.
- Exercise physical notch and external displays, negative origins, Retina,
  Spaces/fullscreen/Stage Manager, Dock autohide/magnification/side placement,
  calibration draft/Cancel/Save, and Live Words following/lifting/click-through.
- Check light/dark system changes, application theme isolation, HUD appearance,
  Reduce Transparency/Motion, starting/stalled/recovery/failure/meeting and
  delivery cues. Verify readable labels, waveform and no idle animation.
- Use VoiceOver to inspect and activate every control and hear inactive-panel
  announcements. Check hidden/disabled controls and restore focus to the target.
- Connect real service adapters; verify generation races, current-state reattach,
  secure Paste Last results, concurrent owned-field saves, meeting consent/token
  revalidation, and exact-once completion/History/statistics through existing
  owners. Verify the real replaced webview is destroyed and record its listing.

The issue's frozen 60 MiB idle, ≤10% one-core recording CPU and ≤50 ms p95 frame
interval criteria remain pending an explicitly requested run. No benchmark or
performance harness is run or added as a gate in this layer.

## Headless verification receipt (Mac mini, 2026-11-06)

All Mac commands ran through Fleet via opti in the fresh
`~/Builds/gpui-checkout-41`, with Rust 1.98.1 and task-owned targets. The capture
helpers were built before application checks. The CI library-only manifest
recipe was applied only in that temporary checkout.

- Native workspace: locked check, strict all-target Clippy and rustfmt passed;
  **53 GPUI tests** passed through `scripts/test_gpui.py`.
- Python source/native shim tests: **6 passed**, including both inert Objective-C
  runtime tests. The retained frontend and neutral geometry fixtures agree.
- Metal: **49 cases / 98 captures**, same-host pairs byte-identical and nonblank.
  All **38 overlay process inventories** reported zero loaded WebKit images.
- Tauri: locked check, workspace/all-target Clippy with `-D warnings` and the
  two requested newer-lint allowances passed. The default serial suite passed
  **1,552 tests / 16 ignored**, plus the windowless main-queue and AppKit shutdown
  integration harnesses. Rustfmt passed after the restart-fixture runner removed
  its temporary source files.
- Core on Mac: **292 passed** (257 unit + 35 integration), including the
  **32 moved geometry tests**. The same core suite passed on Linux.
- `murmur-webkit`: standalone synthetic lifecycle target passed, including the
  real nonpersistent overlay view's **1 → 0 deallocated** listing and retained
  owner failure case. This is separate from production host replacement.
- Capture helper: **46 passed / 1 ignored**. Restart Clippy fixture runner passed.
- Linux source/transport/restart/reference/Markdown/workflow validators passed;
  **121 policy/regression tests** passed. No frontend source or test is changed.

This receipt is headless evidence. Independent Sol review and all native lead
acceptance listed above remain open. The issue is not claimed complete by this
receipt, and no production application, data store or release was changed.

## Review corrections receipt (Mac mini, 2026-11-06)

The five review findings against `e016910a` are corrected without changing the
shipping React/Tauri app or core. The React expansion controller and its tests,
`OverlayPill`/geometry tests, and `OverlayDropdown` timer/control tests were the
behavior references. Disabled hover, collapsed AX disclosure and control actions,
authoritative timer rebasing, the 1080/1150 ms shrink race, and painted notch/pill
content geometry now have regression coverage.

Headless checks ran through `fleet exec --via opti` in a fresh
`~/Builds/gpui-checkout-43`, with its own `~/Builds/gpui-target-43` and
`RUSTUP_TOOLCHAIN=1.98.1`:

- `cargo fmt -p murmur-ui -- --check`, locked gallery check and strict all-target
  gallery Clippy (`-D warnings`) passed.
- `python3 scripts/test_gpui.py -- --test-threads=1`: **58 passed, 0 failed,
  0 ignored**, plus zero doctests. Five tests were added; the existing emitted
  AX test now begins collapsed and dispatches Expand, all four quick controls,
  and Collapse. The disabled keyboard test opens with Down and closes with
  Escape after an AX button moves focus. AX/layout fixtures use Reduced Motion
  to assert the settled layout; painted centers allow half a device pixel of
  snapping, while notch/content nonintersection remains strict.
- `python3 -m unittest scripts/test_test_gpui.py scripts/test_gpui_native.py`:
  **6 passed, 0 skipped**, including both inert Objective-C runtime tests.
- `python3 scripts/check_gpui_goldens.py`: **63 cases / 126 captures**, all
  nonblank and byte-identical within each same-host pair. The **33 overlay
  cases / 66 captures** include physical/pretend-notch disabled states and
  all visible pill states. All **66 overlay process inventories** require zero
  loaded WebKit images.
- Local Markdown/reference, core-dependency and workflow-policy validators
  passed. No core/Tauri source changed. The six changed source/document hashes
  matched the Mac checkout before cleanup. Gate logs, the golden receipt and
  synthetic captures were preserved before removing checkout 43 and its target.

This is headless evidence only. No real application was driven, no OS permission
or production data was accessed, and no other worker's checkout or GUI session
was touched. Independent review of these corrections and the lead's native
acceptance remain separate; the PR stays draft and unmerged.

## Adapter disclosure and duplicate-snapshot corrections (checkout 44)

This supersedes the earlier receipt's disclosure evidence: injecting GPUI Expand
or keyboard events did not establish a usable route through the macOS adapter on
a never-key panel. Quick settings now has a persistent nonvisual AX Button with
an adapter-exported collapsed/expanded title. Its press opens/closes the same
acknowledged surface without focus changes or recording intents. The disabled
case presses Enable Murmur through that adapter path before collapsing again.
The adapter's original press logic is shared with the windowless probe; the
additive `accesskit-press-probe.patch` records every vendor change.

Equal snapshot revisions are rejected, including hydration replay after an epoch
change. Newer initial hydration and current-epoch refreshes still restore/rebase
elapsed time; outdated epochs and generations remain rejected. Both regressions
fail against the original `4c230b58` UI/model: the disclosure control is missing,
and replaying revision 1/generation 7/61000 ms reports 1:01 instead of 1:02.

Headless gates on the Mac mini, Rust 1.98.1, fresh
`~/Builds/gpui-checkout-44` and private `~/Builds/gpui-target-44`, via Fleet/opti:

- Package rustfmt, locked gallery check, and strict all-target gallery Clippy:
  **3 gates passed**.
- `scripts/test_gpui.py -- --test-threads=1`: **60 passed, 0 failed,
  0 ignored**; **0 doc tests**. Includes both adapter flows and two new timer /
  hydration regressions; existing recording activation remains covered.
- Runner-source and inert native-shim unittests: **6 passed, 0 skipped**.
- `scripts/check_gpui_goldens.py`: **63 cases / 126 captures**, byte-identical
  within each pair; **33 overlay cases / 66 captures**, each with zero loaded
  WebKit images.
- Patch reproduction: **102 GPUI files**, **7 component patch-target files**,
  and **all 16 AccessKit files** match upstream plus their recorded patches
  byte-for-byte. Forward and reverse patch checks pass.
- Local Markdown (**72 maintained docs**), reference (**189 commands**),
  core dependency boundary and `git diff --check` pass.

Gate logs and the golden receipt are preserved before removing checkout 44 and
its target. No real app, GUI session, AX client automation, permissions or user
data were touched. Independent review and native acceptance remain with the
lead; this PR stays draft and unmerged.

## Native acceptance corrections (checkout 48)

Layer 9 at `7a22c5c8` is merged, retaining its window AX focus forwarding,
dropdown expanded state and live system appearance watcher.

The floating material was a **child of the Metal-backed GPUIView**. Child
views composite above their parent's layer, regardless of their ordering among
siblings; `NSWindowBelow` on that child did not put it behind GPUI's foreground.
The material is now a non-hit-testing sibling below GPUIView, with its mask
converted from the same presentation rectangle. GPUI paints the semantic tint
and border above the visual effect in the same scene as the foreground;
NSVisualEffectView's backing color did not provide a reliable dark/opaque tint.
Reduce Transparency uses an
opaque surface; Increase Contrast additionally uses an opaque surface, a solid
border and the matching accessible AppKit appearance. Both hide the unused
backdrop effect and retain the existing
opaque semantic foreground colors.

Native frame writes now refresh GPUI's viewport in the same main-thread turn,
before controls are revealed. The AppKit callback alone was deferred while the
window was borrowed: centering the expanded 241 pt card in the stale 120 pt
viewport displaced paint, hitboxes and AX by approximately 60 pt. Paint, input
and AX now share the acknowledged viewport after growth and shrink.

Delivered GPUI hover events and the existing 33 ms watchdog reconcile through
one pointer-presence state, using current timestamps. Events cancel dwell even
when a fast exit/re-entry falls between two inside samples; polling recovers
missed exits during resizing. The stored painted rectangle follows GPUI's
alignment: bottom for floating pills, top for notch cards, including flipped
views and contraction inside retained expanded bounds. Only presence
transitions feed hover intent, preserving the 150 ms dwell, 300 ms leave and
380 ms shrink without repeatedly moving deadlines. An AX-opened panel stays
open until the pointer actually enters and leaves; a stationary pointer cannot
undo an explicit AX close. Headless hosts use the same GPUI event path.

Regression coverage includes native sibling order/material hit-test exclusion
and mask refits, missed-exit recovery and exact collapsed bounds with/without
Reduced Motion, AX-open/close and rapid-graze handling, and repeated native-size
transactions with emitted AX bounds inside the viewport and pointer clicks at
Mode's exported center. The merged adapter reproduction test removes and
reapplies both additive patches and compares every AccessKit vendor file.

Native verification uses only a task-owned background synthetic host and
window-specific captures. Process-local preference fixtures do not write System
Settings. No pointer/key input, focus changes, permission grants or production
profile access are part of this correction run. The lead retains the live HID
recheck and independent review gates.

Correction verification on Rust 1.98.1, fresh `~/Builds/gpui-checkout-48` with
its private `target48` directory:

- Locked gallery check, strict all-target gallery Clippy and rustfmt passed.
- `scripts/test_gpui.py`: **64 passed** plus **1 AccessKit adapter test**;
  ordinary gallery-enabled `cargo test`: **43 passed**; **9 source/native-shim
  tests passed**, including the hierarchy regression.
- Goldens: **63 cases / 126 captures**, nonblank and byte-identical per pair;
  every **33 overlay cases / 66 captures** reported zero WebKit images.
- Actual native window-only captures: **12 cases**, covering light/dark,
  normal/Reduce Transparency/Increase Contrast, collapsed recording/expanded
  controls. The foreground is readable in all cases. Every observed frontmost
  PID differed from the synthetic host. Read-only native AX queries place the
  expanded window and disclosure at x=839, and Mode at x=871 (75×24 pt), inside
  the window. No AX action or HID interaction was used for this receipt.
- Removing the same-turn viewport refresh makes the new AX/geometry regression
  fail at its explicit stale-viewport assertion; the fixed revision passes.
- **102 GPUI files** reproduce byte-for-byte from the pinned upstream and
  recorded patches. All **16 AccessKit files** pass the merged round-trip
  patch check; executable source also reconstructs from the published crate
  plus Phase 0, expanded-state and press-probe patches, retaining the imported
  repository-level license files. No additional vendor edits were needed.
- All **16 changed build/test source hashes** matched the Mac checkout.
  Markdown, workflow-policy, core-dependency and whitespace checks passed.

Captures and gate logs are preserved before deleting the task checkout and its
target. The live HID recheck and independent review remain with the lead; this
stack layer is still draft and unmerged.
