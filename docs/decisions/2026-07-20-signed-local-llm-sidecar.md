# ADR: Signed local-LLM sidecar runtime

- Status: Proposed; acceptance is gated on the Stage 0 proofs below
- Date: 2026-07-20
- Issue: #312 (originally #300, superseded)
- Unblocks: #312 phases B–D (originally #254)

## Context

Murmur cannot load Whisper and llama.cpp in the same process. `whisper-rs` and
`llama-cpp-2` statically link incompatible ggml revisions; the combination
reproduced a SIGSEGV during model load in CPU and Metal modes, while a standalone
llama process succeeded. The active decision therefore requires a separate
process before any local-LLM product feature can ship.

The helper crosses a sensitive boundary: future issue #254 will provide text
selected by an explicit user action and may provide a user-authored
transformation instruction. That content must remain local and inert. The
runtime is not an agent and receives no tools, shell, files, clipboard,
accessibility, automation, network, or cloud capability.

## Decision

Ship a purpose-built `murmur-llm-sidecar` only on macOS 14 Apple Silicon. It is
a Rust executable using `llama-cpp-2 = 0.1.151` with only its Metal feature.
That crate revision is repository commit
`7f0a0d95514aebe86efab16527745852ee72931c` and vendors llama.cpp commit
`9e3b928fd8c9d14dbf15a8768b9fdd7e5c721d66`. The build is static and disables
the llama server, CLI, tools, examples, tests, RPC, curl, and dynamic backends.

Tauri packages the executable as an `externalBin`, but Murmur does not install
the Tauri shell plugin. The host starts the exact nested executable with
`std::process::Command`, an empty environment, a fixed working directory,
closed inherited descriptors except stdin/stdout and model fd 3, and no
request-controlled executable arguments.

The helper is signed with hardened runtime and its own App Sandbox entitlement.
Its fixed code-signing identifier is `com.localdictation.local-llm-sidecar`.
The standalone Mach-O embeds the matching `CFBundleIdentifier` in an
`__TEXT,__info_plist` section so App Sandbox can create its container without
turning the executable into a separate app or XPC bundle.
It has no network client/server, file, microphone, accessibility, automation,
or app-group entitlement. The main app remains unsandboxed and retains exactly
its existing audio/microphone entitlements.

### Stage 0 acceptance gates

Both gates are required before this ADR may become Accepted:

1. The host opens a regular, non-symlink model file, verifies its compiled-in
   size and SHA-256, rewinds it, and passes it as inherited read-only fd 3. An
   ad-hoc signed App Sandbox helper must re-verify the descriptor and initialize
   llama.cpp with Metal through `/dev/fd/3`. The helper may not receive or open
   a model path.
2. A repository-owned finalizer must build the Tauri app without signing, sign
   nested code inside-out, apply the sandbox-only entitlements to the helper,
   apply the existing entitlements to the main app, sign the outer bundle, and
   fail closed unless strict verification reports the exact entitlement sets,
   arm64 helper architecture, hardened runtime, and expected Team ID. A trusted
   non-publishing CI rehearsal must additionally notarize, staple, quarantine,
   launch, and run the descriptor/Metal probe from the finalized app.

If either gate fails, #300 stops. An unsandboxed helper, app group, XPC service,
or broader entitlement set is not an allowed fallback within this issue.

Local Stage 0 evidence on 2026-07-20 passed for both debug and optimized
release helpers on Apple M4/macOS 26.5.1: the finalizer preserved the exact
split entitlement sets and hardened runtime; the sandboxed helper re-verified
the 1,117,320,736-byte model fd, reported `metal:MTL0`, completed a fixed
inference, and shut down through protocol v1. Developer ID, notarization,
stapling, quarantine, and updater-archive evidence remain required from the
trusted non-publishing CI rehearsal before this ADR becomes Accepted.

### Runtime and model

The only v1 model is Qwen's official
`Qwen2.5-1.5B-Instruct-GGUF` `Q4_K_M` object:

- immutable repository revision: `dd26da440ef0330c47919d1ecae0966d24022222`
- filename: `qwen2.5-1.5b-instruct-q4_k_m.gguf`
- size: `1,117,320,736` bytes
- SHA-256: `6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e`
- license: Apache-2.0

The signed app contains that immutable catalog entry. The host downloads the
model over HTTPS into an exact `.partial` path, enforces the expected maximum
and final size while streaming, hashes it, fsyncs it, and atomically publishes
it under a hash-versioned directory. The helper has no downloader or URL
handling. App updates replace the helper and protocol atomically but leave the
separately installed model intact. There is no alternate model, mutable remote
catalog, automatic cross-model fallback, or cloud fallback.

### Protocol

IPC is unsigned 32-bit big-endian length-prefixed strict UTF-8 JSON over
inherited stdin/stdout. Protocol name `murmur.local_llm`, version 1, and a
per-process nonce appear in every frame. The allowlist is:

