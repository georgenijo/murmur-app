# Decisions Log

## 2026-11-07: Freeze GPUI identity and additive downgrade policy through Phase 3 (#997)

**Decision:** Compare native packaging and signing inputs with frozen Tauri identity inputs, retain a null updater minimum and the last compatible Tauri DMG, and preserve opaque same-version fields in shared Rust state. Name the final Tauri release with these compatible Rust loaders before the flip; until that tag is set, signed flip readiness fails closed. Keep the signed installed Tauri → GPUI → GPUI chain and real TCC/data acceptance as George-owned pending gates.

**Rationale:** Linux can enforce inputs, generated manifest policy and synthetic disk compatibility without signing credentials. Only production Developer ID/notarized installed builds can establish TCC continuity and actual update/downgrade recovery; no secretless test is a substitute.

**Status:** active

**References:** #997, [identity/update/downgrade contract](../draft/gpui-identity-update-chain.md), [rewrite plan §2.4](../draft/gpui-rewrite-plan.md#phase-2-gpui-becomes-the-host)

---

## 2026-11-07: Gate native startup on durable legacy import and scale GPUI content coherently (#996)

**Decision:** Add draft layer 23 above the production adapters. A disposable native worker alone loads WebKit to read the fixed legacy origin; the GPUI process retains the shared completion owner, durable receipts and exact key-removal acknowledgements. Main exposes a loading/retry gate until import and production configuration finish. Whole-window zoom scales layout, paint, platform input, AX and IME together, while native window geometry remains in AppKit points.

**Rationale:** Existing users must reach their migrated settings before recording admission. The standalone transport survives Tauri removal, and shares migration decisions with the shipping importer. Changing rem size alone misses fixed-pixel controls and cannot preserve pointer/AX/IME alignment.

**Status:** active

**References:** #996, #1037, #1038, #1039, #1040, [startup integration](../draft/gpui-startup-integration.md)

---

## 2026-11-07: Separate native services from production view composition (#996)

**Decision:** Keep the authorized complete #996 scope, with one further draft
layer: 19 host shell → 20 shared service extraction → 21 native OS services
and native service-admission seams → 22 production adapters and executable.
Layer 22 must compose all production views and the shared service owners; no
integration gaps are handed back as blockers. Each layer must build and pass
CI independently. The production binary remains named `ui`.

**Rationale:** Native OS registration/migration and per-view generation, lease,
file/export and snapshot adapters are substantial independent review domains.
The extra layer keeps the final app composition reviewable without changing
scope or the native-parity acceptance requirement.

**Status:** active

**References:** #996, [host-services contract](../draft/gpui-host-services.md)

---

## 2026-11-07: Include production service extraction in the native host stack (#996)

**Decision:** Split #996 into stacked draft layers 19 (native shell), 20
(service extraction), and 21 (native OS services and production composition).
Extract the remaining recording, meeting, delivery and shutdown owners instead
of leaving adapter injection as an integration blocker. Keep the shipping
Tauri behavior and its tests until Phase 4. Packaging layer 18 remains separate;
the lead will integrate it and retarget the stack. Never target main or merge
these implementation drafts.

**Rationale:** The accepted native-parity route needs a working GPUI app for
MacBook acceptance; gallery-only injection traits cannot deliver that outcome.
Separating ownership extraction makes each executable behavior change reviewable.

**Status:** active

**References:** #996, #994, [native host contract](../draft/gpui-native-host.md)

## 2026-11-07: Reuse the release contract for native packaging; defer signed acceptance (#994)

**Decision:** Use a repository-owned Python packager for GPUI with the existing
Tauri config as the shared application identity/version source, and a standalone
Minisign-compatible Rust signer. Keep the original Tauri release job unchanged
except target selection. GPUI requires a trusted-main manual dispatch and uses
an artifact namespace that promotion cannot consume.

**Rationale:** A fixed four-executable layout does not justify a second package
manager/configuration. Reusing finalization, entitlements, notarization and
provenance preserves the release trust contract. Secretless rehearsal compares
both real bundlers independently with the same native payload. The native-parity
route prohibits a packaging-only main landing, so signed/notarized acceptance
cannot run during the draft rewrite stack.

**Status:** active for packaging; signed rehearsal, exact signed-artifact Mac
mini acceptance, production host wiring and real OTA proof remain deferred.
Issue #994 stays open until its signed gate passes after the authorized stack
landing. No release dispatch or signing-secret access is authorized here.

