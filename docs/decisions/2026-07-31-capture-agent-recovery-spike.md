# Capture recovery spike uses a per-user agent plus a killable worker

- Status: Provisional — signed/notarized runtime matrix required
- Date: 2026-07-31
- Issue: #407
- Parent: #405

## Context

The signed direct-child experiment proved deterministic helper termination and
a single Murmur microphone identity. It also exposed a platform boundary:
macOS 26.0.1 terminated and restarted the LaunchServices-owned main application
when Murmur's microphone permission was disabled during active capture. The
direct child then exited, and PCM already delivered into the main process could
not survive. That shape cannot satisfy interrupted-recording recovery.

An embedded XPC service is also tied to its containing application's lifetime.
The alternate spike therefore needs a per-user process whose lifetime is
independent of one main-app process instance, while preserving the separately
killable HAL worker.

## Provisional shape

The release bundle now contains a probe-only `murmur-capture-agent` and an
`SMAppService` LaunchAgent plist under `Contents/Library/LaunchAgents`.

- The per-user agent owns a versioned XPC Mach service and survives client
  disconnects.
- Capture code remains outside the agent. The earlier
  `murmur-capture-helper` keeps its direct-child entitlement policy for the
  original Phase-0 probe. The agent launches a separately signed
  `murmur-capture-worker` build of that code with exactly App Sandbox plus
  sandbox inheritance. Apple requires a `Process` child of a sandboxed parent
  to inherit its parent's static capabilities, so the agent carries the
  audio-input capability while a static gate forbids capture APIs in the agent
  source. The signed TCC matrix must decide whether that inherited-capability
  shape still produces one understandable Murmur identity.
- The agent starts that worker with an empty environment, root working
  directory, bounded framed stdin/stdout, and no persistent storage. The worker
  establishes and proves its own process group before it reads the first frame
  or touches CoreAudio, avoiding the post-`exec` `setpgid` race.
- A client connection is the capture lease. Client invalidation starts
  cooperative cancellation immediately; after 250 ms the exact worker is
  group-killed. A result is not accepted until the worker has exited, its
  reader has drained, and the owned process group is confirmed empty. A fixed
  signed synthetic fault mode ignores cancel so the matrix exercises and
  proves the `SIGKILL` path.
- The deterministic synthetic probe retains a fixed 64-sequence public fixture,
  its public fixture digest, and callback-derived counts for a 30-second
  in-memory recovery window. It does not retain PCM, device names, transcripts,
  paths, or audio-derived content.
- Recovery is offered with a peer-bound claim ID and remains re-offerable until
  that client explicitly acknowledges it or the monotonic TTL expires. ACK is
  idempotent only for the original peer and claim, so a lost ACK response can
  be retried; the signed matrix deliberately discards the first successful ACK
  response and verifies a full-payload replay on the same connection. Anonymous
  later recovery requests receive a no-content
  `already_acked` tombstone; after the TTL they receive an `expired` tombstone.
- XPC peers must satisfy an exact fixed-identifier, Apple-anchor, same-Team code
  signing requirement. The Team ID is derived from the running agent's own
  signed code, not compiled as a release constant. Requirement installation
  uses the macOS 12 API, checks its return code, and fails closed. Reply
  dictionaries are checked on the wire for exact key count and XPC value types
  before Rust applies its outcome-specific schema and cross-field invariants.
- Registration, status, and unregistration run in the containing main app,
  because `SMAppService.agent(plistName:)` resolves the calling app's
  `Contents/Library/LaunchAgents` directory. The agent cannot register itself.
  Probe and recovery clients still connect to the agent over XPC. All spike
  operations are reachable only through explicit CLI arguments, and production
  dictation remains unchanged.
- On macOS 26.0.1, a valid bundled service can report
  `SMAppServiceStatus.notFound` before its first registration even though
  `registerAndReturnError:` immediately succeeds and transitions it to
  `enabled`. The explicit `status` command preserves that pre-registration
  result as a content-free error; only an explicit `register` operation may
  attempt registration from `notFound`, and its returned error/status remains
  authoritative. The evidence matrix accepts either this exact macOS 26 pair
  (`notFound`, exit 2) or the older (`notRegistered`, exit 0) pair.
- launchd supplies the plist's relative `BundleProgram` as `argv[0]`. The agent
  therefore resolves its own absolute signed executable URL through
  Security.framework (`SecCodeCopyPath`) before locating its worker sibling;
  it never derives a security-sensitive path from `argv[0]` or a working
  directory.
- The standalone sandboxed agent and worker Mach-O files each embed their exact
  role-specific Info.plist identity and app version; macOS refuses to
  initialize a sandboxed standalone executable without coherent embedded code
  identity.

The release finalizer installs the exact launchd plist, verifies the agent,
direct helper, and worker embedded versioned Info.plists, signs the agent and
inherited worker with fixed identifiers and separate entitlements, verifies
every helper and the main app with hardened runtime, and records their hashes,
designated requirements, Team IDs, architectures, and entitlement digests in
release provenance.

## Acceptance gate

This is packaging and lifecycle instrumentation, not an accepted production
architecture. A downloaded, notarized, quarantined build must prove all of the
following through LaunchServices on the release-gate Mac:

- registration yields one understandable Murmur Background Activity item and
  no additional microphone identity or prompt;
- the exact agent PID and opaque instance fingerprint survive main-app
  termination/restart;
- denied preflight starts no worker;
- active permission revocation stops the exact worker within the bound while
  the agent survives and exposes one recoverable interruption;
- client loss stops a blocked worker and permits one recovery claim, with a
  second claim returning no content and the post-TTL query returning an
  explicit expired tombstone;
- agent and worker are both replaced exactly once across a signed app update;
- unregistration removes the launchd job and every agent/worker process.

[`scripts/capture_agent_matrix.py`](../../scripts/capture_agent_matrix.py)
validates the final cross-record artifact. It binds the installed agent and
worker hashes and Team/identifier facts to the trusted release provenance,
requires quarantine, notarization, stapling, Gatekeeper, and LaunchServices
observations, proves the main PID changed during the System Settings
granted → denied → granted transition, requires exactly one Murmur Background
Activity and microphone identity with no additional prompt, binds both sides
of the signed update to distinct trusted workflow provenance, distinguishes
service refresh from that update, and rejects any residual agent or worker
after unregistration. Its runtime records additionally cross-check the exact
agent instance, generation, worker PID, denied-preflight generation, synthetic
sequence, termination signal, process-group proof, same-peer ACK replay,
duplicate anonymous claim, and TTL expiry.

Apple may leave a disabled Background Activity row visible after
unregistration until system maintenance runs. Validation must record that
behavior and must not use the broad `sfltool resetbtm` command.

If the agent receives a second microphone identity/prompt, does not survive the
main-app restart, cannot spawn the sandboxed worker, or cannot unregister
cleanly, this architecture remains blocked. No production routing work in #408
or #409 may treat this provisional spike as passed.

## Consequences

The process topology is now:

`main app → per-user capture agent → killable capture worker`

The agent is the short-lived volatile recovery owner; the worker is the HAL
fault boundary. A later production protocol must replace synthetic canaries
with bounded in-memory PCM ownership, exact-once transcription dispatch, memory
zeroization, and the full privacy contract owned by #408 and #411.

References: [direct-child ADR](2026-07-30-capture-helper-phase-0.md),
[Apple SMAppService](https://developer.apple.com/documentation/servicemanagement/smappservice),
[Apple helper update guidance](https://developer.apple.com/documentation/servicemanagement/updating-helper-executables-from-earlier-versions-of-macos).
