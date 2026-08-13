# Decisions Log

Running log of architectural, scope, and process decisions for this project. Newest entries at the top. Each entry is short — for deep rationale on a single locked decision, write an ADR alongside in `docs/decisions/YYYY-MM-DD-*.md` and reference it here.

Maintained via the `/decisions` skill. See `~/.claude/skills/decisions/SKILL.md` for the entry format and invocation rules.

---

## 2026-08-13: Voice Query declared environment variables never shadow the allowlist, and hold no secrets

**Decision:** Keep the fail-closed environment allowlist (`HOME`, `PATH`,
`TMPDIR`, `LANG`, `LC_ALL`, `LC_CTYPE`, `USER`, `LOGNAME`) as the only inherited
environment for a user CLI, and layer explicit user-declared name/value pairs
*underneath* it inside `spawn_user_cli`. The validator refuses any allowlist
name, `DYLD_*`/`LD_*`, malformed names, duplicates, and anything past 16 pairs
or a 4 KiB value; the spawn ordering enforces the same rule again even if
validation were bypassed. Pairs persist in a Rust-owned `query-env.json` (0600)
behind main-window-gated commands, not in the frontend settings blob, and are
re-validated on read. Values are stored in plain text and Settings states that
API keys and other secrets do not belong there. Storing secrets is deferred
until a Keychain-backed design exists.

**Rationale:** Provider CLIs need a few configuration variables
(`CLAUDE_CONFIG_DIR`, `CODEX_HOME`) that the allowlist cannot anticipate, but a
declared pair that could redefine `HOME` or preload a library would hand the
child a different identity or different code — the exact thing the cleared
environment exists to prevent. Keeping the pairs out of the settings blob keeps
them out of localStorage and out of every webview that has not been handed them.
Plain-text-at-rest is honest for configuration and unacceptable for
credentials, so the surface refuses to imply it can hold them.

**Status:** active

**References:** #550; `app/src-tauri/src/query_env.rs`,
`app/src-tauri/src/managed_child.rs`

---

## 2026-08-13: Voice Query captures a bounded stderr tail and shows it only in the answer popover

**Decision:** Pipe and continuously drain the user CLI's stderr into a bounded
16 KiB tail instead of discarding it. On a terminal failure, store that tail on
the session and serve it to the `query-review` window through the same
requester-gated command as the answer; never broadcast it in a state event,
write it to telemetry, or persist it. When the tail or the partial stdout
matches a known auth-failure signature, remap `exit_nonzero`, `process_failed`,
and `empty_answer` to `provider_not_authenticated` with the exact fix; never
remap codes that describe Murmur's own bounds. The popover shows the error
first and any partial stdout below it as labelled evidence.

