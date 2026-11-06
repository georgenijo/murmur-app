# GPUI 1.4: targeted events

Part of #988. Stack 05, based on draft #1021 (`gpui/04-completion-service`);
**do not merge**. This branch never merges to main.

## Boundary and invariants

The host-independent `murmur-core::events` protocol defines typed event kinds,
named consumers, a generic `EventSink` preserving each existing native payload
DTO's `Serialize` type/wire shape, and waveform interest routing. The Tauri
adapter owns window lookup and transport plus the native callback registry.
GPUI can implement the same sink without Tauri. AppContext is not an event bus.
No recording, completion, transform, secure-field, window geometry, main-thread
dispatch, rdev setup, retention or shutdown ownership moves. No event gains
transcript, audio, clipboard or selection content; existing payloads are unchanged.
In particular, pre-existing transcript-bearing completion/partial events remain
on their existing UI destinations, never in logs or the native registry.

Tauri `listen()` defaults to `Any`. In pinned Tauri 2.11.2,
`event/listener.rs::match_any_or_filter` accepts such listeners even for an
`emit_to` addressed elsewhere. Every production client therefore uses
`targetedEvents.listen`, bound to `EventTarget::WebviewWindow` with its own
label. Existing targeted producers also benefit from this binding. Rust control,
sound cues, Smart Auto and meeting suggestions use named registry subscriptions; they do not depend
on a main window or receive a second copy from a webview emit. Registry/router
locks are released before invoking callbacks, preserving recursive publication.
An absent window is skipped silently. Ordinary status events still reach hidden
consumers so control, sound cues and durable completion continue independently.

## Recount

At the pinned base `dbaa9e4cc7bdd740264f315c361f75d35b86b3d9`, grep of
`app/src-tauri` found **92 broadcast call sites / 55 distinct event names**,
including `Window::emit` (which also broadcasts). The plan's historical count
was 93 emit sites. After conversion: **0 Rust broadcast calls**, **92 converted
sink calls**, plus two previously targeted microphone-preview status and microphone-startup
progress calls moved to the sink to preserve native subscriptions.
No `emit_all`, qualified `AppHandle::emit` or `Emitter::emit` calls were present.
The CI guard scans **every repository Rust source**, including crates, tests,
sidecars and spikes, including Cargo targets/modules rooted in `vendor/`.
Exclusions are exact repository-relative paths: `.git/` (Git metadata),
`target/` and `app/src-tauri/target/` (Cargo output), and `node_modules/` and
`app/node_modules/` (installed npm dependencies). No third-party Rust source
path is currently exempted; nested maintained directories with those names
are scanned. Outside the one sanctioned adapter
`app/src-tauri/src/events.rs`, **any identifier token named `emit` or `emit_*`**
(including raw identifiers and macro arguments) is forbidden after masking
comments, string/byte/raw/C literals and character literals. Local non-Tauri
helpers and capture-sidecar callbacks were renamed to `publish*`; there is no
symbol allowlist. Payload strings and event names were not renamed.

The pinned Tauri 2.11.2 `Emitter` source defines `emit`, `emit_str`, `emit_to`,
`emit_str_to`, `emit_filter` and `emit_str_filter`; the reserved prefix also
covers future `emit_*` names. `Emitter` identifier tokens are forbidden outside
the adapter, including aliased imports and UFCS. Tauri glob imports (including
nested groups), crate aliases and Tauri re-exports are forbidden throughout the
repository (including the adapter), preventing facades from hiding a glob import.
The adapter permits no re-export imports, including aliases of its local trait import. Inside the adapter, only `emit_to`
and `emit_str_to` with the literal `tauri::EventTarget::webview_window(label)`
target are permitted. Broadcast methods, `Any`, `AnyLabel`, other target kinds,
target variables, string targets/conversions and macro metavariables are rejected.
Existing targeted producers use its named-consumer `send_to` transport.

