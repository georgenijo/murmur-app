# Development Setup

## Prerequisites

- macOS 14+ on Apple Silicon (Core ML/ANE requires Apple Silicon)
- Node.js 18+
- Rust (install via rustup)
- Python 3 (for capture-helper and packaging scripts)

## Setup

```bash
git clone https://github.com/georgenijo/murmur-app.git
cd murmur-app

# 1. Build the bundled macOS helpers FIRST
python3 scripts/build_capture_helpers.py

# 2. Install Node dependencies
cd app && npm install

# 3. Run
npm run tauri:dev
```

> **Step 1 is not optional on macOS.** `tauri.macos.conf.json` declares the
> capture-helper externalBin entries, so `tauri dev`, `tauri build`, **and even
> `cargo check` / `cargo test`** fail on a fresh clone until the binary exists at
> the three target-triple-suffixed binaries under `app/src-tauri/binaries/`. The
> binaries are gitignored; release CI builds them before bundling. If you only need the Rust
> crate to compile, a stub file at that path is enough.

A transcription model is downloaded through the app's first-launch setup
assistant — you do not need to place model files by hand.

## Commands

```bash
# Dev
cd app && npm run tauri:dev        # full app, isolated bundle/data, hot reload
cd app && npm run dev              # frontend only (no Rust backend)
cd app && npm run tauri build      # production .app and .dmg

# Tests
cd app/src-tauri && cargo test -- --test-threads=1   # Rust (timing-sensitive; keep single-threaded)
cd app/src-tauri && cargo nextest run --workspace --lib  # Rust unit tests, one process per test (exactly what CI runs)
cd app && npm test                                   # frontend vitest
cd app && npx tsc --noEmit                           # TypeScript
```

