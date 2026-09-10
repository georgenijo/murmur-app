# Auto-Update System

## Overview

The app checks for updates on launch, every six hours while resident, and when
the native app resumes or the main window becomes active after a due interval.
Updates are downloaded from GitHub Releases, verified with ed25519 signatures,
and installed when the user chooses to restart. A `min_version` field in the release
manifest enables forced updates that cannot be skipped or dismissed.

## Update Check Schedule

- **On launch:** A background check runs immediately on mount via the `useAutoUpdater` hook.
- **Periodic:** Every six hours (`CHECK_INTERVAL_MS = 21600000`), gated by
  `isDueForCheck()` which reads the last check timestamp from localStorage
  (`updater-last-check`). A low-cost 15-minute timer only evaluates that local
  due check; it does not make a network request every tick.
- **Lifecycle:** Native resume, webview visibility, and main-window focus request
  the same due-gated background check. Several lifecycle signals therefore
  still collapse into at most one network request per six-hour interval.

## Update Source

The update manifest is fetched from:

```
https://github.com/georgenijo/murmur-app/releases/latest/download/latest-v2.json
```

This URL is configured in `tauri.conf.json` as the updater plugin endpoint.
Tauri fetches it natively and exposes the decoded response as `Update.rawJson`;
the frontend reads `min_version` from that same response instead of issuing a
second cross-origin request from the webview.

### macOS 14 Channel Migration

The Core ML release raises Murmur's minimum macOS version from 13 to 14. Existing v0.14.0 macOS clients request the legacy `latest.json`, so publishing a macOS 14 binary there would let them install an app that cannot launch. Version 0.14.1 is a macOS 13-compatible bridge whose release-facing change is moving its updater endpoint to `latest-v2.json`. Release automation therefore publishes two manifests:

- `latest-v2.json`: the current channel for new builds, with the signed macOS 14+ artifact
- `latest.json`: the legacy channel, with only the signed v0.14.1 macOS bridge artifact

An old Mac installs the bridge from `latest.json`, relaunches on the new endpoint, and then receives the current Core ML build from `latest-v2.json`. The modern release job fails if it cannot resolve the signed v0.14.1 bridge asset, preventing a release that would strand older clients. Keep the bridge release published while pre-v0.14.1 clients may still exist.

The manifest contains version information, download URLs, signatures, and an optional `min_version` field for forced updates.
The modern channel's policy is version-controlled in
`.github/updater-policy.json`. A `null` value omits `min_version`; a stable
`major.minor.patch` value publishes it after verifying that it is not newer
than the release. The legacy bridge manifest intentionally omits this policy.

## Post-release OTA canary

The trusted Mac mini has an opt-in canary installation in a dedicated
`Murmur OTA Canary` directory. The release operator runs:

```bash
python3 scripts/murmur_canary_fleet.py --tag vX.Y.Z
```

The runner finds the previous public release, installs its signed
`Murmur.app.tar.gz` into that directory (never the daily `/Applications`
install), launches it with `MURMUR_UPDATER_CANARY=/path/to/result.json`, and
waits for the relaunched client. The app-side marker is absent for ordinary
launches, so the canary path is inert; canary runs also disable the log shipper.
The app still uses the normal updater hook, including native `Update.rawJson`
policy parsing, download, signature verification, install, and relaunch.

The result file is JSON with this schema:

```json
{
  "schemaVersion": 1,
  "status": "passed",
  "checkedVersion": "0.31.3",
  "offeredVersion": "0.31.4",
  "forced": false,
  "dryRun": false,
  "stages": {
    "discover": "passed",
    "policy": "passed",
    "download": "passed",
    "signatureVerify": "passed",
    "install": "passed",
    "relaunch": "passed"
  },
  "error": null
}
```

`status` must be `passed`, every stage must be `passed`, and `checkedVersion`
must equal the exact resolved previous release while `offeredVersion` equals
the target release. The runner validates all field types and this contract and
exits nonzero with the captured error for any failure. `--dry-run` launches the
previous canary-capable bundle with a second marker, exercises discovery and
policy, writes an identifiable `status: "dry-run"` result with production
stages still `pending`, and exits before download/install/relaunch.

The canary proves manifest consumability, native policy parsing, signature
verification, installation, and relaunch into the new version. It does not
prove per-user network conditions, Gatekeeper App Translocation behavior, or
the many possible daily-install locations.