Literal `#[path]`, `include!`, and Rust-file `include_str!`/`include_bytes!`
destinations must resolve to scanned files. Nonliteral inclusion paths,
conditional path attributes, inline-module path attributes with ambiguous bases,
macro-forwarded attributes/inclusion names, and escaping source/directory
symlinks fail closed. Ordinary non-Rust fixture
inclusions are allowed. This is a **source-level guard**, not a compiler or a
sandbox: arbitrary procedural-macro/build-script code synthesis and full Rust
macro expansion are not proved safe by it. The guard and its regression fixtures remain wired into CI alongside
the core dependency guard in the unconditional `policy` job. Frontend syntax
policy and its fixtures run there too, using only the pinned TypeScript parser;
frontend-only, outside-app Rust and guard-only diffs cannot skip the guard.

Audit commands: `rg -n '\.emit|emit_all|Emitter::emit|AppHandle::emit' app/src-tauri`,
`rg -n 'listen|once|listen_any|once_any' app/src`, plus Rust listener searches. Generic listeners,
constant event names and arrays of dynamic subscription names were checked
alongside literal `listen(` / `listen<` / `once(` / `listen_any(` calls. There are no production
Rust Tauri event listeners remaining; the four native consumers use the registry.

## Broadcast routing inventory

Producer locations below name every converted site's module (multiple sites in
one module are grouped). “Before” lists the actual subscribing modules, rather
than implying that all broadcast recipients used every event. Main means
`main`, Overlay means `overlay`, Diagnostics means `diagnostics`, and
DictationPreview means `dictation-preview`. Other consumer names are native
registry identities. Multiple listeners within a window receive one delivery.