**Rationale:** The incident this came from was a provider that printed "Not
logged in" and exited non-zero while the popover rendered `answer ||
errorMessage` — the stdout won and the failure was invisible. Without stderr
there was nothing to diagnose from; with unbounded stderr a chatty CLI could
grow Murmur's memory or block on a full pipe. A tail is the right bound because
the explanatory line is the last one, and the content is as sensitive as the
answer, so it inherits the answer's requester gating rather than a looser path.

**Status:** active

**References:** #550; `app/src-tauri/src/query_flow.rs`,
`app/src/components/query-review/QueryReviewApp.tsx`

---

## 2026-08-13: Application builds and runtime support are macOS-only

**Decision:** Enforce macOS as the only Murmur application target. The Rust
build script rejects non-macOS targets, Tauri bundles only `.app` and DMG
artifacts, and the catalog reports macOS support exclusively. Remove the Linux
compile/test lane, CUDA dependencies and cache action, platform implementation,
WebKitGTK defaults, and X11/Wayland paste path. Platform-neutral automation may
still run on hosted Ubuntu when it does not build or execute the app.

**Rationale:** The product, signed helpers, permissions, window behavior, and
release contract are macOS-specific. Retaining an unshipped compatibility lane
created maintenance and CI cost while implying a support promise the product
does not make. A fail-fast build boundary keeps contributor expectations and
runtime metadata aligned with the shipped product.

**Status:** active; supersedes the 2026-08-12 release-only decision below

**References:** #570; `app/src-tauri/build.rs`, `app/src-tauri/tauri.conf.json`,
`.github/workflows/ci.yml`

---

## 2026-08-12: Releases are macOS-only

**Decision:** Stop building, rehearsing, promoting, or publishing Linux release
artifacts. The trusted release lane now produces one signed and notarized macOS
artifact set, and both updater manifests contain only the macOS platform entry.
Keep the Linux CI compile/test job and `platform/linux` seams for inexpensive
portability coverage, but do not install AppImage packaging tools in that job.

**Rationale:** Murmur is a macOS product, while mutable Linux packaging
dependencies repeatedly blocked unrelated macOS releases without delivering
user value. A macOS-only artifact contract shortens the critical path and
removes that release availability risk without deleting harmless source seams.

**Status:** active

**References:** #549; `.github/workflows/release-build.yml`,
`.github/workflows/release.yml`, `.github/workflows/release-rehearsal.yml`

---

## 2026-08-11: Meeting capture uses explicit-session CATap in the signed capture worker

**Decision:** Phase-one meeting capture creates a private, unmuted Core Audio
process tap inside the existing signed capture worker and carries microphone
and system output as separate protocol channels. The app keeps its macOS 14.0
minimum, but meeting capture is gated to macOS 14.2 or newer with a typed
unsupported result. There is no passive permission probe: the unknown state is
cached until an explicit user action creates one short-lived tap or starts a
meeting. Both the host app and worker embed `NSAudioCaptureUsageDescription`;
the signed native smoke determines the TCC attribution actually shown by macOS.

**Rationale:** The existing CATap RMS proof captures system output without
cloud access or a virtual driver. Avoiding focus-time and polling taps addresses
the historical `coreaudiod` churn failure mode. Keeping the broader 14.0 app
minimum avoids removing otherwise-supported Macs for one opt-in feature.

**Status:** active

**References:** #539; `sidecars/capture/src/system_audio.rs`,
`crates/capture-helper-protocol`

---

## 2026-08-11: Bounded history and stats use durable JSON with localStorage caches

**Decision:** Keep transcript history and usage statistics in their existing
frontend-owned JSON shapes, but make Rust-owned `history.json` and `stats.json`
the durable sources of truth. The main window hydrates localStorage before
render, migrates an existing cache once when the corresponding file is absent,
and mirrors every mutation back to disk. History remains capped at 200 entries;
`retainHistory=false` still rejects new transcript content at `addEntry` before
either store. Rust checks only bounded top-level containers, publishes
atomically with owner-only permissions, and quarantines invalid files without
logging content.

**Rationale:** History is larger than settings but it is deliberately bounded,
loaded in full, and searched in memory. SQLite would introduce a second schema
and query boundary without changing current product behavior. Reusing the
opaque-blob contract closes the observed WKWebView durability gap while
preserving synchronous frontend reads, privacy, rolling trim, and existing
clear/reset behavior. Statistics are preserved because they power cumulative
Insights and are cheap to store as one bounded aggregate object.

**Status:** active

**References:** `commands/settings_store.rs`, `lib/durableUserData.ts`,
`lib/history.ts`, `lib/stats.ts`, `docs/features/history-workspace.md`

---

## 2026-08-09: Rust-owned settings.json is the source of truth; localStorage is a cache

**Decision:** Frontend settings persist durably to `settings.json` in the
per-bundle app data directory via two new commands (`load_settings_blob`,
`save_settings_blob` in `commands/settings_store.rs`). Every window entry
hydrates localStorage from the file before first render; every save writes
localStorage synchronously and mirrors the same blob to disk. Rust validates
only the container (≤1 MiB, JSON object) and never a field — the whole
schema and every migration rule stay in `lib/settings.ts`. Corrupt files are
quarantined to `settings.json.corrupt-<unix-seconds>`, never deleted.

**Rationale:** localStorage lives in WKWebView's website-data store, which a
manual reinstall or WebKit eviction can silently drop — this happened in
practice (2026-08). The app data directory (knowledge store, diagnostics)
survived the same reinstall. The opaque-blob split keeps settings changes
frontend-only while gaining Rust's durability, and the boot-hydration design
preserves the synchronous `loadSettings()` contract that overlay hooks
depend on. Full source-of-truth migration was chosen over a backup/restore
stopgap because the file format is identical either way and the flip cost
was contained to entry-point gating.

**Status:** active

**References:** `commands/settings_store.rs`, `lib/settings.ts`,
`docs/reference/settings.md` ("Persistence and Migration")

---

## 2026-08-09: Log receiver and fleet-logs dashboard hosting moved from whoop-vm to opti

**Decision:** Move the murmur log receiver and fleet-logs dashboard from
`whoop-vm` (an Oracle Cloud VM with a public IP, now deprecated and currently
offline) to `opti` (a fleet node on the home network/tailnet, no public IP).
Because `opti` has no public IP, the previous nginx+certbot direct listener is
replaced by a Cloudflare Tunnel (`opti-murmur`): the tunnel forwards
`murmur.georgenijo.com` straight to the receiver's dashboard and forwards
`georgenijo.com` to a local nginx site that does the `/murmur/*` path
rewrites, same as before. The receiver process, its systemd units, data
layout, and the app's ingest URL (`https://georgenijo.com/murmur/ingest`,
hardcoded in `log_shipper.rs`) are all unchanged. Historical log data on
`whoop-vm` has not been migrated yet — that's a pending manual step once the
VM is reachable again. `georgenijo.com`'s apex now routes entirely to `opti`,
which only serves `/murmur/*`; anything else the old VM served on the apex
404s until separately migrated.

**Rationale:** `whoop-vm` was being decommissioned, and `opti` (already part
of the fleet) is a free, always-on replacement — but it sits behind a home
router with no public IP, so the old direct nginx+certbot model can't work
unchanged. A Cloudflare Tunnel avoids router port-forwarding and gets TLS
termination for free from Cloudflare, at the cost of routing all apex traffic
through a single ingress that currently only knows about `/murmur/*`.

**Status:** active

**References:** `infra/log-receiver/README.md`,
`infra/log-receiver/cloudflared-config.yml`,
`infra/log-receiver/nginx-murmur-ingest-opti.conf`,
`docs/features/log-shipping.md`

---

## 2026-08-05: Spoken Structure is the single owner for dictated punctuation and layout

**Decision:** Replace the overlapping Voice Commands and Smart Formatting
punctuation scanners with one `spoken_structure` engine and ordered pipeline
stage. Resolve an immutable Off/Basic/Extended/Union policy from the existing
Voice Commands and Smart Formatting settings. Keep user replacements/snippets
in Voice Commands; keep email, URL, enumeration, and backtracking grammar in
Smart Formatting; move punctuation, breaks, paired delimiters, symbols, ASR
punctuation arbitration, and `scratch that` into Spoken Structure. Protect
command-generated text until that pass so authored output remains literal.

**Rationale:** Split ownership made behavior depend on toggle order and forced
the same duplicate-punctuation fix into two scanners. `scratch that` also
requires the sentence boundaries created by punctuation in the same
left-to-right pass. One bounded UTF-8-safe engine removes the drift while the
policy preserves existing Basic and Extended behavior without a settings
migration.

**Status:** active

**References:** `spoken_structure.rs`, `transcript_transform.rs`,
`dictation_context.rs`, `docs/features/spoken-structure.md`

---

## 2026-08-04: Hang diagnostics are server-armed per install, never on by default

**Decision:** The client ships a dormant hang-diagnostics collector (worker
stack sample at capture timeout + coreaudiod unified log + audio/Bluetooth
topology + HAL plug-in list, uploaded as one bounded bundle). It activates
only when the log receiver's ingest reply carries `{"diagnostics": true}` for
this install — the receiver derives that flag from a per-UUID allow-list
(`diag-installs.txt`) — and arming is logged loudly in the install's own
event stream. Rejected alternatives: a separate diagnostic
build (delivery friction — the affected user is remote), and shipping
collection enabled for all installs (breaks the privacy model for users who
never consented; the collected data names devices and installed software).

**Rationale:** The remaining unknown in the capture-hang investigation is
*why* `AudioOutputUnitStart` blocks, which only a native stack of the blocked
call plus coreaudiod's own log can answer. Server-side arming delivers the
collector through the normal release channel with zero user action, scopes
collection to exactly the consenting install, and gives an instant no-release
kill switch.

**Status:** active

**References:** #445, #450; `hang_diagnostics.rs`, receiver `/bundle`

---

## 2026-08-04: Backend promotion is self-disproving; first-attempt-bound hangs get a fast-fail primary budget

**Decision:** The session backend memo (2026-08-03) treats a timeout of a
*promoted* backend as proof that the hang is first-attempt-bound rather than
backend-bound: promotion is disabled for that device key for the rest of the
session, a promoted backend's attempt budget is capped at the default
primary's 8 seconds, and after two consecutive recordings of "primary failed
before first PCM, fallback delivered it within 1 second" the primary attempt
budget shrinks to 2 seconds until a primary attempt succeeds again (a slow
rescue also resets the counter). Budgets only ever shrink; sequence
membership, termination confirmation, and fallback-eligibility rules are
unchanged.

**Rationale:** v0.25.1 field telemetry (M5/macOS 26.6) falsified the
backend-bound model the memo assumed: with CPAL promoted, CPAL hung in
`stream_start` for its full 16-second budget and AUHAL then delivered first
PCM in 164ms — the memo oscillated and doubled the user-visible latency on
alternate recordings. On such machines the second attempt reliably succeeds
in ~160ms, so the dominant latency cost is the first attempt's budget; the
fast-fail budget cuts steady-state startup from ~8.5s to ~2.6s while the
detector (fallback must succeed fast) keeps genuinely slow-but-healthy
machines, like the July coreaudiod slowdown where both backends were slow, on
full budgets.

**Status:** active

**References:** supersedes the promotion behavior of the 2026-08-03 memo
entry below; #445

---

## 2026-08-03: Session-scoped backend preference after first PCM; per-native-call capture telemetry

**Decision:** The capture supervisor keeps an in-memory, per-device-key memo of
the backend that most recently delivered first PCM and orders that backend
first on subsequent recordings in the same app run. The memo reorders the two
attempts only — budgets, confirmed-termination rules, and fallback eligibility
from #436 are untouched — and it is not persisted across launches. Separately,
the capture worker now brackets each native Core Audio call with its own
setup-step marker (unit creation, IO enable/disable, current-device binding,
format, callback install, `AudioOutputUnitStart`), and the supervisor names the
last entered step in the budget-exceeded log line.

**Rationale:** Field telemetry from an M5/macOS 26.6 install showed every
recording paying the full 8-second AUHAL budget in `stream_start`
(`AudioOutputUnitStart`) before CPAL succeeded in ~160 ms. Re-proving a known
hang on every recording is pure latency; remembering the last-good backend
bounds the cost to once per app run and self-corrects if the situation changes.
Persistence was deliberately rejected to avoid stale preferences outliving a
transient coreaudiod wedge. Finer step granularity makes the hanging native
call a measurement instead of an inference.

**Status:** active

**References:** builds on #436; ADR
[`2026-08-01-production-capture-helper.md`](2026-08-01-production-capture-helper.md)

---

## 2026-08-03: Capture fallback uses fixed active budgets and confirmed teardown (#436)

**Decision:** AUHAL and CPAL receive separate 8-second and 16-second
initialization budgets inside one 30-second active-time contract. Each backend
has a 2-second confirmed-termination budget, with 2 seconds reserved for
protocol scheduling. CPAL can start only after AUHAL's process group is proven
empty and a final Stop check passes. Pending TCC prompt time is excluded from
active deadlines but bounded by a separate 120-second watchdog. Protocol v3
reports privacy-safe setup sub-phases.

**Rationale:** A global Stop at 30 seconds previously converted a hung primary
attempt into cancellation before the alternate backend was tried. The repaired
contract makes failover deterministic and leaves enough evidence to locate the
underlying AUHAL or CPAL hang without claiming to eliminate it.

**Status:** active

**References:** issue #436; ADR
[`2026-08-01-production-capture-helper.md`](2026-08-01-production-capture-helper.md)

---

## 2026-08-03: Diagnostics move into the main Performance page (#435)

**Decision:** The local model-comparison page is named **Benchmark**.
Structured Events, live resource charts, run history, transform diagnostics,
and report comparison are embedded together under **Settings → Performance**.
The dedicated `log-viewer` webview, capability, entrypoint, and open-window
command are removed; diagnostics shortcuts navigate the main window.

**Rationale:** Benchmarking and observing production performance are different
tasks. Keeping diagnostics inside Settings removes a second-window lifecycle,
makes the data discoverable, and retains the existing local/privacy boundaries.
The main window already owns transcript history and is therefore an appropriate
scope for explicitly reviewed transform diagnostic captures.

**Status:** active

**References:** issue #435; [`../features/log-viewer.md`](../features/log-viewer.md)

---

## 2026-08-01: macOS capture prefers direct AUHAL before CPAL (#426)

**Decision:** Process-isolated macOS recording tries direct AUHAL first and
retains CPAL as the exact-device, pre-buffer fallback. The helper reports
content-free stream-open and first-callback phases so startup regressions can be
attributed without retaining or logging audio.

**Rationale:** CPAL 0.18.1's synchronous Core Audio stream builder remained
blocked for more than 30 seconds on a healthy USB system-default microphone.
The same device consistently retained its first AUHAL PCM in about 180 ms and
reached app readiness in about 200 ms. Prewarming a microphone-closed helper
saved only process-launch time and did not affect the blocking HAL call.

**Status:** active

**References:** issue #426; ADR
[`2026-08-01-production-capture-helper.md`](2026-08-01-production-capture-helper.md)

---

## 2026-08-01: Production HAL ownership moves to a killable capture worker (#405)

**Decision:** Production microphone enumeration and capture run only inside the
signed managed capture worker over binary protocol v3. The worker exposes CPAL
and direct AUHAL backends with one exact-device pre-buffer fallback. Callback
PCM crosses a preallocated SPSC ring; the app validates capture identity,
sequence, bounds, and sample rate before retaining it. Runtime failure
transcribes retained prefixes of at least 500 ms and labels them interrupted.

**Rationale:** Process isolation is the only reliable kill boundary for macOS
HAL operations that can block synchronously. Strict framing and retained-prefix
handling prevent isolation from trading a wedged app for silent audio loss or
cross-generation corruption.

**Status:** active

**References:** issues #405, #408, #409, #410, #411, #412; ADR
[`2026-08-01-production-capture-helper.md`](2026-08-01-production-capture-helper.md)

---

## 2026-07-31: Capture recovery spike uses a per-user agent plus a killable worker (#407)

**Decision:** Probe an `SMAppService` per-user LaunchAgent as the volatile
recovery owner while retaining the existing sandboxed capture helper as the
separately killable HAL worker. The client XPC connection is a lease; loss of
that lease starts bounded worker teardown. The Phase-0 agent retains only
content-free synthetic canary state in RAM for 30 seconds and production
dictation remains unchanged.

**Rationale:** macOS terminated/restarted the LaunchServices-owned app during
active microphone revocation, so a direct child and app-owned PCM cannot survive
the platform transition. An embedded XPC service shares the app lifetime; a
per-user agent can survive it without making the persistent process itself the
unkillable CoreAudio owner.

**Status:** provisional — requires a downloaded signed/notarized registration,
single-TCC-identity, restart/revocation, exact-once recovery, update, and
unregistration matrix before #407 can close

**References:** issue #407; ADR
[`2026-07-31-capture-agent-recovery-spike.md`](2026-07-31-capture-agent-recovery-spike.md);
direct-child ADR
[`2026-07-30-capture-helper-phase-0.md`](2026-07-30-capture-helper-phase-0.md)

---

## 2026-07-30: Capture helper uses a direct managed child; production waits for notarized TCC proof (#407)

**Decision:** Package the Phase-0 `murmur-capture-helper` as an exact signed external binary with a fixed code identity, microphone-only sandbox capabilities, runtime Security.framework validation, nonce-framed private IPC, and direct process-group ownership. Cooperative cancel is bounded at 250 ms before group `SIGKILL`; a new helper is forbidden until direct-PID exit and an empty owned process group are confirmed. The callback retains no PCM and touches atomics only. The helper is probe-only; #409 owns production routing.

**Rationale:** Deterministic kill semantics can be proven without risking the shipped recording path. Apple's responsible-code behavior predicts that a non-daemonized child may attribute TCC to Murmur, but unsigned/ad-hoc behavior and CI cannot prove the install/update/System Settings experience. Production capture therefore remains blocked until the complete downloaded notarized-bundle TCC matrix passes.

**Status:** provisional — kill/packaging implementation landed; signed/notarized interactive TCC evidence is an external release gate

**References:** issue #407; ADR [`2026-07-30-capture-helper-phase-0.md`](2026-07-30-capture-helper-phase-0.md); evidence matrix [`../evidence/407-capture-helper-phase-0.json`](../evidence/407-capture-helper-phase-0.json)

---

## 2026-07-30: In-process CPAL readiness requires retained PCM and strict ownership (#406)

**Decision:** The stabilization path uses CPAL 0.18.1 with raw backend device
IDs, explicit-device fail-closed selection, typed content-free errors, and
first-retained-buffer readiness. `stream.play()` is not readiness. PCM received
before supervisor acceptance is preserved while waveform publication remains
generation-gated. Cancellation and deadlines never detach a worker: Murmur
retains exclusive ownership until that worker exits, rejecting competing starts.

**Rationale:** CPAL's CoreAudio build timeout bounds sample-rate convergence but
does not make every synchronous AudioUnit operation cancellable. Detaching a
blocked in-process worker would permit overlapping HAL owners and turn a timeout
into a more dangerous race. Strict ownership contains that residual until the
process-isolated capture helper supplies a hard-kill fault boundary.

**Status:** active

**References:** issue #406; `audio.rs`; `audio_lifecycle.rs`;
[`transcription.md`](../features/transcription.md).

---

## 2026-07-30: Zero-config default-on diagnostic log shipping (PR #393)

**Decision:** Every install ships its privacy-stripped `events.jsonl` to `https://georgenijo.com/murmur/ingest` (stdlib Python receiver + nginx on whoop-vm) with no setup, no UI, and no consent toggle. Installs are identified by a random UUID, never hostname. Opt-out is the `MURMUR_LOG_SHIPPER=off` env var only. The endpoint URL and bearer token are compile-time constants; the JSONL file is the retry queue (offset advances only on 2xx).

**Rationale:** George wants "install the update, logs flow" for fleet machines and outside installs alike — any configuration step defeats the purpose. Rejected: PostHog/hosted log services (user logs in third-party custody contradicts the local-first pitch), Tailscale Funnel on a home box (uptime dependency; whoop-vm is already an always-on public origin), and a consent toggle (zero-setup requirement). README updated so "no data leaves your machine" claims are scoped to audio/transcriptions.

**Status:** active

**References:** PR #393; `docs/features/log-shipping.md`; `app/src-tauri/src/log_shipper.rs`; fleet secret `murmur-log-ingest-token`.

---

## 2026-07-28: Appearance is a revisioned local semantic-token document (#377)

**Decision:** Murmur themes the main and log-viewer webviews from a separate
`murmur-appearance` document. Concrete `data-appearance` selectors, a strictly
validated resolved-token cache, and a parser-blocking same-origin bootstrap
land as one contract. Main is the only writer, user-change emitter, and owner
of application-level native `setTheme`; both themed windows handle System-mode
OS changes locally without emitting. Overlay and transform-review remain
unsynchronized transparent, always-dark glass. Theme-file exchange uses
main-window-gated 64 KiB UTF-8 Rust transport with atomic sibling-temp writes
and never uses the clipboard. Untouched Sonic remains byte-for-byte compatible
with the shipped palette; all mutable/custom paths enforce the expanded
all-surface and tinted-status contrast matrix.
Diagnostics preserve at-a-glance stream identity by combining the existing
contrast-checked semantic surfaces, foregrounds, and opaque markers rather than
adding stream-specific palette slots; warning levels use the warning token.
The recording Stop action stays visually dominant through an opaque surface,
strong error border, error label, and safe error-tint hover. It does not place
`on-primary` on error because the deliberately small schema has no `on-error`
pair and does not guarantee that combination.

**Rationale:** Separating appearance from dictation settings prevents unrelated
cross-window settings traffic and preserves immutable recording semantics.
Concrete selector state makes forced appearance agree across Tailwind,
semantic tokens, first paint, and native chrome. A narrow bounded transport
keeps schema/color authority in the tested frontend while preventing partial
exports and clipboard regressions. Grandfathering only untouched Sonic resolves
the direct conflict between exact reset parity and the newer status-matrix
contract without giving user-controlled colors an accessibility escape hatch.

**Status:** active

**References:** issue #377;
[`docs/features/appearance.md`](../features/appearance.md);
[`docs/draft/theme-engine-converged-plan.md`](../draft/theme-engine-converged-plan.md).

---

## 2026-07-27: Workflow features stay local, opt-in, and fail-safe

**Decision:** Three main-window workflow features ship together — the history workspace (search/filter/export), Stop on Silence, and the ⌘K command palette — under three shared constraints. (1) **Export is a document sink, not a file-write primitive:** `save_text_export` refuses anything outside `.json`/`.md`/`.txt`, non-absolute paths, dotfiles, directories, missing parents, and payloads over 8 MB, and publishes atomically; `teachingContext` is excluded from every export format. (2) **Stop on Silence applies to any recording not started by holding the trigger key and must hear speech before it can arm**, with a threshold that only ever rises above an absolute floor — on a quiet microphone it does nothing rather than cutting the speaker off, and any out-of-allow-list persisted duration coerces to Off. (Originally shipped Double-Tap-only; widened to the origin rule after device testing, 2026-07-28 — while the key is physically held, the release owns the stop.) (3) **The palette owns no behavior:** each row carries a `run` callback from `App.tsx`, so there is exactly one definition of each action.

**Rationale:** All three touch surfaces where a wrong default is expensive: writing files the user did not ask for, ending a recording early, and duplicating action semantics. Pinning was built for this batch and then cut before merge after device testing (2026-07-28): copy, export, and the knowledge store already answer "keep this transcript" better than a special history state, and pinning's cost was a second trim budget, a pin ceiling with its own error, and a split clear. The underlying worry — the rolling trim silently dropping something you cared about — is answered by raising the history cap to 200 instead.

**Status:** active

**References:** `docs/features/history-workspace.md`; `docs/features/silence-auto-stop.md`; `docs/features/command-palette.md`; branch `feat/workflow-boosters`.

---

## 2026-07-22: Diagnostics accelerator metrics stay honest (#354)

**Decision:** Diagnostics will not display GPU or ANE utilization percentages. The production follow-up may ship exact backend identity, request timing, real-time factor or token throughput, correctly scoped RSS, the existing explicitly host-wide CPU percentage, and `GPU utilization unavailable` / `Accelerator utilization unavailable`. Public Metal timestamps, counters, and allocation accounting remain developer-only until Murmur's pinned runtime exposes an integration seam and a production rehearsal proves it.

**Rationale:** Public Metal instrumentation measures command buffers, encoders, and resources the caller can access; Murmur's pinned whisper.cpp and llama.cpp runtimes own those objects internally, while Core ML exposes allowed compute-unit selection rather than production execution attribution. The standalone public-API probe proves behavior only for work it owns and cannot justify fabricated percentages or claims about the pinned runtimes.

**Status:** active

**References:** issue #354; parent #350; ADR [`2026-07-22-accelerator-diagnostics-metrics.md`](2026-07-22-accelerator-diagnostics-metrics.md); disposable probe `spikes/354-metal-metrics`.

---

## 2026-07-21: Pre-merge release tuning uses a secretless unsigned rehearsal (#319)

**Decision:** Release-performance experiments are measured by a main-defined, manual-only workflow that builds an immutable source SHA in secretless read-only jobs. Cargo and CUDA caches are source-SHA-isolated. macOS app and Linux deb/AppImage builds remain unsigned; JSON evidence records build timing, cache state, workflow/source identity, and size proxies. Signing, notarization, updater signing, tags, and promotion remain exclusive to the trusted production release path.

**Rationale:** Running feature-branch source inside `Release Build` would expose Apple/updater credentials and trusted cache namespaces. LTO and codegen-unit changes affect compile/link/bundle work, while notarization is external queue noise, so an unsigned proxy is both safer and more causally useful.

**Status:** active

**References:** issue #319; unblocks #305; [ADR](2026-07-21-secure-release-rehearsal.md).

---

## 2026-07-20: Selected-text transform Phase D wrap (#312)

**Decision:** Ship settings + presets + docs for local selected-text transform without expanding scope into AX webview special-cases. Built-in presets (Shorten / Bullets / Professional / Fix grammar / Casual) and user-defined `KnowledgeKind::Transform` names expand in `finish_transform_instruction` before the sidecar runs. Settings owns hold-key wiring, model download/remove/reset, and saved-transform CRUD. Cursor-chat and similar webviews remain best-effort (documented limitation, not a blocker). Native smoke and issue acceptance checkboxes stay a separate pass on a built `.app`.

**Rationale:** Phases A–C delivered the signed sidecar, AX capture/apply, review popover, and end-to-end flow. Phase D only exposes configuration and documents the contract; dogfood follow-ups must not grow apply semantics or link llama into the app crate.

**Status:** active

**References:** issue #312; ADR `2026-07-20-signed-local-llm-sidecar.md`; `docs/features/selected-text-transform.md`; branches `issue/312-transform-flow`, `issue/312-transform-settings`.

---

## 2026-07-20: Overlay geometry & lifecycle contract locked (#280)

**Decision:** Five locked outcomes of the overlay architecture review (issue #280; PRs #290, #299, #301):
1. **Rust is sole author of overlay geometry.** All dimensions derive from `geometry_for()` in `commands/overlay.rs` returning `OverlayGeometry`; the frontend consumes it at runtime (`get_overlay_geometry`, `overlay-geometry-changed`) and contains no geometry pixel constants. Motion timing (ms/easing) is frontend-owned in `lib/overlayMotion.ts`, with the shrink delay derived from the height-transition token — never free-standing.
2. **Contract enforced by a shared checked-in fixture** (`app/src/components/overlay/overlay-geometry.fixture.json`) asserted from both `cargo test` and vitest. No codegen.
3. **Hover expansion is one serialized 4-phase controller** (`useOverlayExpansion`: collapsed/opening/open/closing) with grow-then-reveal / hide-then-shrink ordering, applied-frame acks from `set_overlay_expanded`, and a generation-guarded writer owning every surface resize. No hook may own half of this lifecycle.
4. **Contract + controller + split land before any visual rehaul** (PR4); the rehaul may not touch geometry derivation except via `geometry_for()`.
5. **Cross-window settings stay localStorage + `settings-changed` events**, wrapped in per-window hooks; all overlay settings access goes through `loadSettings()` — no Rust settings store.

**Rationale:** TS and Rust were independent authors of overlay geometry (divergent no-notch fallbacks 185/140/200, hand-mirrored 44px drop height) and the expand choreography was un-acknowledged (CSS could animate into a window that had not grown). Two independent architecture reviews converged on the diagnosis; runtime Rust-owned geometry beats a shared-constants file because it shares the *derivation*, not just values, making formula drift structurally impossible rather than test-guarded.

**Status:** active

**References:** issue #280 (review memo + drift note), PR #290 (PR1 geometry contract), PR #299 (replacement PR2 expansion controller), PR #301 (replacement PR3 component split), `docs/features/overlay.md`.

---

## 2026-07-20: Local LLMs require a signed, sandboxed macOS-arm64 sidecar

**Decision:** Issue #312 (originally #300, superseded) proposes a purpose-built macOS 14 Apple Silicon helper using an exactly pinned static llama.cpp/Metal runtime, a single hash-pinned Qwen2.5 1.5B Q4_K_M model, bounded stdin/stdout IPC, a verified inherited model descriptor, and dedicated App Sandbox entitlements. The helper has no network, files-by-path, shell, tools, clipboard, accessibility, automation, app group, XPC, or cloud fallback. Linux reports the capability as unsupported.

**Rationale:** The active 2026-06-23 decision proves in-process Whisper + llama.cpp is unsafe because their ggml ABIs collide. Tauri's stock macOS signer also applies one entitlement set to the app and external binaries, so a repository-owned no-sign/finalize path is required to preserve a stricter helper sandbox. The design is accepted only after the inherited-descriptor/Metal and split-entitlement signing gates pass, followed by a signed/notarized/quarantined non-publishing rehearsal.

**Status:** active — ADR accepted 2026-07-21 after the CI signing rehearsal (run 29793936645) delivered the outstanding notarization/staple/Gatekeeper/probe evidence; see [ADR](2026-07-20-signed-local-llm-sidecar.md)

**References:** issue #312 (originally #300, superseded); unblocks #312 phases B–D (originally #254).

---

## 2026-06-23: In-process Tier 3 abandoned (ggml ABI clash); deferred to a sidecar

**Decision:** Tiers 1–2 (no-LLM post-model correction) ship as planned. Tier 3 (local-LLM cleanup) is NOT shipped in-process — the `llama-cpp-2` integration and dormant settings/module were removed from the app crate. Tier 3 is deferred to a future sidecar-process design. This supersedes the Tier-3 portion of the entry below (Tiers 1–2 portion still stands).

**Rationale:** `whisper-rs` and `llama-cpp-2` each statically vendor their own `ggml`. They link (matching symbol names) but **SIGSEGV at runtime** — an ABI mismatch between two ggml versions in one process — reproduced in both CPU (`MURMUR_T3_GPU_LAYERS=0`) and Metal modes during model load. Proven by isolation: a standalone binary linking only `llama-cpp-2` loads the GGUF and generates text fine (`MODEL LOADED OK`); the same code inside the app (which also links whisper) crashes. The only viable local-LLM path is a separate sidecar process (proven to run in isolation), which is a substantial new subsystem (persistent helper binary, IPC, lifecycle, Tauri `externalBin` bundling + codesign, model-download UX) with a CI signing/bundling path that can't be validated locally before pushing — too much risk for this release. A secondary finding: the 0.5B model wrapped its output in a ```` ```php ```` code fence (weak instruction-following), so Tier 3 would also need the 1.5B variant + output sanitization + prompt tuning. Working inference code is parked for the sidecar effort.

**Status:** active

**References:** branch `feat/post-model-correction`; commit removing in-process Tier 3; scratchpad `t3probe` (standalone proof) + `correction_model.rs.sidecar-ref` (parked impl).

---

## 2026-06-23: Post-model correction layer — 3 tiers, local-only Tier 3, no routing

**Status (Tier 3 portion):** superseded by 2026-06-23 (in-process Tier 3 abandoned). Tiers 1–2 portion active.

**Decision:** Add a post-model TEXT correction layer that runs on every ASR backend, beside the existing cleanup + voice-command passes. Tier 1 = exact spoken→written term map (Aho-Corasick, single pass). Tier 2 = sounds-like match (Metaphone phonetic key + edit-distance, confidence cutoff, fires only near vocab). Both no-LLM, built on settings-change, target <1ms, logged as a `correction_ms` telemetry phase; one unified vocab config feeds both. Tier 3 = optional model cleanup using a **100% local** model only: Qwen2.5-1.5B-Instruct (Q4_K_M GGUF) via `llama-cpp-2` + Metal (Apache-2.0), with Qwen2.5-0.5B-Instruct as an optional "fast mode." The Tier 3 backend is a trait so a future BYO-OpenAI-compatible cloud option can plug in if approved. Smart per-input routing is dropped in favor of a single configurable backend.

**Rationale:** Vocab previously fed Whisper's `initial_prompt`, a silent no-op on the default Parakeet/sherpa engine (parakeet.rs:208) — moving correction post-model fixes it for every engine and is the only place that can do camelCase/abbrev orthography. Cursor "Composer 2.5 Fast" was the only permitted cloud model, but live probing of `api.cursor.com` with a valid Admin-scoped key confirmed there is NO headless chat/completions endpoint: `/v1/me`, `/v1/models`, `/v0/agents` return 200 but only expose account/model-metadata/repo-bound Cloud Agents; every inference verb (`/v1/chat/completions`, `/v1/responses`, `/v1/completions`, `/v1/chat`, `/v1/messages`, `/v1/generate`) 404s. Composer has "no external API." Per the "no substitute" directive, cloud is dropped (kept as a trait seam). Routing was dropped because its "large input → bigger model = faster" premise only held for cloud; locally a bigger model is *slower* on long input and multiple resident models waste RAM. Mitigations: optional 0.5B fast-mode + a length-guard that skips Tier 3 on very long inputs.

**Status:** active

**References:** branch `feat/post-model-correction`; parakeet.rs:208; recording.rs (post-transcription pipeline, insertion after voice_commands); Qwen2.5 GGUF (Apache-2.0) via `llama-cpp-2`; Cursor probe — `/v1/me`,`/v1/models`,`/v0/agents` = 200, all inference verbs = 404.

---
