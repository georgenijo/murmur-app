# Capture-helper Phase 0: managed child proven; external-binary shape blocked

- Status: Provisional — alternate packaging/recovery decision required
- Date: 2026-07-31
- Issue: #407
- Parent: #405

## Context

Murmur's in-process CPAL worker can become stuck inside CoreAudio. Moving capture
across a process boundary is useful only if the parent can always terminate the
exact owned process and if macOS attributes microphone consent to one durable,
understandable Murmur identity across install and update.

Apple describes TCC/MAC decisions in terms of the responsible code chain. A
direct helper launched by an app is expected to preserve that linkage, while
daemonizing can break it. Apple also documents that a sandboxed helper may have
additional capabilities and that microphone capture requires the audio-input
capability plus user consent. Those are platform hypotheses, not proof of
Murmur's observed behavior. See [Apple DTS on responsible code][responsible-code]
and [App Sandbox device access][sandbox-audio].

[responsible-code]: https://developer.apple.com/forums/thread/678819
[sandbox-audio]: https://developer.apple.com/documentation/security/app-sandbox

## Decision

The Phase-0 prototype is a direct, non-daemonized
`murmur-capture-helper` child inside the application bundle:

- Tauri packages it beside the main executable and local-LLM helper.
- Its capture dependency is pinned to the same exact CPAL 0.18.1 baseline as
  the stabilized in-process audio path.
- Release finalization signs it as
  `com.localdictation.capture-helper`, with hardened runtime, the same Team ID
  as the app, App Sandbox, and only microphone device capabilities.
- Before release spawn, Security.framework checks the exact bundled path,
  fixed identifier, Apple Developer ID anchor, matching Team ID, strict code
  validity, and hardened-runtime flag.
- The parent clears the environment, fixes the working directory, closes
  inherited descriptors, creates a new process group without daemonizing, and
  performs a version/nonce handshake over bounded framed stdin/stdout.
- Cancellation is cooperative for 250 ms, then `SIGKILL` targets the process
  group derived from the exact owned PID. The parent does not permit another
  spawn until the direct PID has exited and the owned process group is empty.
- Process-group cleanup covers descendants that inherit the helper's group; it
  does not claim control over a process that deliberately escapes with
  `setsid`/`setpgid`. The runtime signature gate makes the helper a trusted
  binary, its exact sandbox entitlements expose no process exception, and a
  static regression test forbids process-spawn or daemonization APIs in its
  source.
- The real-time callback performs one atomic counter update only. It
  never allocates, blocks, locks, logs, writes, retains PCM, or sends samples
  over IPC.

The prototype is reachable only through the explicit
`--capture-helper-probe` executable argument. Production dictation,
transcription, device selection, and transform capture remain on the existing
in-process path. Issue #409 owns production routing.

The default probe observes the active callback phase for five seconds. Its
separate five-second handshake bound must first accept the complete
Enumeration → Stream Open → Ready → Awaiting First Callback → First Callback →
Active sequence, so launch latency never shortens the requested observation.
Interactive revocation testing can use
`--capture-helper-probe --observe-seconds <1..300>`; parsing is exact and the
upper bound is enforced before the helper starts.

macOS 26.0.1 does not necessarily stop an already-open CoreAudio stream when
the user revokes microphone permission. A 120-second signed-build baseline
continued receiving callbacks after the System Settings toggle changed from
granted to denied, then stopped cooperatively at the requested observation
deadline. A content-free signed CLI probe launched from the test harness also
reached callbacks while the Murmur identity was denied, showing that the
platform stream result alone cannot enforce the app's TCC policy in every
launch context.

The parent snapshots permission before helper spawn and requires a provable
grant; denied, not-determined, and unknown states create no process. Once the
helper becomes active, it polls the same AVFoundation authorization status used
by Murmur's permission UI. Any observed loss of the proven grant is classified
as the stable `permission_denied` outcome and runs the existing bounded
cooperative-cancel/hard-kill teardown. A queued helper failure or protocol
frame during teardown cannot overwrite that primary outcome.

That active poll is defense in depth, not the macOS revocation contract. The
merged, signed/notarized LaunchServices run showed that macOS 26.0.1 terminates
and restarts the real Murmur process when the user disables Murmur while capture
is active. The direct capture helper subsequently exited and no helper survived.
The probe could not serialize a terminal result because the parent was
terminated first. The exact mechanism of the helper exit was not established;
the helper may simply have observed its parent pipe closing.

This makes the current external-binary shape unsuitable for production capture.
Audio already delivered into the app's memory cannot survive the forced app
restart, and a content-free "capture was active" marker cannot satisfy #411's
requirement to preserve and transcribe that PCM. Issue #407 therefore owns the
alternate packaging/recovery investigation. Candidates must retain the existing
privacy boundary while providing a bounded, revocation-safe handoff across the
forced restart. The protocol work in #408 remains gated until that decision is
proven; #411 is not a prerequisite for #407.

