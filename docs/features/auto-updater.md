# Auto-Update System

## Overview

The app checks for updates on launch, every six hours while resident, and when
the native app resumes or the main window becomes active after a due interval.
Updates are downloaded from GitHub Releases, verified with ed25519 signatures,
and installed with an automatic relaunch. A `min_version` field in the release
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

- `latest-v2.json`: the current dual-platform channel for new builds, with macOS 14+ and Linux artifacts
- `latest.json`: the legacy channel, with the current Linux artifact and the signed v0.14.1 macOS bridge artifact

An old Mac installs the bridge from `latest.json`, relaunches on the new endpoint, and then receives the current Core ML build from `latest-v2.json`. The modern release job fails if it cannot resolve the signed v0.14.1 bridge asset, preventing a release that would strand older clients. Keep the bridge release published while pre-v0.14.1 clients may still exist.

The manifest contains version information, download URLs, signatures, and an optional `min_version` field for forced updates.
The modern channel's policy is version-controlled in
`.github/updater-policy.json`. A `null` value omits `min_version`; a stable
`major.minor.patch` value publishes it after verifying that it is not newer
than the release. The legacy bridge manifest intentionally omits this policy.

## Update Flow

### Normal Updates

When a new version is available and the current version is above `min_version`:

1. **Available** — Background discovery is passive: the main Record/File row
   shows an update pill and the menu-bar action changes to the available
   version. Manual checks open the modal immediately; clicking either passive
   indicator opens it later. The modal shows version number and release notes
   with three buttons:
   - "Update Now" — begins download
   - "Skip This Version" — stores the version in localStorage (`skipped-update-version`), suppresses future background checks for that version
   - "Later" — dismisses the modal without skipping; the update pill remains
2. **Preparing** — A single updater owner verifies that the app is installed
   in a writable location. Repeated actions and manual, timer, focus,
   visibility, or wake checks cannot enter while this owner is active.
3. **Downloading** — Progress bar with percentage. Progress reported via Tauri's `downloadAndInstall` callback.
4. **Ready** — "Installing and relaunching..." text displayed.
5. **Relaunch** — App restarts automatically via `@tauri-apps/plugin-process`.
6. **What's New** — After the relaunched binary confirms that it is the
   downloaded version, a one-time modal shows that release's features, fixes,
   and other changes.

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
3. Only two buttons are available: "Update Now" and "Quit" (calls `exit(0)`)
4. No "Skip" or "Later" options
5. Backdrop click is disabled — the modal cannot be dismissed
6. The close button (X) is hidden

### Error State

If the download or install fails, the modal shows a red error banner with the error message and a "Retry" button. For forced updates in error state, the "Quit" button remains available.

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
`min_version` from unavailable or malformed native metadata. Once update
availability is known, Murmur refuses to present it as optional unless the
policy was verified. A policy failure is retryable as an update-check error and
does not advance the successful-check timestamp.

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
  | { phase: 'downloading'; version: string; progress: number }
  | { phase: 'ready'; version: string }
  | {
      phase: 'error';
      stage: 'check' | 'install';
      message: string;
      isForced: boolean;
      recovery?: 'reinstall';
    };
```

The update modal renders for `available`, `preparing`, `downloading`, `ready`,
and `error` phases. The `idle`, `checking`, and `up-to-date` phases return null
(no modal).

Post-update notes use separate `CompletedUpdate` state rather than adding a
phase to `UpdateStatus`; this prevents the Settings update checker from treating
an already-installed release as an available update.

## Settings Integration

- The "Check for Updates" button in the About section of settings triggers a manual check. It is disabled during `checking`, `preparing`, `downloading`, and `ready` phases.
- Status text shows: "Checking...", "You're up to date", "vX.Y.Z available", or "Update check failed".
- The macOS menu-bar menu exposes the same manual check. It brings the main
  window forward, reports checking/up-to-date/error status beside the Record
  tabs, and opens the existing update dialog when a release is available.
- Optional background updates do not interrupt the user with a modal. A
  persistent `Update available · vX.Y.Z` pill and versioned menu item remain
  until the release is installed or explicitly skipped.
- Skipped version is stored in localStorage under `skipped-update-version`.
- Pending post-update notes are stored under `pending-update-release-notes` and
  removed when dismissed or when the running version does not match.

## Dependencies

- `tauri-plugin-updater` — Tauri 2 updater plugin (check, download, install)
- `tauri-plugin-notification` — Native macOS notifications for background updates
- `tauri-plugin-process` — App restart after install, `exit(0)` for forced update quit
- `react-markdown` + `rehype-sanitize` — Release notes rendering