CI runs clippy, the nextest unit suite, `npx tsc --noEmit`, **and** `npm test`
on PRs and on pushes to main (see [CI and merging](#ci-and-merging)). A
tsc-only check is not sufficient verification — several regressions have
shipped past it.

Model-backed integration tests are opt-in so CI never downloads hundreds of
megabytes:

```bash
cd app/src-tauri && cargo test --test transcription_integration -- --test-threads=1
cd app/src-tauri && MURMUR_COREML_TEST_WAV=/path/to/16khz-mono.wav \
    cargo test --test coreml_transcription_integration -- --ignored
```

## macOS Permissions

Murmur needs **Microphone** and **Accessibility**. Which process needs them
depends on how you are running it:

- **`npm run tauri:dev`** — grant them to your *terminal app* (Ghostty, iTerm,
  Terminal). Dev builds are re-signed on every rebuild, so granting the dev
  binary itself is futile. Restart the dev server after granting.
- **A built `.app`** — grant them to the app itself. To keep a dev bundle's
  grants across rebuilds, sign it with a stable identity (see
  [Stable signing for dev bundles](#stable-signing-for-dev-bundles)).

The setup assistant hides its Microphone, Accessibility, and Calendar Reset
buttons under `tauri:dev`: their `tccutil` commands target the running bundle
identifier, which is not the terminal's permission identity. Recovery guidance
instead points to the launching terminal in System Settings. For a built
`.app`, the Reset actions remain available and target that app only.

## Stable signing for dev bundles

macOS saves each Microphone and Accessibility grant together with the app's
*designated requirement*, a rule its code signature must keep satisfying.
Ad-hoc signatures use the code hash as that rule, so every rebuild becomes a
new app. macOS then prompts again, and System Settings keeps a switch for the
old build that looks on but no longer applies. A certificate-backed signature
uses a rule based on the bundle identifier and the certificate, which stays the
same across rebuilds.

`scripts/build_dev_bundle.py` builds the dev bundle (`Local Dictation Dev.app`,
`com.localdictation.dev`) unsigned. It then signs every binary with the
identity named by `MURMUR_DEV_SIGNING_IDENTITY`: the main app and the capture
agent, helper and worker. Signing goes through the release finalizer, so each
helper keeps its own entitlements and fixed identifier. If the variable is
unset, empty, or `-`, the script signs ad-hoc as before and warns before and
after the build. If it is set but matches no valid identity, matches several,
or the keychain cannot be read, the script stops before building. A quiet
ad-hoc fallback would lose the grants again, for example after a typo or once
the certificate expires. CI and the release workflows never read the variable.
The script refuses the production identifier and release identities
(Developer ID, Apple Distribution).

### Which certificate

| Certificate | Grants survive rebuilds | Dictation in a release-profile dev bundle |
|---|---|---|
| **Apple Development** (recommended) | Yes | Yes |
| Self-signed code-signing certificate | Yes, while you keep the same certificate | No, use `--debug` |
| Ad-hoc (no identity) | No | No, use `--debug` |

Release-profile builds check each bundled helper before they spawn it. The
helper must chain to Apple and carry the main app's Team ID. A self-signed or
ad-hoc signature has no Team ID, so dictation fails with `signature_invalid`.
Debug-profile builds (`--debug`) skip that check. An Apple Development
certificate passes the check and is free with any Apple ID, including a
personal team. Developer ID would also pass, but it is reserved for releases,
so the script refuses it.

### One-time setup

1. Find an existing identity:

   ```bash
   security find-identity -v -p codesigning
   ```

   Look for a line such as
   `"Apple Development: George Nijo (R99GZN6ALR)"`.
2. If there is none, create one: Xcode → Settings → Accounts → add your Apple
   ID → *Manage Certificates…* → **+** → *Apple Development*. Run the command
   from step 1 again to confirm it is listed as valid.
3. Export the variable in your shell profile. It accepts the full name, a
   unique part of it, or the 40-character hash from step 1:

   ```bash
   export MURMUR_DEV_SIGNING_IDENTITY="Apple Development: George Nijo (R99GZN6ALR)"
   ```

4. Build from the repository root, in a Terminal on the Mac's desktop session.
   The first signature may ask to use the key in your keychain; choose *Always
   Allow*. Over SSH the login keychain is often locked, and signing fails.

   ```bash
   python3 scripts/build_dev_bundle.py --require-identity
   ```

   `--require-identity` also fails when the variable is unset. Other options
   are `--debug`, `--skip-helpers`, `--sign-only` (re-sign the last bundle),
   and `--config app/src-tauri/tauri.refactor-test.conf.json` for the
   refactor-test bundle. The script prints the Team ID and designated
   requirement it produced, and the bundle path as `DEV_APP=…`.

   Certificate signing adds a secure timestamp, so the build needs network
   access to Apple's timestamp service (`timestamp.apple.com`). Offline,
   codesign fails; build ad-hoc meanwhile by unsetting the variable.
5. Clear the stale rows left by earlier ad-hoc builds once. Quit System
   Settings first, because it keeps showing old rows until it reopens:

   ```bash
   tccutil reset Microphone com.localdictation.dev
   tccutil reset Accessibility com.localdictation.dev
   ```

   Open System Settings → Privacy & Security → Accessibility. If a *Local
   Dictation Dev* row is still listed, remove it with **−**. Start the new
   bundle with `open "$DEV_APP"` (or from Finder) and grant Microphone and
   Accessibility once. Do not run `Contents/MacOS/…` from Terminal: Terminal
   then becomes the responsible process, and macOS checks Terminal's grants
   instead of the bundle's. Later rebuilds signed with the same identity keep
   both grants.

Grants follow the bundle identifier and the certificate, not the file path.
You can run the bundle from `target/` or copy it anywhere. A grant made under
one identity does not carry over to another, so switching identities, or
falling back to ad-hoc, means granting again.

### Shared helper identifiers

The finalizer gives the dev bundle's helpers the same signing identifiers as
production's, `com.localdictation.capture-agent`, `-helper` and `-worker`. It
also gives them the same app-sandbox entitlement, and the same launchd label
and Mach service, `com.localdictation.capture-agent.xpc`. The identifiers are
hard-coded in the Rust and Swift validation code, so a dev bundle cannot
change them yet. Two consequences on a Mac that also runs
`/Applications/Murmur.app`:

- Dev and production helpers share the sandbox containers
  `~/Library/Containers/com.localdictation.capture-*`. Since macOS 14 each
  container records the code identity that owns it, and macOS asks before
  other code may use it. Apple's guidance is that code signed by the same Team
  ID counts as the same owner, while ad-hoc code can never be recognised and
  always triggers the warning. An Apple Development dev bundle on team
  `P2U3P8B923` should therefore share production's containers without a
  prompt. An ad-hoc dev bundle may prompt, or fail to start a helper. The
  helper and agent sources write no files; they exchange audio with the app
  over pipes and XPC. So a shared container passes no data between dev and
  production.
- Normal launches never register the capture agent with launchd. Only the
  `--capture-agent-service register` probe does. Do not run that probe from a
  dev bundle on a Mac where production has registered its agent, because both
  use one label.

## Project Structure

```text
murmur-app/
├── app/
│   ├── src/                     # React frontend
│   │   ├── components/          # UI (settings, overlay, log-viewer, …)
│   │   └── lib/                 # hooks, settings, pure logic
│   └── src-tauri/
│       ├── src/                 # Rust backend (see docs/ARCHITECTURE.md for the module map)
│       ├── sidecars/capture/    # signed capture helpers
│       ├── crates/              # capture-helper-protocol
│       ├── binaries/            # built sidecar (gitignored)
│       └── tests/               # integration tests
├── bench/                       # benchmark audio fixtures
├── docs/                        # architecture, features, references, decisions
├── prompts/                     # agent prompt files (chat / release)
├── scripts/                     # capture helper build, notarization, release artifacts
├── tests/                       # release-artifact and workflow-policy tests (Python)
└── tools/murmur-diag/           # local MCP log-diagnostics tool
```

## Building for Production

```bash
cd app && npm run tauri build
```

Output:
- `app/src-tauri/target/release/bundle/macos/Murmur.app`
- `app/src-tauri/target/release/bundle/dmg/Murmur_x.x.x_aarch64.dmg`

The `build` shell function installs the result into `/Applications` and resets
the stale Accessibility grant for you (see below).

## Logs

Both build flavors write to `~/Library/Application Support/local-dictation/logs/`:

| File | Build |
|------|-------|
| `app.log` / `events.jsonl` | release |
| `app.dev.log` / `events.dev.jsonl` | dev |

Structured JSONL rotates at 5 MB. In-app: **Settings → Performance**. See
[features/log-viewer.md](features/log-viewer.md) and
[`tools/murmur-diag/README.md`](../tools/murmur-diag/README.md).

## Common Issues

### Permission prompt on every rebuild

An ad-hoc signed dev bundle gets a different signature on every build. Sign it
with a stable identity instead (see
[Stable signing for dev bundles](#stable-signing-for-dev-bundles)). Under
`npm run tauri:dev`, grant Accessibility to your terminal app instead of the dev
binary. If its grant is stale, turn the
terminal's permission off and on under Privacy & Security, then restart the dev
server. Use the same terminal identity for Microphone and optional Calendar
access. The wizard's bundle-targeted Reset actions are hidden in development;
resetting `com.localdictation.dev` would not reset the terminal's permission.

### Accessibility silently broken after a local production build

When you copy a freshly built `.app` over the old one, macOS keeps the old
Accessibility entry — but it is stale, because the new binary has a different
signature. Microphone carries over fine; anything keyboard-related (double-tap,
auto-paste) silently fails.

```bash
tccutil reset Accessibility com.localdictation
```

The `build` shell function does this automatically. Murmur remembers the build that last had Accessibility granted; after a new build the old entry can still read as granted-then-missing, so the first Accessibility request in the new build resets the entry and relaunches once (installed `.app` only, never under `tauri dev`). If the shell function's reset already cleared the entry, that one restart is expected and harmless. **Note:** the
Accessibility-denied banner cannot be reproduced under `tauri dev` — you need a
built `.app` to exercise that path.

### "No input device available"

Grant Microphone to your terminal app, then restart the dev server.

### Model not found

Use the in-app setup assistant. If you are placing files manually, Whisper
`.bin` models are searched in this order:

1. `$WHISPER_MODEL_DIR`
2. `~/Library/Application Support/local-dictation/models/`
3. `~/Library/Application Support/pywhispercpp/models/`
4. `~/.cache/whisper.cpp/`
5. `~/.cache/whisper/`
6. `~/.whisper/models/`

Core ML models are managed by FluidAudio under
`~/Library/Application Support/FluidAudio/Models/`.

## CI and merging

The Rust test job configures its own checkout to emit only the Rust library,
avoiding the additional C-compatible dynamic and static libraries used for
packaging. The committed manifest and release builds retain all output formats.
The test-only adjustment runs before cache restore so the cache key describes
the manifest actually built.
The temporary manifest also records a digest of the workflow and configuration
helper. This refreshes the dependency-cache key when the build recipe changes,
so `main` can save newly built dependency variants instead of treating an old
immutable cache as complete. PRs still restore caches without saving them.

`ci-pass` is the only required status check, and `main` requires branches to be
up to date before merging (`strict`). Strict stays on: it catches PRs that are
green alone but break together, such as one PR adding tests that build a struct
while another adds a required field to it.

- **Push to main skips an already-verified tree.** `scripts/ci_verified_tree.sh`
  finds the PR whose merge commit was pushed. If the commit's tree matches that
  PR head, the head already contained the pre-merge `main`, and the head's
  latest `ci-pass` succeeded, CI skips the tests and
  `ci-pass` passes with that reason in its log. Admin merges over a stale,
  pending, or red PR, conflict resolutions, and direct pushes run the full
  suite. When the push changes Cargo manifests or `ci.yml`, the Rust jobs still
  build, without running tests, to refresh the Rust caches. Only `main` saves
  those caches.
- **Merge trains instead of a rerun cascade.** Every merge makes the other open
  PRs `BEHIND`. Instead of updating and retesting each one, combine the green
  PRs: `scripts/merge_train.sh 918 919 921` merges their heads onto current
  `main` in a `train/…` branch and opens one PR. CI runs once on the
  combination. Merge that PR with `gh pr merge --merge`, not a squash, so GitHub
  marks every constituent PR merged and the push to `main` is skipped as
  verified. If a PR conflicts, the train stops; update that PR and rerun.
  GitHub's merge queue would automate this, but it is unavailable for
  personal-account repositories.

## Release

Releases are tag-based off a trusted `main` version-bump commit, with signed and
notarized artifacts promoted only when the commit SHA, run ID, and artifact
hashes all match. See [release.md](release.md).
