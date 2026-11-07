# Release process

This branch builds Murmur with GPUI and Rust. The removed frontend and Tauri
bundler are not release inputs. Application metadata lives in
`app/macos/bundle.json`; `scripts/release_version.py` synchronizes the native UI
and services package versions, `app/Cargo.lock`, and `CHANGELOG.md`.

## Current Phase 4 boundary

The GPUI stack remains a draft. Do not publish the removal release from this
implementation task. Signed 0.62 → this release → next-native continuity, TCC
carryover, installed failure recovery and direct skipped-release migration
require the manual signed acceptance rehearsal on the actual final bundle.
No such rehearsal or release dispatch is claimed by local unsigned previews.

`release-build.yml` accepts only `bundle_target=gpui`, on an explicit manual
trusted-main dispatch. It does not run a publishing build on a version-bump push.
The old Tauri job is removed. `release-start.yml` and the promotion resolver
remain historical publication machinery; do not use the Release Murmur button
for this stack until the signed acceptance and native promotion handoff are
completed. Existing signing, publication, OTA and authorization gates remain.
The draft PR does not authorize changing them or dispatching these workflows.

## Unsigned native bundle

On Apple Silicon macOS, with Rust 1.98.1 and the Xcode/Metal toolchain:

```sh
export RUSTUP_TOOLCHAIN=1.98.1
python3 scripts/build_capture_helpers.py --release
cargo build --locked --manifest-path app/gpui/Cargo.toml --release --features production-host --bin ui
python3 scripts/bundle_gpui.py --binary app/target/release/ui --output /tmp/Murmur.app
```

Set `CARGO_TARGET_DIR` to choose another output root. Keep `ui` as the main
executable, the production bundle identifier, all permission descriptions, the
three helper names and identifiers, capture-agent LaunchAgent location, and
signing entitlements. The bundle enforces macOS 14, FoundationModels weak
linkage, the Swift runtime rpath and the executable allowlist. Never launch a
production-identity unsigned bundle against a real profile for QA.

`scripts/rehearse_gpui_bundle.py` assembles and ad-hoc signs a native bundle,
checks the Mach-O contracts and executable allowlist, and runs a disposable copy
with a synthetic identity/HOME. It no longer invokes a Node or Tauri oracle.
The frozen `scripts/gpui-identity-policy.json` remains the independent identity,
permission-description and entitlement contract captured before the host flip.

## Trusted signed rehearsal

Only after explicit authorization and the whole stack landing on main, manually
dispatch Release Build from main with `bundle_target=gpui`. No signing keys are
needed on the Mac mini. Separate fresh hosted jobs compile app/helpers and the
updater signer with no credentials. Their immutable, hash-checked allowlisted
archives are the only binary inputs admitted to the signing job. The signing
job does no compilation. It uses trusted scripts from the exact approved main
commit, notarizes/staples, and uploads a nonpublishing `gpui-rehearsal-*` artifact.
Whole-workflow and individual-job pins remain checked by policy mutation tests.

The updater signer preserves the existing minisign key and installed-client
signature format. Use `--native-signer`; the Node signer path is removed.
`min_version` stays null and the last signed Tauri DMG must remain downloadable.
Do not delete old public assets as part of the host removal.

## Acceptance before publishing

Use installed signed test identities and synthetic profiles for stepwise legacy
upgrades and direct 0.62 → final-native updates. Verify all migration destinations,
Clear History/retention, source appearance, successful empty-record fetch and the
absence of the legacy WebKit directory. Relaunch after the retired marker and
verify no app-owned WebKit process. Rehearse granted/denied permission carryover,
updater recovery and quit/relaunch drains without resetting TCC.

After Phase 4, downgrades to Tauri are unsupported. State this in the removal
release notes and on the old DMG download page while retaining that DMG. Signed
acceptance is a manual gate, not implied by green CI. The plan's final memory
criterion is pending an explicitly requested measurement run; ordinary build,
test and review work does not run it.

## CI and promotion

The required check remains `ci-pass`, aggregating native Rust, service, importer,
GPUI interaction/AX and offscreen Metal checks. The former frontend job names
run real native replacements. A skipped or failed replacement cannot satisfy
the sentinel. No branch protection is changed.

The existing release promotion policy still requires exact trusted source/run
identity, immutable provenance, synchronized versions, safe tag reuse and OTA
completion; manual rehearsal artifacts cannot masquerade as publishing artifacts.
Native publication remains a separate authorized handoff after signed acceptance.