**References:** #994, #996, #997,
[Phase 0 route](2026-11-05-gpui-phase-0-exit.md),
[rehearsal trust boundary](2026-07-21-secure-release-rehearsal.md),
[release runbook](../release.md)

---

## 2026-11-06: Keep Live Words ephemeral behind a generation-checked native adapter (#1000)

**Decision:** Build the GPUI dictation preview as a separate murmur-ui consumer
of durable consent, lifecycle and provisional results. Reuse shared overlay
geometry, panel treatment, preferences and motion. Keep preview WebKit
retirement on the same weak-deallocation barrier as the overlay, outside the
native UI graph.

**Rationale:** The Phase 0 native-parity route postpones the production host
switch to Phase 2. A typed, bounded presentation boundary preserves privacy and
stale-generation rejection without copying backend inference, settings or final
delivery logic. Revision tickets recheck deferred first-frame work on main;
consent epochs reject stale durable reads independently of recording ownership.

**Status:** active (draft stack; host integration, Sol review and native
acceptance remain open).

**References:** #1000, #1011, PR #1028,
[preview contract](../draft/gpui-dictation-preview.md),
[overlay contract](../draft/gpui-overlay.md)

---

## 2026-11-06: Share overlay geometry and keep the native port host-independent (#999)

**Decision:** Move the existing pure overlay geometry and its characterization
tests into murmur-core, with the Tauri adapter re-exporting the same contracts.
Build the GPUI screen in murmur-ui with a typed, bounded event inbox and an
explicit service-intent boundary. Keep legacy WKWebView destruction in
murmur-webkit, outside the native UI graph; require actual deallocation rather
than hiding before treating a replaced overlay as retired.

**Rationale:** Phase 0 selected native parity first, so this layer cannot embed
GPUI into Tauri or silently flip the shipping host. Sharing geometry prevents
notch, calibration, Dock and preview behavior from diverging. The presentation
layer must consume the existing dictation, completion, settings and shutdown
owners. A weak view listing and an own-process WebKit image inventory distinguish
legacy destruction from the native renderer's lack of a WebKit dependency.

**Status:** active (draft stack; native host integration, Sol review and native
acceptance remain open).

**References:** #999, #978, #1011,
[overlay contract](../draft/gpui-overlay.md),
[Phase 0 decision](2026-11-05-gpui-phase-0-exit.md)

---

## 2026-11-06: Patch deferred GPUI accessibility in production (#998)

**Decision:** Vendor only the Apache-2.0 GPUI crate at the unchanged Phase 0
revision `654efae441bc8eb117d0042bb28cd7cf3570bdc9`. The native workspace's
Cargo git-source patch applies the recorded deferred-AX ancestry fix to
ordinary library/gallery builds. Keep the test transport patch separate.

**Rationale:** Pinned deferred rendering restores dispatch ancestry but loses
AX ancestry, rejecting dropdown active descendants. Deferring the entire
combobox also changes Tab order; inline rendering loses popup paint/hitbox
stacking. The engine fix preserves these existing contracts together. Record
the self-contained manifest and source changes as reproducible patch hunks.
Single-line range geometry uses shaped, scrolled editor layout; unsupported
multi-line/nonmonotonic layouts retain AXValue without advertising range APIs.

**Status:** active (draft stack; native acceptance remains open). This refines
the dependency source in the earlier isolated-workspace decision; its revision,
UI/core boundary and test-transport isolation remain in force.

**References:** #998, PR #1027,
[component contract](../draft/gpui-accessible-components.md),
`app/gpui/patches/gpui-deferred-accessibility.patch`

---

## 2026-11-06: Distinguish typed GPUI events from Tauri transport tokens (#998)

**Decision:** Authored GPUI code remains in the repository-wide transport and
restart source inventories. A native event adapter permits only the exact typed
two-argument `gpui::Context::<T>::emit(cx, event)` call; Tauri emission APIs stay
reserved to their existing targeted adapter. Exclude only the exact vendored
GPUI Kit/AccessKit dependency roots and native Cargo output, following the
existing Tao/build-output convention.

**Why:** Upstream local entity events and animation variables use `emit` and
`restart` without performing Tauri transport or process relaunch. Renaming the
upstream API or exempting all native application code would obscure that
boundary. Guard tests reject Tauri calls in the native adapter, maintained
lookalike paths and Rust inclusions into excluded trees.

**Status:** active (draft stack; independent review pending).

**References:** #998, `scripts/check_targeted_events.py`,
`app/gpui/src/events.rs`, [component contract](../draft/gpui-accessible-components.md)


## 2026-11-06: Isolate the reusable GPUI component workspace (#998)

