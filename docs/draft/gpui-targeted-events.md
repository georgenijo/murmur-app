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
was 93 emit sites. After conversion: **0 broadcast calls**, **92 converted
sink calls**, plus two previously targeted microphone-preview status and microphone-startup
progress calls moved to the sink to preserve native subscriptions.
No `emit_all`, qualified `AppHandle::emit` or `Emitter::emit` calls were present.
Capture-sidecar `emit(observation)` callbacks and the gesture-correlation helper
are not Tauri broadcasts. The CI guard scans all Rust files under
`app/src-tauri`, including tests and sidecars, and confines Tauri's `Emitter` trait and every `emit_to`/`emit_str_to` call to
`src/events.rs`. Existing targeted producers use its named-consumer `send_to`
transport, preserving their labels and DTOs. Inside the adapter, every transport
must use the literal `EventTarget::webview_window(label)` constructor; arbitrary
target variables, string targets (Tauri converts these to `AnyLabel`), `Any`,
`AnyLabel`, `any()`, `labeled()` and other target kinds are rejected. The guard
normalizes raw identifiers (`r#emit`), rejects broadcast/filtered/string methods,
trait aliases and method-ident macro forwarding. The same guard/tests remain in
CI alongside the core dependency guard.

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
their subscriptions; neither has a current producer. Frontend-originated
`settings-changed`, calibration and preview-lift notifications retain their
existing producers and intended listeners. `open-settings`, `show-about` and
`accept-meeting-suggestion` have client listeners but no current Rust emit site.

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

Capture serializes the existing numeric level payload and replaces a single
pending sample under a brief mailbox lock. At most one main-thread delivery task
is queued (plus a task already executing); further samples overwrite the pending
value. Capture never queries windows or waits for a main-thread reply. The task
takes the latest sample and releases the mailbox lock before querying native
visibility/minimization and routing. It uses the current interests and window
state, so hiding/minimizing main prevents the next delivery even if React is
throttled, and showing/restoring a subscribed consumer receives the next sample
without restarting capture. A blocked main thread retains one latest sample,
not an unbounded queue. Dispatch failure resets the mailbox for the next sample.

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
adapter rather than by the fake host. A wiring contract also checks that native
startup and capture/recording still call these shared adapters.

The busy-main regression drives the actual capture publisher and sink with a
deferred dispatcher and a window-query function that panics: 5,000 publications
queue exactly one task without querying windows; draining delivers only the
latest unchanged numeric payload. Hide/show, dispatch-failure recovery and
cleanup before a failing reload are covered. Existing completion/gesture
regressions run with no retired assertions. Guard regression fixtures include
`Any`, `AnyLabel`, string conversions, raw identifiers, method macros, aliases,
filtered/string methods and constrained-adapter targets, plus nested-comment and
raw-string false positives.

The command reference now contains **180 commands**, including
`set_waveform_subscription`; all maintained counts are updated. Exact Linux,
macOS and CI results are recorded in the draft PR. No native synthetic input is
sent while another task's Murmur occupies the Mac mini GUI. No production data,
logs, transcript, audio, clipboard contents or WebKit directory are inspected.
