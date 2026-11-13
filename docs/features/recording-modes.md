# Recording Modes

The app supports Hold-Down, Double-Tap, and a combined Both mode, selectable in Settings. All use `rdev` for low-level keyboard event listening and require Accessibility permission.

## Transform hold key (issue #312)


## Fn and Globe key-down capture

Fn / Globe (`fn`) works as a dictation trigger in Hold, Double-Tap, and Both
modes. rdev reports it as `Key::Function`: a `FlagsChanged` event for keycode 63,
which is a press or release of the secondary-Fn flag. Like Shift, Option, and
Control, it is a modifier-only trigger. It gets the same 150 ms hold arming
window in Hold mode and the 200 ms deferred promotion in Both mode.

**Key-down capture (Fn only).** Fn is the one trigger that opens the microphone
before the gesture is confirmed. When an Fn press begins a fresh gesture, the
listener asks the audio supervisor for a speculative capture. A fresh gesture
means the detectors were idle, no Command/Shift/Option/Control is held, the
press is not the single tap that stops a toggle recording, and Murmur is neither
processing nor disabled. The capture opens the same input the next dictation
would use, which the frontend mirrors through `set_fn_capture_input`. It is
admitted only when microphone permission is already granted and nothing else
owns the microphone. It never prompts. The grant is checked at keyboard
admission, at attempt start, after the helper handshake, and finally by the
capture helper itself (`StartPreauthorized`): immediately before native setup
begins, and again on entering each step that touches the input (binding the
device, opening the stream, starting IO). Earlier setup steps never open
input, and each TCC query costs about 5 ms, so they are not re-checked. A
grant lost at any of those points refuses the
capture. It shows no UI and publishes no levels.

- **Confirmed** (`hold-down-start` or `double-tap-toggle`): the listener binds
  the capture to that gesture token before emitting the event. If
  `start_native_recording` for the same token, input, and topology arrives
  within the backstop, it adopts the running worker. Audio retained since
  key-down becomes the start of the recording. If PCM is already flowing, the
  start skips the visible "starting"/Connecting phase.
- **Not confirmed**: a stray tap, Fn plus another key, holding too long for a
  tap, a second tap that never comes, Escape, a trigger-key or listener change,
  processing, disable, or a refused start. The capture is discarded as soon as
  the gesture is known to have ended. Under the buffer lock, its retained
  samples are cleared and retention is revoked, so PCM still in flight or
  flushed by the helper while it drains Stop is dropped too. The exact worker
  is then stopped through the supervisor's confirmed teardown:
  cooperative stop within 250 ms, then a process-group kill within the 2 s
  termination budget. Ownership is kept until exit is confirmed. Discarded
  audio is never transcribed, written to disk, or logged.
- **Backstop**: the supervisor discards any capture that no start adopted
  within 1.5 s of key-down (`SPECULATIVE_CAPTURE_TTL`). It is measured from the
  keyboard listener's key-down stamp, not from supervisor acceptance, and a
  request that arrives after it has elapsed is never opened.

- **Settings microphone test**: an open test never delays a dictation or
  clips its first word. When the visible test is flowing on the same input,
  Fn key-down takes over its open device in place
  (`audio.preview_handed_off`): no teardown, no reopen, and PCM from key-down.
  From there the capture is an ordinary unconfirmed key-down capture: it is
  adopted by its confirmed start or discarded with its samples. A confirmed
  start with no key-down capture (any other trigger) adopts the flowing test
  the same way. Nothing the test heard before the handoff is kept. The
  taken-over device was opened before the dictation, so it counts as a warm
  capture: like an adopted key-down capture, a newer input topology cancels
  it.
  A test that is not taken over is stopped first. The confirmed start then
  queues behind the test's confirmed exit (`audio.preview_yielded`) and opens
  its input cold, waiting up to `START_AFTER_PREVIEW_TIMEOUT` for that exit.
  What key-down does depends on why the test was not taken over:
  - Connecting, stopping, failed or automatic test: the app skips key-down
    (`keyboard.fn_capture_skipped reason="preview_active"`).
  - Flowing test on another input, or opened before the latest topology
    change: the app admits key-down, and the supervisor refuses it
    (`audio.speculative_capture_refused reason=preview_not_handed_off`).

  A test that restarts while an unconfirmed key-down capture is open waits
  for the gesture to resolve (bounded by the backstop) instead of superseding
  it. Closing or hiding Settings during that wait revokes the restart, so a
  plain Fn tap never reopens the microphone with Settings closed. A dictation
  that wins the race pauses the test quietly ("Level monitoring pauses…")
  rather than showing an error, and the test resumes when the dictation ends.
