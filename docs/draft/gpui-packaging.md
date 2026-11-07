# GPUI packaged production entry (layer 24)

Layer 18's bundle/artifact contract is retained: `Contents/MacOS/ui`, the three
capture sidecars, shared Info.plist/version sources, and the existing updater
archive layout. Layer 24 removes the obsolete packaging shell and builds the
production `src/main.rs` as `ui` with `--features production-host`. No release
workflow or signing/publication policy is changed.

Build with Rust 1.98.1 using the checkout's paired `CARGO_TARGET_DIR`:

```sh
cd app/gpui
cargo build --locked --features production-host --bin ui
```

`--version` is a storage-free metadata query. Normal launch runs the integrated
startup, services and production sections from layers 19–23. Bundle identity
selects data paths and clipboard isolation before startup; a test bundle must
use a non-production identifier. Do not run a production-identity rehearsal
bundle against real user storage. `rehearse_gpui_bundle.py` uses a separate
disposable identity/HOME for its launch proof after the parity comparison.

The release build script enables `production-host` in the credential-free build
phase. The signing phase still consumes only verified prebuilt inputs. Release
publication stays disabled for GPUI; this layer is a draft stack layer.

Native acceptance and constraints are recorded in
[gpui-packaged-production-qa.md](gpui-packaged-production-qa.md).