| Event | Producer sites | Consumers before | Consumers after |
|---|---|---|---|
| `app-disabled-changed` | `commands/keyboard.rs` | useOverlayRuntime; useSettings; smart_auto_probe (native); meeting_suggestions (native listen_any) | Main Overlay SmartAuto MeetingSuggestions |
| `app-event` | `telemetry.rs` | useEventStore | Main Diagnostics |
| `audio-initialization-stalled` | `commands/recording.rs` | OverlayWidget | Overlay |
| `audio-level` | `audio.rs` | CorpusRecorder (main Performance Lab); useRecordingState (main Home); useWaveform (overlay) | Visible, subscribed Overlay; visible main Home recording or corpus waveform (one delivery per window) |
| `audio-recovery-started` | `commands/recording.rs` | None (retired/unused notification) | None: legacy notification has no current listener; native input already calls control directly |
| `auto-paste-failed` | `commands/recording.rs` | useRecordingState | Main |
| `benchmark-progress` | `benchmark.rs` | PerformanceLab; meeting_suggestions (native listen_any) | Main MeetingSuggestions |
| `check-for-updates-requested` | `lib.rs` | App | Main |
| `corpus-recording-status` | `commands/corpus.rs` | CorpusRecorder | Main |
| `delivery-retry-feedback` | `delivery_recovery.rs` | useDeliveryRecoveryListeners | Main |
| `diagnostics-tab-requested` | `commands/performance.rs` | DiagnosticsWindowApp (DIAGNOSTICS_TAB_REQUESTED_EVENT) | Diagnostics |
| `dictation-control-error` | `dictation_control.rs` | useRecordingState | Main |
| `dictation-delivery-outcome` | `commands/recording.rs` | useOverlayRuntime; useRecordingState; dictation_cues (native) | Main Overlay SoundCues |
| `dictation-generation-started` | `commands/recording.rs` | useDictationPartial; useOverlayRuntime; useRecordingState; dictation_cues (native) | Main Overlay DictationPreview SoundCues |
| `dictation-lock-changed` | `dictation_control.rs` | useRecordingControls | Overlay |
| `dictation-stop-failed` | `dictation_control.rs` | useRecordingState | Main |
| `double-tap-toggle` | `keyboard.rs` | None (retired/unused notification) | None: legacy notification has no current listener; native input already calls control directly |
| `download-progress` | `commands/models.rs`, `diarization_model.rs` | MeetingDiarizationSettings; ModelDownloader; PerformanceLab; SettingsPanel | Main |
| `escape-cancel` | `keyboard.rs` | None (retired/unused notification) | None: legacy notification has no current listener; native input already calls control directly |
| `file-output-failed` | `commands/recording.rs` | useRecordingState | Main |
| `file-transcription-status-changed` | `commands/recording.rs` | PerformanceLab; smart_auto_probe (native); meeting_suggestions (native listen_any) | Main SmartAuto MeetingSuggestions |
| `history-changed` | `completion.rs` | completionData | Main Diagnostics |
| `hold-down-start` | `keyboard.rs` | None (retired/unused notification) | None: legacy notification has no current listener; native input already calls control directly |
| `hold-down-stop` | `commands/keyboard.rs`, `keyboard.rs` | None (retired/unused notification) | None: legacy notification has no current listener; native input already calls control directly |
| `hold-over-recording-end` | `keyboard.rs` | None (retired/unused notification) | None: legacy notification has no current listener; native input already calls control directly |
| `hold-over-recording-start` | `keyboard.rs` | None (retired/unused notification) | None: legacy notification has no current listener; native input already calls control directly |
| `hotkey-tap-rejected` | `keyboard.rs` | useDiscovery; useOverlayRuntime | Main Overlay |
| `keyboard-listener-error` | `keyboard.rs` | None (retired/unused notification) | None: legacy notification has no current listener; native input already calls control directly |
| `meeting-segment-failed` | `meeting_capture.rs` | useMeetings (segment failure listener) | Main |
| `meeting-segment-finalized` | `meeting_capture.rs` | useMeetings | Main |
| `meeting-speakers-updated` | `meeting_diarization.rs` | useMeetings | Main |
| `meeting-status-changed` | `meeting_capture.rs` | OverlayWidget; useMeetingAudio (dynamic events); useMeetings; smart_auto_probe (native); meeting_suggestions (native listen_any) | Main Overlay SmartAuto MeetingSuggestions |
| `mode-manual-changed` | `commands/mode_runtime.rs` | useSettings | Main |
| `mode-runtime-changed` | `commands/mode_runtime.rs` | useModeRuntime | Main Overlay |
| `model-runtime-status-changed` | `model_runtime.rs` | modelRuntime; useModelRuntimeCatalog; meeting_suggestions (native listen_any) | Main MeetingSuggestions |
| `overlay-geometry-changed` | `commands/overlay.rs` | useOverlayExpansion; useOverlayGeometry | Overlay |
| `overlay-position-applied` | `commands/overlay.rs` | OverlayCalibrationControl | Main |
| `overlay-surface-changed` | `commands/overlay.rs` | useOverlayGeometry | Overlay |
| `overlay-visible-changed` | `commands/overlay.rs` | useOverlayExpansion | Overlay |
| `performance-diagnostics-cleared` | `performance_metrics/mod.rs` | performance; performance subscriptions (DiagnosticsWorkspace in main/diagnostics) | Main Diagnostics |
| `performance-resource-sample` | `performance_metrics/mod.rs` | performance; performance subscriptions (DiagnosticsWorkspace in main/diagnostics) | Main Diagnostics |
| `performance-run-completed` | `performance_metrics/mod.rs` | performance; performance subscriptions (DiagnosticsWorkspace in main/diagnostics) | Main Diagnostics |
| `recording-cancelled` | `commands/recording.rs` | useDictationPartial; useOverlayRuntime; useRecordingState | Main Overlay DictationPreview |
| `recording-cleanup-finished` | `commands/recording.rs` | useRecordingState | Main |
| `recording-initialization-failed` | `commands/recording.rs` | useOverlayRuntime; useRecordingState; useSmartAutoMicrophoneStatus (REFRESH_EVENTS); dictation_cues (native) | Main Overlay SoundCues |
| `recording-interrupted` | `commands/recording.rs` | useOverlayRuntime; useRecordingState; dictation_cues (native) | Main Overlay SoundCues |
| `recording-recovery-stalled` | `commands/recording.rs` | useRecordingState | Main |
| `recording-status-changed` | `commands/recording.rs` | useDictationPartial; useMeetingAudio (dynamic events); useOverlayRecordingStatus; useRecordingState; dictation_control (native); dictation_cues (native); smart_auto_probe (native); meeting_suggestions (native listen_any) | Main Overlay DictationPreview DictationControl SoundCues SmartAuto MeetingSuggestions |
| `smart-auto-microphone-changed` | `audio_inventory.rs`, `smart_auto_probe.rs` | useSmartAutoMicrophoneStatus (REFRESH_EVENTS) | Main Overlay |
| `smart-destination-selected` | `commands/recording.rs` | useOverlayRuntime; useRecordingState | Main Overlay |
| `statistics-changed` | `completion.rs` | completionData | Main Diagnostics |
| `system-audio-permission-changed` | `commands/meeting.rs` | PermissionsBanner; useMeetings | Main |
| `transcription-complete` | `commands/recording.rs` | useDiscovery; useRecordingState | Main |
| `updater-background-check-requested` | `commands/tray.rs` | useAutoUpdater | Main |
| `vocab-scan-progress` | `commands/recording.rs` | useVocabScan (EVENT) | Main |

