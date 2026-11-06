# Plan: Replace the Tauri + React UI with GPUI

- Status: Draft, revision 13 (after twelve rounds of Sol, Astra, and Opus review)
- Date: 2026-11-05
- Base: `main` at `ed756336` (0.62.0)

## Goal

Replace Murmur's WebKit UI and Tauri host with one native GPUI UI process.
Keep the transcription, audio, and meeting backends, and keep the isolated
helper processes (capture worker, capture helper, diarization and Core ML
workers) as they are.

Success means:

- UI memory falls substantially. The 2026-11-05 MacBook investigation
  (artifact `art_20261105T161840Z_a187506793`) measured about 490 MB idle
  across the core and the WebKit processes (four WebContent processes plus
  the shared GPU and Networking processes), with the main window peaking at
  474 MiB after scrolling History. The rewrite estimate is 80–120 MB idle and
  120–200 MB with the main window open. These are provisional budgets until
  Phase 0 measures them and freezes the numbers.
- No user-visible regression in dictation, the overlay, VoiceOver, keyboard
  use, updates, permissions, or saved data.
- Every phase ships independently and `main` stays releasable. Each
  checkpoint below is also a valid place to stop with a shipped product.

## GPUI facts this plan relies on

Verified by all three reviewers against Zed revision
`654efae441bc8eb117d0042bb28cd7cf3570bdc9`:

- **Use a pinned Git revision, not crates.io.** The published `gpui` 0.2.2
  (October 2025) has no AccessKit integration and no embedding API. AccessKit
  landed on Zed `main` in zed-industries/zed#56065 (2026-05-27). Building on
  0.2.2 would block the accessibility requirement. Pin `gpui`,
  `gpui_platform`, and `gpui_macos` to one Git revision, commit `Cargo.lock`,
  mirror or vendor the crates, and record the Rust toolchain (Zed pins 1.98.1;
  Murmur CI pins 1.96.0) and the Xcode Metal toolchain needed by
  `gpui_apple/build.rs`.
- **Accessibility exists but is unproven for Murmur.** Current GPUI exposes
  AccessKit roles, labels, values, toggled/expanded state, and actions. It has
  no live-region API (the overlay relies on `aria-live`), and open issues
  #65623, #65740, and #65111 affect node consistency and focused-text-field
  reporting. GPUI attaches an AccessKit adapter to each window
  (`gpui_macos/src/window.rs:2415`) that overrides `accessibilityChildren`,
  `accessibilityFocusedUIElement`, and `accessibilityHitTest`, so a webview
  placed inside a GPUI window may be invisible to VoiceOver. GPUI's own tree
  updates depend on that adapter, so turning it off would hide the native
  controls instead. Exposing a webview as a child of the GPUI window likely
  means patching `accesskit_macos`'s subclassing adapter
  (`accessibilityChildren`, `accessibilityHitTest`), which Murmur would then
  maintain as a fork until upstream accepts it.
- **Overlay window primitives exist.** `WindowKind::PopUp` creates a
  non-activating `NSPanel` at popup level with `CanJoinAllSpaces |
  FullScreenAuxiliary`. Transparent backgrounds work, and `HasWindowHandle`
  returns the `NSView` (window via `[view window]`). Gaps: the panel returns
  YES from `canBecomeKeyWindow`; built-in blur uses `UnderWindowBackground`,
  not the `.hudWindow` material required by the 2026-11-02 decision; inactive
  windows are throttled to about 30 fps by default.
- **Run loop.** Stock GPUI and Tauri (tao) each install their own
  `NSApplication` delegate and run the loop, so they cannot simply share a
  process. `Application::with_platform` and `run_embedded` exist, but
  `run_embedded` only works with a platform whose run call returns
  immediately (`gpui/src/app.rs:331-339`), and the stock macOS platform does
  not. Embedding therefore means writing a macOS platform that shares tao's
  `NSApplication` and delegate. Phase 0c tests whether that is practical.
- **Quit.** GPUI gives asynchronous quit handlers 200 ms
  (`SHUTDOWN_TIMEOUT`, `gpui/src/app.rs:1274-1310`), and quitting cannot be
  cancelled once those handlers run. Murmur's meeting shutdown can wait up to
  5 s (`meeting_capture.rs:438`), so teardown must finish before GPUI's quit
  begins (step 1.8). Today's Tauri host cannot hold a quit either: tao 0.35.3
  implements only `applicationWillTerminate:`
  (`platform_impl/macos/app_delegate.rs:62-64`), `RunEvent::ExitRequested`
  fires only for `app.exit()` and last-window close, and the default menu's
  Quit sends `terminate:`. ⌘Q, Dock Quit, and logout therefore go straight to
  `RunEvent::Exit`, which blocks the main thread on the meeting wait.
- **Licensing.** GPUI is Apache-2.0. Zed's `ui`, `editor`, and `ui_input`
  crates are GPL-3.0-or-later and must not be copied into MIT-licensed
  Murmur. Controls come from GPUI primitives or the Apache-2.0
  `gpui-component` library (which constrains the GPUI pin).

**Fallback:** if Phase 0 fails on accessibility or native windowing, build
the UI in SwiftUI/AppKit on the same Rust core (UniFFI or swift-bridge).
Phase 1 is needed for either path.

## Invariants every phase must preserve

From `CLAUDE.md`, with acceptance tests carried across each extraction:

- Resolve recording configuration once at start; recheck the recording
  generation before any async continuation (including GPUI `cx.spawn`)
  mutates shared state; stop only the owning generation
  (`commands/recording.rs`).
- Exactly-once transcript publication and exactly-once History and
  statistics writes per recording, ordered transcript transforms behind the
  single pipeline entry point, and refusal of unprovable secure-field access.
- Rust owns native window geometry. Core code off the main thread reaches
  windows only through a main-thread dispatcher; the main thread never blocks
  on inference, teardown, or the rdev thread.
- Start rdev once, with `set_is_main_thread(false)` before the listener
  (`keyboard.rs`).
- No transcript, audio, clipboard, or selection content in logs, bridge
  traces, goldens, or recordings. Test data is synthetic.
- `retainHistory=false`, Clear History, and export exclusions enforced at the
  core boundary, including every copy of the data (disk files and any WebKit
  storage).
- Every quit path drains capture, stops helpers, and flushes durable state
  before the process exits.

## Measurement method

Used for the Phase 0 gate and at every phase boundary:

- Same machine, OS, and display for baseline and candidate: Retina display,
  release builds, default main-window size.
- Fixed model state (unloaded and warm, reported separately), synthetic
  History of 200 entries.
- Report idle, settled-after-hide, and peak-after-History-scroll
  `phys_footprint`, with UI processes reported separately from the core and
  helpers. Repeat runs; report the median and the spread.
- Report frame interval (median and p95), dropped frames, and CPU for overlay
  animation with Murmur inactive and in Low Power Mode, compared with today's
  overlay measured the same way.

## Sizing and checkpoints

Planning estimates for one owner directing AI coding agents. They will be
recalibrated at each checkpoint, not treated as commitments.

| Phase | Scope | Size today | Estimate |
|---|---|---|---|
| 0 | Five prototypes, two stages | Scratch | 4–5 weeks |
| 1 | Core extraction, completion service, state, updater, shutdown | 178 command attributes, 93 emit sites, 23 `localStorage` files | 8–12 weeks |
| 2 | Packaging, host, bridge (host-flip path) | 172 commands behind the bridge | 4–6 weeks (2–3 if embedded) |
| 3.1–3.4 | Components, overlay, preview, diagnostics | UI kit 3.9k, overlay 1.0k, preview 0.1k, log viewer 3.5k TS lines | 6–9 weeks |
| 3.5 | Main window, section by section | History 2.1k, Settings 9.7k, onboarding 0.6k, home 0.5k, plus 23.7k of shared `lib/` | 12–18 weeks, plus 2–3 for the bridge in the embedded route with child views |
| 4 | Delete WebKit, Tauri, React | — | 1–2 weeks; 4–6 in the embedded routes, which do the 2.3 host replacements and the host change here |

About 49k lines of non-test TypeScript in total.

Checkpoints, each with a written stop/continue decision in
`docs/decisions/`:

1. **After Phase 0 stage A:** GPUI or the SwiftUI fallback.
2. **After Phase 0:** the migration route from the route table; frozen
   budgets and thresholds.
3. **After Phase 1:** re-estimate. Phase 1 alone is worth shipping (headless
   dictation, smaller hidden-window cost, Rust-owned data).
4. **After the History port (3.5a):** in the child-view route, continue only
   if History meets its memory number and the measured porting rate keeps the
   remaining estimate within 1.5× of this table; otherwise stop at the hybrid
   state (native overlay, preview, diagnostics, and History, with the
   remaining sections in lazily created webviews). In the whole-window routes
   (embedded whole main window, native parity first), native History is an
   internal milestone that does not ship, so checkpoint 4 is a re-estimate
   from the measured porting rate. Stopping in the embedded whole-window
   route leaves native overlay, preview, and diagnostics with today's React
   main window. Stopping in the native-parity route leaves the shipping
   Phase 1 Tauri app; the native work stays internal.

## Phase 0: Feasibility (two stages, four to five weeks, scratch)

Built and run on the Mac mini, outside the repository. Output: a decision
record in `docs/decisions/` with measurements, screenshots, and go/no-go
results for each item.

### Stage A: can GPUI do Murmur's UI? (two weeks)

**0a. Overlay prototype (up to one week).** The real 120×34 pill states, red
recording dot, Live Words card, and a waveform fed at the real 16 ms
`audio-level` rate (coalesce redraws), the pulsing dot, and the processing
spinner. Pass criteria:

| Area | Criterion |
|---|---|
| Focus | After clicking every overlay control, dictation still pastes into the previously focused app and the overlay never becomes key (swizzle `canBecomeKeyWindow` or add an upstream option; keep `_setPreventsActivation:`) |
| Placement | Notch and pill placements, all Spaces, over full-screen apps, display and Dock changes, Live Words click-through |
| Material | A `.hudWindow` behind-window `NSVisualEffectView` added through the raw view, refit in the same main-thread turn as each resize; Reduce Transparency honored |
| Announcements | VoiceOver announces state changes from the non-activating overlay (AccessKit or `NSAccessibilityAnnouncementRequestedNotification`) |
| Motion | While recording into another app (Murmur inactive), p95 frame interval no worse than today's overlay and CPU lower. GPUI's default inactive-window throttle (about 30 fps) is acceptable if it passes; the `audio-level` rate may drop to match. Low Power Mode reported; Reduce Motion honored |
| Memory | Measured with the method above; provisional budget 60 MiB idle |

**0b. Settings and History prototype (up to one week).** Toggles, a text
field, a dropdown, a shortcut recorder, and a virtualized 200-entry synthetic
list. Pass criteria: full keyboard and VoiceOver operation with correct roles
and labels; text field reported as focused to assistive and dictation tools;
IME, selection, copy/paste, and undo; focus restoration; provisional budget
90 MiB with both windows open. Decide here between `gpui-component` and
in-house controls.

**0e. Build, CI, and quit path (in parallel, up to three days).** A pinned
GPUI build on CI runners, headless macOS screenshot capture for GPUI goldens,
release build size, and a quit interception check. Menu Quit, ⌘Q, Dock Quit,
logout, and a programmatic relaunch each hold termination for at least 6 s
while an artificial delayed teardown runs (for example
`applicationShouldTerminate:` returning `NSTerminateLater`, then
`replyToApplicationShouldTerminate:`), before GPUI's quit handlers start.
Main-thread dispatch keeps running while AppKit waits for the reply.

**Stage A go/no-go:** 0a, 0b, and 0e pass every criterion. A failed
criterion counts as passed only when its fix (an upstream patch or a small
local platform change) is implemented and the criterion passes on a rerun.
Otherwise take the SwiftUI fallback.

### Stage B: which migration path? (about two weeks; 0c and 0d run in parallel)

**0c. Embedded GPUI under Tauri (up to one week, stop rule).** A macOS
platform for GPUI that shares tao's `NSApplication` and delegate, running one
GPUI window inside the current Tauri process: executor integration,
application delegate, teardown, and accessibility. Stop rule: if by the end
of the week a GPUI window does not render, take input, pass VoiceOver, and
tear down cleanly inside the Tauri process, stop and take the host flip.

**0d. React inside a GPUI window (up to one and a half weeks).** One real
production React screen with a text field running in a wry webview inside a
GPUI window, served from a `tauri://localhost` custom protocol. The scope
includes a minimal invoke and event shim for that screen (the full shim is
built in 2.2). Pass criteria:

- In one window that holds both native GPUI controls and the webview,
  VoiceOver traverses and operates both. If AccessKit hides the webview, the
  fix is to expose the webview as an accessibility child of the GPUI window.
  Turning off the AccessKit adapter is not acceptable, because it hides the
  native controls.
- Tab and first-responder focus move between GPUI controls and the webview
  in both directions.
- ⌘C, ⌘V, ⌘X, ⌘Z, and Select All work in React text fields under GPUI menus.
- IME and dictation input work in the React text field.

Every 0d criterion gates the bridge. As in Stage A, a failed criterion
counts as passed only when its fix is implemented and the criterion passes on
a rerun; the expected VoiceOver fix is the `accesskit_macos` patch above, and
its maintenance cost goes into the 2.2 estimate. Otherwise React cannot run
inside GPUI windows without a regression.

**Route table.** Phase 0 exit picks the migration route from the 0c and 0d
results:

| 0c embedding | 0d React in GPUI | Route |
|---|---|---|
| Pass | Pass | **Embedded with child views.** Port screens one at a time inside the Tauri app; the native main-window shell hosts unported sections as wry child views through a bridge scoped to the main window |
| Pass | Fail | **Embedded, whole main window.** Port the overlay, preview, and diagnostics inside the Tauri app; the main window stays a Tauri window until every section is native, then switches in one release |
| Fail | Pass | **Host flip with the bridge.** Phase 2 as written |
| Fail | Fail | **Native parity first, or the fallback.** Build every window natively in a GPUI build developed beside the shipping Tauri app, then flip once (Phase 3 before Phase 2, re-estimated, no hybrid checkpoint); or take the SwiftUI fallback. Decided at Phase 0 exit |

**Phase 0 exit** freezes, in the decision record:

- Memory budgets per window and per phase boundary, as numbers.
- Overlay frame-interval and CPU thresholds from 0a.
- Supported macOS versions to test (the macOS 14 minimum through current).
- The route from the route table.
- Controls choice.
- The main-window port strategy (default in 3.5).

## Phase 1: Prepare the core (shipped in the current Tauri app)

Each step is a normal PR with Sol review and native verification. Ship
mechanical moves separately from behavior changes.

**1.0 Feature and ownership matrix.** Before moving code, write
`docs/draft/gpui-feature-matrix.md` from `docs/FEATURES.md`. One row per
user-facing behavior, with: owning module after Phase 1, the phase and step
that moves it, prerequisites, a native acceptance flow (the clicks, keys, and
expected result), the existing tests that cover it, and the GPUI tests that
will replace them. Cover at least dictation, the overlay, Live Words, the
preview, History, statistics, Settings, onboarding, the command palette,
meeting capture, review, playback (Web Audio today, needs a native
replacement), export, calendar and diarization, file import and drop,
Insights, model management, knowledge and modes, themes (including Open VSX
fetching and compilation), recovery, sound cues, the tray, menus,
notifications, autostart, and update UI. Later phase exits are defined by
this matrix.

**1.1 Dependency map and core crate.** Map transitive dependencies before
moving code: the "Tauri-free" modules still reach Tauri through
`resource_monitor` (transcribers), `crate::audio` (`vad.rs`), and
`model_digest` (`smart_turn.rs`); `audio.rs`, `audio_lifecycle.rs`, and
`meeting_capture.rs` use managed state, lifecycle callbacks, and window
lookups, not just events. Extract leaves first (RMS/peak math, RSS and heap
readers, model artifact and digest checks), then introduce an `AppContext`
passed to the core in place of `AppHandle::state`, with adapters for paths,
permissions, background work, and main-thread dispatch. Decide who owns the
Tokio runtime. The `#[global_allocator]` and the FoundationModels Swift build
(`build.rs`) stay in the binary crate.

**1.2 Dictation control in the core.** Move hotkey handling (hold,
double-tap, combined, escape), silence and Smart Turn auto-stop, Paste Last,
and launch-gate arming out of React hooks (`App.tsx:282-296`,
`useSilenceAutoStop.ts`) into the core.

**1.3 Completion service in the core.** History entries, completion
de-duplication, and statistics updates run in the main window's
`transcription-complete` listener today (`useRecordingState.ts:421-447`,
`useHistoryManagement.ts:24`, `stats.ts:132`); file transcription adds
entries through the same `addEntry` path (`useFileTranscription.ts`); and
activity counters for meetings, corrections, and Paste Last are written to
the same statistics file from `useLocalStats.ts:19-47` through
`stats.ts:177`. Move all of them into one host-independent Rust service that
owns the History and statistics files, keeping today's semantics:

- `retainHistory=false` stops new History entries only; existing entries
  stay, and content-free usage statistics continue
  (`docs/features/history-workspace.md:26`).
- The statistics file has two kinds of counter. **Dictation counters**
  (totals, words-per-minute samples, day buckets, recordings by mode) change
  only on live dictation (`useRecordingState.ts:446` is the only
  `updateStats` caller), in the bucket of the completion's local day
  (`stats.ts:147`). **Activity counters** (`activityStats.ts:99-135`) change
  on a finished meeting (lifetime count and minutes, plus that month's
  bucket), a correction proposed or taught (lifetime counts; taught also
  counts by scope), and a Paste Last delivery (lifetime count). Corrections
  are sent from the review window by `correctAndTeach.ts` through
  `localStatsEvents.ts:28`. In the core, each producer submits its event
  once at the source, so the React de-duplication sets
  (`useLocalStats.ts:10-12`) go away. The `transform` kind has no producer
  today.
- File transcriptions add History entries with no statistics
  (`useFileTranscription.ts:18`), and derived entries (`history.ts:55`) are
  not completions.

**Write protocol.** Recovery uses a small **statistics journal** with no
transcript text, instead of wall-clock time (recording ids restart at zero
on each launch, `state.rs:593`, and legacy History ids are bare millisecond
timestamps that can collide, `history.ts:63-65`; ids from current builds add
a per-session suffix, `history.ts:153-156`). One serialized task handles every
History write, statistics event, clear, reset, and legacy import. The journal
lives in a small **state file** together with the next sequence number. On
its first start the service creates the state file (next sequence 1, empty
journal); until that write succeeds it accepts no writes and arming waits,
with a visible error and a content-free diagnostic, as for migration.