- **Diagnostics**: `keyboard.fn_capture_skipped` carries the first blocking
  fact as `reason` (`listener_changed`, `processing_skipped`, `disabled`,
  `gesture_ended`, `permission_not_granted`, `dictation_busy`,
  `preview_active`, `microphone_busy`, `cleanup_pending`,
  `input_unconfigured`, or `smart_auto_unavailable`). A capture the helper
  could not open or keep logs `audio.speculative_capture_discarded` with
  `reason=capture_failed` and the typed `failure_kind` and `failure_phase`. A
  supervisor refusal names the busy owner kind. None of these carry device
  names, error text, or audio.

The key-down start uses production helper protocol v11, which adds
`StartPreauthorized`. A helper built before that change fails the handshake
instead of exiting on the unknown message, so rebuild the helpers
(`python3 scripts/build_capture_helpers.py`) after pulling this change.

In Hold mode, the microphone is on only while Fn is held, and after that only
if the hold became a dictation. In Double-Tap mode, the capture stays open
through the gap between taps, so the microphone indicator can stay on for up
to about 400 ms after the first release, plus teardown. Other trigger keys
never open the microphone before a confirmed start.

**Globe key action.** macOS may also act on Fn / Globe (emoji picker, input
source switching, dictation, or nothing), depending on System Settings ›
Keyboard › "Press 🌐 key to". Murmur's listener is listen-only and does not
suppress it. Murmur does not change that setting for you. When Fn is selected,
the trigger-key picker in Settings and onboarding shows a hint to set it to
"Do Nothing".

**Globe companion event.** When macOS performs the Globe action it also posts
an ordinary key-down/key-up pair for the Globe key (keycode 179), confirmed
natively. rdev reports it as `Key::Unknown(179)`, and it arrives within a few
milliseconds of the Fn release. With Fn as the trigger, the listener drops a
179 press that arrives within 100 ms of an Fn edge, and the one release that
balances it, before any detector sees them. Without this, every Fn double-tap
is cancelled as a chord. Each dropped event logs
`keyboard.globe_companion_ignored` with `companion=globe`. Other `Unknown`
codes are real keys (keypad digits, ISO Section, JIS Yen) and still cancel an
Fn gesture, as does a 179 press outside the window.

**Fn held with other shortcuts.** Murmur cannot consume or alter keys, so it
cannot block a system shortcut. Pressing a non-modifier key during a promoted
Fn hold ends that hold, as it does for every hold trigger. macOS does not
treat ⇧⌘4 with Fn/Globe held as the plain ⇧⌘4 screenshot shortcut, so the
screenshot does not fire. Release Fn before using such a shortcut, or choose a
different trigger key.

## Hold-Down Mode (default)

Hold the configured trigger key to record, then release it to stop and transcribe.

**Behavior:** Press and hold the trigger key to start recording. Release it to stop recording and begin transcription.

**Available keys:**

| Setting Value | Key |
|---------------|-----|
| `shift_l` | Left Shift |
| `alt_l` | Left Option |
| `ctrl_r` | Right Control |
| `fn` | 🌐 Fn / Globe |
| `f1` through `f20` | F1 through F20 |

On Apple keyboards, F1 through F12 can control brightness, audio, and other
system features. Those keys may require holding Fn or enabling "Use F1, F2,
etc. keys as standard function keys" in macOS Keyboard settings. Murmur observes
global key events and does not suppress the key's normal system or app action.