## Existing targeted producers

These already use explicit labels and retain their wire payloads. The shared
client listener binding now enforces those targets as well.

| Event | Producer sites | Consumers before | Consumers after |
|---|---|---|---|
| `dictation-partial` | `commands/recording.rs` | DictationPreview / useDictationPartial | DictationPreview |
| `audio-input-inventory-changed` | `audio_inventory.rs` | Main and Overlay / useAudioInputInventory, useSmartAutoMicrophoneStatus | Main and Overlay |
| `completion-storage-error` | `completion.rs` | Main / CompletionNotice | Main |
| `meeting-audio-invalidated` | `meeting_audio.rs` | Main / useMeetingAudio | Main |
| `meeting-suggestion-changed` | `meeting_suggestions.rs` | Overlay / useMeetingSuggestion; Main is also addressed | Same labels |
| `microphone-preview-level` | `audio.rs` | Main / MicrophoneInputTest | Main |
| `microphone-preview-vad` | `microphone_preview.rs` | Main / MicrophoneInputTest | Main |
| `microphone-preview-status` | `commands/microphone_preview.rs` | Main / MicrophoneInputTest, useMeetingAudio; Smart Auto app listener was excluded by the main target; meeting_suggestions listen_any | Main, SmartAuto and MeetingSuggestions via typed sink |
| `microphone-startup-benchmark-progress` | `commands/microphone_startup_benchmark.rs` | Main / MicrophoneStartupBenchmark, useMeetingAudio; meeting_suggestions listen_any | Main and MeetingSuggestions via typed sink |
| `live-words-consent-persisted` | `commands/settings_store.rs` | DictationPreview / DictationPreviewApp | DictationPreview |
| `overlay-settings-persisted` | `commands/settings_store.rs` | Overlay / useOverlaySettingsMirror | Overlay |
| `overlay-owned-setting-committed` | `commands/settings_store.rs` | Main / useOverlaySettingsSync | Main |

`query-state-changed` (Main playback, native Smart Auto) and
`transform-state-changed` (native Smart Auto) retain named protocol routes for
their subscriptions; neither has a current producer. `show-about` has a client
listener but no current Rust or frontend producer.

## Frontend producer audit

The eight frontend broadcasts at `60933c29` (including two appearance calls
found by the repository grep) now use `targetedEvents.sendTo`, which calls
Tauri `emitTo` with a named label. Payloads remain undefined/no payload,
`{ active }`, or the existing appearance receipt; no host command is added.
There are no Rust/native listeners for these events. A missing label is an
empty Tauri recipient set, and the helper also silently tolerates a companion
closing during IPC.

