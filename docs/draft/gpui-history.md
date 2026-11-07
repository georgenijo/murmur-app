# GPUI History contract (#1002)

Layer 13 of #1011, based on `gpui/12-diagnostics` at
`a327c29baa05eeb9510cd33fddd7d4ddcc53c303` (#1030). Draft; never merge
independently. The [Phase 0 exit](../decisions/2026-11-05-gpui-phase-0-exit.md)
selects **native parity first**: each main-window section reaches parity in the
parallel native build before the entire main window switches. There is no
mixed native/WebKit main window, no child-view bridge, and no shipping History
webview to destroy at this step. U01 main navigation/Home belongs to 3.5c;
production host chrome and activation belong to 2.3. This layer provides a
History workspace inside the same `gpui_base::Root` used by prior screens.
No fake navigation/sidebar or second production shell is introduced.

## Owners and integration

`murmur-core::history` owns the bounded read projection, complete-snapshot
queries, Unicode-safe highlights, stable display ordering and explicit export
projection. Existing completion owns retention, pins/caps, exact-target
mutations, durable writes, tombstones, holding/import and statistics. This layer
never publishes a completion or writes statistics. `murmur_ui::history::History`
owns presentation and typed service requests. `history-qa` uses synthetic data
and in-memory clipboard/save adapters only.

Create History inside Root after `murmur_ui::init` on the main thread. Call
`attach`, then `follow` once for that attachment. On hide/navigation away call
`hide(window, cx)` to synchronously revoke leases, close the inbox, stop the
subscription and release row/detail/query text, including the search editor.
Drop also detaches. Reopening uses a new inbox and attachment. A host restart
creates a new screen/model. Main-window geometry stays with Rust/main dispatch.

The Phase 2 host (#994/#996/#997) must implement `history::host::Host`:

1. Subscribe as `Consumer::Main` **before any read**, with listeners fully
   registered before `subscribe` returns. Attachment drains queued notifications
   before issuing its initial ticketed read. Revisions are positive and strictly
   increasing across reads, notifications, errors and reopenings, including
   same-row edits. Capture revisions from the serialized service owner; never
   synthesize ordering from asynchronous completion order. Echo exact tickets.
   Notifications invalidate hydration epochs; rejected same-attachment reads
   retry with fresh tickets. Duplicate notifications/actions do not replay.
2. Publish oldest-first, complete bounded completion snapshots (maximum 200
   entries), with stable nonzero service row keys distinct for duplicate legacy
   IDs. Preserve those keys across pin/edit/reordering; never use timestamp or
   legacy ID alone. `Entry::target` is the exact original completion value used
   for `HistoryMutation`; it is never an export. Enforce the existing 20-pin
   policy again at the core mutation boundary. Core mutations remain authoritative.
3. Carry the service's History clear epoch. Advance it only after successful
   durable clear, including clears from other consumers. Old reads/imports/work
   cannot republish pre-clear data. Map snapshot retention and held-import Retry
   state from the current service; disabling retention preserves existing rows.
4. Offload reads, exports, file panels, clipboard and recovery work. Check
   `Request::lease.is_current()` immediately before each side effect on its
   owning service, along with exact target, snapshot revision and clear epoch.
   A filter/query/data change, another action, hide or drop revokes prior work.
   File-panel cancellation returns `Outcome::Cancelled` and writes nothing.
   Only typed, fixed errors reach presentation; never format transcripts, paths,
   JSON/parser errors or payloads into logs. Backpressure must be retried without
   logging content; terminal inbox closure stops producers. Eight messages and
   192 MiB total inbox bytes are the maximum, with a 96 MiB retained snapshot.
5. Row copy uses the full delivered text. Markdown and plain export use only
   delivered text. Explicit JSON export alone includes raw recognition and the
   whitelisted recording/derived metadata. All exports exclude teaching scope,
   arbitrary unknown fields and audio; include all filtered matches newest first,
   irrespective of visible rows. Use `history::export`, the local timestamp
   renderer, ISO export stamp for JSON and local export stamp for prose. Enforce
   its 8 MiB output limit; save via the existing absolute-path/extension/atomic
   document sink after the native save panel. Suggested names remain
   `murmur-history-YYYY-MM-DD-HHMM.{md,txt,json}`. No path is retained by the view.
6. Pin/Delete use existing completion commands. Clear All uses the durable clear
   operation and import tombstones, not a list edit. Keep exact-target collision
   behavior and do not resurrect legacy entries. Correct & Teach routes only the
   newest stored entry to the existing consent/capability review owner; the full
   knowledge review UI remains 3.5e (K06). No automatic teaching or rule write is
   possible through this interface. File transcription opens the existing picker
   owner (F01); sequential queue display belongs to 3.5c and native panels to 2.3.
   Retry Import goes to the existing migration/holding owner, never a UI importer.
7. Before switching the **whole** main window in Phase 2, release the real legacy
   main webview and every other owner, and verify its destruction. This isolated
   QA process loading zero WebKit images is not evidence of production retirement.
   Keep React/Vitest/visual/backend suites until integrated replacements pass.

The projection admits transcript text up to the completion store's existing
32 MiB document bound rather than imposing a smaller per-row limit. The snapshot
budget includes both exact-target originals and delivered-text projections;
rendering limits remain independent of retained text size. Oversized exports
return a typed failure rather than silently truncating.

## Bounded rendering and accessibility

The list uses GPUI's `uniform_list`, constructing only the viewport's fixed
112pt rows. Each row shapes at most 160 preview scalars. Filtering/search scans
all retained entries before virtualization. Detail renders one 2,048-scalar page;
Previous/Next exposes every part of long text without an unbounded label or text
layout. Page byte boundaries are cached by row key and service revision; redraws
borrow the retained entry and shape only the selected slice, without cloning the
original JSON or rescanning preceding pages. The detail viewport is a Tab stop
with arrow, Page Up/Down, Home/End, Space and Shift-Space scrolling. Viewport
pages overlap by one line; Previous/Next text page resets scrolling to the top.
Copy always includes the full transcript. Detail mode removes the list,
so there is no second transcript layout/cache. Stored originals stay private to
exact-target mutation projection; neither DTOs nor events implement Debug or
automatic serialization.

The List is one keyboard stop with Up/Down, Home/End, Page Up/Down, Enter/Space
copy and Right for detail. Stable row keys preserve selection across updates;
Back/Escape restores list focus and scrolls the selection into view. Visible rows
expose button action, position/set size, preview value and active descendant.
The final regression observed an automatic keyboard-click gap for a focused
Button rendered inside a deferred `uniform_list` callback: AX Focus correctly
changed the real focus handle, but Space did not activate the Button. History
routes Space for its exact owned row-action focus handle and preserves the
shared Button for pointer/AX activation. The bounded handle map survives data
refreshes and is pruned on snapshot changes/hide. This is an observed lower-layer
integration gap, not a vendored fix; no lower-layer source is changed.

Previous/Next transcript controls make offscreen rows reachable with AX actions
without relying on a screen reader synthesizing wheel events. Show more and
other nested controls stop row-copy propagation. Detail exposes named paged text,
copy/Markdown, pin/unpin and Delete; Correct & Teach appears only for the newest
whole-history entry, irrespective of sort/filter state.

Search is always visible, matching the current React History implementation.
Command-F exits detail while preserving list selection and focuses the visible
shared editor; Escape clears search, exits detail,
closes filters or disarms deletion in priority order. Search is case-insensitive
AND matching over delivered text and file names. Combined source/date/pinned
filters have one truthful trigger label and a Show all action; no resting count.
Reopening resets both filter dropdowns and the query, matching React mount
defaults. Date cutoffs use the earliest valid local instant of each calendar
date: midnight gaps advance to the first valid time, and ambiguous midnights
choose the earlier instant. The attached host
refreshes them across midnight/resume. Unicode fold expansions map back to original
scalar boundaries; at most 200 merged ranges are highlighted per visible text.

Deletion uses two discrete activations and a four-second deadline. The second
activation must retain the same query/data/attachment ticket and exact key set;
query, matches, data, epoch, hide and Escape disarm it. Confirmation is an inline,
named group with explicit Cancel and Confirm; Return is never a destructive
default. Copy/saved/failure status text is fixed and expires; the existing native
announcement bridge speaks only those content-free labels. No transcript is
announced automatically. Native VoiceOver speech/action acceptance remains a lead
gate, separate from emitted AX tree/action tests.

Controls reuse shared Button, TextField, Choices, Toggle, Progress, semantic
Palette, system typography, 8pt controls, 12pt cards, 5pt list gaps and 8pt row
insets. Toolbars wrap at compact sizes. Appearance follows the existing theme
and display preferences without rewriting them during render. Surfaces are
opaque, including Reduce Transparency; there are no screen transitions or
perpetual animations, and the shared Toggle honors Reduce Motion.

## Matrix and acceptance

| Rows | Automated coverage in this layer | Integrated native lead acceptance |
|---|---|---|
| H01 / R02 | Revision/attachment/clear rejection, subscribe-before-read, listener teardown, retained-entry preference notice and held-import Retry. Existing core retention/crash fixtures remain. | Hidden-main dictation/import exactly once; retention-off/relaunch; held v1 retry/clear races with isolated data. |
| H02 | Complete AND search, combined filters, Unicode highlights, date boundaries, Command-F/Escape, AX field/filter labels. | Keyboard/IME/VoiceOver editing; midnight/resume/time-zone change; live appearance and reduced preferences. |
| H03 | Real virtualized first/last navigation; stable keys, bounded paint count, core pin gate. | Native wheel/keyboard/VoiceOver scrolling and focus retention; 200-entry pin rollover. |
| H04 | Full-text fake copy from bounded preview; nested detail action isolation, paged text, AX status/detail. | Pointer/key/AX copy to disposable clipboard adapter, VoiceOver announcement and nested actions. |
| H05 | Timeout and scope-change disarming, clear epochs, current leases and synthetic clear. Core tombstone tests remain. | Native two-activation single/shown/all delete; cancel/timeout/scope changes; clear and relaunch without resurrection. |
| H06 | Complete 200-entry JSON/Markdown/plain exports; exclusion/ordering/bounds; fake save/cancel. | Native file panel cancel, real atomic document sink with synthetic files, clipboard adapter and truthful AX format labels. |
| F01 / K06 | Typed file-picker and newest-entry teaching entry points only. | Host panels/queue and full teaching consent review in their assigned phases. |

Verification commands run only on the Mac mini in fresh checkout 71, via Fleet
through opti with `RUSTUP_TOOLCHAIN=1.98.1`. Native GUI input, AX actions/focus
changes on the shared desktop, production application/profile/pasteboard, and
memory measurements are prohibited for this worker. Windowless GPUI tests may
exercise synthetic key and AX dispatch; offscreen Metal captures never open a
visible window. Any own-window native observation uses `focus:false`, a
synthetic title, read-only AX queries and `screencapture -l` only.

Verification results and immutable synthetic evidence are recorded in the PR.
Independent Sol high review and native interactive/VoiceOver acceptance remain
with the lead. This implementation sub-agent starts no delegates.

## Pending lead measurement and Checkpoint 4

**No memory benchmark or Checkpoint 4 measurement is run by this layer.** An
explicit request from George is required. The eventual measurement uses the
plan's method and frozen Phase 0 budgets: main-window idle **≤90 MiB**, peak
History scrolling **≤130 MiB**, provisional until Retina validation.

On the same authorized Mac/display/OS/scale, compare matched release builds at
the default main-window size with 200 synthetic entries, fixed unloaded/warm
model states reported separately. Sample process `phys_footprint` for core, UI
and helpers separately at idle, settled-after-hide and peak during repeated
complete History scrolls; repeat runs and report median and spread, retaining
process inventories to avoid counting hidden WebKit processes as removed.
Record build revisions, OS/display/scale, dimensions, synthetic fixture, sampler
cadence and exact scroll sequence. No production History/profile or audio is
needed. Use the same sampling/scroll procedure for baseline and candidate.

For the selected **whole-window native-parity route**, Checkpoint 4 is a
re-estimate using measured porting rate, not a child-view ship/continue gate.
The lead records elapsed implementation/verification effort and revises the
remaining 3.5 estimate; the window cannot flip until every section reaches
parity. No continuation decision or satisfied memory budget is claimed here.