**Requires Accessibility permission** (rdev needs it for global keyboard events).

### State Machine (`HoldDownDetector` in `keyboard.rs`)

To start:

```text
Idle → KeyPress(target) → Held  (emit hold-down-start)
```

For a modifier-only trigger (Shift, Option, Control), `Held` begins with a
150ms arming window (`MODIFIER_HOLD_ARMING_MS`) before the listener commits the
start:

```text
Idle → KeyPress(target) → Held (arming; nothing emitted)
Held (arming) → any other KeyPress  → Idle (silent; no start)
Held (arming) → KeyRelease(target)  → Idle (silent; no start)
Held (arming) → 150ms elapse, still held → emit hold-down-start
```

Function-key triggers keep starting on press.

To stop:

```text
Held → KeyRelease(target) → Idle (emit hold-down-stop)
```

### Modifier arming window

Shift is pressed for every capital letter, and Option, Control, and Command
start ordinary shortcuts. Before the arming window existed, each of those
presses started a recording: the frontend requested a dictation, the capture
helper and AUHAL unit spawned, and the overlay flashed. The next key then
cancelled the start before capture was ready. In production telemetry those
`cancelled_starting` outcomes came a p50 of 81–98ms after
`pipeline.dictation_requested`, while capture-ready p50 is ~220ms.