Every durable write in this section, to the state file, the holding file
(below), the Rust-owned migration file (1.5), or a destination file, is
atomic: write a temporary file, `F_FULLFSYNC` it, rename it over the old
file, then fsync the directory. A torn write cannot be observed. If a step
fails after the rename (for example, the directory fsync fails), its outcome
is unknown. Before retrying, the task rereads the published file and
continues from what it holds (for statistics, its `applied_through`; for the
migration file, its committed steps). Such a write does not count as durable
until a later directory fsync succeeds: the task retries the fsync, or
republishes the file it reread, before it compacts the journal, drains the
holding file, or removes a WebKit key on the strength of that write.

The migration file holds only per-version and per-step migration state
(1.5). It is not the settings file, because React saves whole settings
snapshots (`settings.ts:736`) and Rust replaces the settings file with them
(`settings_store.rs:232-238`), so a stale snapshot could erase state kept
there. React never writes the migration file. Losing it only reruns steps
that are safe to rerun (1.5).

**Valid destinations.** Today's checks (`settings_store.rs:88-170`) cover
only size, JSON syntax, and the outer shape, so `[null]` passes as History
even though the History page cannot load it (`history.ts:94-104`, `:107`). A
destination file (History, statistics, benchmark reports) is *valid* only if
it also passes a destination-specific schema check: a supported version and
every entry, statistics field, or report well formed. Supported versions
include today's unversioned statistics and legacy History entries, which load
through today's migration (`history.ts:107-115`). Anything else is
*unusable*: it is quarantined as today, leaving the destination *missing*.

Once v1 has committed or a destination carries its "v1 imported" marker
(below), each completion runs these steps for it, and every write to that
destination carries the marker:

1. **Commit.** For a statistics event, take the next sequence number and
   append a versioned journal record holding the sequence, a UUID, and the
   event's full typed inputs:

   | Event | Inputs |
   |---|---|
   | Dictation | word count, duration, mode, local day |
   | Meeting finished | duration, local month |
   | Correction proposed | — |
   | Correction taught | scope |
   | Paste Last | — |

2. **History.** For dictation with History retained, write the History
   entry, carrying the UUID, atomically. File transcriptions write only this
   step, which is their commit point. If this write fails, the statistics
   still apply, no History entry exists, and a content-free diagnostic is
   recorded.
3. **Statistics.** Apply **every** journal record above `applied_through`,
   in sequence order, and set `applied_through` to the last one, in one
   atomic write. If the write fails, nothing advances and the records stay
   for the next run of this step. The same step runs at launch.
4. **Compact.** Delete journal records at or below `applied_through`.

Since `applied_through` advances only by applying every earlier record, no
record can be skipped, and clock changes or equal timestamps do not matter.
Deleting entries, the 200-entry cap, and `retainHistory=false` do not affect
statistics, because statistics read the journal, not History. Imported
History and statistics have no sequence numbers and are never replayed.

**Holding new data until v1.** The v1 import (below) judges each destination
by the file the user had before the upgrade. So while v1 is uncommitted and a
destination lacks its marker, the service never creates that file, and new
data waits:

- Step 1 runs as usual. Statistics records stay in the journal: step 3 does
  not run for a held destination, so nothing is compacted.
- New History entries and benchmark reports go to a Rust-owned **holding
  file** in the app data directory instead of step 2, each with its UUID.
  The holding file keeps one list per destination, each with a generation
  id.
- Edits, pins, and deletes of held entries rewrite the holding file. Edits,
  pins, and deletes of entries in a valid original History file change that
  file in place, and each deleted entry also adds a record (its id and a
  digest of its text) to a deletion list in the holding file. Legacy ids can
  collide (`history.ts:63-65`) and today's delete removes exact entries
  (`history.ts:167-173`), so each record skips exactly one legacy entry with
  the same id and text in a later merge.
- Held data follows today's limits on the combined view: 200 History
  entries with the newest 20 pinned kept (`history.ts:59-90`), evicting from
  the held list first and then from the original file in place, and the
  newest 10 benchmark reports. Statistics records carry no content, so the
  journal is left unbounded while held.
- Views read the destination plus held data: History shows held entries
  above the file's entries, and statistics show the file's counters with the
  pending records applied in memory.
- Each launch that still holds data records a content-free diagnostic with
  the held counts. From the third such launch, Murmur shows a visible notice
  with a Retry action. It never discards the legacy data on its own, so the
  hold ends only when v1 commits.

So at import time a destination file is either the user's original (perhaps
edited in place, deleted, or corrupted) or a file that already carries its
marker. A file created by new activity cannot be mistaken for the original.

**Clear and reset.** Clear History, statistics reset, and clearing benchmark
reports are each one atomic write to the destination file, which is the
commit point. It writes an explicit empty file (History, reports) or empty
counters with `applied_through` moved to the latest sequence (statistics),
with a "cleared at" or "reset at" time; marks the destination's held list
generation as discarded; and, before v1, adds the destination's marker,
because there is nothing left to import. The task then replaces that held
list with an empty one under a new generation and removes the destination's
legacy WebKit keys (1.5), so a cleared copy does not wait in WebKit for the
other destinations. At launch, before anything drains, a held list whose
generation its destination marks as discarded is dropped, so a crash between
the two writes cannot bring cleared entries back. The clear lives only in the
destination file, so losing the migration file cannot undo it.

If the state file is missing or unusable after its first creation, the
service recreates it with the next sequence number after the statistics
file's `applied_through` (1 if it has none) and an empty journal. Pending
records are lost, and their number cannot be known; a content-free
diagnostic records the loss. Their sequence numbers may be reused, which is
harmless because they were never applied and only `applied_through` is
compared. If the holding file is missing or unusable, the service recreates
it empty, and held entries, reports, and the deletion list are lost with a
diagnostic. The v1 decision reads only destination files, so neither loss
changes how v1 merges.

React stops caching History and statistics in `localStorage`
(`durableUserData.ts:80-87`, `history.ts:115-118`) and reads them through
commands, so no transcript copy is written to WebKit storage after this step.
The service emits a "History changed" event that React only reads. History
edits, pins, deletes, Clear History, statistics reset, and clearing benchmark
reports become core commands.

**Migration comes first.** 1.3 ships with migration version 1 (1.5), which
moves any WebKit-only History, statistics, and benchmark reports to disk and
removes the transcript-bearing keys. Dictation arming waits for that commit,
as today's launch gate waits for hydration (`durableUserData.ts:110-139`).
There is nothing to wait for when v1 is already committed or no WebKit data
directory exists. If the main webview has not committed v1 within a timeout,
Murmur reloads it once, then arms anyway and records a content-free
diagnostic. New data is held until v1 commits, even on a later launch. When
no WebKit data directory exists (a fresh install, or a profile whose store
was retired), v1 commits at once with an empty source in one migration-file
write; any destination file that already exists is kept as it is.

