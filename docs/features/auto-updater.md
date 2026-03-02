# Auto-Update System

## Overview

The app checks for updates on launch and every 24 hours. Updates are downloaded from GitHub Releases, verified with ed25519 signatures, and installed with an automatic relaunch. A `min_version` field in the release manifest enables forced updates that cannot be skipped or dismissed.

## Update Check Schedule

- **On launch:** A background check runs immediately on mount via the `useAutoUpdater` hook.
- **Periodic:** Every 24 hours (`CHECK_INTERVAL_MS = 86400000`), gated by `isDueForCheck()` which reads the last check timestamp from localStorage (`updater-last-check`).

## Update Source

The update manifest is fetched from:

```
https://github.com/georgenijo/murmur-app/releases/latest/download/latest.json
```

This URL is configured in `tauri.conf.json` as the updater plugin endpoint. The fetch uses `cache: 'no-store'` to bypass browser caching.

The manifest contains version information, download URLs, signatures, and an optional `min_version` field for forced updates.

## Update Flow

### Normal Updates

When a new version is available and the current version is above `min_version`:

1. **Available** — Modal shows version number and release notes. Three buttons:
   - "Update Now" — begins download
   - "Skip This Version" — stores the version in localStorage (`skipped-update-version`), suppresses future background checks for that version
   - "Later" — dismisses the modal without skipping
2. **Downloading** — Progress bar with percentage. Progress reported via Tauri's `downloadAndInstall` callback.
3. **Ready** — "Installing and relaunching..." text displayed.
4. **Relaunch** — App restarts automatically via `@tauri-apps/plugin-process`.

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
  | { phase: 'downloading'; version: string; progress: number }
  | { phase: 'ready'; version: string }
  | { phase: 'error'; message: string; isForced: boolean };
```

The update modal renders for `available`, `downloading`, `ready`, and `error` phases. The `idle`, `checking`, and `up-to-date` phases return null (no modal).

## Settings Integration

- The "Check for Updates" button in the About section of settings triggers a manual check. It is disabled during `checking` or `downloading` phases.
- Status text shows: "Checking...", "You're up to date", "vX.Y.Z available", or "Update check failed".
- Skipped version is stored in localStorage under `skipped-update-version`.

## Dependencies

- `tauri-plugin-updater` — Tauri 2 updater plugin (check, download, install)
- `tauri-plugin-notification` — Native macOS notifications for background updates
- `tauri-plugin-process` — App restart after install, `exit(0)` for forced update quit
- `react-markdown` + `rehype-sanitize` — Release notes rendering