| Event | Producer sites | Actual listeners | Destination |
|---|---|---|---|
| `settings-changed` | useSettings (four sites: policy recovery, manual mode, updates, overlay-write reconciliation) | useOverlaySettingsMirror in Overlay | Overlay |
| `open-settings` | useOverlaySettingsMirror gear action | useOpenSettingsListener in Main/App | Main |
| `overlay-calibration-changed` | OverlayCalibrationControl | OverlayWidget | Overlay |
| `appearance-changed` | useAppearance (repair and edit) | None: legacy notification; Main owns the only themed surface and applies changes directly | Main only, preserving the notification/payload without involving companions |

Previously targeted frontend producers were audited too and retain their labels:

| Event | Producer | Actual listeners | Destination |
|---|---|---|---|
| `overlay-live-words-lift-request` | DictationPreviewApp | useLiveWordsLift in OverlayWidget | Overlay |
| `overlay-live-words-lift` | useLiveWordsLift (two sites) | DictationPreviewApp | DictationPreview |
| `accept-meeting-suggestion` | useMeetingSuggestion | useMeetingSuggestionAcceptance in Main/App | Main |
| `local-stats-completion` | localStatsEvents | None: legacy local receipt (current statistics use completionData) | Main only |

Audit: `git grep` over frontend `emit(`, generic `emit<`, `emitTo`, `.emit`
and `@tauri-apps/api/event` imports, followed by listener/constant searches.
The TypeScript AST guard reserves `emit` identifiers and statically computed
`emit` names in shipped `app/src` JS/TS, including Window/WebviewWindow methods,
aliases, destructuring, optional calls, Unicode escapes and template expressions.
Direct `emitTo` calls must name one of the four consumer labels; the helper
alone can forward its typed `label` parameter. `Any`/dynamic targets fail closed.
It rejects Tauri namespace/default/dynamic imports and re-export facades;
broadcast imports/re-exports are forbidden even in test files. Tests may use
local synthetic `emit`/mock helpers, never an imported Tauri broadcast. This
source policy does not prove arbitrary runtime reflection, `eval`, computed
module loading, or generated code safe.

## Waveform lifecycle

`useAudioLevel` listens before announcing interest, ignores callbacks after
cleanup, serializes activation/deactivation IPC (including StrictMode remounts),
and unregisters on cleanup. Its three named interests are:

- Overlay, only while recording with visible waveform bars (priority cues
  suspend interest); native visibility/minimization is checked on
  each coalesced main-thread delivery. A newly shown overlay receives the next sample without restarting.
- Main recording, only on the Home surface, after onboarding/model readiness,
  with Settings closed and recording active. MainHeader's recording controls are
  disabled by the current App entry point, so it has no additional waveform.
- Corpus, only while recording and the Performance Lab settings page is active.
  Its interest is separate from main recording; disabling either cannot disable
  the other, and both still result in at most one event per window per sample.

Capture snapshots the supervisor's active capture generation **before** computing
the numeric level. The existing capture-active state carries an atomic epoch plus separate activity
and waveform-ownership bits. Activation (including warm standby reuse) advances
it; stop, cancel/recovery, cleanup and worker exit revoke it. Every backend
attempt retires waveform ownership before blocking helper teardown: protocol
failure/disconnect, helper crash, permission loss, timeout, helper auto-stop,
probe cancellation and host stop/shutdown all pass this boundary. Attempt exit
also retires ownership. A permitted fallback gets fresh waveform ownership only
while supervisor activity remains set; it cannot reactivate a stopped capture. This extends the existing
capture ownership state rather than creating a second lifecycle registry. It
also covers corpus capture, whose identity is independent of dictation IDs.
Each queued sample carries that immutable ownership token internally; the wire
payload is still the same numeric level with no generation or content added.
The generic sink refuses unowned `AudioLevel` publication.

