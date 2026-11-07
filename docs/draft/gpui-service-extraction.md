# Shared service owners for the native host (#996, layer 20)

The shipping `ui_lib` becomes a shared service library with two compile-time
host adapters. `tauri-host` remains the default and keeps the current IPC ABI,
managed owners, plugins, window transport and startup behavior. `native-host`
with default features disabled compiles the same recording, capture, model,
meeting, delivery, completion, source-write, hotkey and shutdown implementations
without Tauri, tao, wry or WebKit in the normal dependency tree. The GPUI
executable will consume its `native_services::Services` facade in layer 21.
No production route is switched by this layer.

The shared owners access concrete capabilities through `ServiceAccess` rather
than an erased app registry. The Tauri adapter forwards those capabilities to
its existing managed owners. The native context owns concrete fields and
initialization slots for recording state, controller, event router, completion,
source/window writers, updater and shutdown. It receives its paths, task runtime,
queued main-thread dispatcher and native presentation ports explicitly. There
is one owner of each service, not one per section/window.

All algorithm bodies and generation checks remain shared. The ordered
transcription pipeline, immutable start configuration, exact helper ownership,
secure-field refusal, final delivery, durable completion de-duplication and
shutdown receipts keep their existing implementations. Native requests use
these commands directly and retain the real sender identity for Main-only
operations. Binary retained-audio reads use a separate bounded byte API.

The native adapter reuses #992's driver and drain owners, including the original
+5/+7/+10-second deadlines, writer permits, pending publications, signal relay
and restart suppression. Its only different terminal effect is a typed native
port callback. Layer 21 connects that committed effect to the layer 19 AppKit
admission hook and GPUI quit. The layer 19 standalone reducer adapter remains
available for isolated tests; production does not start a second coordinator.

The clipboard facade calls the existing identity-aware primitives, so dev/QA
identities cannot reach the real pasteboard or post external input. Layer 21
adapts each presentation field intent to the existing source/configuration owners.

Legacy model/log/export roots are injected before worker dispatch, with the
same production path and injective non-production namespace. Startup rejects
an identity/data-root mismatch before opening a store. Path tests only compute
strings under `/synthetic-home`; no Application Support directory is opened.
The native Dock adapter reuses the current AX sampler/glide owner and publishes
its facts to GPUI. GPUI is the native overlay geometry owner; the legacy Tauri
placement/material observer does not run beside it.

## Checks

From `app/src-tauri` on macOS with Rust 1.98.1:

```sh
cargo check --lib --no-default-features --features native-host
cargo clippy --lib --no-default-features --features native-host -- -D warnings
cargo test --no-default-features --features native-host --test native_host_contract -- --test-threads=1
cargo clippy --all-targets -- -D warnings
cargo test --lib -- --test-threads=1
cargo test -p murmur-core -- --test-threads=1
cargo fmt --all -- --check
```

CI also rejects Tauri/tao/wry/WebKit in the native normal dependency tree. The
shipping adapter's existing tests stay enabled. The native contract executable
is windowless and performs calculation-only identity checks. No OS registration,
TCC request, clipboard operation, native input or benchmark is needed.

## Native acceptance handoff

In addition to [layer 19's chrome/lifecycle flows](gpui-native-host.md), the GUI
QA worker must exercise the final composition with synthetic fixtures:

- D09/D11 and P01/P04: real clipboard-only result and retry on the private test
  pasteboard; stale target/generation refusal; hidden Main; quit during capture
  and processing; one completion and activity receipt only.
- S04 and N02: native inventory/default/pinned/missing-input states; explicit
  permission requests and denial/return; no automatic permission prompt on view
  subscription. Change a device during recording and preserve its start snapshot.
- M09–M12 and P04: real synthetic meeting start/stop and pending recovery; text,
  captions and retained-audio export; explicit Calendar naming; quit during each
  capture/writer phase; exact helpers gone and correct durable rows/spools.
- U01/U03, K12/K13 and A02: attach/hide/reopen every production section; change a
  Mode or settings during an active recording; the current recording keeps its
  snapshot, the next one receives the accepted change, and source fields owned
  elsewhere survive.

This layer supplies shared service ownership. The final native adapters,
interactive acceptance and independent Sol review are separate gates; a passing
windowless test is not an app-flow acceptance claim.
