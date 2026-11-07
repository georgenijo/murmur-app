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
- **A built `.app`** — grant them to the app itself.

The setup assistant hides its Microphone, Accessibility, and Calendar Reset
buttons under `tauri:dev`: their `tccutil` commands target the running bundle
identifier, which is not the terminal's permission identity. Recovery guidance
instead points to the launching terminal in System Settings. For a built
`.app`, the Reset actions remain available and target that app only.

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
├── prompts/                     # agent prompt files (work / chat / release / bug / swarm)
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

Dev builds get a different signature each time. Grant Accessibility to your
terminal app instead of the dev binary. If its grant is stale, turn the
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

The `build` shell function does this automatically. **Note:** the
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


## Synthetic WebKit lifecycle checks

`cargo test -p murmur-core` covers schemas and durable migration boundaries on
Linux. On macOS, `cargo test -p murmur-webkit` runs a main-thread test executable
with a custom nonpersistent WKWebsiteDataStore and a temporary Rust data root.
It never obtains the default store or opens an application window. Do not point
fixtures at real WebKit/app-data directories. Native GUI verification remains a
separate check against an isolated bundle and synthetic state.

## Shutdown coordinator native QA

The termination guard combines `scripts/check_restart_policy.py` with Clippy to prevent accidental first-party coordinator bypasses; it is a source-level policy check, not a sandbox, and code review covers deliberately obfuscated termination (see the [covered cases and threat model](draft/gpui-shutdown-coordinator.md#termination-guard-threat-model)).

Use a dedicated checkout and target directory, synthetic fixtures, and the
`tauri.shutdown-test.conf.json` identity. Build with Rust 1.98.1 and
`--features shutdown-test-support`; never use the production bundle/profile.
Both the feature and QA configuration are required: the feature compiles the
private clipboard plugin and its permission manifest, while only the QA config
grants its read/write commands to Main, Diagnostics, Overlay and Dictation Preview.
There is no clipboard capability file in the auto-loaded production directory.
For direct Cargo checks, set `export TAURI_CONFIG="$(cat tauri.shutdown-test.conf.json)"`
from `app/src-tauri` when enabling the feature. Default builds use neither.
This feature adds a six-and-a-half-second final hold after real owner work and
per-phase delay variables documented in the [shutdown contract](draft/gpui-shutdown-coordinator.md).
During that hold require 195 callbacks across AppContext, Tauri and a native
common-mode heartbeat. It accepts mixed requests before terminal commit.

Read terminal evidence from the filesystem, not the telemetry event stream.
Only `shutdown-test-support` compiles this sink; it also requires both the
configured and initialized identity to equal `com.localdictation.shutdown-test`
before resolving a path. On macOS the path is:

```text
~/Library/Application Support/com.localdictation.shutdown-test/shutdown-evidence/<pid>-<process_start_id>.json
```

`process_start_id` is a UUID generated once when that process initializes its
coordinator. At the permitted terminal callback, after the hold and before the
native terminal action, a dedicated worker syncs a staging file, atomically
publishes an immutable JSON name with exclusive rename, and syncs the directory.
On macOS this uses `renamex_np(RENAME_EXCL)`; Linux uses
`renameat2(RENAME_NOREPLACE)`, and other platforms fail closed. Unsupported
filesystems never fall back to a clobbering rename or hard-link/unlink pair.
The rename removes staging at the same instant it publishes JSON, so a terminal
timeout during the final directory sync cannot strand both names. An uncertain
directory-sync failure retains the complete JSON and rejects replacement on
retry. There are three syncs: the evidence directory's parent, the staging file,
and the evidence directory after rename. The terminal caller
waits at most 100 ms in total for this worker, shared across both entry routes.
Repeated callbacks cannot overwrite the receipt. The permitted runtime terminal event also attempts the same write,
covering native termination that observes the permit before the queued terminal
callback runs. Both routes share the same once-per-process publication. It has
no IPC read command and never uses the sealed logger.
The bounded-wait outcome is recorded atomically; late completion cannot change
it. Off-main diagnostics report `pid=<pid> process_start_id=<uuid>`, then
`completed_within_bound=true` or `completed_within_bound=false` with a fixed
code (`timed_out`, `receipt_io_failure`, `evidence_panicked`,
`worker_unavailable`). A relaunch replacement inherits the same stderr, so
attribute a diagnostic only to the receipt named exactly
`<pid>-<process_start_id>.json`. A complete receipt without its matching
`completed_within_bound=true` line is an evidence failure. All diagnostic
writes ignore I/O errors; evidence preparation and worker panics are contained,
including panic payloads with panicking destructors. The shutdown telemetry
diagnostic also ignores write errors, and the pretty-log layer suppresses its
internal stderr fallback when the writer disconnects; queue failure counts remain.
No evidence failure, blocked stderr, or slow fsync prevents the native action.
If the process ends before the worker emits its diagnostic or publishes its
receipt, record an evidence failure. The 100 ms software wait has a 250 ms
elapsed regression threshold for scheduling; it is not an OS real-time guarantee.

Before each launch inventory existing receipt filenames. After observing that
exact process finish, require exactly one **new** receipt for its validated PID
and start UUID, excluding any replacement process's receipt. Require schema 1,
five ordered phases (`admission`, `capture`, `helpers`, `writers`, `terminate`),
65 callbacks from each of `app_context`, `tauri_dispatch`, and `cf_heartbeat`
(195 total/expected), and `terminal_action_count=1`. Check `terminal_action`
(`quit`/`restart`) and `replacement_requested` against that cell's requests and
restart prerequisites. Phase outcomes are `completed`, `preserved_for_replay`,
or `failed`; failure codes are `none`, `owner_failed`, `deadline_expired`, or
`unpreserved_failure`. Deadline cells must assert the affected phase's code and
restart suppression, rather than treating process disappearance as success.
This receipt proves entry to the committed terminal callback; independently
check process death, replacement PID count, and absence of owned helpers.
Missing/duplicate/malformed receipts or `.pending` files are evidence failures.
Keep receipts until the cell's sanitized results have been collected; remove
only that run's inventoried QA files afterward. History Clear and transcript
retention do not delete these content-free QA receipts. Production builds contain
neither the sink nor its receipt types; default/feature ACL tests and the sender
policy tests cover feature, identity, and capability boundaries.

Every grid cell must run the cheap Clear/retention/source/theme/window replay
procedure in [Round-eight QA terminal evidence](draft/gpui-shutdown-coordinator.md#round-eight-qa-terminal-evidence).
Record each assertion per cell; supplementary runs cannot fill missing cells.
Every cell, including dictation and each pending-write variant, must complete a
successful nonempty synthetic file-transcription preflight with retention off.
Await the `transcribe_file` acknowledgement and derive success only from its
fulfilled result; require `text.trim().length > 0` locally. Record only success,
positive output character/byte count, and `fileRunId` or a locally computed
digest; discard text from the harness result and never export it. Empty/error/
Idle-only results fail. Require zero retained additions before and after replay.

Each pending-write cell has two distinct variants. The successful in-budget
variant keeps a real owner-produced pending settings receipt, removes the
Writers delay (or keeps it within budget), and requires exact revision/digest
commit, coordinator `timed_out=false` / `writes=Committed`, and terminal JSON
Writers `completed` / `none`. The deadline variant keeps a separate pending
receipt and sets `MURMUR_SHUTDOWN_DELAY_WRITERS_MS=20000`; require
`timed_out=true` / `writes=UnpreservedFailure`, Writers `failed` /
`deadline_expired`, restart suppression, and exact pending recovery on replay.
The terminal JSON uses closed outcome/failure codes rather than raw coordinator
fields. A deadline failure cannot stand in for the successful variant.

From the isolated Main context, invoke
`plugin:shutdown-test|shutdown_test_action` with action quit, relaunch, exit, or
destroy-last. The feature generates its inline-plugin permission manifest; only
the isolated configuration grants that command, and only to Main. The plugin rejects other bundle identities and windows. Quit-path
fixtures must verify the executable revision, synthetic profile and exact PID.
For simulated system reasons use:

```sh
python3 scripts/send_shutdown_test_quit.py --bundle /path/to/isolated/Murmur-Shutdown-Test.app --pid 12345 --reason logout
```

Reasons are quit/logout/restart/shutdown. The sender checks CFBundleIdentifier
and proc_pidpath twice, then sends a PID-targeted aevt/quit with a bounded reason,
no reply, no user interaction, and a ten-second sender timeout. It never sends
to loginwindow or performs real system logout/restart/shutdown. Never reuse a
PID without revalidation. The full three-fixture matrix is in the shutdown doc.
The implementation worker runs windowless tests only; GUI interaction belongs
to the separately assigned native worker.


### Shutdown QA identity isolation audit

Only `com.localdictation` selects the production legacy paths. `identity.rs`
encodes every other bundle ID as UTF-8 hex under `murmur-isolated/<hex-id>`;
there is no list of recognized test IDs that can fall through to production.
Initialization precedes CLI worker dispatch as well as Tauri setup.

| Persistent/global surface | Ownership / isolation for every non-production identity |
|---|---|
| Application Support | Tauri app data uses the configured bundle identifier: settings, completion, UI, window state, knowledge, meetings and performance data all descend from it. Legacy global stores instead descend from `murmur-isolated/<hex-id>`. |
| Default transcript/audio/report exports | Empty output settings resolve below the isolated legacy app-data namespace (`murmur-isolated/<hex-id>/exports`), never Documents/Murmur. Resolution fails if identity or the isolated data root is unavailable; sequence scans occur only after resolution. Production retains its Documents/home default. |
| Telemetry and shipper state | `identity::namespace` selects isolated `local-dictation/logs`; buffer seeding and rotating writers use that path. All non-production identities are internal bundles and never start the shipper. |
| Whisper, VAD, Parakeet, Smart Turn models | Legacy model paths descend from isolated Application Support; Whisper disables production/global search and environment overrides for internal identities. Parakeet explicitly uses CPU, with no provider disk cache. |
| FluidAudio/shared model caches | Inspection, download destinations and repair markers are namespaced. The pinned FluidAudio SDK has no cache-path API: its ASR initializer is refused for internal bundles, and speaker diarization is unavailable there, preventing implicit SDK access to the shared production cache. Use isolated Whisper/Parakeet for QA. |
| Corpus stores | Namespaced legacy roots; internal identities ignore global corpus directory overrides. Synthetic fixtures must be provisioned inside the isolated root. |
| WebKit | Wry uses the configured bundle identity; lifecycle checks only the exact identity subdirectory. QA uses `com.localdictation.shutdown-test`, never the production store or a directory enumeration of the global WebKit root. |
| Shared clipboard and pasteboard | Only an explicitly initialized `com.localdictation` process uses the general pasteboard. Every other identity, including absent/empty/future IDs, routes text reads, writes and change-count checks to its own injectively named `NSPasteboard`. Clipboard snippets and transform apply/undo use the same primitives. The `shutdown-test-support` feature with its QA config installs a text clipboard bridge before application scripts in every WebKit frame, so History/diagnostic/meeting Copy actions also use the private board; missing IPC and unsupported item types fail closed. The plugin is absent in production and its commands independently reject production/unproven identities. QA refuses all external key posts and Smart insertion before activation or mutation; no QA-only target exemption exists. Routing tests use strings only and never access any OS pasteboard. |
| OS-managed crash reports | macOS may create identity/process-attributed crash reports outside the application-owned roots. Murmur has no application-owned reader for those reports and does not enumerate, import, upload or delete them. These OS-managed files are not claimed to be application-namespaced stores. |
| UserDefaults/preferences | App defaults use NSBundle identity. Explicit appearance/Dock reads access system appearance/layout preferences only; no app data is read or written in another app domain. |
| Keychain | No application keychain store/API is used. Updater signature verification uses the bundled public key. |
| Temp directories | Generated cue files use a namespaced temp root and UUID; explicit test fixtures use unique synthetic roots. Owned publication temps are exclusive 0600 siblings of their isolated destinations. |
| Helper pipes | Ordinary capture and inference helpers use inherited anonymous pipes/stdio and generation-bound IDs. Their fixed executable bundle IDs do not select persistent storage. |
| Capture-agent Mach IPC | The experimental agent uses the fixed production endpoint `com.localdictation.capture-agent.xpc`. Every capture-agent CLI action (status, recovery, probes including synthetic probes, and service register/refresh/status/unregister) is refused before dispatch unless the initialized identity is exactly production. Unknown/uninitialized identities fail closed. The shutdown QA matrix uses ordinary capture helpers and does not require this agent. |
| Capture spools | Meeting WAV/SQLite and optional capture fixtures descend from Tauri app data. No persistent dictation audio is introduced. |
| Pending files | Completion, UI/source, window and memo pending documents are siblings of isolated destinations. Startup removes abandoned completion/destination temps. Transcript pending evidence is pinned until explicit recovery, then bounded; Clear/Delete/caps remove superseded pending evidence. |
| Update caches | Updater plugin is not registered for any non-production identity; no update cache/installation is started by the QA bundle. |
| Launch agents | Autostart plugin is not registered for any non-production identity. Capture-agent service registration and all other CLI operations are refused for non-production or unproven identities. |
| User-selected imports/exports | Explicit paths are user-controlled; QA supplies synthetic fixtures only. Implicit exports use the isolated default above. |

Tests cover bench, refactor-test, shutdown-test, future/empty/path-like IDs,
ensuring namespace separation and injectivity, plus the actual telemetry resolver.
No production data is accessed by the tests. Regression coverage includes independent
production/QA default-export numbering, all refused capture-agent operations,
and queued early logs during an injected stalled telemetry history reader.

#### Startup disk-I/O audit (round three)

Setup registers the shutdown coordinator before starting optional storage work.
Telemetry installs its subscriber using memory/channel setup. Its worker reads
history, opens/rotates files and writes both log formats. Restoration captures
Clear's generation **before scheduling**; Clear invalidates the live ring before
waiting for disk. Early records remain queued behind a stalled startup reader.
The shutdown Writers phase starts an independent telemetry waiter. It accepts
final owner events until content writers finish or the absolute +10 s deadline,
then seals producers and queues a file-flush barrier using only the remaining
budget. A blocked content writer cannot postpone that drain.
The +5/+7/+10 s coordinator driver never joins a disk worker. A content-free
worker stderr receipt reports pending, failed and post-seal rejected records,
including records that cannot reach disk by the deadline.

| Setup call | Worker ownership and main-thread behavior |
|---|---|
| `completion::initialize` | A retained startup worker creates/locks the identity root, checks only the exact identity WebKit path and starts the completion owner. Readiness waits and initial Snapshot/migration IPC run off main; commands arriving before publication fail safely or await readiness asynchronously. Recording remains gated until migration admission. |
| Legacy query/transform cleanup | A separate startup worker enumerates/removes retired identity app-data files. |
| Capture-health | A separate startup worker loads history/replays prior-launch JSONL and publishes migration. Disk reads/writes never hold the live accumulator lock used by tracing/main. Live observations are appended after restored observations; current-instance JSONL events are not counted twice. Clear invalidates scheduled/in-progress restoration. |
| Performance / dictation diagnostics | A startup worker creates machine identity, opens/migrates SQLite, creates private directories and prunes captures. Machine-ID I/O does not hold the live production accumulator; pruning completes before publishing its root. Initial performance read/recovery and Clear IPC run off main. |
| Knowledge / correction refresh | A separate startup worker opens/migrates/queries SQLite and loads the correction matcher. Publication locks cover only in-memory state; early status/read calls see unavailable state rather than waiting for initialization I/O. |
| Meeting / abandoned spools | A separate startup worker sweeps abandoned identity spools, opens/migrates/queries SQLite and recovers pending segments. Repository publication does not hold the reader lock across initialization I/O. |

Each startup worker owns an accepted producer permit for shutdown recovery.
A stalled reader cannot keep setup on main, and the deadline driver can finish
without joining that worker. Window snapshots, capture memo and launch settings
recovery also run on workers. Cue WAV creation occurs in its playback worker.
Audio inventory, Smart Auto, meeting suggestions, log shipping and heartbeats
schedule workers/tasks without application-file reads during registration.
Native menu/window mutations, AppKit observers, WebKit identity validation and
system-device/display calls remain on main because their APIs require it. They
perform no application-owned file replay or migrations and acquire no startup
storage lock; OS framework dispatch itself has no application-controlled hard
time bound. Synthetic stalled-reader tests cover setup returning, live logging,
Clear precedence, and quit reaching the existing terminal deadline while I/O
is still blocked. This is windowless verification, not a GUI acceptance claim.

### Completion pending recovery

A damaged `completion-pending.json` is quarantined and blocks admission. In the
Saved data needs recovery screen, choose **Preserve rejected save and recover**.
This main-window-only typed command requires readable current completion stores,
keeps the rejected bytes, and durably acknowledges using those readable stores
instead of replaying the damaged intent. Restart afterward. Recovery never
manufactures defaults or imports the damaged content. If a destination is also
unreadable, preserve the directory for manual recovery first. Clear History,
Delete and subsequent content caps discard superseded pending evidence; unresolved
evidence remains pinned until explicit recovery. No evidence is exported or logged.