Capture replaces one pending sample under a brief mailbox lock. At most one
main-thread task is queued (plus a task already executing); further samples
coalesce into the latest value. Capture never queries windows or waits for a
main-thread reply. The task takes the sample and releases the mailbox lock,
then reads current interests and visibility/minimization. After all native window
queries, each callback/transport entry takes a capture delivery read permit,
checks the epoch, and keeps the permit through the call. Revocation takes the
write side, so it cannot return (or acknowledge stop/cancel) while an old delivery
is in flight. Once it returns that epoch cannot deliver again. Router, mailbox
and callback-registry locks are released before callbacks. The writer only
updates the atomic epoch and never dispatches to or waits for main. No capture
path window query is introduced.

Pinned Tauri 2.11.2 was inspected locally: `Emitter::emit_str_to` delegates to
`AppManager::emit_to`/`emit_filter`, whose JS path uses `Listeners::emit_js_filter`,
`Webview::emit_js` and `Webview::eval`. `tauri-runtime-wry` 2.11.2 dispatches
`EvaluateScript`; on main it calls Wry immediately, while Wry 0.55.1's macOS
`InnerWebView::eval` submits `WKWebView::evaluateJavaScript_completionHandler`
and returns without waiting for JS. The permit covers this short submission,
not JS execution. Already submitted JS may run later. Tauri native listeners
run synchronously; there are no production Tauri native `audio-level` listeners
(the test observer exercises the same callback permit). No synchronous native
control round trip occurs under it. Hide/minimize
suppresses delivery; show/restore resumes on the next sample without restarting.
A blocked main thread retains only one pending sample; dispatch failure resets
the mailbox for retry.

Main stays free of level-driven renders while hidden. Navigation/reload and
destruction clear interests for that label. WebContent termination clears the
crashed label's interests **before** native reload, so a failed reload cannot
retain them. Pinned `tauri-runtime-wry` owns the previous default termination
recovery; the application hook calls the same `Webview::reload()` operation
after cleanup, with no added JS reload, retry loop or recovery policy.
Synthetic native regression listeners use a test-only typed observer registry;
their ordering/terminal assertions are unchanged. The subscription command authorizes the caller's window against the fixed
consumer identity; a companion cannot register main interest. No caller chooses
arbitrary labels or native consumer identities. Native silence/Smart Turn control
continues receiving levels directly from capture, independently of UI routing.

## Verification

Core route tests cover named native/window recipients, absent windows, independent
corpus/main interest, no duplicate delivery, visibility/minimization gating,
hide/show without capture restart, and clearing a reloaded/destroyed window.
The macOS adapter test drives the actual sink with Tauri's mock host, a named
Diagnostics window, an unrelated window, and no main. It checks exact native
callback delivery and silent absent-window handling. Client tests cover label
binding and each waveform's activation, cleanup, reactivation and delayed listen.
The existing core headless control/completion test retains all 1.2/1.3 controller
and durable-state assertions; its fabricated `FakeSink` receipts are removed.
A separate app adapter regression initializes the production consumer subscription
registrations (substituting only their OS/worker effects), production completion
state with a temporary synthetic service, and the authorized waveform command.
It invokes the recording/capture publication helpers used by production and
`completion::produce`, then asserts actual native callbacks and window listeners
without creating main. Completion notifications are generated by the production
adapter rather than by the fake host. The former source-text wiring assertion was
removed: adapter/lifecycle behavior is asserted by executable tests, not by
searching for call text.

