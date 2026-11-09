# Production GPUI adapters (#996)

Layer 22 adds the `ui` executable and composes the native windows with the same
recording, meeting, delivery, completion, settings and shutdown owners extracted
in layer 20. `production-host` is an explicit feature. Gallery/QA entry points
retain their synthetic adapters; importing a presentation module does not create
production services. Layer 23 supplies the remaining startup/legacy transport and
whole-window scaling integration. Packaging remains in parallel layer 18.

The executable establishes its bundle/home identity and dispatches worker modes
before constructing GPUI. A single Tokio runtime executes service calls. GPUI
entities and AppKit identities stay on main; bounded event queues carry plain
projections. Main and Diagnostics retain their named consumer permissions.

A section subscribes before its first read. Hiding/navigation revokes its lease
and cancels owned panels, scans, previews and onboarding downloads. Durable
settings, spelling/Mode imports, appearance edits and correction confirmations
admit before returning to presentation; their source owner retains publication
across hide. Source comparisons use the native settings revision or exact source
value, rather than an unrelated presentation sequence. A commit notification
waits behind its own original action reply. Download progress additionally checks
the native request ID and monotonic attempt, preserving the original UI ticket.

History row keys disambiguate legacy duplicate IDs without changing durable
entries. Clipboard/export effects check the history content epoch and exact
rows inside the completion FIFO. File imports preserve picker/drop order and
use exact operation IDs for acceptance/cancellation. Knowledge import review
retains bounded bytes and the reviewed SQLite revision, rather than rereading a
mutable path. Bulk operations retain each record's original revision and report
partial failures. Meeting playback uses the existing manifest/range validator;
hide fences output immediately, and shutdown confirms its exact worker teardown
on a blocking coordinator worker before allowing restart.

The host configures #990's updater and the existing meeting-suggestion owner
from committed setup/settings state. Updater indicators, menu actions, notes,
required policy, stage-specific retry and notifications share that owner. Notes
use bounded native Markdown, suppress image/HTML resource loading and open only
explicit allowed web links. Correct & Teach uses the existing proposal and
scope validation, ordered history mutation, knowledge publication and counters.
Its review flow (`host/services/teaching.rs`) only proposes: History changes on
an explicit Save correction only, or after Remember correction commits the rule.
Back discards the pending proposal; a failed commit leaves History untouched, and
a committed rule with a failed edit is reported and retries only the edit. A
specific-term form proposes an exact heard term and replacement for review.
No transcript, audio, clipboard, selection or private capture payload is logged.

## Implementation checks

The implementation worker uses the dedicated Mac checkout/target with Rust
1.98.1. In addition to unit/AX transport tests, `native-host-qa` provides an
explicit `--native-host-contract <receipt>` mode. It is inert unless the bundle
is exactly `com.localdictation.gpui-host-qa110` and the receipt is a direct child
of its synthetic HOME. It constructs the real GPUI application, passively reads
and validates all eleven production projections, records only Boolean results,
and quits through #992. It does not capture input, prompt for permissions,
register login launch, download models, run tools, or access a pasteboard.

The isolated launch exposed and fixed native fullscreen Boolean encoding and
fresh-profile settings normalization. This proves startup/composition, not real
dictation, installed updates, OS permission flows or interactive acceptance.
Shipping Tauri tests remain required and no frontend test is retired.

## Native acceptance checklist

Use a unique isolated test bundle and synthetic data. The separate GUI QA worker
performs interaction/AX checks. Signed installation/identity continuity remains
the final stack's coordinated #994 / Phase 2.4 rehearsal.