TCC conclusions are valid only when the bundle is launched through
LaunchServices. Direct executable and `launchctl submit` contexts produced
different TCC/capture outcomes and are retained only as diagnostics.

The child-management and runtime signature-validation primitives are generic.
The local-LLM sidecar now uses the same signature gate; a later focused
refactor may move its bespoke kill loop onto `ManagedChild` after its inherited
model-fd contract is represented explicitly.

## Evidence and gate

Deterministic tests cover blocking before handshake, during enumeration,
during stream open, after first callback, while ignoring cancel, during
graceful stop, and with an inherited-process-group descendant. They also reject
wrong nonce/version, malformed, truncated, oversized, duplicated, regressed,
and out-of-order control frames. Every case must settle through confirmed
direct-PID exit and an empty process group within one second, followed by a
successful fresh spawn.

The trusted `Release Build` remains non-publishing when manually dispatched. It
now uploads:

- the signed/notarized DMG and updater archive in
  `macos-release-<source-sha>`;
- content-free signature and non-interactive probe evidence in
  `capture-helper-tcc-evidence-<source-sha>`.

Interactive TCC results must be collected from that downloaded, quarantined,
notarized bundle. CI cannot grant/deny dialogs or prove System Settings
responsible-process attribution. The matrix in
[`docs/evidence/407-capture-helper-phase-0.json`](../evidence/407-capture-helper-phase-0.json)
records expected and actual states without audio, transcripts, device names, or
paths.

Three non-publishing trusted-main releases have now exercised the installed
update boundary on Shawn's arm64 macOS 26.0.1 release-gate machine:

- workflow run `30588206915`, source `e8cf2607`, helper 0.1.0/build 1;
- workflow run `30590132800`, source `a8603c27`, helper 0.1.1/build 2.
- workflow run `30635340250`, merged source `ab6f7513`, app 0.23.9.

All three artifacts were Developer ID signed, notarized, stapled, accepted by
Gatekeeper after quarantine, and matched their uploaded helper hashes. With an
existing Murmur grant, the first helper started without a second prompt. The
same grant then survived a moved bundle, an exact helper `SIGKILL` followed by
a fresh helper process, and a same-path update whose signed helper SHA changed.
Two fresh System Settings snapshots showed one enabled `Murmur` microphone
identity. Every probe confirmed an empty owned process group and
`audio_content_retained=false`.

An authorized reset of the exact installed identity produced one native Murmur
prompt. Choosing **Don't Allow** produced a stable denial without starting the
helper. A second clean reset and **Allow** produced first-buffer readiness in
2,175 ms. A subsequent helper probe produced no second prompt. Fresh System
Settings snapshots taken during the grant and revocation checks each showed
one Murmur identity.

The merged artifact passed Gatekeeper, staple, signature, an authoritative
LaunchServices denied preflight, and LaunchServices control runs. The denied
preflight returned the stable `permission_denied` result in 70 ms, retained no
audio, and spawned no helper during a five-second 25 ms polling window. Its
five-second granted control retained the first callback at 112 ms and settled
cooperatively; a separate 120,225 ms control retained the first callback at 107
ms and also left an empty process group.

The authoritative LaunchServices revocation run began with the exact signed
parent and helper alive after four seconds. Disabling the visible Murmur entry
removed the TCC grant, terminated and restarted the parent, and left no helper.
The helper subsequently exited; this evidence does not distinguish a direct
system termination from an exit caused by the parent pipe closing. Because the
parent terminated before stdout could flush, there was no probe outcome. A new
normal Murmur app was observed in its microphone-denied onboarding screen.
Murmur's native reset/request flow then restored a user grant and the normal app
screen without starting a helper.

This differs from two deliberately non-authoritative diagnostics: a direct
shell launch failed closed before spawn, while a `launchctl submit`
launchd-submitted context continued after the Murmur toggle changed. The
different outcomes are consistent with different responsible-process chains,
but the diagnostic runs did not prove that attribution mechanism. They do prove
that only the LaunchServices app context is acceptable TCC evidence.

Until alternate packaging or a privacy-preserving recovery handoff survives the
forced app restart, the runtime-revocation row is blocked and production capture
must not move into the helper. Ad-hoc, unsigned, direct-shell, or
launchd-submitted behavior cannot satisfy that gate.

## Consequences

Hard-kill recovery and the signed helper prototype remain reusable evidence, but
the exact production packaging decision is not complete. #408 and dependent
issues must not begin until #407 proves an alternate shape that can preserve
already-delivered PCM across the observed forced restart without weakening
revocation or privacy guarantees. The follow-up provisional shape is a
per-user LaunchAgent recovery owner plus the existing separately killable
capture worker; see
[`2026-07-31-capture-agent-recovery-spike.md`](2026-07-31-capture-agent-recovery-spike.md).
A volatile recovery handoff is acceptable only if its lifetime, access control,
cleanup, and swap/crash behavior are explicitly bounded and tested. Silently
shipping the current external-binary shape or moving the problem into #411 is
not allowed.