**The v1 import** decides for each destination from that file alone, at
import time:

- **Carries its marker:** no legacy merge. It was imported or cleared
  already; a held list left over from that write is handled at launch
  (below).
- **Valid:** disk wins, and nothing is taken from WebKit. Today every save is
  mirrored to disk and hydration copies WebKit-only data to disk
  (`durableUserData.ts:80-92`, `:121-127`), so on most 0.62 profiles the file
  already holds the WebKit data.
- **Missing or unusable:** the WebKit copy is the recovery copy. Benchmark
  reports are WebKit-only today (`benchmark.ts:264-272`), so their file is
  always missing at first import.
  - History: legacy entries are added, each deletion record skipping one
    matching entry.
  - Statistics: legacy values fill the empty counters.
  - Benchmark reports: legacy reports are added.

The same atomic write then adds the held data, marks the held list's
generation as consumed, and adds the destination's marker:

- History: held entries go above the file's entries (an entry whose id and
  text match is skipped, and a colliding id with different text gets a new
  id), then the 200-entry cap applies.
- Statistics: every journal record above `applied_through` (0 for a file
  without one) is applied in order, and `applied_through` advances.
  Words-per-minute samples are ordered legacy first, then new, and the newest
  100 are kept (`stats.ts:84`).
- Benchmark reports: held reports are merged by creation time, skipping any
  with the same creation time and content, and the newest 10 are kept.

Once that write is durable, the destination's held list is replaced with an
empty one under a new generation, and the journal is compacted. At launch,
before any window shows data, a held list whose generation its destination
marks as consumed or discarded is dropped, never merged. So a crash or a
failed holding-file write after the import adds nothing twice, and an
imported entry the user deletes or the cap evicts afterwards cannot return.
Then the destination's legacy keys are removed. v1 commits in the migration file once all three destinations carry
the marker.

Legacy data reaches a destination only when its file is missing or unusable,
and new activity never creates a destination before its marker, so nothing
is counted twice and nothing recoverable is discarded. Two accepted losses
remain, each needing two failures before v1 commits: if the holding file and
the History file are both lost, an entry deleted before v1 can return from
the legacy copy; and if a destination that carries its marker is lost before
its legacy keys are removed, the next import merges the legacy copy into it,
which can undo a clear made in that window. Key removal follows the marker
at once to keep that window short.

**Defined outcomes and tests.** Each row is a test; the exit check below
runs all of them.

| Case | Defined outcome |
|---|---|
| Crash after the journal append, before History | No History entry for that completion (the text was already delivered); statistics are replayed at launch |
| Crash after History, before statistics | Statistics are replayed at launch; History is not duplicated |
| Crash after statistics, before compaction | Nothing is applied twice |
| A meeting, a proposed correction, a taught correction, and a Paste Last, each with a crash before statistics | Lifetime and monthly meeting counts and minutes, proposed and taught counts, taught-by-scope counts, and the Paste Last count all match a run with no crash |
| Clock moved backwards; equal timestamps | Same counters as with a steady clock |
| Statistics write for event N fails, then event N+1 commits | Both applied exactly once, in order |
| History write fails | Statistics applied, no History entry, one diagnostic |
| Statistics rename succeeds but the directory fsync fails, then retry | Reread the published file; each record applied once; no compaction until a directory fsync succeeds |
| The same, with power lost before the retry | After relaunch, no journal record is lost and each is applied once |
| Statistics reset with records pending | Empty counters, `applied_through` moved to the latest sequence, "reset at" set, in one write; pending records are not reapplied |
| Clear History; clear benchmark reports | An explicit empty file with a "cleared at" time instead of today's file deletion (`commands/settings_store.rs:327`) |
| Entry deletion; the 200-entry cap; `retainHistory=false` | Statistics unchanged |
| State-file creation fails on first start | No History or statistics writes and no arming until it succeeds; visible error and diagnostic |
| State file deleted or corrupted, before and after v1 commits | Recreated; pending records lost with a diagnostic; v1 decisions unchanged |
| Holding file deleted or corrupted before v1 commits | Recreated empty; held data lost with a diagnostic; v1 decisions unchanged |
| Stale React settings save while records are pending, then a restart | Migration state and pending records unaffected |
| Shape-valid but malformed disk History (for example `[null]`) or an unsupported version, with a valid WebKit copy | *Unusable*, quarantined; the WebKit copy is merged |
| Unversioned statistics and legacy History entries on disk | *Valid* |
| Late v1 import with disk and WebKit holding the same data | Only the held data and the marker are added |
| Late v1 import with WebKit-only data | Legacy data merged once, below the held data |
| Valid disk statistics, migration timeout, new dictation, statistics file corrupted, relaunch, more dictation and a meeting, late v1 import | No statistics file exists before the import; the result holds the legacy totals plus every post-upgrade event, each once |
| The same sequence for History | Legacy entries below every held entry, each once |
| Timeout on first launch, new dictation, quit, v1 commits on the next launch | Held data added once; WebKit-only legacy data kept |
| Clear History, statistics reset, or clearing reports, before a late v1 import | Stays in effect; the import skips that destination; its legacy keys are absent after relaunch |
| Crash after the clear's destination write, before the holding-file write | Held entries from before the clear are dropped at launch; later ones kept |
| A clear before v1, then the migration file moved aside | The clear stays in effect |
| Delete a held entry and an entry of the original History file, then corrupt the History file before a late v1 import | Neither deleted entry returns |
| Crash between a destination's import write and draining the holding file | Each held entry, report, and record added once |
| Import write commits, the holding-file write fails, the user deletes an imported entry, relaunch | The deleted entry does not return |
| Two legacy entries share an id; the user deletes one before v1; History file corrupted; late v1 import | Only the deleted entry is skipped; the other is recovered |
| Repeated timeouts while new dictation and reports accumulate | Combined History stays within 200 entries and 20 pins, reports within 10; a notice with Retry from the third launch |
| Fresh install with no WebKit data directory; relaunch | v1 commits at once; nothing is held |
| Migration file moved aside after the store is retired | v1 commits again with an empty source; destinations unchanged |
| Crash between a destination's import write and the v1 commit | The retry skips that destination |
| Migration file moved aside before or after v1 commits | Steps rerun with the same outcome; nothing is imported twice |
| Colliding legacy and new History ids | Both entries kept |
| Killed WebContent process during v1 | Reload once, then arm with a diagnostic; new data is held |

