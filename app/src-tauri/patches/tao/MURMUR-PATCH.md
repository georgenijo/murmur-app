# Murmur's tao 0.35.3 patch

Source: the published tao 0.35.3 crate. Retain LICENSE and LICENSE.spdx.
Pinned by the application manifest and workspace patch; one instance must appear
in cargo tree -i tao.

Changes are limited to macOS termination: main-thread once-only host handler,
typed applicationShouldTerminate callback on tao's existing delegate, and a
common-mode CF relay from TaoApp.terminate to the superclass outside the original
callback. No delegate replacement or swizzling. The crate adds the existing
objc2-core-foundation/block2 family for CF dispatch. Other platform source is
retained verbatim. Upgrades require rechecking modal termination dispatch.

The repository transport/restart policy excludes this exact third-party source
root. Maintained application adapters remain scanned. Upstream warnings are not
silenced by changing application Clippy policy.
