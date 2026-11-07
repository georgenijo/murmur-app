# GPUI 3.5c: Onboarding and Home

Part of #1004 / #1011. Layer 15 starts at `0875b1b5` on
`gpui/14-settings` (#1032), following History (#1031).
**Draft; do not merge independently.** This is the native-parity route:
`Onboarding` and `Home` are views inside `gpui_base::Root`. The shipping Tauri
main window and its React tests remain intact; Phase 2 injects production owners.

## Parity and ownership

- Onboarding preserves the eight ordered steps, headings, permission guidance,
  optional skips, selected-model/shortcut summary and completion retry. Existing
  permissions and any installed model grandfather a synthetic install; a completed
  gate whose selected model is missing shows the model-only gate. Rerun setup
  explicitly sets `replay` before attach and starts at Welcome.
- Core settings schemas/defaults remain authoritative. Persisted names are
  `model`, `recordingMode`, `doubleTapKey`, `launchOnboardingComplete`,
  `murmur_onboarding_complete` and `murmur_discovery_v1`. No new durable store,
  preference, migration or recording controller is introduced. Completion calls
  the existing configure/resume owners before durable completion and discovery.
  Permission skips never fabricate grants or prevent later setup completion.
- `onboarding/host.rs` maps permission/model actions to the exact existing commands.
  Passive permission observation does not open audio. Calendar is not probed on
  unrelated visits. Microphone requests settle for 600 ms, run automatically once,
  and distinguish IPC completion from the 15-second macOS-answer wait. Microphone
  and Accessibility poll each second; `refresh` handles native window return.
  Entry grants are baselines; fresh grants advance after 900 ms. Navigation/hide
  cancel presentation timers and revoke previous visit replies.
- The model step offers the same four curated choices plus every installed
  noncurated model. It prefers the configured installed model, then the first
  installed catalog model. Installed selections continue without downloading.
  Hardware advice never overrides selection. Progress is determinate or
  indeterminate; cancellation keeps Back locked until exact termination.
- Home composes the existing native `History`; it owns no transcript snapshot,
  second search, copy/export/delete logic, completion writer or statistics writer.
  Its recording title/hint covers starting, listening, processing, recovery,
  meeting exclusion and initialization. Stop carries the displayed recording
  generation; the host independently checks native admission and ownership.
- Home displays the existing monthly words/dictations projection, discovery
  checklist and three fixed discovery hints. Checklist clicks for meeting,
  correction and mode binding never claim completion: existing source evidence
  and baselines own that proof. Hint actions dismiss once and route to their
  existing owner. Settings, Notetaker, Insights and the command palette remain
  injected destinations; managers belonging to 3.5d/e are not duplicated here.
- Home navigation uses a 256–384-point expanded sidebar and a 48-point rail below
  768 points. Toggle and breakpoint transitions restore separator focus. Width
  is session-local, matching React. Rust owns native geometry; hosts dispatch
  NSWindow mutations on the main thread. Shared controls, semantic palettes,
  focus rings, reduced motion and transparency remain in use.
- Permission warnings preserve genuine denied versus unknown/not-determined
  microphone states, optional System Audio priority, explicit reset/recheck,
  session dismissal, and renewed warning on a fresh System Audio denial.

`workspace.rs` shares only the bounded presentation transport between these two
new ports. Main subscribes before reading; attachment, request generation and
source revision reject stale work. Notifications supersede pending reads/actions.
Progress retains its exact original ticket; the host additionally correlates its
native request ID and attempt. Hiding/drop closes the inbox, stops the exact
subscription and releases local state. Accepted durable intent remains owned by
the service; UI teardown only revokes unaccepted effects and late UI replies.
No private payloads or raw service error strings are formatted into logs.

## Verification and review handoff

The real Root views have keyboard, AccessKit and lifecycle tests. They cover
step progression/optional skips, baseline versus fresh grants, request timing,
stale replies, model availability, cancellation, completion failure/retry,
recording-generation stop, meeting/busy exclusion, sidebar bounds/focus,
checklist evidence, embedded History and hide/reopen. No existing suite is retired.

Gallery-gated `onboarding-qa` and `home-qa` use synthetic hosts exclusively.
They can be bundled as separate review apps without a production identity,
permissions, capture, model download, clipboard or persistence. Review apps are
not release artifacts. Goldens cover Light, Dark, imported Midnight and Increase
Contrast, including compact, loading, denied, unsupported, restricted, relaunch,
model failure/progress, all wizard steps and Home recording states.
The runner keeps strict repeated byte equality, `check_inventory`, each family's
zero-image launch/linkage baseline and the shared `webkit-policy.log`.
No WebKit policy allowlist or shared native attribution code changes here.

`scripts/check_gpui_onboarding_home_native.py` optionally drives only the freshly
built synthetic test PIDs through native AXPress and read APIs. It never requests
focus, posts HID input, runs VoiceOver or prompts for TCC. Missing existing trust
is a failed/missing check, never grounds to change OS permissions.

The lead's separate native interactive/VoiceOver pass must cover:

- Both appearances plus Midnight, 1120×820 / 880×720 / 720×560, native titlebar
  clearance, sidebar pointer resize/cancel/blur, keyboard/AX resize, breakpoint
  focus return, scrolling and Reduce Motion/Transparency/Increase Contrast.
- Welcome Return/default scope; heading and Back focus on every step; native
  microphone prompt timeout, denial/reset, return from System Settings, fresh
  grant announcements, optional unsupported/restricted/relaunch states, skip
  without Calendar or tap access, and no stale navigation after Back/hide.
- Installed and missing models, recommendations, progress/retry/fallback,
  exact download cancellation, final shortcut summary, completion failure/retry,
  durable gate/configuration/discovery ordering, rerun and grandfathered installs.
- Home real recording start/cancel/stop and generation races; meeting/import/
  recovery exclusion; destination fallback; complete History keyboard/AX actions;
  month-boundary counters; checklist/hints once-only persistence and destinations;
  Settings/editor return focus and native Edit-menu/IME shortcut behavior.

Production adapters, actual permission grants, global shortcuts, audio/delivery
and signed identity continuity retain their later host/QA gates. The implementation
sub-agent starts no delegates, watches no PR, never merges, and leaves independent
Sol high review to the lead. Exact Mac mini test counts, captures, source/binary
hashes, native AX observations and remaining CI state belong in the draft PR
and immutable Fleet artifact.