Exit check for 1.2 and 1.3: a test build that never creates the main
webview, on a profile whose v1 migration is committed, can arm, start, stop,
cancel, auto-stop, and paste. Each completed recording produces exactly one
durable History entry (none with `retainHistory=false`) and exactly one
statistics update (also with `retainHistory=false`); each file transcription
produces one History entry and no statistics update; a finished meeting, a
taught correction from the review window, and a Paste Last each add exactly
one activity update. Opening, reloading, or reconnecting the main window and
relaunching add nothing extra. Every row of the table above passes, and all
counters, including activity counters, are unchanged across the upgrade and
repeated launches.

**1.4 Targeted events.** After 1.2 and 1.3, replace broadcast `emit` calls
with a typed event sink that delivers to named consumers. `audio-level` then
goes only to visible waveform consumers, which removes the hidden main
window's re-render on every event (`audio.rs:2958`,
`useRecordingState.ts:341`). Also check `CorpusRecorder.tsx:127`.

**1.5 State ownership and the WebKit data lifecycle.** Settings, onboarding,
history, statistics, and the theme library already have Rust-backed blob
files, but their schemas and migrations live in TypeScript
(`lib/settings.ts`, DECISIONS 2026-08-09). Move schema and migration
ownership into Rust, with a new decision entry that supersedes 2026-08-09 and
2026-07-20 item 5. Give every WebKit-only key a disposition:

| Key | Disposition |
|---|---|
| `murmur-appearance` | Migrate its source state to Rust: `mode`, `theme` (including a custom theme with no library entry), and `selection`. Regenerate only the derived `cache` (`appearance/types.ts:92-100`) |
| `murmur_discovery_v1`, `murmur.settings-disclosure.v1`, `usage-dashboard-collapsed` | Migrate to Rust |
| `updater-last-check`, `pending-update-release-notes` | Migrate with the updater (1.6) |
| `murmur-ui-latency-v1` | Drop |
| `murmur-benchmark-report`, `murmur-benchmark-reports` (they hold transcript and reference text, `benchmark.ts:161-163`) | Import into the Rust-owned reports file in v1, then remove with the transcript-bearing keys |
| Legacy `murmur_onboarding_complete`, `murmur-overlay-vertical-offset` | Read once by the importer |

Specify atomic writes, concurrent writers, corrupt and future schemas, and
failed-write behavior. Appearance fixtures: a custom theme with no library
entry, mixed light and dark selections, an empty library, and a
pre-library document with no `selection`.

The **legacy migration** moves WebKit-only data to disk. It must work for
users who skip releases, because `min_version` stays `null` and a 0.62 user
can update straight to the final release (`docs/release.md:85-90`).

It is versioned by stage, and each version is committed separately:

| Version | Ships with | Keys |
|---|---|---|
| v1 | 1.3 | History, statistics, and benchmark reports |
| v2 | 1.5 | The other keys in the table above |
| v3 | 1.6 | The updater keys |

A key's import ships in the same release in which React stops writing that
key to `localStorage`, so no edit can be left only in WebKit. A committed
version never re-imports, so History cleared afterwards cannot return.

The **transcript-bearing keys** are `dictation-history`, `dictation-stats`,
`murmur-benchmark-report`, and `murmur-benchmark-reports`. The release that
removes them also stops every writer: History and statistics move to the 1.3
service, and Performance Lab's saved reports
(`saveBenchmarkReports`, `benchmark.ts:264-272`, called from
`PerformanceLab.tsx:277`) move to a Rust-owned file in the app data
directory with the same 10-report cap and Clear action. Both readers, the
Performance Lab (`PerformanceLab.tsx:165`) and the diagnostics window's
report comparison (`ReportCompareView.tsx:28`), read through it. The new load
path reads only that file; today's `loadBenchmarkReports` rewrites the legacy
keys when it reads them (`benchmark.ts:279,285`), so it is removed rather
than kept as a fallback. Auto-save to the chosen output folder is
unchanged. Test that after removal, running a
synthetic benchmark and relaunching leaves all four keys absent.

It has two forms:

- **While the main window is a React page** (Phase 1 until the main window
  becomes a native shell or fully native), the main window's startup code
  does it, because Tauri builds the config windows before `setup` runs
  (`tauri-2.11.2/src/app.rs:2513-2524`) and the pages already read
  `tauri://localhost` storage. Startup sends the keys of each uncommitted
  version to a one-shot Rust command, waits for the commit, then removes the
  transcript-bearing keys with targeted `removeItem`. It never clears the
  whole store while React pages are live, because the overlay and recording
  controls still read settings from it (`useOverlayRuntime.ts:267`,
  `useRecordingControls.ts:85`).
- **From the first release with a native main window**, a small standalone
  importer with no Tauri or wry dependency does it: an objc2 `WKWebView` on
  the default data store with a `tauri` URL-scheme handler serving a fixed
  page at `tauri://localhost`. It reads storage through that page,
  not WebKit's undocumented on-disk layout. It runs before any window shows
  user data and before any React child view is created. It stays in every
  later release, including after Phase 4.

Each launch evaluates three separate steps, each with its own durable state
in the Rust-owned migration file (1.3) and its own condition. An interrupted
step retries only that step, and every step is safe to rerun if the
migration file is lost: the import skips destinations that carry their v1
marker, and removal and retirement repeat harmlessly.

1. **Import:** runs when any version is uncommitted and the WebKit data
   directory (`~/Library/WebKit/com.localdictation`) exists. Commits each
   version once its data is atomically on disk. With no WebKit data
   directory, an uncommitted version commits at once with nothing to import
   (1.3).
2. **Remove transcript keys:** runs when removal is incomplete, including on
   profiles whose import committed in an earlier release. Both forms do it.
   A destination's keys are removed as soon as that destination carries its
   v1 marker (1.3), without waiting for the others; in the React form a
   clear before v1 removes them at once with `removeItem`, and in the
   native-window form the importer removes them on the next launch.
3. **Retire the store:** runs in every release with no React webview at all,
   while retirement is incomplete, and only after every version has
   committed durably. It purges the whole store through
   `WKWebsiteDataStore` (no records returned by `fetchDataRecordsOfTypes:`)
   and, once no webview is alive, removes
   `~/Library/WebKit/com.localdictation`. This also clears caches that React
   refilled after an earlier migration.

