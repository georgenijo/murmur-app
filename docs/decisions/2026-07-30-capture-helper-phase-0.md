# Capture-helper Phase 0: managed child proven; TCC rollout still gated

- Status: Provisional
- Date: 2026-07-30
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
- The real-time callback performs one atomic counter update only. It
  never allocates, blocks, locks, logs, writes, retains PCM, or sends samples
  over IPC.

The prototype is reachable only through the explicit
`--capture-helper-probe` executable argument. Production dictation,
transcription, device selection, and transform capture remain on the existing
in-process path. Issue #409 owns production routing.

The child-management and runtime signature-validation primitives are generic.
The local-LLM sidecar now uses the same signature gate; a later focused
refactor may move its bespoke kill loop onto `ManagedChild` after its inherited
model-fd contract is represented explicitly.

## Evidence and gate

Deterministic tests cover blocking before handshake, during enumeration,
during stream open, after first callback, while ignoring cancel, during
graceful stop, and with a spawned descendant. Every case must settle through
confirmed direct-PID exit and an empty process group within one second, followed
by a successful fresh spawn.

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

Until every signed/notarized matrix row passes, this ADR remains Provisional and
production capture must not move into the helper. Ad-hoc or unsigned behavior
cannot satisfy that gate.

## Consequences

Hard-kill recovery and exact packaging have an implementation path that later
audio issues can reuse. TCC durability remains an explicit external release
gate rather than an inferred property. If the notarized matrix shows a second
permission identity, repeated prompts, or update instability, the fallback is a
dedicated bundled app/XPC packaging investigation; silently shipping the
external-binary shape is not allowed.