**Decision:** Build the native presentation layer in `app/gpui`, a separate
Cargo workspace/lockfile on Rust 1.98.1, with a one-way dependency on
`murmur-core`. Vendor the four Apache-2.0 gpui-component v0.9.0 crates from the
proven Phase 0 port. Keep its keyboard/editor infrastructure, add Murmur's
styled wrappers and port the text-range patch to the existing editor owner.

**Why:** Later screen ports need one reusable native layer while the shipping
Tauri graph/toolchain remains independent. The pinned engine's TestWindow drops
AX callbacks, so a test-only transport patch is applied to a private GPUI source
copy in the task target directory; actual tree building and action dispatch are
tested without live windows or modifying shared Cargo sources. Production GPUI
continues to resolve to the exact Phase 0 git revision.

**Status:** active (draft stack; live native and MacBook acceptance remain open).

**References:** #998, #1011,
[component contract](../draft/gpui-accessible-components.md),
[Phase 0 exit](2026-11-05-gpui-phase-0-exit.md)


## 2026-11-06: Own the updater on minisign-verify 0.3 (#990)

**Decision:** Use a Tauri-free in-house Rust updater on `minisign-verify`
0.3.0, preserving the existing latest-v2.json endpoint and public key. Own
static platform lookup, minimum policy, durable state and atomic macOS bundle
exchange; only the shutdown coordinator authorizes compatible relaunch.

**Why:** Published cargo-packager-updater 0.2.3 uses minisign-verify 0.2,
`macos` platform names and an install fallback that removes the installed app
before replacement. Adapting it would still require our policy, translocation,
recovery and shutdown layers. The selected verifier has no dependencies and
rejects noncanonical Ed25519 signatures; the remaining dependencies already
exist in our lockfile. This works with Phase 2's non-Tauri packager if it keeps
the signed .app.tar.gz, ui executable and bundle metadata contract. No product
or cost trade-off needs lead approval.

**Status:** active (stack implementation; native QA and two authorized real OTA
canaries remain acceptance gates before Phase 2).