### One-time Mac mini setup and bootstrap exception

The first public build containing this canary support cannot be tested by the
previous public build: that older client has no canary marker or result writer.
This is a one-time bootstrap exception. Physically install the first
canary-capable public build into the dedicated `Murmur OTA Canary` location on
the trusted mini and launch it once there to grant permissions. Do not run the
normal release gate against this bootstrap build: its predecessor is not
canary-capable. Use `--dry-run` beginning with the next release, when this
installed build is the previous public client. Never use the daily
`/Applications` install for this bootstrap.

On the trusted mini, install Fleet and GitHub CLI access, then run the command
above from a checkout at `/Users/george-mac-mini/Documents/code/murmur-app`.
Grant Murmur the required macOS permissions to the dedicated canary bundle,
and keep the mini on AC power with the screen session available for any first
launch prompts. Confirm `--dry-run` first. The first real run creates
`~/Library/Application Support/Murmur OTA Canary/Murmur Canary.app`; do not
move it into `/Applications` or point the runner at the daily installation.

## Update Flow

### Normal Updates

When a new version is available and the current version is at or above a
verified `min_version` (or no verifiable minimum policy is available):

1. **Available** — Background discovery shows a download icon with an availability
   dot in the header. Settings changes **Check for Updates** to **Download Update**.
   Either control starts the download directly. Manual checks open the release
   notes dialog with **Download Update** and **Later**. Later closes the dialog
   and keeps both update controls available. Releases are no longer skippable.
2. **Preparing** — A single updater owner verifies the install location. Duplicate
   actions and background checks cannot enter while an update is downloading,
   ready, or restarting.
3. **Downloading** — A compact progress dialog shows a percentage and progress
   bar. The header shows a progress ring. Unknown download sizes use indeterminate
   progress. Tauri's `Update.download()` verifies and retains the downloaded bytes.
4. **Ready** — The dialog offers **Restart Now** and **Later**. Later keeps Murmur
   running and leaves the header and Settings restart controls available. The
   download stays in memory for this app session. Quitting before installing
   requires downloading again on a later launch.
5. **Restarting** — **Restart Now** installs the retained download with
   `Update.install()` and relaunches Murmur. Repeated clicks cannot duplicate
   either operation. Installation failures retry installation using the retained
   bytes; relaunch failures retry only the relaunch.
6. **What's New** — After the relaunched binary confirms the downloaded version,
   a one-time modal shows that release's features, fixes, and other changes.

The opt-in OTA canary explicitly runs both download and restart steps to retain
its unattended six-stage verification. Ordinary updates always wait for the
user to choose **Restart Now**.

Before download begins, the app stores a bounded `{ version, notes }` payload in
localStorage under `pending-update-release-notes`. The payload is intentionally
kept until the user dismisses the post-update modal, so an app quit while the
modal is open does not lose it. A fresh install has no pending payload and never
shows the modal. A malformed payload or one whose version does not exactly match
the running app is removed without being displayed.

### Forced Updates

When the current version is below the `min_version` field from the release manifest:

1. The update modal shows "Required Update" instead of "Update Available"
2. An amber warning reads "This update is required to continue using the app"
3. The available dialog offers **Download Update** and **Quit**. After download,
   it offers **Restart Now** and **Quit**. Quit calls `exit(0)`.
4. No **Later** option at either step
5. Backdrop click is disabled — the modal cannot be dismissed
6. The close button (X) is hidden

### Error State

If the update check fails, the modal shows a red error banner with the error
message, a "Retry" button that runs the check again, and an inline
"Download latest version" link that opens the latest GitHub release page.
This recovery link appears only for `stage: 'check'` errors; it is absent from
all normal states and from download/install errors. For forced updates in error
state, the "Quit" button remains available.

Before downloading, Murmur asks the Rust host whether the current executable is
running under macOS Gatekeeper's `AppTranslocation` path. A translocated app is
mounted read-only, so the updater cannot replace its bundle. Murmur blocks the
download, explains that the user must quit and use Finder to move or reinstall
Murmur in `/Applications`, and offers a Quit action instead of a futile Retry.
Normal writable installations continue through the existing updater path.

Check failures and installation failures remain distinct in `UpdateStatus`, so
the Settings page and homepage indicator do not describe a completed download
or failed installation as an update-check failure.