The busy-main regression drives the actual capture publisher and sink with a
deferred dispatcher and a window-query function that panics: 5,000 publications
queue exactly one task without querying windows; draining delivers only the
latest unchanged numeric payload. Deferred-drain tests use real supervisor
stop, cancel and replacement transitions, asserting that old samples disappear
and the replacement's sample arrives. Additional tests cover warm reactivation
of the same activity allocation and revocation during the visibility query.
A barrier at the production seam after the final check and before callback/
transport entry verifies that revocation must wait for delivery. A separate
barrier holds actual `terminate_or_quarantine` open before helper teardown while
the production mailbox drains and asserts no sample arrives. Ownership-check
and delivery-permit mutation testing must fail the corresponding regressions. Hide/show,
dispatch-failure recovery and cleanup before a failing reload are covered. Existing completion/gesture
regressions run with no retired assertions. Guard regression fixtures include
macro repetition, glob/aliased imports, both
UFCS forms, `AnyLabel`, filtered/string broadcasts, cfg-gated code, source outside
the former app scan root, external/excluded `#[path]`/include escapes, symlinks,
raw identifiers, re-export facades and constrained-adapter targets, plus literal
and nested-comment false positives. Additional Cargo-root and ordinary-module
fixtures cover maintained `vendor/`, `src/vendor/`, `third_party/` and
`src/target/` paths. Frontend fixtures cover aliases, re-exports, namespace imports,
Window methods, escaped/computed properties and nested template expressions.

The command reference now contains **180 commands**, including
`set_waveform_subscription`; all maintained counts are updated. Exact Linux,
macOS and CI results are recorded in the draft PR. No native synthetic input is
sent while another task's Murmur occupies the Mac mini GUI. No production data,
logs, transcript, audio, clipboard contents or WebKit directory are inspected.

## Historical test-count reconciliation (through `60933c29`)

On the Mac mini with Rust 1.98.1, the identical command from `app/src-tauri`,
`cargo test -- --test-threads=1`, selects the app package because the workspace
sets `default-members = ["."]`. It includes all that package's unit/integration
binaries and doc tests; it does **not** select the other workspace packages.

| Binary | `0e853722` passed / ignored | Generation fix passed / ignored |
|---|---:|---:|
| `ui_lib` unit tests | 1,505 / 14 | 1,513 / 14 |
| `ui` main | 0 / 0 | 0 / 0 |
| `capture_helper_spike` | 12 / 0 | 12 / 0 |
| `coreml_transcription_integration` | 0 / 1 | 0 / 1 |
| `headless_benchmark` ordinary unit tests | 4 / 1 | 4 / 1 |
| `transcription_integration` | 1 / 0 | 1 / 0 |
| `ui_lib` doc tests | 0 / 0 | 0 / 0 |
| **libtest total** | **1,522 / 16** | **1,530 / 16** |

The separate `main_thread_queue` executable has `harness = false` and passed
at both revisions; it has no libtest count. Comparing app test names finds no
removed test and eight additions: three from the preceding coalescing/crash
fixes, plus five capture-ownership regressions in this fix.

An explicit reproduction at `4617b7a2` with `cargo test --workspace --
--test-threads=1` returns exactly the earlier **1,692 passed / 17 ignored**:
the same 1,522 / 16 app subtotal, plus capture-helper **46 / 0**, its
`topology_watch` integration binary **0 / 1**, capture-helper-protocol **13 / 0**,
and murmur-core **111 / 0** (83 unit, 19 dictation-control, 5 foundation,
3 gesture, 1 main-thread tests; doc tests 0). The reported discrepancy was
workspace selection, not lost tests. The intermediate `3047a800` app subtotal
was 1,525 / 16, before the five new ownership tests. Removing the weak
source-text-only integration assertion reduces the separate core subtotal from
112 back to 111; its behavioral adapter coverage remains in the app suite.
The PR body records the exact final revision and check outcomes.

## Round-2 re-check verification

Linux: core tests **111 passed**, strict core Clippy, TypeScript and all Vitest
**175 files / 1,812 tests** passed. Both transport guards pass, with **11 Rust
fixture groups** and **3 frontend fixture groups**. Reference/Markdown/workflow
validator tests **74 passed**; reference, Markdown, workflow and capture-boundary
validators passed. Command count stays **180**. macOS results, mutation evidence,
final CI state and independent review status are recorded in the draft PR body.
Native GUI verification belongs to the separate worker; this implementation
worker sends no GUI input and makes no native UI pass claim.