| Phase 2 matrix rows | Concrete native flow |
|---|---|
| D09, D11, H04 | Dictate synthetic text, copy a row, exercise clipboard-only/guarded auto-paste and inline retry/Paste Last from overlay, tray and palette. Change recording ownership before a delayed retry; no newer recording or destination may be changed. |
| D12, H06 | Choose an isolated output directory; copy and save the complete filtered History in each format, including expanded/duplicate-ID rows. Cancel the panel and clear History while export is pending; no revoked data may publish. |
| D13 | Enable/disable final NotchPill mirroring using an isolated install fixture. Only final captions reach the existing integration; absence remains a passive status. |
| O01, O09 | Exercise notch/floating placement, expanded quick settings, calibration and Live Words consent. Hide/reopen Main while dictating. Companion windows remain nonactivating/click-through where required and use main-thread Rust geometry. Revoke consent while text is queued: provisional content clears. |
| F01, F02 | Pick or drop several synthetic audio/video files in a known order. Cancel/hide during a pending import and retry. Each accepted file adds one History entry and no dictation statistics. |
| S04, N02 | Visit permissions and Recording settings; change/default/remove an isolated input, deny/grant/reset microphone/AX/System Audio/Calendar access, then return. Passive mounting never prompts or enumerates calendars; only the explicit Calendar visit does. |
| U01, U03 | Navigate every section, open editor destinations and return, bind an app from Modes, invoke palette actions from another section and preserve focus. Test ordinary text editing, IME and application shortcuts in actual fields. |
| M09, M10, M11 | With a synthetic retained meeting, edit labels/metadata, play/seek/change channel, copy/save text/JSON/SRT/VTT and preview/export an audio ZIP. Cancel dialogs and change the selected session while reads are pending. No stale selection may publish. |
| K08, K14, A04 | Import/export synthetic knowledge, spellings, Modes and local themes. Inspect then change source revision/file contents before confirmation; reviewed bytes and owner revisions must prevent silent replacement. Cancel panels; malformed/oversized files preserve current state. |
| K12, K13, P01 | Left-click tray to show Main; right-click and disable/re-enable, retry delivery, cycle Mode, select/clear Next recording and open updates. Open a stale submenu after changing Modes: revision/ID checks reject it. Browser probing is explicit/opted in. |
| A02 | Choose System/Light/Dark, compile/import a palette and change OS appearance. Persistence and native chrome stay coherent; OS changes alone never rewrite the appearance source. |
| P02 | Use App/Edit/Dock menus, About, Settings and Updates. Cut/Copy/Paste/Undo/Redo/Select All target the focused editor. Click an allowed help/source/release link; unsupported schemes never execute. |
| P03 | Move/resize, close/hide, reopen from Dock/tray, toggle fullscreen and relaunch on changed display topology. Valid bounds restore and overlay interaction does not activate hidden Main. Exercise window zoom with IME/hit testing/AX in the completed startup layer. |
| P04 | Quit via menu, shortcut, Dock, tray, exact-test-PID Apple event/signal and updater relaunch with synthetic active capture/meeting/pending writes/playback. Delay owners, confirm responsive main dispatch, one drain and no restart after failed teardown/publication. |
| P05, V01 | Complete setup, use launch/focus/wake checks inside one six-hour interval, then manually check a synthetic update. Header/Settings/tray share status; notifications deduplicate by version. Deny notification permission without a prompt loop; incomplete setup suppresses automatic checks. |
| P06 | Toggle Launch at login, relaunch and compare native registration with UI; cover approval-required and failure rollback. Upgrade verified enabled/disabled synthetic LaunchAgents. Disabled users stay disabled and no duplicate launch authority remains. Unregister and verify from a fresh process. |
| V02, V03, V04 | Exercise optional Later/download/Ready/restart, required Download-or-Quit, signature/policy failure, translocation guidance and stage-specific retry using test-only updater fixtures. Inspect paged cumulative notes and one-time What's New; close/relaunch/dismiss preserves the #990 handoff contract. |
| V05 | Run path equality tests without opening production paths; invoke every worker mode before NSApplication/Metal setup. With #994, rehearse signed Tauri→GPUI→GPUI and the supported downgrade without resetting TCC or data. |
| G03 | Import two compatible synthetic reports through NSOpenPanel, then invalid/oversized/incompatible reports. Comparison remains session-only; cancellation and relaunch do not adopt reports into durable runs. |

Other Phase 3 injection points are production-wired too: Insights/statistics,
model lifecycle, knowledge CRUD/bulk, app overrides/IDE/explicit vocabulary scans,
Calendar suggestions, settings meter/cues and Diagnostics events/runs/recovery/
latency/private review. Explicit Lab/microphone/corpus tool execution requires
its separately authorized validation; no tool starts during subscription/read.