**References:** #990, #1009, [updater design](../draft/gpui-host-neutral-updater.md),
[verifier](https://github.com/jedisct1/rust-minisign-verify),
[cargo-packager-updater 0.2.3 source](https://docs.rs/crate/cargo-packager-updater/0.2.3/source/src/lib.rs),
[dependency manifest](https://docs.rs/crate/cargo-packager-updater/0.2.3/source/Cargo.toml)

## 2026-11-05: GPUI, native parity first, on one stacked branch (#983)

**Decision:** Murmur's UI moves to GPUI (Zed rev `654efae4`, Rust 1.98.1)
through the native-parity route. Every plan issue is one stacked PR, and
nothing merges to `main` until the top of the stack works fully on George's
MacBook.

**Why:** No Stage A prototype hit a fundamental blocker (see the record).
Because nothing ships part-way, the hybrid work (0c, 0d, 1.7, 1.9, 2.2) is
dropped.

**References:** [Phase 0 exit](2026-11-05-gpui-phase-0-exit.md),
[GPUI rewrite plan](../draft/gpui-rewrite-plan.md)

## 2026-11-04: Keep strict branch protection; skip verified pushes and use merge trains (#924)

**Decision:** `main` keeps `strict` up-to-date protection with `ci-pass` as the
only required check. A push to `main` skips its tests when the commit's tree
equals an up-to-date PR head whose latest `ci-pass` succeeded. Several green PRs land
together through one train PR (`scripts/merge_train.sh`) that merges with a
merge commit.

**Why:** Strict protection catches PRs that pass alone but fail together. On
2026-11-04, #918 and #920 compiled separately but not combined. Dropping
`strict` would let `main` break until a push run noticed. GitHub's merge queue
batches PRs safely, but it is unavailable for personal-account repositories. A
train reruns CI once for N PRs instead of about N² times. PR CI tests the head merged with the base of that moment, so a
matching tree counts only when the head already contained the pre-merge `main`;
then every tested merge equals the head, and the push run has nothing new to
test. Any mismatch falls back to the full suite.

**References:** `scripts/ci_verified_tree.sh`, `scripts/merge_train.sh`,
[CI and merging](../DEVELOPMENT.md#ci-and-merging)

## 2026-11-02: Bezel floating pill on a native HUD material

**Decision:** The floating pill and the Live Words card above it use a native
`NSVisualEffectView` (`.hudWindow`, behind-window, always active) under the
webview, with a 74–76% tint of the same tone and a 1pt hairline. It follows the
system light/dark appearance, is monochrome except the red recording dot, and
pairs every state with one shape and one word in a fixed 120×34 capsule. Rust
owns the material's rect: the webview sends only semantic state (shown,
expanded, content) or, for Live Words, the card's laid-out size. The notch
island is unchanged.

**Why:** `backdrop-filter` cannot see behind a transparent window, so a real
blur has to be native. Sending semantic state instead of pixel rects keeps
geometry in one place: Rust refits the mask in the same main-thread turn as each
window resize and Dock move, so it never trails the window by a frame. The tint
floor is what lets labels hold 7:1 over any wallpaper, including pure white and
pure black, without an opaque slab; Reduce Transparency makes it opaque. A
native layer cannot follow a CSS scale, so the Bezel pill fades without the
notch's scale. Live Words size is measured by the webview because only it lays
out the words; that costs one IPC round trip per resize. Details:
[Overlay placements](../features/overlay.md#placements).

## 2026-11-01: Parallel, sharded CI with process-isolated Rust tests

**Decision:** `changes` only filters paths; workflow validation runs in a
separate `policy` job that every run requires. Vitest, the Linux and macOS
Playwright goldens, and Rust lint vs. Rust tests run as parallel shards or jobs,
and `ci-pass` requires all of them. Rust unit tests run under `cargo nextest`
instead of `cargo test -- --test-threads=1`. Docs, scripts, and Python tests no
longer start the frontend or Rust builds, and CI no longer runs a separate
`cargo check` before clippy.

**Why:** PR CI took 7–10 minutes, mostly in serial work: validation blocked
every job, Linux goldens ran after vitest, and a single Rust job ran check,
clippy, the test build, and serial tests back to back. nextest runs every test
in its own process, so the shared process state that forced one thread stays
isolated. Locally the suite went from 29 s to 9 s, and it passed repeated runs
at 3 and 20 threads. Clippy `--all-targets` already type-checks everything
`cargo check` did. Only `ci-pass` is a required check, so renaming or splitting
jobs does not touch branch protection.

**References:** `.github/workflows/ci.yml`, `scripts/validate_workflow_policy.py`

## 2026-10-29: No decode-time vocabulary biasing for Parakeet yet (#832)

**Decision:** Keep vocabulary correction after recognition. Do not wire
FluidAudio's CTC word-spotter rescoring, and do not add a transcribe.cpp
backend now.

**Why:** FluidAudio 0.14.1 has no decode-time biasing for Parakeet TDT. Its
custom-vocabulary path rescores with an auxiliary ~98 MB CTC encoder (a second
encoder pass), and `fluidaudio-rs` does not expose it. On a synthetic `say`
fixture (52 clips), stock thresholds raised look-alike WER from 0% to 10.87%
and wrote 16 wrong terms. Conservative thresholds were safe but only joined
split compounds, which Murmur's existing Smart Correction already fixes as
text with no added latency (20/30 term hits vs 19/30). transcribe.cpp would
need a second ggml copy, which can't link beside whisper.cpp, or a whole second
Parakeet runtime in a sidecar. Real-voice accuracy is unmeasured; revisit with
the #827 accuracy corpus. Details: [Transcription](../features/transcription.md#vocabulary-biasing-during-decoding-no-go-for-now-832).

## 2026-10-28: Mode-aware spoken numbers and context-cued vocabulary

**Decision:** Freeze one content-free `TextRegister` per recording: the
built-in Mode family that the effective writing style resolves to (Everyday,
Messages, Email, Notes, Technical, Terminal, Verbatim). Copy it into the
transcript stage configuration. Spoken numbers use a **prose** policy for
Everyday, Messages, Email, and Notes, and a **technical** policy for Technical,
Terminal, or any recording with Local IDE project context. Prose keeps small
standalone counts from zero through nine as words ("two of them", "two or
three days"). It still renders digits for values of ten or more, units,
currency, percent, times, dates, years, labels, and dotted versions. Technical
converts eagerly and also joins spaced digit runs (`8080`), dotted versions
and addresses (`192.168.1.1`), and ordinals of ten or more (`21st`). Every
policy leaves hyphenated compounds such as `two-factor` unchanged and accepts
a bare `point` as a decimal only in a value context.

Generalize the hard-coded `Maine`/`me` → `main` rule into data. A contextual
variant applies when the frozen register is allowed, when project context is
active, or when every window of a cue clause matches. A blocking word within
its window refuses it, and two simultaneously applicable variants fail closed.
The built-in pack adds `Tori`/`Tory` → `Tauri`, `Postgres SQL` →
`PostgreSQL`, `npxtsc` → `npx tsc`, and `CoreML` → `Core ML`, gated to
Technical/Terminal or project context (Tauri also by nearby framework cues).
Cue and blocker windows stay within the candidate's clause, and variants
match Unicode-aware whole words. Explicit aliases and learned replacements
keep precedence. Context requirements stay internal to the built-in pack.
User-authored ones need Settings support and a storage shape an older build
cannot read as an unrestricted alias, so they are deferred.

Explicit alias and learned-rule output is literal to number rendering. Rules
whose heard phrase contains a digit are matched again at the end of the
spoken-numbers stage, only over digits that stage rendered and never over
explicit output. When an intermediate stage rewrote the text, the re-match is
skipped.

**Why:** Dictation took numbers too literally ("2 of them", "2-factor",
"20 26", "at this 0.5 people", "4 30 PM") and could not use a word's other
meaning: a person named Tori is not the framework. The frozen register already
decides the writing style, so reusing it keeps the policy deterministic,
local, inspectable, and identical for a given Mode. It never infers the
policy from text or adds a model. Correct and Teach learns from final text,
which already contains digits, but Smart Correction runs before numbers, so
taught rules containing digits never matched. Matching them after rendering
fixes that without reordering the pipeline.

**Status:** active

**References:** `docs/features/spoken-number-formatting.md`,
`docs/features/vocabulary-aliases.md#context-cued-variants`,
`docs/features/modes.md#text-register`,
`app/src-tauri/src/spoken_numbers.rs`, `app/src-tauri/src/correction.rs`

## 2026-10-17: Unknown menu-bar occupancy retains the last proven slot

**Decision:** Read the menu-bar owning application, falling back to the
frontmost app only if no owner is reported. Skip AX reads for Murmur itself
and non-Regular owners, and classify unavailable samples without recording
identity. An unavailable menu sample retains the last proven overlay slot
only on an unchanged, fully measured display with a visible bar. Fresh status
items revalidate the entire slot and the unanswered-app reserve; successful
menu samples remain authoritative and can move or evict it. Startup/display
triggers discard retention. All placement triggers share the 600/1500/3000ms
settle ladder, ending after the first known settle sample.

**Why:** The three investigations behind this fix found every observed
below-band placement followed an unavailable menu sample (an accessory owner
or Murmur's own process), never a measured lack of space. Unknown occupancy
should not be treated as proof that an established slot is crowded. Status
revalidation, display identity checks and bounded retries limit stale-slot
risk without restoring the status-item cache reverted in #800.

## 2026-10-15: Smart destination falls back to the stop-anchored paste after an app switch

**Decision:** When a Smart destination recording ends in an app other than the
one it started in, and that app may hold a text input (editable or unreadable
focus), the transcript goes to the app the user stopped in through the normal
verified paste, exactly as with Smart destination off (#596's stop anchor).
This replaces the 0.51.11 rule that left it on the clipboard. Only an app that
positively holds no text input hands the paste back to the original field; if
that handoff cannot complete, the transcript stays on the clipboard.

**Why:** Production logs from 0.51.9 to 0.51.12 attributed 10 of 12 recent
missed pastes to this case: the same-app fallback re-bound verification to the
start app, so every paste after a mid-recording app switch was refused as
`different_application`. The normal paste path still verifies the exact app
instance and refuses secure fields, so this adds no new authorization path.

## 2026-09-09: AGENTS.md is a symlink to CLAUDE.md

**Decision:** AGENTS.md and CLAUDE.md were two hand-maintained copies of the
same agent guide and had drifted (stale file references, mismatched File Map
rows). AGENTS.md is now a symlink to CLAUDE.md so there is exactly one agent
guide; edit CLAUDE.md only. The H1 was changed to `# Murmur — agent guide
(CLAUDE.md / AGENTS.md)` to reflect that it now serves both entry points.

## 2026-09-07: Label remote meeting speakers locally with session-scoped names (#685)

**Decision:** Add optional post-meeting speaker refinement for the remote
`Them` channel. When enabled at capture start, Murmur writes that channel to
a private temporary 16 kHz WAV capped at two hours. After transcription
finishes, an idle-only child runs the pinned Core ML speaker model with network
access denied. Murmur accepts only bounded turns and assigns a label when one
speaker covers a complete final remote segment with sufficient confidence and
coverage. Ambiguous segments remain `Them`.

Speaker numbers and renamed labels belong to one meeting. Murmur stores no
cross-meeting voice profile or embedding. Foreground transcription preempts the
exact child process group and confirms its exit. Every terminal path deletes
the temporary WAV.

**Rationale:** Channel separation distinguishes the local speaker from remote
audio but cannot distinguish several people on a call. Conservative local
assignment makes clear passages easier to review without changing transcript
text or guessing through overlap.

**Status:** active

**References:** PR #685 and issue #540, `docs/features/meeting-diarization.md`,
`app/src-tauri/src/meeting_diarization.rs`, and
`app/src-tauri/src/diarization_assignment.rs`

---

## 2026-09-07: Add private native capture smoke to first PCM (#687)

**Decision:** Run physical-microphone capture checks in the private,
owner-only `georgenijo/murmur-native-ci` controller repository. Do not attach
its self-hosted Mac runner to the public repository. Each dispatch names an
immutable Murmur commit and a trusted branch that contains it. The job builds
the production-role worker and tests AUHAL and CPAL separately. Each backend
must complete ordered setup, deliver at least three continuous PCM frames,
reach first PCM within two seconds, acknowledge stop, and exit within the
bounded deadline.

The public repository keeps the probe, hardware-free parser tests, and the
private controller template. A local Fleet poller may dispatch changed
capture-path commits after the probe exists on `main`. The controller has
read-only repository permission and no cross-repository publish credential.

**Rationale:** Hosted CI can compile the capture boundary but cannot prove that
either native backend reaches a real microphone callback. A separate private
runner supplies that evidence without allowing public pull-request code to run
on George's Mac.

**Status:** active

**References:** PR #687 and issue #448, `docs/testing/capture-first-pcm.md`,
`scripts/smoke_test_capture_first_pcm.py`, and `infra/native-capture-ci/`

---

## 2026-09-07: Compare production versions only with explicit cohort evidence (#430)

Persist a versioned content-free production companion within each native
performance run. Capture observations keep the canonical recording owner and
use the existing database retention and clear epoch. Older runs remain readable
but cannot establish compatible cohorts. Compare exact known machine/OS,
runtime, microphone, configuration and size classes; retain missing values and
show incompatible or insufficient evidence instead of manufacturing a verdict.
Custom-content configurations have no diagnostic fingerprint. Keep failure,
fallback and worker-invariant observations visible independently of latency
sample thresholds. The production comparison is local and separate from both
imported benchmark reports and the remote capture dashboard.


Running log of architectural, scope, and process decisions for this project. Newest entries at the top. Each entry is short — for deep rationale on a single locked decision, write an ADR alongside in `docs/decisions/YYYY-MM-DD-*.md` and reference it here.

Maintained via the `/decisions` skill. See `~/.claude/skills/decisions/SKILL.md` for the entry format and invocation rules.

---

## 2026-08-18: The capture fast-fail primary budget gets a deeper second tier

**Decision:** Keep the 2026-08-04 fast-fail tier as-is and add a second one:
after four consecutive recordings of "primary failed before first PCM, fallback
delivered it within 1 second", the primary attempt budget shrinks again from 2
seconds to 750ms. A primary success or a slow rescue still resets the counter
and restores full budgets from either tier. Nothing else changes — both
backends stay in the sequence, budgets only ever shrink, termination and
fallback rules are untouched, and the memo remains session-only.

**Rationale:** On an affected machine (macOS 26.6 / M5) the first tier leaves a
steady state of roughly 2.45s of dead time per recording — a 2s sacrificial
primary, ~250ms of confirmed termination, and a ~200ms rescue. Field telemetry
shows the rescue landing at 160–450ms, well inside the 1s detector threshold,
so the sacrifice can shrink further once the pattern is proven more deeply. Four
consecutive qualifying recordings, rather than two, is what makes the smaller
budget safe: a slow-but-working primary cannot be cut off by one fluke, and
750ms still clears the ~300ms a healthy start takes on known-good machines.
Expected effect is ~2.45s → ~1.2s per recording.

**Status:** active

**References:** extends the 2026-08-04 fast-fail decision; `app/src-tauri/src/audio.rs`

---

## 2026-08-17: Community themes are data-only and compile into the local appearance cache

**Decision:** Keep the parser-blocking `murmur-appearance` document as the
small, authoritative active light/dark cache and add a separate revisioned,
main-window-only durable theme library. Each saved entry has a stable ID,
variant coverage, and local or Open VSX provenance; the active document records
an owner for each appearance and embeds the resolved tokens. Manual edits
detach to Custom, active removals fall back to Sonic, and confirmed extension
updates atomically replace the expected collection.

Open VSX access is opt-in from a disclosure-bearing dialog. Search and package
requests go directly to `open-vsx.org` without credentials. Only supported
open-source licenses are listed. Add verifies response bounds, SHA-256, ZIP
structure/expansion/path constraints, packaged identity/version/license, and
bounded JSONC includes. Murmur converts an allowlist of workbench colors
through its existing accessibility resolver and never loads extension code,
commands, fonts, icons, grammars, binaries, or dependencies.

**Rationale:** Reusing editor themes is useful only if it does not compromise
flash-free first paint, window-specific glass invariants, accessibility, or the
privacy-first no-background-network contract. A data-only converter and a
separate library preserve those boundaries while allowing independent
light/dark selection, local imports, and explicit community discovery.

**Status:** active

**References:** issue #592; `docs/features/appearance.md`,
`app/src/lib/appearance/themeLibrary.ts`,
`app/src/lib/appearance/openVsxThemes.ts`

---

## 2026-08-16: Shipped diagnostic history uses a gated SQLite query projection

**Decision:** Keep each production install's append-only, already-sanitized
JSONL as the recovery/export source and add a stdlib-only SQLite projection for
historical dashboard reads. Canonical install-scoped SHA-256 hashes make the
at-least-once shipper idempotent. The receiver validates a whole batch, inserts
new hashes in one immediate transaction, appends and fsyncs only those new raw
lines, then commits before acknowledging. Request-scoped connections use WAL,
foreign keys, bounded busy/cache/journal settings, explicit quotas, and
integrity checks. No retention job auto-deletes history.

A bounded streaming backfill persists source byte checkpoints with each
transaction. Dashboard reads remain on JSONL until a content-free
reconciliation proves per-install unique counts and timestamp bounds and marks
the database ready. Historical search then uses bounded filters, FTS5 summary
terms, escaped output, and signed filter-bound keyset cursors; raw and LLM-ready
exports continue to use JSONL. Development batches remain acknowledged and
discarded. Only the current aggregate microphone-state contract is accepted or
rendered.

**Rationale:** JSONL is simple and recoverable but full rescans do not support
bounded arbitrary-date investigation. Treating SQLite as a rebuildable,
explicitly gated projection preserves the established privacy/export boundary
and provides indexed history without allowing a database failure or lost HTTP
response to advance the client past uncommitted data.

**Status:** active

**References:** issue #434; `infra/log-receiver/event_store.py`,
`murmur-logs-receiver.py`, `docs/features/log-shipping.md`

---

## 2026-08-14: Pinned microphone re-resolution is bounded and never substitutes

**Decision:** When both AUHAL and CPAL fail `device_unavailable` before first
PCM for an explicit pinned microphone, allow up to two additional complete
primary-to-fallback passes in the session's immutable memo-resolved order. Wait
500 ms between passes with Stop/command-channel-disconnect-aware
cancellation, and pass the same immutable device ID to every helper. Never
substitute the system default. System-default capture, mixed failure kinds,
retained audio, or any other terminal result do not retry. Exhaustion emits the
typed terminal failure; the overlay shows a five-second mic-off status labelled
“Selected microphone unavailable. Open Settings to choose another.” without
expanding, focusing, or changing native geometry. Other failures retain the
generic cue.

Production protocol v6 adds one helper-live `InputResolution` record before
each backend opens the microphone. It carries only backend, enumeration
success, knowable pinned-input presence, an input count capped at 256 with its
cap flag, and default-input availability. The host validates the cross-field
contract. The telemetry projection has an exact bounded allowlist, strips
unknowns, device identity, raw errors, paths, and content, and reduces
contradictory evidence to the stable event code in debug and release builds.

**Rationale:** A temporarily detached USB input can reappear during Core Audio
re-enumeration, but an indefinite retry or default substitution would violate
the user's explicit microphone choice. Attempt-scoped, content-free evidence
makes the recovery result diagnosable without exposing local device identity.

**Status:** active

**References:** issue #542; `audio.rs`, `audio_lifecycle.rs`,
`crates/capture-helper-protocol`, `telemetry.rs`, `useOverlayRuntime`

---

## 2026-08-14: Microphone inventory uses an idle-only shared cache with bounded refresh

**Decision:** Replace helper-per-consumer microphone enumeration with one
Rust-owned, schema-versioned inventory. A supervised signed worker installs
passive Core Audio topology/default-input listeners and sends content-free
invalidation signals; it never enumerates or opens a device. Refresh once at
startup and every five minutes as the bounded fallback if a listener signal is
missed or the watcher exhausts its restart budget. Concurrent requests
coalesce; a signal or tick during any capture-owned phase remains pending until
the lifecycle supervisor has joined the capture worker and published Idle.
Settings focus is a cache read, never a refresh trigger. Production protocol v6
carries the actual default input's optional stable ID in the bounded `Devices`
reply and adds the watcher controls.

**Rationale:** macOS exposes the authoritative topology/default signal through
Core Audio, but the app-process HAL isolation boundary forbids a listener
there. The app therefore owns one passive listener worker instead of launching
a new enumeration worker on every focus or consumer read. Enumeration remains
short-lived, coalesced, and idle-only; the bounded backend timer supplies a
deterministic fallback. Keeping the last successful topology explicitly stale
gives the UI continuity without allowing old data to prove that a pinned
microphone is present. The shipper derives only aggregate availability and
counts, so labels and UIDs remain local.

**Status:** active

**References:** issue #534; `audio_inventory.rs`, `audio_lifecycle.rs`,
`crates/capture-helper-protocol`, `useAudioInputInventory`

---

## 2026-08-14: Overlay calibration previews native movement and commits explicitly

**Decision:** Replace the overlay-resident pointer drag band with Settings
controls that move the native window one logical point at a time within ±12pt.
Cancel restores the session baseline; Preview default is transient, while Save
and the inactive Reset action write `overlayVerticalOffset` into the durable Settings document. Settings schema v2
resets every pre-v2 offset once because the old flow's stored values cannot be
distinguished from its broken +48pt pointer-up state. Startup and applied
positions log both target and actual physical coordinates.

**Rationale:** The former preview transformed DOM content rather than the native
window and persisted automatically on pointer release. A stale maximum offset
therefore reopened Murmur far below the MacBook notch, while `show_overlay`
could also jump a live preview back to default. Native preview plus an explicit
commit boundary makes the behavior visible, reversible, and testable.

**Status:** active; supersedes the disabled overlay position save/restore note
in the 2025-12 overlay architecture decision

**References:** issue #580, `OverlayCalibrationControl.tsx`,
`commands/overlay.rs`, `docs/features/overlay.md`

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

**Status:** superseded by [Rust-owned state and WebKit lifecycle](2026-11-06-rust-owned-webkit-lifecycle.md).

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
active deadlines but bounded by a separate 120-second watchdog. Protocol v6
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
signed managed capture worker over binary protocol v6. The worker exposes CPAL
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

## 2026-07-20: Overlay geometry & lifecycle contract locked (#280)

**Decision:** Five locked outcomes of the overlay architecture review (issue #280; PRs #290, #299, #301):
1. **Rust is sole author of overlay geometry.** All dimensions derive from `geometry_for()` in `commands/overlay.rs` returning `OverlayGeometry`; the frontend consumes it at runtime (`get_overlay_geometry`, `overlay-geometry-changed`) and contains no geometry pixel constants. Motion timing (ms/easing) is frontend-owned in `lib/overlayMotion.ts`, with the shrink delay derived from the height-transition token — never free-standing.
2. **Contract enforced by a shared checked-in fixture** (`app/src/components/overlay/overlay-geometry.fixture.json`) asserted from both `cargo test` and vitest. No codegen.
3. **Hover expansion is one serialized 4-phase controller** (`useOverlayExpansion`: collapsed/opening/open/closing) with grow-then-reveal / hide-then-shrink ordering, applied-frame acks from `set_overlay_expanded`, and a generation-guarded writer owning every surface resize. No hook may own half of this lifecycle.
4. **Contract + controller + split land before any visual rehaul** (PR4); the rehaul may not touch geometry derivation except via `geometry_for()`.
5. **Cross-window settings stay localStorage + `settings-changed` events**, wrapped in per-window hooks; all overlay settings access goes through `loadSettings()` — no Rust settings store.

**Rationale:** TS and Rust were independent authors of overlay geometry (divergent no-notch fallbacks 185/140/200, hand-mirrored 44px drop height) and the expand choreography was un-acknowledged (CSS could animate into a window that had not grown). Two independent architecture reviews converged on the diagnosis; runtime Rust-owned geometry beats a shared-constants file because it shares the *derivation*, not just values, making formula drift structurally impossible rather than test-guarded.

**Status:** items 1–4 remain active; item 5 is superseded by [Rust-owned state and WebKit lifecycle](2026-11-06-rust-owned-webkit-lifecycle.md).

**References:** issue #280 (review memo + drift note), PR #290 (PR1 geometry contract), PR #299 (replacement PR2 expansion controller), PR #301 (replacement PR3 component split), `docs/features/overlay.md`.

---
