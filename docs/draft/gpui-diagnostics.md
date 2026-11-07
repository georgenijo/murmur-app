# GPUI Diagnostics contract (#1001)

Stack layer 12 of #1011, based on `gpui/11-dictation-preview` at
`6ab0ca8a86c3f5141aa569a6c941f59ba54765f0` (PR #1029). This is a standalone
`murmur-ui` surface and synthetic QA executable. It does not replace a production
window or retire any React, Tauri, Vitest or visual suite. Native application
host integration belongs to #994/#996/#997.

The #993 dependency is moot: the ownership matrix gives step 1.9 no work under
the selected native-parity route. This layer does not change legacy lazy-window
behavior. The plan's memory criterion remains pending explicit authorization;
no performance run or new performance gate is part of this layer.

## Services and admission

Create `Diagnostics` inside `gpui_base::Root` after `murmur_ui::init`, on GPUI's
main thread. Call `attach` and `follow` after window creation. Call `detach` when
hidden and `attach`/`follow` when reopening; drop also detaches. Rust and the
native host own window size, position, traffic lights, fullscreen and activation.
There are no JavaScript geometry operations or off-thread NSWindow mutations.

Supply `host::Host`, using existing core/service owners. The host must:

1. Subscribe as `Consumer::Diagnostics` before any read. `subscribe` must finish
   registering listeners before returning its subscription. `attach` drains
   queued notifications before capturing read tickets. Stop every subscription
   synchronously in `Subscription::stop`. Stop producers on terminal inbox
   closure, and retry backpressure without formatting payloads.
2. Give each lane positive, strictly increasing revisions across snapshots,
   notifications and error results, including same-record updates. Hydration
   returns its exact `Ticket`; notifications carry the attachment and service
   clear epoch. Notifications invalidate hydration epochs. Rejected same-session
   reads are retried with a fresh ticket and their original read kind. Capture
   detail retries retain their UUID and stop when that selection is cleared or
   superseded; older replies cannot replace a newer pending read. Duplicate
   revisions never replay.
   Revisions must remain monotonic across hide/reopen. Never reset counters on
   subscription creation. A host restart creates a new screen/model.
3. Preserve distinct event and performance clear epochs. A clear notification
   must advance the appropriate service epoch; work started before that epoch
   cannot publish a run/sample/event afterward. Publish the clear after durable
   service success, including clears from another consumer. Event clears leave
   runs alone; Performance clears leave Events alone. Do not invent revisions
   or epochs from completion order of asynchronous reads.
4. Offload blocking reads, JSON parsing, file panels, copy and recovery work.
   `Request::lease.is_current()` must be checked immediately before a side
   effect, on the owning service. Also revalidate the recording generation,
   service clear epoch and capture UUID where applicable. The screen revokes
   leases on superseding reads, notifications, clear boundaries, hide and drop.
   A rejected lease must not copy, recover, delete or upload. Late replies must
   use their original ticket; never relabel them with the current attachment.
5. Project existing sanitized telemetry into `EventRow`, not a second tracing
   sink. Never feed dictation partials or private-capture text into Events.
   `Run`, `Sample`, `Health` and `Latency` are content-free presentation DTOs
   projected from the existing V1 service types; unavailable and not-applicable
   measurements remain distinct. Include the existing runtime, input-size,
   production cohort/startup, follow-up and resource-range facts in run details.
   Map the last-failure classification into StoreProblem so corrupt data, invalid
   records and schema migration failures have distinct guidance; newer stores
   remain restart-only. The repository still owns retention, interruption, schema validation,
   quarantine/recovery and safe service errors.
6. Read only the last 500 events, 200 runs, 600 resource observations and 10
   saved Lab reports. Map a selected report file through `Report::parse` after
   enforcing the 8 MiB bounded read. Imports remain session-only (20 maximum),
   never adopted into run history or the completion store. Do not retain file
   paths, raw JSON, parser errors, transcript text or evaluator failures.
   Every JSON string and key is limited to 4 KiB before schema/identity work,
   including nullable and subsequently stripped fields. Comparison preflight
   caps metrics at 8,192, shared scope storage at 4 MiB and pair-matching work at
   1,000,000 before generating metrics. Legal larger reports remain importable
   but visibly block comparison. Metrics share their model/fixture scope via
   `Arc<str>`. Report selections use source-qualified SHA-256 identities of the
   normalized content; saved-report refreshes preserve selections across order
   changes and visibly clear a selection whose report disappears.
7. A native report-panel cancellation returns `Import { result: Ok(None) }`.
   Copy writes only explicit filtered Events or content-free latency JSON.
   The synthetic adapter never reads or writes the real pasteboard.
8. Only list private capture summaries initially. Read exact text after a
   selected `ReadCapture` request, return `CaptureDetail`, and bind it to the
   selected UUID and request ticket. Keep it out of logs, serialization, Events,
   general copy, charts, goldens and reports. Clear the review on Back, tab
   navigation, hide and drop. Local deletion and reviewed upload require their
   own confirmation; upload still uses the existing restricted receiver and
   seven-day retention. Arming/disarming remains main-window-only: this host
   interface has neither operation. Benchmark, microphone startup, recorder,
   updater, delivery and arbitrary command APIs are absent too.
9. Before replacing the real legacy Diagnostics window, consume its retained
   WKWebView on main through `murmur_webkit::DiagnosticsRetirement::destroy`.
   Release every other strong owner and require `remaining_webviews == 0`.
   Hiding or removing the view is insufficient. Per-window retirement does not
   purge a website store still used by other windows or the migration importer.

The ordered inbox has both a 64-message and 96 MiB budget and rejects oversized
batches. Model/event/report/private-text containers implement neither Debug nor
serialization. Clearing releases allocations; it does not claim secure RAM
erasure. The host remains responsible for the source service's consent,
redaction and data-retention contracts.

## Reusable data components

`data_components` adds screen-independent controls alongside the #1027 kit:

| Component | Contract |
|---|---|
| `Tabs` | Horizontal wrapping tablist, selected tab and active descendant, one Tab stop, wrapping arrows, Home/End, pointer and AX activation; controlled updates preserve focus. |
| `DataTable` | Named Table, ColumnHeader, Row and Cell nodes; column/row counts and indices; per-cell column names and exact text values. Row actions are named Buttons inside their corresponding first cells. |
| `DetailList` | Explicit newest-N bound (Diagnostics paints 100 of 500 retained events), stable identities, one keyboard stop, arrows/Home/End, Enter/Space disclosure and Escape collapse, monospace expandable data, AX expanded state; wheel/keyboard scroll pauses following and Resume restores it. Copy includes all filtered retained events. |
| `Chart` | At most 600 exact-value points, one keyboard stop, arrows/Home/End, shared hover/click/AX values, fixed value region separate from plot, missing values visibly unavailable rather than zero; unit-bearing labels. |
| `Progress` | Named ProgressIndicator, optional clamped finite 0–100 value and bounds; indeterminate state is explicit. No perpetual animation, including Reduced Motion. |
| `Choices::replace_choices` | Refresh data-backed choices while preserving the existing focus owner and valid selection; unchanged data does not close an open picker. |

Controls use the shared semantic Palette, system font, explicit Menlo data text,
8px control/12px card radii, 32px actions and 28px filters from the
[design system](../features/ui-design-system.md). Live system light/dark and
Increase Contrast follow the existing appearance/preferences owners; the screen
never overwrites an application theme during rendering. The standalone QA host
owns its system-appearance observer. Surfaces are opaque,
including Reduce Transparency. Charts/disclosures do not animate. Confirmation
mounts only its named AlertDialog, removes background controls from AX/Tab order,
starts on Cancel, traps Tab between Cancel/Confirm, supports Escape and restores
originating focus. Destructive actions never become a Return default.

## Matrix acceptance and native checklist

The automated tests live in `app/gpui/src/diagnostics/tests.rs`. Goldens use the
production view with the synthetic adapter, rendered to offscreen Metal.
The four portable-report fixtures now live in `tests/fixtures/diagnostic-reports`;
Rust/core and React use the same unchanged synthetic files. CI watches that
neutral directory for all three consumers.
Every Diagnostics capture inventories only its own loaded images through the
shared overlay/preview WebKit attribution policy. Launch-only baselines require
zero WebKit images and clean native linkage; later accessibility-attributed
loads are allowed, while app-owned and unknown loads fail closed. The golden
policy log preserves each inventory. Tests and fixtures never inspect
production app data.

| Rows | This layer | Native acceptance still required with the integrated host |
|---|---|---|
| G01 Events | Six tabs; stream/level/correlation filters; bounded expandable rows; manual-scroll pause/resume; copy/clear; attachment cleanup; read/notification/clear admission and host capability restrictions. | Open from menu/palette/Settings and select requested tab. Filter synthetic events, expand with keyboard/AX, manually scroll, copy to a disposable test clipboard adapter, confirm clear, hide/reopen and prove no duplicate listeners. Verify native focus, resize and fullscreen. Companion windows must fail main-only operations. |
| G02 Runs / Performance | Kind/outcome filters, detail, stages, resource ranges, exact-value CPU/memory charts, correlated Events; distinct loading/error/empty states; store-health guidance and explicit permitted recovery; clear epochs. | Complete synthetic dictation/import; inspect every stage/runtime/input/cohort/startup/resource/follow-up fact. Clear while work is pending and prove old work cannot repopulate. In isolated corrupt/invalid/newer-schema fixtures verify guidance, quarantine and forbidden reinitialization for newer stores. Test retry failure and confirmation cancellation. |
| G03 Reports | Strict legacy/v2 benchmark and evaluation-v1 parser, bounds/uniqueness/consistency checks, text stripping, compatibility before deltas, machine/app warnings, zero-baseline handling, session imports, local saved-report adapter and clear. | Use native file panel with two synthetic compatible reports; cancel panel; import incompatible/oversized/malformed/unknown-version/unknown-field reports. Confirm blockers precede deltas, warnings prevent recommendations, import cap, clear during a read, and relaunch leaves files/durable runs unchanged. |
| G04 Lab | Saved-report read/comparison only. Engine, explicit Run/Cancel, configuration selection, export/clear lifecycle and ten-report store remain with 3.5b and their existing service owners. | Separately authorized 3.5b tool validation; do not start a run by opening Diagnostics. |
| G05 Startup / recorder | No automatic or exposed start APIs. Main-settings tools and internal distribution boundary remain with 3.5b. | Separately authorized 3.5b stub/native validation; recorder absent in normal builds. |
| Current Latency / Dictation tabs | Build-filtered content-free latency edges/copy/clear; summary-only private-capture listing, explicit review, confirmed delete/upload; no arm/disarm capability. | Synthetic latency values and build selection; main-only consent/status, exact UUID/epoch races, expiry, private review/Back, delete cancellation, upload confirmation/cancellation and receiver retention. Never use real capture content. |
| Retirement | Shared main-thread weak-deallocation barrier, real windowless nonpersistent WKWebView extra-owner 1 → 0 test, per-capture native-process inventory. | Invoke barrier on the actual replaced production view, verify its deallocation and process listing. Production retirement is not claimed here. |

For every affected native flow, also verify Tab/Shift-Tab, arrows, Space, Return,
Escape, VoiceOver roles/names/values/actions/announcements, focus restoration,
narrow-window wrapping and scroll reachability, and live light/dark, Reduce
Motion, Increase Contrast and Reduce Transparency. Confirm the running build
contains the reviewed head. Native interactions remain a lead handoff: another
worker owns the Mac mini GUI, so this implementation worker must not generate
HID, AX actions, focus changes, permission changes or System Settings actions.
Only a task-owned background host, read-only AX inspection and its own-window
noninteractive capture are permitted. Headless tests do not establish completion
of this native checklist.

## Verification receipt (Mac mini, checkout 54)

All native builds/checks ran through Fleet via opti, with Rust 1.98.1, fresh
`~/Builds/gpui-checkout-54` and task-owned targets. Capture helpers were built
first. The CI library-only recipe was used only in this disposable checkout;
policy tests used an unchanged-manifest source snapshot inside its target tree.

- GPUI: locked gallery/default checks, strict all-target gallery Clippy with
  `-D warnings`, rustfmt, **98 unit/keyboard/AX tests**, and **8 runner/native-shim
  tests** pass. The new screen never rewrites the application's shared theme
  while rendering; the standalone QA host owns its appearance observer.
- Offscreen Metal: **123 cases / 246 captures**, including **39 Diagnostics
  cases / 78 captures**. Every same-host pair is byte-identical and nonblank.
  Each Diagnostics process inventories zero WebKit images. Light, dark and
  Increase Contrast differ; disclosure, detail, empty/loading and blocked-report
  states have separate captures. Only synthetic fixture content is used.
- Tauri: **1,535 library tests passed / 14 ignored**; strict workspace/all-target
  Clippy on Rust 1.98.1 (without lint allowances), rustfmt, windowless main-queue
  and shutdown integration, and capture-helper **46 passed / 1 ignored** pass.
  Restart-policy Clippy fixtures pass. The standalone WebKit lifecycle test
  passes, including the Diagnostics retained-owner **1 → 0** barrier.
- Core: the moved neutral-report fixture test passes. The initial default
  parallel suite passed 255 unit tests and timed out in two unchanged updater
  gating tests; the serial retry and final CI results are recorded in the PR.
- React: TypeScript and the full **1,833-test Vitest suite** pass. After moving
  fixtures, TypeScript and all **28 report parser/comparison/view tests** pass.
  The retained visual suite passed **521/522** initially; its unchanged knowledge
  editor hit a screenshot-stability timeout, and that case passes on retry on a
  dedicated port. No screenshots or baseline expectations were changed.
- Source/dependency/transport/restart/reference/Markdown/workflow policy checks,
  their Python/Node regression suites and **25 canary unit tests** pass. The
  manifest-shape test runs against the unchanged manifest; the sender-identity
  test uses a task-local canonical temporary path to avoid macOS `/var` aliasing.
- No vendored source changed; existing patch records remain untouched. The four
  relocated report fixtures are byte-identical to the stack base.

The PR records final-head CI and any additional read-only native observations.
These results do not establish the native interaction checklist or production
host integration. Independent Sol review remains a lead gate; this implementation
sub-agent does not delegate another agent. The PR stays draft and unmerged.

## Sol finding repair receipt (Mac mini, checkout 61)

The four review findings are repaired: bounded report strings/comparison work and
shared metric scopes; centralized route/tab cleanup (including queued routes
before confirmation); UUID-preserving current-selection detail retries; stable
report identities across saved-report refreshes. The same-pattern scan also
bounds latency strings, includes identities in inbox accounting, preserves early
import selection and removes private text from synthetic review goldens.

Rust 1.98.1, fresh `~/Builds/gpui-checkout-61`, via opti only:
**104 GPUI tests**, **8 runner/native-shim tests**, locked default/gallery checks,
strict all-target gallery Clippy, rustfmt, TypeScript, **28 React report tests**
and the relocated core report-schema fixture test pass. Helpers build first for
the core test. No native interaction is claimed; the other worker owns the GUI.

The full golden run passed the first **30 control cases / 60 captures**, then
stopped at the overlay host's unchanged zero-WebKit-image assertion: **5 loaded
images**. A separate Diagnostics probe fails the same guard. Rebuilding and
running the original `a327c29b` overlay host reproduces the identical failure.
The guard is unchanged; Diagnostics golden acceptance remains blocked on this
Mac mini. Test output and both failure receipts are published with the PR.
Checkout 61 and its targets are removed after evidence collection. Sol recheck
and the native interaction checklist remain with the lead. The PR stays draft.

## Native QA defect repairs (Mac mini, checkout 66)

The merged stack head `7e9910c1` passed the locked gallery build and all
115 GPUI tests before these repairs; no semantic merge fallout was found.

- Tabs, bounded lists and charts now participate in Tab/Shift-Tab traversal.
  Tables add a composite Tab stop, visible selected-cell highlight and
  arrow/Home/End cell navigation, with explicit row/column indices.
- The macOS adapter exposes nonselectable table rows, stable virtual columns,
  counts, header associations and indexed cell lookup/ranges. The additive
  `accesskit-tables.patch` reproduces the vendored files byte-for-byte after
  the existing expanded/press patches; the runner verifies the original hashes.
- Confirmation origins own persistent focus handles. Escape/Cancel and ordinary
  confirmation restore focus after remount; disappearing origins fall back to
  the selected tab. Deferred restoration rechecks navigation and attachment.
- Events now allows outer scrolling around its independently bounded 320-point
  list. The QA window declares a 480×440 minimum; a resized layout test proves
  that Events and Resume can both fit in the scrolled viewport.
- `MURMUR_DIAGNOSTICS_LIVE=1` enables an opt-in, synthetic-only QA event every
  two seconds. Pending updates coalesce; closed/detached subscriptions stop the
  producer. It adds no per-event live region or production service access.

Rust 1.98.1, fresh `~/Builds/gpui-checkout-66`, Mac mini via opti only:
**121 GPUI tests**, the separate main-thread native Objective-C table-selector
executable (flat/grouped fixtures), **13 runner/native tests**, locked
normal/gallery checks, strict all-target gallery Clippy, rustfmt, TypeScript
and **28 React report tests** pass. The adapter test creates no NSWindow or
AX client and performs no action. No HID, VoiceOver, focus automation,
production data, pasteboard access or benchmark was used.

Goldens completed **30 control cases / 60 captures** with identical pairs on
both attempts, then the unchanged overlay guard aborted with **5 WebKit
images**. The single permitted retry reproduced it; the independent WebKit
investigation retains ownership. No golden baseline was edited. The expected
Diagnostics visual change is the bounded Events viewport and outer overflow;
table cell highlights appear only under keyboard focus. Diagnostics captures
were not reached, so their visual acceptance is not claimed.

The PR notes link the immutable evidence. Checkout 66 and its targets are
removed after evidence preservation. Native interactive/VoiceOver acceptance
and independent Sol review remain with the lead; the PR stays draft/unmerged.

The CI follow-up renames the synthetic helper from `emit_live` to `publish_live`:
the repository reserves every `emit`/`emit_*` identifier for transport adapters.
This changes no behavior or transport boundary. The existing guard is preserved;
the follow-up rechecks it, its regressions, the GPUI suite and native selectors
on the Mac mini. The final PR evidence supersedes the initial repair receipt.

## Native QA repairs (checkout 82)

The focused-control visibility and Run detail keyboard failures are repaired.
**The standard-launch WebKit failure remains a blocking native QA failure.**
No WebKit attribution rule is relaxed. An opt-in native regression now catches
this failure after standard launch, real text focus/typing and orderly quit.

The original load chain is `finishLaunching → _customizeMainMenu →
_addTextInputMenuItems: → NSWritingToolsCoordinator.isWritingToolsAvailable →
WritingToolsUI → WebKit`. A separate activation path is `NSTextInputContext.activate
→ _startDwellingCheck → _shouldSuppressWritingToolsAffordanceWithCompletionHandler:
→ NSCampoLightweightUIController.isEligible`.

The Mac mini still loaded seven unattributed images after trying early menu
`automaticallyInsertsWritingToolsItems = NO`, the GPUI view's
`writingToolsBehavior = none`, optional `NSTextInputTraits` adoption, and the
[custom-input affordance gate used by WebKit](https://commits.webkit.org/315560@main).
Those ineffective hooks were removed; the attempted diff and exact load chains
are retained in the Fleet artifact. A run that loads the same images via AX first
can pass attribution and mask the Writing Tools path, so it does not establish
prevention. The shared editor/initialization shim originates in layer 9
(`d2b38b70`, `app/gpui/src/field.rs` and `app/gpui/native/ax_focus_fix.m`), and GPUI's
native client is in pinned `gpui_macos/src/window.rs` at `654efae4`.
Diagnostics adds no alternate editor. The lead must resolve this remaining
native boundary before accepting the PR; this repair does not whitelist it.

The QA host finalizes its inventory at orderly quit, after startup and input.
Finalization seals the observer; doing that inside the launch callback caused
legitimate later framework loads to terminate the host with exit 86. Headless
captures still finalize at capture completion. The observer, load-chain checks,
startup/linkage baselines and rejection of unattributed Writing Tools loads are
unchanged.

Charts, pickers, text fields, actions and table focus entry share the existing
table reveal calculation. Focus entry reveals the active paused event row inside
its nested viewport. Scrolling is immediate, including under Reduce Motion;
wheel scrolling while focused does not snap back. Run detail entry defers focus
to Back until remount, and Back restores the run action. Both reuse confirmation
focus revalidation and fall back to the selected tab if the run disappears.

For the opt-in standard native launch regression, while owning the Mac GUI:

```sh
python3 scripts/check_gpui_diagnostics_native.py \
  --binary "$CARGO_TARGET_DIR/debug/diagnostics-qa" \
  --output /path/to/new/synthetic-evidence
```

It requires existing Accessibility permission, uses only its exact synthetic
process, types `7` through native events, quits normally and requires zero
unattributed WebKit images. It currently fails with seven unattributed images. It does not change VoiceOver or the pasteboard.
The independent Sol review and final PR CI remain lead gates; no merge is
part of this implementation handoff.
