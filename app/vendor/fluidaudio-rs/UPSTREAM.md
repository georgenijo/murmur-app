# FluidAudio Rust bridge patch

Vendored from FluidInference/fluidaudio-rs revision
`2d1083314104c812944b5150866d1e334db8eed7` (0.14.1), licensed MIT
as recorded in the upstream Cargo manifest. Swift FluidAudio remains pinned
to 0.14.1 / `d302273d49ef4d8914b27f20d342be482e8810f1`.

Murmur adds `init_asr_from_cache`, a load-only path using Core ML's direct
compiled-model initializer and the SDK's public `AsrModels` constructor.
It deliberately avoids every FluidAudio downloading/recovery loader, because
those delete the repository cache on a load error. Runtime inference uses this
path; explicit model installation keeps the original downloading initializer.
No shared Cargo checkout is patched. Upstream Rust/FFI/Swift files are otherwise
retained; upstream examples and tests are omitted from this dependency snapshot.
