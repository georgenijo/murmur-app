# First-Launch Setup Assistant

Guided onboarding wizard shown on first launch, replacing the old flow where
the mic TCC prompt only fired on the first recording attempt and missing
permissions were a dismissible banner.

## Flow

Six steps with a persistent top-left Back control after Welcome and per-step
Skip where the app can partially function without the grant. Back remains
visible but disabled while a model download is active, because leaving the
step cannot cancel the underlying download safely:

1. **Welcome** — privacy pitch (local-only processing), what setup covers.
2. **Microphone** — fires the *native* macOS permission dialog in-app via the
   `request_microphone_access` command (`AVCaptureDevice.requestAccessForMediaType`,
   fire-and-forget; never opens the device, so it can't duck other apps' audio —
   see issue #177). Skippable, but Continue stays disabled until granted.
3. **Accessibility** — explains the global recording key + auto-paste need,
   triggers `request_accessibility_permission` (system dialog + opens the pane).
   Skippable.
4. **Model** — embeds `ModelDownloadPanel` (shared with the standalone
   `ModelDownloader` gate); reads every offered model's install state from the
   shared runtime catalog and shows "already installed" on re-runs.
5. **Hotkey** — chooses Hold / Double-tap / Both and the trigger key.
6. **Done** — live summary of the checks plus a quick-start card derived from
   the selected recording mode and trigger key.

## Permission-state handling

Both permission steps poll every second (plus on window focus) for the whole
wizard lifetime, so a grant made during the System Settings roundtrip flips the
step live. The wishy-washy TCC states are handled explicitly:

| State | UI |
|-------|-----|
| mic `notDetermined` / `unknown` | "Allow Microphone Access" → native in-app dialog |
| mic `denied` | Open System Settings + "Reset the permission" (`tccutil reset` via `reset_microphone_permission`) — the reset returns the status to `notDetermined`, so the in-app dialog works again |
| accessibility not granted | Grant button (dialog + pane); after an attempt, a reset path for the listed-but-stale entry (common after reinstall/rebuild — see DEVELOPMENT.md) |

## Gating and grandfathering

`App.tsx` checks a localStorage flag (`murmur_onboarding_complete`,
`lib/onboarding.ts`) at mount:

- Flag present → straight to the main UI (the existing `PermissionsBanner`
  still catches later permission drift, and the standalone `ModelDownloader`
  gate still catches a deleted model file).
- Flag absent → probe mic + accessibility + the runtime model catalog. **All three already in
  place → set the flag silently** so existing installs never see the wizard on
  upgrade. Anything missing → show `OnboardingFlow`.

Settings → App → **Run Setup Assistant** clears the flag and relaunches the
wizard — the recovery path when a user revokes a permission and wants a guided
re-grant instead of spelunking through System Settings.

## Files

| File | Purpose |
|------|---------|
| `app/src/components/onboarding/OnboardingFlow.tsx` | The wizard |
| `app/src/lib/onboarding.ts` | Completion-flag persistence |
| `app/src/components/ModelDownloader.tsx` | `ModelDownloadPanel` extracted for reuse |
| `app/src-tauri/src/commands/permissions.rs` | `request_microphone_access` (block2 completion handler) |