The disk copy wins over WebKit, as in DECISIONS 2026-08-09, and an import
only fills keys that have no disk value; the explicit empty History from 1.3
counts as a disk value. That fill-only rule applies to v2 and v3 keys. The
v1 destinations (History, statistics, benchmark reports) follow 1.3 instead:
the service never creates them before the import, so the legacy copy merges
only into a file that is missing or unusable at import time, and the held
data is added after it.

Test unmigrated, migrated, interrupted-import, interrupted-removal,
interrupted-retirement, corrupt, and already-imported stores; a profile that
goes v1 → v2 → v3 one release at a time and one that jumps straight to v3;
Clear History on a 1.3 build followed by an update; and a direct 0.62 →
first post-Phase-4 update with WebKit-only appearance and History data.

**1.6 Host-neutral updater.** Move the update flow out of React into a Rust
updater on the same `latest-v2.json` feed and minisign key: platform key
lookup, `min_version` policy, the OTA canary command and
`MURMUR_UPDATER_CANARY`, translocation checks, relaunch, and failure
recovery. It must pass at least two real OTA canaries in the Tauri app
before Phase 2. Sparkle is dropped: it needs its own key and appcast, which
installed clients cannot read.

**1.7 Frontend host adapter.** Route every `@tauri-apps/*` import through one
module so the remaining React code has a single, enumerated host surface.
Remove plugin calls from React where the logic now lives in the core.

**1.8 Shutdown coordinator.** One Tauri-free reducer and asynchronous host adapter
coordinate menu Quit, Cmd-Q, Dock/tray Quit, system termination, ordinary runtime
exit, actual final-window destruction, catchable signals and compatible process
relaunch. Admission closes permanently; original capture generations drain by
+5 s, helper receipts by +7 s, writer barriers by +10 s, then one terminal action
is committed. Unconfirmed helper groups or unpreserved writes suppress restart.
RunEvent::Exit records only the terminal receipt.

The owned licensed tao 0.35.3 patch retains its delegate and adds typed
applicationShouldTerminate plus a common-mode CF terminate relay. AppContext
main dispatch continues during the wait. A private pending completion publication
supports replay once under retention/Clear/Delete/cap rules; no persistent
dictation audio is introduced. Settings/theme/UI/window writers preserve their
native merge and pending-document contracts. A process compatibility plugin
retains process:allow-restart until native updater ownership in #990. The sole
request_restart call follows the drained permit, guarded by CI source policy.

See [implementation, deviations and native matrix](gpui-shutdown-coordinator.md).
Each native quit path still requires verification against the actual Tauri app
with synthetic active dictation, meeting and pending-write fixtures, including a
six-second delayed release, continued dispatch, mixed requests, and independent
shutdown of the replacement process. Headless reducer/AppKit tests supplement
that matrix; they do not establish its pass.

**1.9 Lazy diagnostics window.** Create the diagnostics window on first open
instead of at launch. This is a small, immediate memory win.

Exit criteria:

- The core builds and tests without Tauri.
- The 1.2/1.3 headless exit check passes.
- No user-facing state lives only in WebKit storage, and the importer and its
  completion marker ship.
- No broadcast emits.
- Every quit path goes through the coordinator.
- The Rust updater has passed real canaries.
- Every row in the feature matrix has an owner and an acceptance flow.

## Phase 2: GPUI becomes the host

The Phase 0 route table decides this phase's shape. In both embedded routes
it becomes "embed GPUI in the Tauri app" and needs no bridge for the host
change. In the native-parity route it runs after Phase 3.

In every route, the 2.1 packaging proofs and the 2.4 signed update and
permission-continuity rehearsal run during this phase, using a non-Tauri
test bundle, so signing and TCC risks surface early. In the embedded routes,
the 2.3 host replacements and the release that removes Tauri move to Phase
4, which carries their exit criteria and a larger estimate.

**2.1 Packaging first.** A non-Tauri bundler (`cargo-packager` or a script)
that produces the same bundle: `CFBundleExecutable=ui`, the same Info.plist
keys, the three helpers in place, the capture-agent plist at
`Contents/Library/LaunchAgents`, entitlements, the FoundationModels weak
linking and Swift runtime paths, and the macOS 14 minimum. A
minisign-compatible signer with the existing key, a single version source,
notarization, provenance, and the executable allowlist in
`finalize_macos_bundle.py`. Two separate proofs before any release:

- The **secretless rehearsal** (DECISIONS 2026-07-21) proves bundle layout,
  Info.plist, helper placement, and the executable allowlist. It does not
  prove signing.
- A **signed rehearsal** proves code signing, notarization, Gatekeeper
  acceptance, minisign signatures readable by installed clients, and the
  designated requirements that TCC depends on. Signed artifacts come only
  from the trusted path: the GPUI bundle target lands on `main` as a
  non-default target, a manual non-publishing `Release Build` dispatch on
  `main` builds, signs, and notarizes it (`docs/release.md`, DECISIONS
  2026-07-21), and that exact artifact is installed and tested on the Mac
  mini. `release-build.yml` gains a `bundle_target` dispatch input (default
  `tauri`) beside `prime_caches`; a `gpui` value is accepted only on
  `workflow_dispatch`, so it can never publish. No signing credentials go to
  the Mac mini.

**2.2 Bridge for unported React screens.** wry webviews inside GPUI windows,
not a hand-built WKWebView bridge, meeting the 0d accessibility and keyboard
criteria:

- A `tauri` custom protocol so the origin stays `tauri://localhost`, keeping
  the same `WKWebsiteDataStore` for the rebuildable caches React still uses
  (such as the theme bootstrap cache). No transcript content lives there
  after the 1.5 transcript-key removal, which covers History, statistics,
  and benchmark reports.
- An `ipc` custom protocol for invoke, including binary responses
  (`meeting_audio.rs` WAV bytes).
- A minimal `window.__TAURI_INTERNALS__` shim so `@tauri-apps/api` works
  unchanged.
- Caller identity bound natively per webview, never taken from JS. The
  existing `require_main_window` and `require_diagnostics_window` checks and
  each `capabilities/*.json` file become per-window allowlists with denial
  tests for companion windows, subframes, and navigated content.
- Every `plugin:*` command used from JS (event, window, dialog, process,
  opener, notification, autostart, app) is implemented natively or its
  caller removed in Phase 1.