The window removes them. It runs in Rust before anything reaches the frontend,
so a cancelled press never emits `hold-down-start`, never calls
`start_native_recording`, and never shows the overlay. During the window **any**
other key press ends the hold, including a second modifier (Shift+Command+Z).
Releasing the trigger within the window is treated as a stray tap. After the
window, another modifier is ignored. Space is ignored, and any other
non-modifier key stops the recording until the hold is one second old (see
[Typing during a hold](#typing-during-a-hold)).

Each cancelled window logs one content-free `keyboard.hotkey_arming_cancelled`
event. It carries only `mode`, `reason` (`other_key` or `released`), and
`held_ms`, never the key that ended the hold.

**Timing trade-off.** The arming window adds 150ms to a modifier hold's start.
It is longer than the measured p50 gap between the trigger and the next key
(~81–98ms), but shorter than Both mode's 200ms promotion
(`MAX_HOLD_DURATION_MS`), so Hold-Down remains the faster-starting mode. Capture
startup (~220–300ms to first PCM) still makes up most of the time to a live
microphone. Speech before first PCM was never captured, so the window adds
150ms to an existing gap rather than opening a new one. There is no pre-roll
buffer, so for a modifier hold the uncaptured interval at the start of speech
grows by ~150–160ms (timer overshoot measured at p50 7.9ms, p95 10.1ms). This
is an accepted trade-off for eliminating typing false starts. Warm model preparation
also waits for the committed start. Shift is pressed constantly while typing,
so starting a model load on every press would defeat idle unloading and cost
memory and CPU for presses that are almost always typing.

The window is deterministic and has no user setting. The promotion uses the same
`HoldPromotion` arbitration as Both mode. A release before the timer fires
invalidates it, a release after publishes its stop after the start, and Escape,
listener changes, entering processing, and disabling the app invalidate a
pending window.

**Release right after the window.** A release that lands just after the
timer publishes `hold-down-start` (a ~160ms press) arrives while the frontend's
`start_native_recording` call is still pending and status is still `idle`.
`useRecordingState` keeps that stop instead of ignoring it. When the start
response returns `recording_starting` with its `recordingId`, the hook calls
`cancel_native_recording({ expectedRecordingId })`. That cancels exactly the
generation the start created and is a no-op for any other generation. A refused
start drops the pending stop. A start response for a generation whose lifecycle
events already ended it (`recovering`, `idle`, or `processing`) is stale. Its
state is not applied, so a late response cannot restore an externally
cancelled start.

**Residual cases.** A modifier held alone for longer than the window before the
next key, for example a slow Shift+arrow selection, still starts and then
cancels. So does Shift+click, because the listener sees only key events.

### Typing during a hold

A non-modifier key pressed after the arming window, while the trigger is still
held, is judged by the key and the hold's age (`HOLD_CHORD_WINDOW_MS`,
1 second). For a modifier or Fn trigger, every key inside the arming window
still ends the hold before a recording starts, so a fast Shift+Space or
Fn+Space never records. F-key triggers have no arming window. In Both mode an
F-key hold still waits 200 ms before it becomes a recording, and a space in
that time is ignored rather than ending the hold, so the recording then starts
and runs until the key is released.

- **Space, at any age**: ignored. The recording continues until the trigger
  is released (#1109).
- **Any other key, hold younger than one second**: a chord with the trigger.
  It ends the hold and emits `hold-down-stop`, as before. A slow Shift+arrow
  selection or an Fn+arrow (Home/End) press lands here. The recording it
  started is stopped and processed like a release. Under 0.3 s of audio it
  ends as `too_short` (or `cancelled_starting` if capture was not ready yet).
  Longer, it is transcribed: silence ends as `no_speech`, and speech in the
  room becomes a short fragment that is delivered.
- **Any other key, hold one second or older**: typing during a dictation. The
  key is ignored and the recording continues until the trigger is released.

The first ignored key of a hold logs one content-free `keyboard.hold_key_ignored`
event with `mode`, `key_class`, and `held_ms`. Later ignored keys, including
auto-repeat, are only counted. When the hold ends, one
`keyboard.hold_keys_ignored_summary` event carries `mode` and the count
`ignored_keys`. Neither event names the key.

Murmur listens only, so the key still reaches the frontmost app as usual.
Ignoring it adds no work to start or stop: the start still waits only for the
arming window, and the stop still fires on the trigger's release event.

In Both mode, a hold promoted over a recording it did not start still ends on
any key, space included, and emits `hold-over-recording-end`. The toggle
recording continues, and the trigger's later release cannot stop it. A
shortcut modifier during a function-key hold still ends the hold at any age.

**Right Control + Space.** With Right Control as the trigger, this is the one
space chord on a supported trigger: macOS's default "Select the previous input
source" shortcut. Typed quickly, inside the 150 ms arming window, it still
never records. Held past the window, the hold now continues until Control is
released, instead of ending at the space. That recording is transcribed on
release: silence ends as `no_speech`, and speech in the room is delivered
into the app the user switched input sources in.

**Fn + letter (Globe shortcuts).** Murmur listens only, so it cannot stop
macOS from running a Globe shortcut such as Globe+E (Emoji & Symbols) or
Globe+N (Notification Center) for a letter typed while Fn is held; observed
natively with HID-level events. In the first second the letter still ends the
hold, as before. After that the shortcut runs and the dictation continues, so
the transcript is delivered on release into whatever has focus then, for
example the emoji picker's search field.

**Known limit: a missed release.** If the trigger's release never reaches the
listener (the event tap was disabled by a timeout, or secure input was on
when the key came up), a hold stays held. Before #1109 the next typed key
ended it. Now the recording continues until the next press and release of
the trigger, or Escape, which discards the recording.

### Rejection Rules

- **Key repeat** while held: Ignored (stays in Held state)
- **Any other key during a modifier trigger's arming window**: Ends the hold silently; no start is emitted
- **Space after the arming window**: Ignored at any hold age; the dictation continues until release
- **Trigger key + another non-modifier within one second** (for example Shift+A or F8+A): Cancels hold, emits stop
- **Another non-modifier after one second of holding**: Ignored; the dictation continues until release
- **Shortcut modifier held before a function key** (for example Command, then F8): Ignored while idle; Fn remains allowed for Apple function-key rows
- **Shortcut modifier pressed during a function-key hold**: Cancels the hold and emits stop if recording already started
- **Cooldown**: 50ms after stop before re-trigger is allowed

### Code Path

- `useHoldDownToggle` hook manages lifecycle (start/stop listener, listen for events)
- Listens for two distinct events: `hold-down-start` and `hold-down-stop`
- Rust `keyboard::start_listener(app_handle, hotkey, "hold_down")` spawns rdev thread
- On key press: emits `"hold-down-start"` event → frontend calls `handleStart()` (modifier triggers: after the arming window)
- On key release: emits `"hold-down-stop"` event → frontend calls `handleStop()`

## Double-Tap Mode

Uses `rdev` for low-level keyboard event listening. Detects quick double-taps on the configured trigger key.

**Behavior:** Double-tap the trigger key to start recording, then tap it once to stop.

**Available keys:** Same as Hold-Down mode (Left Shift, Left Option, Right Control, Fn / Globe, and F1 through F20).

**Requires Accessibility permission** (rdev needs it for global keyboard events).

### State Machine (`DoubleTapDetector` in `keyboard.rs`)

To start (when not recording):

```text
Idle → KeyDown(target) → WaitingFirstUp
WaitingFirstUp → KeyUp(target) within 200ms → WaitingSecondDown
WaitingSecondDown → KeyDown(target) within 400ms → WaitingSecondUp
WaitingSecondUp → KeyUp(target) within 200ms → FIRE
```

To stop (when recording):

```text
Idle → KeyDown(target) → WaitingFirstUp
WaitingFirstUp → KeyUp(target) within 200ms → FIRE
```

### Rejection Rules

- **Held key** (>200ms): Resets to Idle
- **Trigger key + another key** (for example Shift+A or F8+A): Resets on the other KeyPress
- **Shortcut modifier held before a function key** (for example Control, then F8): Does not begin or stop a recording; Fn remains allowed
- **Shortcut modifier pressed during either function-key tap**: Resets the tap sequence
- **Slow gap** between taps (>400ms): A timer resets to Idle at expiry, without waiting for another keyboard event
- **Triple-tap spam**: 50ms cooldown after firing
- **Key repeat events**: Ignored while within hold duration

### Code Path

- `useDoubleTapToggle` hook manages lifecycle (start/stop listener, listen for events)
- Hook syncs recording status to backend via `set_keyboard_recording` command
- Rust `keyboard::start_listener(app_handle, hotkey, "double_tap")` spawns rdev thread
- On detection: emits `"double-tap-toggle"` event to frontend via `app_handle.emit()`
- Frontend event handler calls `toggleRecording()`

### Optional Timing-Miss Feedback

The `hotkeyMissFeedback` setting is off by default. When enabled, expiration of the 400ms second-tap window in Double-Tap or Both mode emits `hotkey-tap-rejected` with `{ reason: "second_tap_expired", mode }`. The overlay shows a distinct amber `Tap missed` flash for 500ms.

Only the expired second-tap window is surfaced. Existing structured diagnostics still record other rejection reasons, but the UI stays silent for long holds, key combinations, processing skips, Both mode's first short tap, and valid double-taps. This prevents ordinary key use from producing feedback noise.

## Shared Infrastructure

### Audio startup lifecycle

All three modes treat `Starting` as an active, cancelable attempt. Hold release
or a second toggle while `Starting` cancels initialization and moves the app to
`Recovering`, closes that generation's publication gate, and requests stop.
The supervisor retains exclusive ownership until the worker exits; a retry
during recovery is rejected, so blocked CoreAudio calls can never overlap.
Late callback events cannot activate a new generation. Double-Tap and Both
synchronize `Starting` as active to the Rust detector, so the next single tap
cancels. Silence auto-stop remains disarmed until the first retained nonempty
buffer drives the `Recording` transition.

The hold/toggle origin is latched across `Starting` and resets after either a
normal recording end or a pre-readiness cancellation. Model preparation begins
when startup is accepted so model loading still overlaps microphone latency.

### Both-mode promotion ordering

The 200ms hold promotion doubles as Both mode's arming window: a press that
ends before promotion never emits `hold-down-start`. For a modifier trigger,
any other key press during those 200ms, including a second modifier, ends the
hold without a start and logs `keyboard.hotkey_arming_cancelled` with
`mode: "both"`. Any key press that ends the window, typed key or second
modifier, resets the double-tap sequence directly instead of being fed to it,
so no key reaches the double-tap rejection log. For a typed key this is the
same state `ComboCancelled` would reach. A Shift+Command press therefore no
longer counts as a first tap. Double-tap detection is otherwise unchanged: the double-tap detector is
fed independently, so short taps still advance it and a single tap still stops
a toggle-started recording.

The 200ms hold promotion and key release share one arbitration lock through
start/stop event publication. A release before promotion invalidates the timer;
a release after promotion publishes its stop after the start. Listener changes,
Escape, processing transitions, and disabling the app invalidate pending presses.
A timer also checks the listener generation and current held state. Reading
backend recording status happens outside this lock to avoid reversing the
backend-to-keyboard lock order. Hold timing is
unchanged.

### Threading

- Both modes share a single `rdev::listen()` background thread (spawned once, lives for app lifetime)
- `set_is_main_thread(false)` is called before `listen()` — this is **critical** on macOS because rdev's keyboard translation calls TIS/TSM APIs that Apple requires on the main thread. Without this flag, the app segfaults on key press.
- rdev is pinned to Murmur's fork by commit revision. Its macOS listener derives modifier press/release directly from the physical keycode and device-specific flag (no cached global modifier state), automatically re-enables a disabled event tap, listens only for key events, and skips key-name translation for modifier events.
- `AtomicBool` (`LISTENER_ACTIVE`) gates event processing without killing the thread
- `DetectorMode` enum (`DoubleTap` | `HoldDown`) determines which detector processes events
- Separate `Mutex`-wrapped detectors: `DOUBLE_TAP_DETECTOR` and `HOLD_DOWN_DETECTOR`

### Escape cancellation

The shared rdev listener emits `escape-cancel` before mode-specific handling and resets the hold-down, double-tap, and transform detectors so a later trigger-key release cannot advance a cancelled flow. Its content-free payload is `{ transformPassId }`: the exact active/queued transform pass for Capturing, Connecting, Listening, Thinking, or ReviewPending, or `null` when Escape did not target a transform. Rust snapshots active ownership on both sides of the status read and fails closed if it changes, then publishes the exact pass's cancellation marker before emitting. `useTransformFlow` mirrors the detector reset only when that ID still matches its local held pass, so a delayed Escape for pass N cannot reset pass N+1. The main-window cancellation listener sends `cancel_transform({ transformPassId })` without an asynchronous status lookup; the backend no-ops unless that exact pass still owns the flow. Including ReviewPending closes the transition-before-focus gap; once the Ready/Failed popover is focusable its local Esc may race the global route, but both carry the same exact pass ID and duplicate cancellation is an idempotent no-op. Applying is left untouched, and a `null` payload falls back to dictation recording/processing cancellation. In-flight duplicate suppression is bounded and keyed per target, so pass N cannot suppress cancellation of N+1.

### Tests

The detector unit tests live in `keyboard.rs` (`#[cfg(test)] mod tests`). Run with:
```bash
cd app/src-tauri && cargo test -- --test-threads=1
```

Single-threaded because timing tests use `sleep()`.

## Settings Integration

All modes share the `doubleTapKey` setting. Its allow-list contains `shift_l`,
`alt_l`, `ctrl_r`, and `f1` through `f20`. Loading settings validates this value,
and the native listener commands reject values outside the same list. The
`recordingMode` setting (`'hold_down' | 'double_tap' | 'both'`) determines which
hook is active.

Settings and onboarding present one grouped picker with modifier keys first and
function keys second. Home, the main header, onboarding's final tip, and the
expanded overlay derive their labels from the same catalog.

All three hooks are always called (React Rules of Hooks) but only the active one registers listeners, via the `enabled` prop.

Mode switching is disabled while recording (`status !== 'idle'`).