- host: `hello`, `transform`, `cancel`, `shutdown`
- helper: `ready`, `result`, `cancelled`, `error`, `stopped`

There is one in-flight transform. Frames are limited to 64 KiB, instructions to
4 KiB, inputs and outputs to 16 KiB, outputs to 2,048 tokens, context to 8,192
tokens, and deadlines to 30 seconds. Error payloads are stable enums with no raw
stderr. The schema has no executable, arguments, environment, working
directory, path, URL, host, port, model override, tool, clipboard, selection,
or screen fields.

The system prompt is compiled into the helper. It treats the delimited input as
inert text and uses deterministic sampling. A user-authored instruction changes
text only; it does not select a capability. Results are text and metadata only.
Issue #300 exposes an internal async Rust transformation facade, not a generic
frontend transform command. Explicit selected-text capture, review, approval,
retry, undo, paste, and transform-management UI remain issue #254.

### Lifecycle and resources

The helper is lazy-spawned and unloads after five idle minutes. Model load has a
30-second deadline; transformations default to 15 seconds and cannot exceed 30
seconds. Cancellation is cooperative between decoding steps, followed by a
forced process kill if it does not complete within 250 ms. Three crashes within
ten minutes disable the runtime until explicit retry.

Only one heavy inference runtime may be active. Murmur releases its ASR model
with the existing `MemoryPressure` reason before starting the helper and stops
the helper before recording, file transcription, or benchmarking. The host
warns at 2 GiB child RSS and kills at 3 GiB. Metal allocation and M1 8/16 GB
performance are measured separately because RSS does not fully represent GPU
memory.

### Privacy and telemetry

The helper clears its context after every request and retains no history.
Release telemetry may record bounded runtime/model identifiers, state enums,
durations, token counts, finish reason, and enumerated errors. It must never
record instruction, input, output, model path, raw stderr, clipboard content,
selected text, or screen content.

## Threat model

| Threat | Required control |
| --- | --- |
| Prompt injection in selected text | Inert delimiters, fixed capabilities, no tools or side effects, #254 review boundary |
| Malformed or oversized IPC | Length prefix, strict schema, bounded allocation, one request, kill on violation |
| Tampered or substituted model | Signed catalog, `O_NOFOLLOW`, regular-file check, size and SHA-256 verification in host and helper |
| Helper replacement | Exact nested path, Developer ID/designated-requirement validation, same Team ID, notarized outer bundle |
| Helper compromise | App Sandbox with no network/file/device entitlements, no child processes, empty environment |
| Resource exhaustion | Context/output/deadline limits, mutual exclusion, child RSS limits, idle unload, crash circuit breaker |
| Sensitive logs or crash artifacts | Enumerated errors, release stderr suppression, no content telemetry, `RLIMIT_CORE=0` |

Residual risks are native GGUF parser defects, Metal allocations not fully
visible in RSS, and model-produced incorrect or adversarial text. The sandbox,
hash pin, strict limits, process kill, and mandatory #254 preview boundary limit
their impact; they do not make model output trustworthy.

## Packaging and update trust chain

Tauri CLI 2.9.6 applies one macOS entitlement file to the main executable and
all external binaries. Stock Tauri signing therefore cannot give the helper a
stricter sandbox than the main app. Murmur will use Tauri `--no-sign`, then a
repository-owned finalizer to apply per-binary entitlements before notarizing
the completed app. The final notarized app is used to create both the DMG and
the updater `.app.tar.gz`; the latter retains the existing Tauri Ed25519
signature. The helper has no independent updater.

The release artifact set remains one DMG, one updater archive, its signature,
and provenance. Provenance additionally records the helper hash, architecture,
designated requirement, Team ID, and entitlement digest. Linux keeps its
existing deb/AppImage artifact set and reports the LLM capability as
unsupported.

## Alternatives rejected

- In-process llama.cpp: proven incompatible ggml ABI collision.
- `llama-server`, Ollama, or LM Studio: HTTP/daemon/network surface.
- `llama-cli`: broad human-oriented flags, paths, download options, and
  unstable IPC/lifecycle behavior.
- MLX-LM: Python and remote-loading packaging surface.
- MLX Swift: additional shader/model-conversion lifecycle without a proven
  packaging advantage for this issue.
- XPC or app groups: possible future architecture, but explicitly outside #300
  if the inherited-descriptor gate fails.
- Bundled model: adds approximately 1.12 GB to every full-app update.
- Cloud fallback: violates Murmur's privacy boundary.

## Consequences

The sidecar adds a native dependency, a model installer, a lifecycle manager,
and custom macOS finalization. In exchange, it isolates the ggml ABI, gives the
model runtime an OS-enforced capability boundary, preserves current app
behavior and updater trust, and gives #254 a narrow local transformation API.