- Window-targeted events, subscription cleanup, native file drops with real
  paths, fullscreen and resize calls, and transparent webviews for any
  remaining overlay or preview.
- No logging of invoke or event payloads.

**2.3 Native host replacements.**

| Today | Replacement |
|---|---|
| Tauri windows | GPUI windows; native settings through the raw handle on the main thread |
| `tray-icon` and tray menu, including the dynamic "Next recording" submenu | `NSStatusItem` via objc2 |
| App and Edit menus, Dock menu | GPUI menus |
| `RunEvent::Reopen`, hide-on-close | GPUI equivalents |
| `RunEvent::Exit` teardown | The 1.8 shutdown coordinator, entered before GPUI's quit handlers; GPUI's 200 ms quit window is used only for final, non-blocking cleanup |
| `tauri-plugin-autostart` (LaunchAgent) | `SMAppService`, migrating the existing plist without duplicate launches or enabling it for users who had it off |
| `tauri-plugin-notification` | `UNUserNotificationCenter` |
| `tauri-plugin-dialog` | `NSOpenPanel` / `NSSavePanel` / `NSAlert` |
| `tauri-plugin-opener` | `NSWorkspace` |
| `tauri-plugin-process` | `NSApplication` terminate and relaunch, through the coordinator |
| `tauri-plugin-updater` | The Rust updater from 1.6 |
| Tauri-derived data paths | Injected paths, tested identical to `~/Library/Application Support/com.localdictation` (and the dev and bench identifiers) |
| Worker modes dispatched in `main.rs` | Same dispatch order before any `NSApplication` or Metal setup |

**2.4 Identity, the update chain, and downgrades.** Keep the bundle id, Team
ID, and designated requirements for the host and helpers so TCC grants carry
over: Microphone, Accessibility, System Audio Recording, Calendars, and Apple
Events. Rehearse a signed, installed **pre-Phase-1 Tauri → first GPUI → next
GPUI** path with permissions both granted and denied and without resetting
TCC, including relaunch, failed-update recovery, and canary reporting. At
the flip release, `min_version` stays `null` and the last Tauri DMG stays
downloadable.

Downgrade rule: until Phase 4, Rust schema changes are additive and preserve
unknown keys. The supported downgrade target is the last Tauri release,
named in the flip release notes; it uses the 1.5 Rust loader, which keeps
unknown keys. Older Tauri releases, whose TypeScript loader rewrites settings
at `settingsVersion: 7` (`settings.ts:641`, `:1325-1344`), are not supported
targets. Test GPUI → the named Tauri release with real settings, History, and
appearance files. After Phase 4, downgrades are unsupported, and the release
notes and the old DMG's download page say so.

Memory may rise during this phase (GPUI plus remaining webviews). Cap it at
the baseline number frozen at Phase 0 exit.

Exit criteria: every feature-matrix row marked for Phase 2 passes its native
acceptance flow on the Mac mini; memory is at or below the frozen baseline;
the signed update chain and downgrade rehearsals pass.

## Phase 3: Port screens

Each port is a matrix row set with its own PR, memory number, and native
acceptance flows. A process counts as removed only when its webview is
destroyed, verified by process listing.

1. **Accessible component layer** (buttons, toggles, fields, dropdowns,
   lists, navigation, motion) following `docs/features/ui-design-system.md`.
2. **Overlay** (always present).
3. **Dictation preview.**
4. **Diagnostics window.**
5. **Main window, section by section.** Default strategy: a native GPUI
   main-window shell (sidebar, navigation, window chrome) in which each
   unported section is a lazily created wry child view, destroyed when the
   user leaves it. This needs a route with child views (route table); in the
   other routes the main window switches in one release after every section
   below is native. Order:
   - **3.5a History** first, because the 474 MiB peak came from scrolling
     History. Followed by checkpoint 4.
   - **3.5b Settings** (the largest section), split into its existing
     panes.
   - **3.5c Onboarding and home.**
   - **3.5d Meetings:** capture, review, native playback, export, calendar,
     and diarization.
   - **3.5e The rest:** command palette, Insights, model management,
     knowledge and modes, and themes.

   Each step's exit in the child-view route: its rows pass, its webview is
   destroyed, and the main-window memory number for that step is met. In the
   whole-window routes, each step's rows pass in development builds, and
   webview destruction and the memory numbers are checked once, at the
   release that switches the whole main window.

Tests: move fixtures that Rust loads from `app/src` (for example
`commands/overlay.rs:1805`, `meeting_capture.rs:2200`) to a neutral location
first. A Vitest or Playwright suite is retired only when its matrix rows list
passing GPUI interaction, accessibility, or golden tests that replace each
behavior it covered.

## Phase 4: Remove WebKit, Tauri, and React

Delete the bridge, remaining webviews, `app/src`, Node tooling, and Tauri
dependencies only after their replacements work. Update
`docs/ARCHITECTURE.md`, `docs/DEVELOPMENT.md`, CI, and release docs.

The standalone importer from 1.5 stays.

In the embedded routes, this phase also does the 2.3 host replacements and
ships the release that removes Tauri, with the 2.4 rehearsal repeated on that
release.

Exit criteria: once the "store retired" state is set, no WebKit process
starts; on upgraded test installs (step-by-step through the Tauri releases,
and a direct 0.62 → this release update) `fetchDataRecordsOfTypes:` returns
no records and `~/Library/WebKit/com.localdictation` is absent; final memory
meets the frozen budgets.

## Verification per phase

- Required CI and Sol review on every product PR.
- Native verification on the Mac mini for each changed flow, per `CLAUDE.md`,
  using the matrix acceptance flows.
- Memory at each phase boundary using the measurement method above.

## Open decisions

1. **Migration route.** Decided by Phase 0c and 0d through the route table.
2. **SwiftUI fallback.** Whether SwiftUI on the Rust core is acceptable if
   Phase 0 stage A fails.
3. **Controls.** `gpui-component` or in-house controls, decided in Phase 0b.
4. **Main-window strategy.** Native shell with child views (default) or one
   all-at-once port, confirmed at Phase 0 exit.
5. **Updater implementation.** In-house on `minisign-verify` or
   `cargo-packager-updater`.

## Reference counts

178 `#[tauri::command]` attributes (172 registered; track against
`docs/reference/commands.md`), 93 emit call sites, 23 non-test frontend
files that reference `localStorage` (22 excluding `visual-fixtures.tsx`), and
about 49k lines of non-test TypeScript.
