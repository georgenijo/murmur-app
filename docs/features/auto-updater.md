# Auto-update system

Murmur's Rust updater checks, downloads, verifies, installs and requests a
coordinated relaunch. `murmur-core::updater` is Tauri-free; React renders its
revisioned snapshots and forwards user intent. See the
[host-neutral design](../draft/gpui-host-neutral-updater.md) for the decision,
transaction details, isolated fixture and native QA procedure.

## Feed and trust

The production endpoint remains:

```
https://github.com/georgenijo/murmur-app/releases/latest/download/latest-v2.json
```

The embedded public key is identical to the existing tauri.conf.json key.
The updater decodes the same base64-wrapped minisign key and signatures as
installed clients. `minisign-verify` 0.3 verifies legacy and prehashed Ed25519
signatures; an unsigned, tampered or wrong-key archive cannot become Ready.
No runtime configuration can override production trust. The old updater
plugin is removed; tauri.conf.json retains the release-facing public-key
configuration for tooling and compatibility policy checks.

The static platform map selects a bundle-specific key first, then the platform
key (darwin-aarch64-app, then darwin-aarch64 on Apple Silicon). It never falls
back to another architecture. The release automation still creates immutable
latest-v2.json and legacy latest.json manifests. The legacy channel retains
the signed macOS 13-compatible v0.14.1 bridge; current builds require macOS 14.
The existing release workflow, signing process and bridge asset remain intact.

## Scheduling and policy

After onboarding/model setup, Rust checks on launch when due and every six
hours while resident. A native 15-minute timer and focus, visibility and wake
intents evaluate the same durable due gate. Manual checks remain available
during setup. The setup/automatic gate also suppresses background notifications.
Concurrent background requests collapse; checks cannot replace a ready update.
Discovery retries once after a transient failure.

Minimum policy is read from the exact native response. Absent or structurally
malformed metadata offers an optional signed update. A present string policy
forces updates when the current numeric version triple is below min_version;
unparseable string versions fail safe to forced. Prerelease/build suffixes do
not affect this minimum comparison. `.github/updater-policy.json` still controls
publication: null omits min_version; a stable version must not exceed the release.

## User flow

1. **Available:** Passive header and Settings controls show the update. Manual
   checks open release notes; background optional discovery sends a native
   notification. Later dismisses the dialog and leaves update controls visible.
2. **Preparing:** Rust verifies the installed executable's location. An exact
   AppTranslocation path component blocks download and directs the user to quit
   and move/reinstall the app using Finder. A retained offer older than five
   minutes gets a freshness check; discovery failure preserves the prior offer.
3. **Downloading:** Rust streams the signed archive with bounded size/time,
   publishes progress (indeterminate for unknown length), and verifies it.
4. **Ready:** Verified bytes remain in memory until Restart Now. Later keeps
   the app usable and both restart controls available. Quitting before installation
   means downloading again on a later launch.
5. **Restarting:** Rust verifies again, validates bundle identity/version and
   executable, stages beside the installed app, and atomically exchanges bundles.
   The installed launch path remains present. Relaunch goes through the shutdown
   coordinator's compatible path; Quit/System precedence and drain deadlines
   still apply. Repeated clicks cannot install or request relaunch twice.
6. **What's New:** The new binary confirms the exact pending version (including
   prerelease/build identity), focuses Main once and shows the saved notes.
   Dismissal writes explicit null; quitting while the modal is open retains it.

Forced Available/Ready states offer Download or Restart and Quit, with no Later,
backdrop dismissal or close button. Quit uses the coordinated process adapter.
Check errors offer the existing public latest-release recovery link. Download,
install and relaunch errors keep their distinct stages and recovery actions.

A pre-exchange interrupted transaction leaves the original app installed;
a confirmed updated process removes its retained old bundle and receipt.
Corrupt/future recovery evidence fails visibly and is preserved. Non-writable
locations or unsupported atomic-exchange filesystems leave the current app
intact and require correcting the location or reinstalling. No elevation occurs.

## Durable state and release notes

Migration v3 moves updater-last-check and pending-update-release-notes to the
completion actor's Rust-owned UI-state document. Disk values win; absent keys
fill once. A per-destination marker precedes the global v3 commit, so interrupted
imports and lost migration flags do not resurrect dismissed notes. Legacy keys
are removed only after durable v3, with interrupted removal retried next launch.
React never writes updater keys back to localStorage.

release_history selects stable releases strictly above the installed version
and no newer than the offered one, newest first. The target's notes remain
authoritative. Saved notes are bounded to one million characters and rendered
with the existing react-markdown/rehype-sanitize view. Missing history preserves
the latest release's notes. Promotion still verifies release body/manifest
parity; published assets remain immutable.

## OTA canary

MURMUR_UPDATER_CANARY remains the opt-in result path. An absent marker is inert;
canary launches continue disabling the log shipper. Rust drives the same
schemaVersion:1 result, checkedVersion/offeredVersion/forced/dryRun fields and
six stages: discover, policy, download, signatureVerify, install and relaunch.
Only the relaunched offered binary marks relaunch passed. Dry-run exercises
discovery/policy and leaves production stages pending with status dry-run.
The updater_canary read/write IPC remains available for existing tooling.

The release workflow still runs the disposable GitHub-hosted OTA canary.
`scripts/murmur_canary_fleet.py --tag vX.Y.Z` retains its opt-in dedicated Mac
installation and isolated profile. A first canary-capable public build still
needs the documented bootstrap exception: its predecessor cannot run the marker.
Never substitute the daily /Applications installation for a canary profile.

Before Phase 2, #990 requires at least two real signed OTA canaries. Publishing
those releases needs George's authorization; a local throwaway-key fixture or
headless unit pass does not satisfy that gate.
