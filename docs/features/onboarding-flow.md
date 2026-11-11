# First-Launch Setup Assistant

Guided onboarding wizard shown on first launch, replacing the old flow where
the mic TCC prompt only fired on the first recording attempt and missing
permissions were a dismissible banner.

## Flow

Eight steps share a fixed frame with a scrollable content region and a pinned
bottom navigation row. A named stage row stays 32px above the window bottom:
Welcome, Permissions (the four permission steps), Speech model, Shortcut, Ready.
Its completed checks indicate navigation progress, not permission grants.
Welcome centers the existing waveform mark, local-storage copy, and Get started
action in one block. The window header retains its drag region without a wordmark. Back stays
left, while an optional Skip and the primary action stay right. Back remains visible but disabled while a model download is
active, until its exact native download owner confirms cancellation or completion:

1. **Welcome** — explains that dictation runs on this Mac, saved history and retained meeting audio stay local.
2. **Microphone** — fires the *native* macOS permission dialog in-app via the
   `request_microphone_access` command (`AVCaptureDevice.requestAccessForMediaType`,
   fire-and-forget; never opens the device, so it can't duck other apps' audio —
   see issue #177). On arrival with `notDetermined`, wait 600 ms for the explanation
   to render, then request once per wizard session. Back cancels an unfired timer;
   revisits, permission polls, and StrictMode effect replay never duplicate a request.
   The IPC returns immediately; the waiting card and disabled Continue persist until
   polling detects a grant, denial, or unknown result. Skip is hidden while the
   native dialog is pending. Unknown status retains a manual Continue fallback;
   denied status offers System Settings and the existing reset path.
3. **Accessibility** — explains the global recording key + auto-paste need,
   triggers `request_accessibility_permission` (system dialog + opens the pane).
   Skippable; never opens System Settings automatically on arrival.
4. **System Audio (optional)** — creates a short-lived native tap only after an
   explicit Continue click (with copy explaining that macOS will ask for permission),
   reports granted/denied/unsupported, and links a denial to
   Privacy & Security → Screen & System Audio Recording. Skippable.
5. **Calendar (optional)** — reads Calendar permission status only while this
   step is visible. The native permission prompt requires an explicit Continue click,
   explained below the permission card.
   Denied and restricted states explain that manual meeting names still work;
   recovery links to Privacy & Security → Calendars, with a reset action for a
   denied TCC entry. Skippable.
6. **Model** — embeds `ModelDownloadPanel` (shared with the standalone
   `ModelDownloader` gate); reads every offered model's install state from the
   shared runtime catalog and shows "already installed" on re-runs. Core ML
   setup reports bounded phases from a killable worker; terminal failure
   restores Back, Retry, and one-click Whisper Base / CPU Parakeet fallbacks.
   Download/install status, Cancel, memory notices, and terminal errors render
   in a fixed footer above the navigation row, so they stay visible while the
   model list scrolls inside the wizard's fixed-height frame. The right action
   becomes Cancel during an active download, with Back disabled until it settles.
7. **Hotkey** — chooses Hold / Double-tap / Both and a trigger key. The grouped
   picker contains Left Shift, Left Option, Right Control, Fn / Globe, and F1
   through F20. Selecting a function key explains the macOS Fn and
   standard-function-key setting and notes that Murmur does not suppress the
   key's normal action. Selecting Fn / Globe explains key-down capture and the
   "Press 🌐 key to" → "Do Nothing" system setting.
8. **Done** — live summary of the checks plus a quick-start card derived from
   the selected recording mode and trigger key.

Permission rows also expose an inline **Allow…** action while access is missing.
It uses the same native handler as the footer: microphone requests TCC unless
denied (then opens System Settings), Accessibility requests and opens its pane,
System Audio requests capture unless denied, and Calendar requests access unless
denied. Granted rows show Allowed with no Allow button. Waiting actions are
disabled; unsupported/restricted permissions retain their explanatory state
without offering a request that cannot succeed. The existing footer actions,
reset paths, skip choices, polling, and microphone arrival prompt remain.

## Permission-state handling

Microphone and Accessibility poll every second (plus on window focus) for the
whole wizard lifetime, so a grant made during the System Settings roundtrip
flips the step live. System Audio and Calendar status reads run only when their
respective optional steps are visible. Calendar refreshes again on window focus
while its step remains visible. Neither access request runs without an explicit
click.

On any permission step, a fresh transition from not granted to granted announces
success through the persistent live region, animates the Allowed check, and advances
after 900 ms. Already-granted permissions on arrival (including the optional steps'
initial passive probes) stay visible with Continue as the Return default. Manual
Back, Skip, Continue, step changes, and unmount cancel pending navigation, so late
answers on another step never advance it. Reduced motion removes the slide/check
animations but preserves the delay and automatic navigation.

A fresh Accessibility grant detected while Murmur is unfocused invokes
`show_main_window` before the delayed advance. The same applies to a denied
microphone permission granted after opening System Settings. Foreground failures
do not discard the grant or block navigation.

The wishy-washy TCC states are handled explicitly:

| State | UI |
|-------|-----|
| mic `notDetermined` / `unknown` | Settled, once-only auto-request for `notDetermined`; manual Continue fallback for `unknown` |
| mic `denied` | Open System Settings + "Reset the permission" (`tccutil reset` via `reset_microphone_permission`) — the reset returns the status to `notDetermined`, so the in-app dialog works again |
| accessibility not granted | "Open System Settings…" (dialog + pane); after an attempt, a reset path for the listed-but-stale entry (common after reinstall/rebuild — see DEVELOPMENT.md) |
| system audio `unknown` / `denied` | "Continue" runs an explicit check; denial makes "Open System Settings…" the primary action, with a quiet re-check. Passive status reads never create a tap. |
| system audio `unsupported` | Meeting capture requires macOS 14.2+, while the rest of Murmur remains available. |
| calendar `notDetermined` | "Continue" opens the native prompt. No Calendar event query runs during onboarding. |
| calendar `denied` | Manual naming explanation, "Open System Settings…" primary action, and a quiet reset action so macOS can ask again. |
| calendar `restricted` / `unsupported` | Manual naming remains available; Continue advances. |

## Accessibility and development recovery

Every step heading is programmatically focusable. Continue, Skip, and Back move
focus to the new heading, including Welcome on return; the initial Welcome
mount also focuses its heading once, enabling Return → Get started and title
announcement by VoiceOver. Permission polling and window-focus refreshes
do not move focus. Permission failures use alerts, while a persistent polite
status region announces permission-state changes. Decorative checkmarks are
hidden from assistive technology; status pills and summary rows retain meaningful
text alongside Lucide icons.

The fixed 520×560 frame uses a centered setup-assistant hero (64 px accent-tinted
icon tile and 24 px title), full-width permission/model/shortcut cards, and a pinned
navigation row. Step content scrolls, with its scrollbar hidden, without moving progress or navigation. The model
step portals its download status into that fixed area instead of leaving it
below the scrolling model cards. Forward
and Back entry transitions fade and slide 12 px in opposite directions over 220 ms;
progress dots transition over 200 ms, fresh checks over 160 ms. All colors use
semantic appearance tokens, and new recurring geometry uses `--ui-*` tokens.
Playwright screenshot assertions disable animations; the onboarding matrix also
uses reduced motion for deterministic geometry checks.

In `tauri:dev`, permission guidance names the launching terminal. Microphone,
Accessibility, and Calendar Reset actions are hidden because their native
commands target Murmur's bundle identifier rather than that terminal. Turn
the terminal's permission off and on in System Settings, then restart the
dev server. Built apps retain the bundle-targeted Reset actions described above.

## Gating and grandfathering

`App.tsx` checks a localStorage flag (`murmur_onboarding_complete`,
`lib/onboarding.ts`) at mount:

- Flag present → straight to the main UI (the existing `PermissionsBanner`
  still catches later permission drift, and the standalone `ModelDownloader`
  gate still catches a deleted model file).
- Flag absent → probe mic + accessibility + the runtime model catalog. **All three already in
  place → set the flag silently** so existing installs never see the wizard on
  upgrade. Anything missing → show `OnboardingFlow`.

Settings → General → **Run setup assistant…** clears the flag and relaunches the
wizard — the recovery path when a user revokes a permission and wants a guided
re-grant instead of spelunking through System Settings.

## Contextual hints after setup

The wizard remains eight steps. Home goes directly to dictation and history.
Three once-only hints respond to an expired double-tap window, a new history
entry, and an existing teaching context identifying an allow-listed browser.
The local record stores fixed hint IDs, without transcript, selection, site,
window-title or meeting content. Dismissed hints stay dismissed after relaunch.
Both setup-assistant entry points reset hint state.

`murmur_discovery_v2` preserves validated pending/dismissed hint IDs from the
legacy v1 record, filtering unknown IDs individually. Invalid v2 data falls back
to v1; valid dismissals from both versions are preserved. Retired progress fields
are ignored. Recoverable v1 data is removed only after v2 is written successfully;
unreadable old JSON is removed. Migration can be retried safely.
No native settings or registered commands changed.

## Files

| File | Purpose |
|------|---------|
| `app/src/components/onboarding/OnboardingFlow.tsx` | The wizard |
| `app/src/lib/onboarding.ts` | Completion-flag persistence |
| `app/src/lib/discovery.ts` | Once-only hint persistence and legacy migration |
| `app/src/lib/hooks/useDiscovery.ts` | Existing-event hint listeners |
| `app/src/components/ModelDownloader.tsx` | `ModelDownloadPanel` extracted for reuse |
| `app/src-tauri/src/commands/permissions.rs` | `request_microphone_access` (block2 completion handler) |

## Visual fixture controls

`visual-fixtures.html?state=first-run-onboarding` reproduces Welcome; use its real
Get started / Skip / Continue controls to reach Permissions and Speech model.
`permission=unknown|granted|denied` controls native-boundary permission responses;
inline Allow requests resolve through those mocked commands and subsequent polls.
`state=first-run-model` shows the standalone gate. Add
`download=progress|indeterminate|failed`, then press Download to exercise the real
picker's progress, cancel, failure, retry, and explicit fallback branches. These
are deterministic fixture signals, not actual downloads or OS permission grants.
Both first-run fixtures provide an explicit Apple M4 hardware-guidance response;
production uses the native hardware probe. All states support `appearance=light`
and `appearance=dark`, at 1100×780 and 840×620, without injected CSS or DOM edits.

The native title bar stays above Welcome's centered wrapper. Every wizard step
and the standalone model gate retain hit-testable native drag regions without
moving the approved content. Model cards use an inset keyboard focus outline
inside the scrolling wizard column, including the first and last choices.