The updater manifest policy parser also distinguishes an intentionally absent
`min_version` from unavailable or structurally malformed native metadata. Once
update availability is known, an absent policy leaves the signed update
optional. Unavailable or structurally malformed metadata is logged as a warning
and the signed update is still offered as optional (`isForced: false`). Only a
present string policy can enter forced-update enforcement. If that policy's
`min_version`, or the installed version, is unparseable, the existing
`isBelowMinVersion` fail-safe forces the update. A secondary policy read may
fail to force an update; it must never fail to offer one.

### Background Notifications

When an update is detected during a background check (not user-initiated), a native macOS notification is sent: "Murmur vX.Y.Z is ready to install." This requires notification permission to be granted.

## Semver Comparison

The updater includes a semver parser (`updater.ts`) that:

- Strips `v` prefix and whitespace
- Parses major.minor.patch components
- Strips pre-release and build metadata for comparison
- Returns `-1 / 0 / 1 / null` (null = unparseable)

**Fail-safe:** If either version is unparseable, `isBelowMinVersion` returns `true`, forcing the update. This ensures that broken version strings do not allow users to skip required updates.

## Release Notes

Release notes from the manifest are rendered as Markdown using `react-markdown` with `rehype-sanitize` (default sanitization schema). Custom HTML in release notes is stripped by the sanitizer.

The promotion workflow creates the draft GitHub release with generated notes,
then reads that exact Markdown body into `latest-v2.json`. Manifest generation
fails if the body is missing or empty. `.github/release.yml` groups merged pull
requests into **New Features** (`enhancement`), **Bug Fixes** (`bug`), and
**Other Changes** categories.

Immediately before publication, promotion compares the draft release body with
the remotely downloaded updater manifest after symmetrically normalizing line
endings and outer whitespace, and fails if any remaining content differs.
Published updater assets are immutable: do not edit a published release body
independently. A correction that must reach both the public release and the
in-app dialogs requires a patch release.

The same sanitized notes appear in two places:

- the update-available dialog, before download
- the one-time What's New dialog, after the updated app relaunches

For a useful customer-facing summary, release pull requests should carry either
the `enhancement` or `bug` label when applicable. Unlabeled pull requests remain
visible under Other Changes.

## Signed Updates

Updates are signed with ed25519. The public key is embedded in `tauri.conf.json` as a base64-encoded minisign public key. The Tauri updater plugin handles signature verification automatically — unsigned or incorrectly signed updates are rejected.

The build system generates updater artifacts (`createUpdaterArtifacts: true` in bundle config).

## Update Status Lifecycle

```typescript
type UpdateStatus =
  | { phase: 'idle' }
  | { phase: 'checking' }
  | { phase: 'up-to-date' }
  | { phase: 'available'; version: string; notes: string; isForced: boolean }
  | { phase: 'preparing'; version: string }
  | { phase: 'downloading'; version: string; progress: number | null }
  | { phase: 'ready'; version: string; isForced: boolean }
  | { phase: 'restarting'; version: string }
  | {
      phase: 'error';
      stage: 'check' | 'install' | 'restart';
      message: string;
      isForced: boolean;
      recovery?: 'reinstall';
    };
```

The update modal renders for `available`, `preparing`, `downloading`, `ready`,
`restarting`, and `error` phases. The `idle`, `checking`, and `up-to-date` phases return null
(no modal).

Post-update notes use separate `CompletedUpdate` state rather than adding a
phase to `UpdateStatus`; this prevents the Settings update checker from treating
an already-installed release as an available update.

## Settings Integration

- General Settings and the header use the same `UpdateIndicator` component.
  Settings shows text labels; the header uses an icon and accessible action name.
- Settings changes from **Check for Updates** to **Download Update** and then
  **Restart Now**. Busy states show progress and disable repeated actions.
- The native menu brings the main window forward. It checks when there is no
  pending update and reopens the current update dialog otherwise.
- Optional background updates remain passive. **Later** never hides the update
  controls. Legacy `skipped-update-version` values are ignored.
- Pending post-update notes are stored under `pending-update-release-notes` and
  removed when dismissed or when the running version does not match.

## Dependencies

- `tauri-plugin-updater` — Tauri 2 updater plugin (check, download, install)
- `tauri-plugin-notification` — Native macOS notifications for background updates
- `tauri-plugin-process` — App restart after install, `exit(0)` for forced update quit
- `react-markdown` + `rehype-sanitize` — Release notes rendering
