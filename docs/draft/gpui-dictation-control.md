# GPUI 1.2: dictation control

Part of #986. Stack 03; do not merge. Based on `gpui/02-core-crate`.

`murmur-core::keyboard` owns hold/double-tap timing, combo rejection,
preparation signals, and injected clocks. `hold_promotion` owns ordered deferred
hold publication. rdev remains one native listener and converts native input
into content-free core input types; its macOS thread setup is unchanged.

`murmur-core::dictation::Controller` owns launch admission, recording origin,
locked mode, Escape/release/toggle routing, silence accumulation, and Smart Turn
pause policy. `QueueGuard` binds delayed input to the generation visible at
queueing or the exact generation its pending start accepted. Capture, inference,
secure-field proof, delivery, and the ordered transform pipeline retain their
existing implementations. The Tauri adapter schedules work through `AppContext`;
Stop and Cancel always carry an expected recording generation.

Native startup applies a committed v7 settings snapshot before arming. Legacy or
unknown settings still await the existing migration client (settings ownership
and its standalone importer belong to 1.5). The opted-in Smart Auto probe policy
is restored without a main window using the existing approval validation. Full
settings writes and control configuration belong to main; the overlay retains
its two-field patch API. Setup closes native admission under the recording
transition fence and requires an explicit resume after successful setup.

React reads native recording events and sends button/settings requests. Overlay
click disambiguation remains presentation code; lock state is native and is
hydrated after attaching its event listener. Dictation sound cues are synthesized
by the host with the same tones/envelopes, independent of a webview; meeting
sound cues remain React presentation.

## 1.3 seam

`transcription-complete` and the existing delivery-recovery receipts are the
completion boundary. `useRecordingState` still owns History insertion,
completion de-duplication, and dictation statistics; `useLocalStats` still owns
activity counts. No completion/history/statistics behavior is claimed to be
headless in this PR. 1.3 must replace these listeners together, implement the
v1 migration/holding protocol, and satisfy the combined durable exit gate.

## Test port

Rust replacements run in `cargo test -p murmur-core` before the old hook suites
are retired. All times are injected; no tests record a microphone or typed text.

| TypeScript coverage | Rust replacement / retained client coverage |
|---|---|
| `useKeyboardGestureRouting.test.tsx` (hold/double/both tokens, absent token and full u64 precision) | `gesture_tokens_are_forwarded_without_reuse_in_all_modes`; `gestures.rs` covers 200 ms tap, 400 ms gap, 50 ms cooldown, 150/200 ms hold arming and combo rejection. Existing native keyboard integration tests remain. |
| `useEscapeCancel.test.tsx` (recording/processing/idle, disabled registration, duplicate events, disposal) | `escape_idle_processing_starting_and_duplicate_cancellation`, `launch_requires_each_gate_but_disabled_still_allows_escape`, `escape_after_stop_can_cancel_processing_and_disabled_does_not_block_autostop`. There is no per-webview controller subscription to dispose. |
| `silenceAutoStop.test.ts` (floor/relative threshold, cumulative speech, reset, once-only, off, stalled/backwards time, nonfinite and quiet mic) | `detector_threshold_cumulative_speech_resumption_gaps_backwards_and_nonfinite`, `silence_once_after_speech_and_fresh_per_recording`, `disabled_off_and_nonrecording_levels_are_ignored`. Rust unsigned configuration rejects negative durations as Off. |
| `smartTurn.test.ts` (pause identity, due time, once/pause, stopped/unarmed, threshold, stale speech/pause) | `smart_threshold_once_per_pause_and_fallback`, `smart_asks_again_after_low_answer_and_next_pause`, `smart_rejects_resumed_speech_new_pause_new_generation_and_disabled_answer`. |
| `useSilenceAutoStop.test.tsx` (subscription lifecycle, enabled/status/origin/reset, hold-over combo, cancelled/stranded hold, early smart stop, fallback/failure, next pause and stale answer) | The silence/smart tests above plus `hold_exclusion_hold_over_combo_and_cancelled_origin_healing`, `release_before_readiness_cancels_exact_start_and_stale_levels_cannot_stop_new_owner`, and the headless fake-host check. The host is app-lifetime, so main unmount no longer disposes control. |
| `App.captureSafety.test.tsx` hotkey bridge assertions | Native gate/setup behavior is core-owned. Client tests retain setup busy/pending-button cases, gate request and rearm assertions; `delayed_native_inputs_belong_only_to_their_accepted_start` checks queue ownership. |
| `useSoundCues.test.tsx` dictation event subscriptions | Dictation subscriptions are native; the client test asserts React cannot duplicate them. Meeting presentation remains React. Tone frequencies, offsets, duration, volume and exponential envelope are retained in `dictation_cues.rs`. |

Additional Rust tests cover locked start refusal, stale refusal rejection,
Escape after a stop entered Processing, and headless Paste Last routing.

## Headless check

`headless_fake_host_can_arm_start_stop_cancel_autostop_and_paste` runs automatically
on Linux and macOS without Tauri or a webview. It checks the effect seam, including
Paste Last; it does not assert actual OS clipboard insertion or durable counts.

For native verification, `tauri.control-test.conf.json` uses the isolated refactor
test identity and defines only companion windows: **main is never constructed**.
Run from `app`:

```sh
npm ci
npm run tauri:dev -- --config src-tauri/tauri.control-test.conf.json --no-watch
```

Use only an explicitly isolated synthetic profile and synthetic/silent capture
routes. Never aim input at another app or open production data. A native smoke
pass must be reported separately from the fake-host test; the latter does not
prove native input, capture, or OS paste. The combined 1.2/1.3 durable exit gate
remains pending 1.3.
