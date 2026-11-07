# GPUI 3.5b: Settings

Part of #1003 / #1011. Stack layer 14 starts at `ab58ac46` on
`gpui/13-history` (#1031), through #1030 and the earlier layers to #1015.
**Draft; do not merge independently.** The Phase 0 native-parity decision still
applies: this workspace is a native view inside `gpui_base::Root`, not a new
production host or an intermediate mixed WebKit window.

## Surface and parity

`Settings::new(host, window, cx)` uses the established shared native controls,
semantic palette, display preferences, and targeted Main consumer. Call
`attach` then `follow` when showing it, and `hide` when leaving the workspace.
The host supplies existing services, not another settings store or recorder.

| Existing React owner | Native behavior in this layer |
|---|---|
| General | Opens by default and from Command-comma; launch-at-login preference, live words, overlay placement, shortcuts, setup/checklist, updates, remembered Advanced calibration and statistics reset |
| Recording | Exact saved input including unavailable/stale inventory, local input-level state and quiet/clipping guidance, capture locks, VAD range, trigger/key/silence choices, standby/pre-roll, sound controls and previews, Smart Auto consents and approved/preferred inputs, Smart Turn install/remove requests |
| Customize | Independent Text and Modes routes with Back and caller focus; Voice commands and App overrides use the existing injected sheet destinations |
| Delivery | Effective Auto-paste pauses during either file output without overwriting the saved choice; dependent delay/destination controls, live Paste Last choice, separate transcript/audio output, folder selection, retention and capability-gated NotchPill; Advanced app-editor launch |
| Meetings | Separate suggestion consent and Calendar settings action, audio retention, speaker-label consent and model management, Advanced echo and retention limits |
| Text & vocabulary | Punctuation, cleanup dependencies, vocabulary/alias/knowledge entry points, Advanced corrections, homophones, formatting, spoken commands and project-scan entry |
| AI & models | Speech-to-text drill-in with Back/caller focus, exact catalog selection, English-only language lock, model-storage owner entry and Advanced idle policy |
| Modes / Appearance | Native Settings destinations and explicit owner entry points; the feature matrix assigns their complete managers, parsers and editors to 3.5e |
| Performance Lab | Explicit model selection using installed-model facts, run/cancel/progress, saved report export/clear, report preferences and microphone startup test/cancel requests; opening a pane never starts work |
| Diagnostics | Entry to the existing native Diagnostics owner; it retains its own data, subscriptions and disclosure boundaries |
| Keyboard shortcuts | Read-only current global bindings and fixed main-window shortcuts, paused recording status, links to owners, nested Back focus restoration |

Settings search includes preference rows, dependent/Advanced controls, editor
entry points and Tools. Searching never enables a feature, grants permission,
starts a recording, runs a diagnostic or downloads a model. An off dependency
reveals its controlling preference. Search reveals once and leaves Advanced
under user control. Unknown route aliases fall back to General, matching React.

The expanded navigation is 240 points; compact navigation is a 56-point rail
with full accessible names. Focus transfers between the rail and composite on
resize. Rows stack when their usable content width is below 512 points. Labels
and descriptions wrap; controls use the shared type, radius and spacing roles.
The native component library's disclosure is immediate: no hidden children
remain in the Tab/AX tree and no animation delays removal. Saved backend values
remain independent of disclosure. Reduce Motion introduces no delayed motion.

## State, persistence and lifetime

- `schema.rs` is a presentation catalog. `murmur_core::state` owns schemas,
  defaults, migrations and validation. `settings_edit` validates one explicit
  field edit, rejects fallback/coercion as a successful user choice, rejects
  metadata/unknown field edits and preserves unrelated fields. Future schema
  versions fail closed. No frontend migration or durable owner is added.
- The host registers `Consumer::Main` before reading. Full source/runtime
  snapshots carry a monotonic revision; replies also carry the original view
  attachment and request generation. A notification supersedes stale reads and
  revokes unaccepted effects. Hiding closes the bounded inbox, stops the exact
  subscription and operational page lifetime, and releases local private state.
- Preference admission belongs to the existing host owner. It checks the lease
  and expected revision synchronously, retains accepted user intent, and does
  blocking publication off the UI thread. Closing the presentation must not
  cancel an already accepted preference. File-backed settings use the ordered
  host merge API under `SOURCE_WRITE_LOCK` and core `save_source`; disclosure
  uses completion's `lifecycle::Operation::Save` and `ui-state.json`. Never wait
  on completion while holding the source lock. Owned Auto-paste/disabled intent
  fences remain the existing host's responsibility.
- Capture-sensitive changes and all operational actions must revalidate native
  service admission at their effect, independently of UI disablement. No action
  re-resolves an active recording's immutable configuration. Async recording
  work rechecks the recording generation and stops only its owner.
- Ordinary preference failure leaves the committed value selected; invalid
  numeric input retains a correction opportunity and a fixed error message.
  Disclosure persistence failure retains its UI-only session choice. Errors
  and inbox failures never format source values, paths or service error text.
- Ranges support pointer, arrows, Home/End and AX increment/decrement. Pointer
  changes retain a draft and commit on release, blur, window deactivation or
  leaving; keyboard/AX steps commit immediately. The final draft is handed to
  the existing owner on hide/drop. Committed values never depend on a webview.
- Editor requests preserve the caller. The owning sheet calls `editor_closed`
  after dismissal; Settings never steals focus when opening an editor is
  merely acknowledged. Nested shortcut-owner navigation keeps the return stack.
- Calibration sends bounded one-point preview requests and explicit Save/Cancel.
  Only Save publishes a durable offset. Page exit/hide restores the durable
  placement through the host's page-lifetime teardown. Rust/native code owns
  geometry and queues all NSWindow mutations on the main thread.

The only vendored delta is `settings-slider.patch`, applied after the existing
component patches. It adds opt-in keyboard focus/arrow handling to the base
slider and routes assistive changes through its ordinary change event. The
patch preserves attribution and changes no dependency pin or WebKit guard.

## Verification and native acceptance

`Settings` tests use real Root/view paths, key down/up events and the patched
AccessKit test transport. They cover catalog and React routes, every pane's
control roles/names, Tab order, picker save/reversion, collapsed AX exclusion,
search without consent side effects, nested and sheet caller focus, compact
focus transfer, numeric validation, real core settings/disclosure persistence,
range keys/AX/gesture commits, destructive confirmation, calibration and
revision/lifetime fencing. The isolated fixture host never invokes OS services.
No existing React, Tauri or native-control suite is retired.

`settings-qa` uses only synthetic state. `scripts/check_gpui_goldens.py` includes
all 11 destinations, drill-ins, six Advanced panes, compact/search/loading/error
states in light, dark and Increase Contrast. `--settings-only` runs that family
independently when a pre-existing earlier-family failure prevents reaching it;
the default runner and every zero-WebKit assertion remain strict.

Verification uses the Mac mini through Fleet via opti with Rust 1.98.1 and the
fresh task checkout `~/Builds/gpui-checkout-68`. Exact final counts, source hashes,
capture inventories and native observations belong in the PR and immutable Fleet
artifact. The CI library-only manifest recipe is confined to that checkout.

The GUI owner still performs integrated pointer/keyboard/VoiceOver acceptance
on the exact built revision: every pane at 1120×820 and 720×560, search into off
features, focus return from all owning editors, actual permission/registration
failure, microphone pause/switch/resume, true native model/updater/tool actions,
and calibration Save/Cancel. This worker uses no HID, VoiceOver, AX actions or
focus mutation on the desktop. Read-only AX and exact own-window capture prove
rendering/adapter exposure only. Full production adapters and whole-main-window
switching retain their Phase 2 gates. Independent Sol high review belongs to the
lead; this implementation sub-agent starts no delegates and never merges.
