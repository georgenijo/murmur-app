# GPUI 3.5d: Meetings

Part of #1005 / #1011. Layer 16 starts at `c1dce176` on
`gpui/15-onboarding-home` (#1033), following History (#1031) and Settings (#1032).
Draft stacked layer; do not merge independently. `Meetings` is a native Root
view. The shipping React UI and its tests remain active. Like the preceding
layers, Phase 2 injects production host adapters; no production route is flipped.

## Parity and ownership

The native workspace covers capture status, explicit System Audio checks,
permission denial/unsupported/relaunch guidance, elapsed time, independent
Me/Them indicators, echo-reduction notices, saved meeting search/pagination,
metadata naming, Calendar selection, channel and remote speaker names,
transcript review, native playback and all five export formats. Live segment
projection rechecks recording generation and session, deduplicates/orders the
200-row live tail and leaves durable transcript publication in core. Explicit
overlay suggestion acceptance carries its opaque token to normal start admission. Empty, loading,
service failure/retry, pending/failed transcript segments, unretained audio,
busy output, bundle omissions and limits remain explicit states.

`meetings/host.rs` maps intents to existing command names and wire payloads.
Capture configuration comes from the existing settings keys and defaults; Start
resolves it once under native admission. Stop carries the displayed generation;
the adapter rechecks it before stopping, never a new owner. No new store,
persisted key, migration, transform pipeline, inference owner or calendar cache
is introduced. Durable writes, raw evidence, atomic export and deletion remain
owned by the existing services. Copy is injected native clipboard work; fixture
QA never reads or writes the real pasteboard.

The bounded workspace inbox subscribes Main before reading. Attachment,
request generation and source revision fence every reply. Search changes,
selection, picker cancellation and hide revoke old leases. The host checks
those leases and exact service ownership before asynchronous side effects.
Calendar candidates require explicit selection and Apply, including a single
match. Cancel/hide drops the ephemeral candidates; tokens are revalidated by
Rust on Apply. Denied/restricted/unsupported reads keep manual naming usable.
Speaker names never change canonical channels, timestamps or raw transcript.
Labels-only review saves preserve legacy derived bytes and review revisions.

Text inputs use the shared editor for composition, selection, undo and native
Edit-menu handling; attendees use a multiline editor. Escape cancels naming or
the Calendar picker and restores the originating control. Search has the React
250 ms debounce plus Enter and Escape; Cmd-F focuses search. Deletion requires
two distinct activations within four seconds, and Return is never a destructive
default. Compact layout stacks the list and review rather than clipping them.
The saved-meeting list renders only visible rows and can page through the
existing 10,000-session retention ceiling. Transcript shaping is bounded to 24 segments per page and 2,048 characters per
text page; all text remains reachable. Semantic palettes/shared controls carry
focus, contrast and reduced-motion behavior. Rust retains native window geometry.

## Native retained-audio path

`meetings/playback.rs` replaces the Web Audio scheduling dependency with an
AVAudioEngine output sink. The injected Source uses the existing validated
`get_meeting_audio_manifest`, `read_meeting_audio_range` and
`get_meeting_audio_capture_busy` services. Only IDs cross that boundary; no
frontend or native output code opens paths. Existing consent, finished-session,
file identity, symlink/hardlink and revision checks remain authoritative.

The player reads 128-entry metadata pages and <=64 KiB ranges, validates the
canonical PCM16/16 kHz mono WAV, and holds at most current/next decoded chunks
per channel. It mixes saved capture offsets into 100 ms mono output blocks;
overlaps add at recorded levels, gaps remain silence, and no normalization or
whole-session buffer is introduced. At most 300 ms is scheduled. Native
completion counters report played position; pause/seek/channel/hide/busy/delete
revoke a generation and clear queued output. Start/enqueue/reset share native
serialization, so a cancellation between a Rust check and enqueue cannot resume
old audio. Completion blocks retain per-generation counters rather than the
output object or its lock, preventing stale callbacks from changing new state.
No microphone/input node, TCC request, webview or model is involved.

## Verification handoff

Gallery-gated `meetings-qa` is exclusively synthetic. Its app identity is separate
from production; it has no production data/permission/capture/clipboard adapter.
The optional native output mode and audio self-test use quiet generated tones.
`check_gpui_goldens.py --meetings-only` renders Light/Dark/Increase Contrast twice
and requires exact byte equality. Every capture checks native attribution,
including text focus/typing; launch-only inventory must be zero and native linkage
must pass. `webkit-policy.log` also preserves a failing inventory. No attribution
policy or Writing Tools exception is added here.

`check_gpui_meetings_native.py` uses only AX reads/AXPress on its own subprocess
and internal input hooks. It requests no trust, TCC grant, activation, synthetic
keyboard/mouse or VoiceOver. Missing trust is a missing check. Actual native text
focus/input checks remain separate from headless goldens because AppKit Writing
Tools can load frameworks only on the native path.

The later interactive/VoiceOver pass must cover:

- Real native Edit menu/IME/selection, metadata multiline input, search debounce,
  keyboard/AX playback seeking and channel switching, focus return after naming,
  Calendar cancellation, scrolling and all export-format controls.
- 1120×820 and 720×560, Light/Dark/Increase Contrast, reduced motion/transparency,
  long transcripts, pending/failed segments and conservative remote labels.
- Capture generation races and frozen configuration through the production host;
  real permission attribution only in an explicitly authorized separate pass;
  busy admission across dictation/import/recovery/microphone preview.
- Calendar denial/reset/return, changed-token rejection and manual fallback;
  all transcript/caption formats, save-panel cancellation, bundle omissions/limit,
  deletion during playback/reads/export, retention pruning and session isolation.
- Native output pause/resume, overlapping Me/Them, long silent gaps, end-of-audio,
  segment play, unavailable/corrupt files and route/device changes.
- Writing Tools inventory with native focused typing and any contextual editor
  commands, without changing the shared WebKit policy.

Exact test results, remaining CI state, native evidence and build provenance
belong in the draft PR and immutable Fleet artifact. This implementation child
starts no delegates, does not watch the PR and never merges. Independent Sol high
review remains the lead's gate.

## Observed packaged-app WebKit blocker

At product revision `da2cbbad`, unbundled native AX/input/audio checks and all 75
headless golden cases pass with zero WebKit images. The ad hoc `.app` bundle has
a zero-image launch-only baseline, but its actual AppKit run loads **seven
unattributed WebKit images** from `WritingToolsUILibraryCore` →
`+[NSWritingToolsCoordinator isWritingToolsAvailable]` →
`-[NSApplication(NSMenuUpdating) _addTextInputMenuItems:]` during
`finishLaunching`. Its unchanged strict policy asserts at shutdown. This is a
failed packaged-app gate, consistent with the known sibling-layer Writing Tools
work; it is not an accessibility-attributed exception. The fixture flow and
native output still run, and the artifact retains the exact failing inventory.

Use `check_gpui_meetings_native.py --binary <review.app>/Contents/MacOS/meetings-qa
--evidence <separate-directory>` for the packaged gate as well as the default
unbundled gate. Repeat it after the shared AppKit fix reaches this stack. No
policy allowance, production identity or OS setting was changed. This dependency
and the user-reserved interactive/VoiceOver pass remain explicit gates.

Native review app, window recording, passing unbundled checks and failing packaged
Writing Tools inventories: [immutable Fleet artifact](https://george-ubuntu.tail1fdfb0.ts.net:8844/a/art_20261107T085056Z_685f66750f/),
ID `art_20261107T085056Z_685f66750f`. Product build `da2cbbad`; subsequent
changes only document the blocker and expose packaged QA CLI arguments.
