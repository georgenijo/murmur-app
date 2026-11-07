# Vendored native UI dependencies

`gpui-component/`: GPUI Kit v0.9.0, tag commit
`7591a35eced761e3613c01e71ff04c10129dae1a`, from
<https://github.com/longbridge/gpui-kit>. Apache-2.0 code, copyright its
contributors. Four crates only: base, component, component-macros, assets.
Upstream license files are retained. Lucide icons retain their ISC license in
`crates/assets/LICENSE-LUCIDE`; upstream documentation retains LICENSE-DOCS.md.
Examples, benchmarks, story/shell/kit/webview applications and tooling were
not imported. The README is upstream attribution, not Murmur instructions.

The starting code is the **proven Phase 0 port**, read from the Mac mini's
`~/Builds/gpui-phase0/0b-settings-history/scratch/component-v0.9.0`.
Dependencies point to Zed's exact Apache-2.0 GPUI revision
`654efae441bc8eb117d0042bb28cd7cf3570bdc9`, including the matching reqwest fork.
The macro path resolver recognizes `gpui`. Workspace dependency tables live
in `app/gpui/Cargo.toml`; this is a single workspace, without nested inheritance.

`accesskit_macos/`: crates.io 0.26.3, MIT OR Apache-2.0, copyright the AccessKit
contributors. The source and both licenses are retained. The Phase 0 patch
adds the selected-text setter; its selector is exposed only when the node
supports ranges, is editable and advertises ReplaceSelectedText.

Patch history and maintenance instructions are in
`../../../../docs/draft/gpui-accessible-components.md`. No Zed GPL UI/editor
crate or GPL source has been copied. The GPUI revision remains frozen. Its Apache-2.0 crate is now also vendored
under `gpui/` so the recorded deferred-AX fix applies in ordinary production
builds through Cargo's git-source patch. All other Zed dependencies retain
the exact git revision. `gpui-manifest.patch` and
`gpui-deferred-accessibility.patch` reproduce this crate from the pin.
`gpui-hover-source.patch` then adds source-aware hover observations while
preserving the existing transition-only `on_hover` API. Delivered event positions
let the overlay distinguish pointer movement from resize-generated exits.
Only `src/`, `resources/`, the build script, README and Apache license are copied;
no Zed GPL crate, examples, benchmark or upstream test asset is included.

`component-unfocused-tab.patch` records the shared Root correction for unhandled
Tab entry from a window with no focused element; it preserves activation focus
and the existing focus-trap traversal policy.

`gpui-window-zoom.patch` records the native host's whole-window content zoom.
It scales layout/paint and AX consistently, maps platform pointer/scroll/drop
coordinates into layout space once, and converts IME bounds back to native
points. Native window geometry and default zoom remain unchanged. The patch
applies after the production host clipboard additions from layer 21.
