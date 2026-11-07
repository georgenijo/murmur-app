# Development

The native app requires Apple Silicon macOS 14+, Xcode and its Metal toolchain,
Rust 1.98.1, Python 3, CMake, Meson, Ninja and pkg-config. Node is not required.
`app/rust-toolchain.toml` pins Rust; from outside `app/`, set
`RUSTUP_TOOLCHAIN=1.98.1`. GPUI/GPUI platform/Apple crates share the recorded
Zed revision; do not update one independently.

## Commands

From the repository root:

```sh
export RUSTUP_TOOLCHAIN=1.98.1
python3 scripts/build_capture_helpers.py
cargo check --locked --manifest-path app/Cargo.toml -p murmur-ui --features production-host
cargo clippy --locked --manifest-path app/Cargo.toml -p murmur-ui -p murmur-services -p murmur-core -p murmur-webkit -p murmur-capture-helper-protocol --all-targets --features gallery,production-host -- -D warnings
cargo test --locked --manifest-path app/Cargo.toml -p murmur-services -p murmur-core -p murmur-webkit -p murmur-capture-helper-protocol -- --test-threads=1
python3 scripts/test_gpui.py --features production-host -- --test-threads=1
python3 scripts/check_gpui_goldens.py
cargo test --locked --manifest-path app/sidecars/capture/Cargo.toml -- --test-threads=1
```

Helpers stage into ignored `app/binaries`; `CARGO_TARGET_DIR` controls build
outputs, including a separate worker build underneath it. Run heavy local work
under `nice -n 15`. Core/protocol tests also run on Linux. Services require macOS.

## Native previews and app bundles

Synthetic gallery binaries never construct production service owners:

```sh
cargo run --manifest-path app/gpui/Cargo.toml --features gallery --bin settings-qa
cargo run --manifest-path app/gpui/Cargo.toml --features gallery --bin overlay-qa
cargo build --manifest-path app/gpui/Cargo.toml --features production-host --bin ui
python3 scripts/bundle_gpui.py --binary app/target/debug/ui --output /tmp/Murmur-preview.app
```

The bundle script uses production metadata for identity parity; **before launching
any test bundle**, give the disposable copy a unique QA identifier and launch
with a synthetic HOME and CFFIXED_USER_HOME. Ad-hoc sign the test copy after
editing its plist. Never point a test build at production data or the real
pasteboard. The identity-aware ports use a private board for nonproduction IDs.
Launch only the exact test bundle executable, retain its PID, and quit through
its native menu/coordinator. Do not send bundle-wide signals or Apple events to
an installed production app. See the [native acceptance contract](draft/gpui-native-host.md).

Interactive verification runs against the native app. GPUI test/AX fixtures and
offscreen Metal goldens supplement it. They do not prove native window behavior.
Leave VoiceOver enabled. Do not change system preferences as a test shortcut.

## Project structure

`app/gpui` owns views and host ports; `app/crates/murmur-services` owns native
services; `app/crates/murmur-core` owns reducers and durable protocols. Capture
helper/worker code lives in its own `app/sidecars/capture` workspace. See
[Architecture](ARCHITECTURE.md) for boundaries and [native requests](reference/commands.md)
for the in-process facade. `app/macos/bundle.json` owns application version and
bundle identity; `scripts/release_version.py` synchronizes released Rust packages,
lockfile and changelog. There are no Tauri configs or frontend build steps.

## Permissions and isolation

Production permission grants depend on the signed bundle and helper designated
requirements. Ad-hoc previews cannot prove TCC carryover. Test explicit permission
flows only with a synthetic identity. Never reset production TCC, modify the
installed app, register production LaunchAgents, or inspect real recordings,
transcripts or clipboard contents. macOS 14 compatibility, weak FoundationModels
linking and `/usr/lib/swift` rpath remain bundle contracts.

## Migration and recovery

The permanent importer remains available to users skipping directly from 0.62.
It imports raw bounded legacy values through the original origin, rather than
reading WebKit's undocumented files. Independently durable version, removal and
retirement receipts keep Clear History and fill-only migration safe on retry.
The host never spawns the importer after the retired marker. The standalone
`murmur-webkit` synthetic executable uses a nonpersistent custom store and temp
roots; it never obtains a production store.

```sh
cargo test --manifest-path app/Cargo.toml -p murmur-webkit --test synthetic_store
python3 scripts/check_native_only.py
python3 scripts/validate_reference_docs.py
```

## CI and merging

`ci-pass` remains the required sentinel. Former frontend check names now run
native Rust or GPUI checks, and fail if any replacement is skipped or fails.
See [Release](release.md) for unsigned bundles and the trusted manual signed
rehearsal. Do not dispatch a release or request signing credentials for ordinary
implementation verification. Synthetic flow tests do not establish signed
0.62 → native → next-native upgrade continuity or final memory-budget acceptance.

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
