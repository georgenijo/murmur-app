# Release build and promotion

Murmur builds signed release artifacts once on trusted `main`, then automatically
promotes those exact artifacts after the version-bump build succeeds. Promotion
creates the matching version tag; neither automatic nor recovery tag runs compile
the application or save Cargo caches.

## Trust and cache policy

- `Release Build` runs automatically only for a `main` push whose commit starts
  with `chore: bump version`, or by an explicit `workflow_dispatch` rehearsal.
- Frontend validation and the macOS build/sign/notarization job run concurrently.
  A workflow run is successful only when both pass.
- Release-profile Cargo cache writes are authorized only for trusted `main`
  pushes or a manually dispatched cache-prime rehearsal. Pull requests restore
  default-branch caches but never save release-profile caches.
- No release workflow uses a self-hosted runner. In particular, pull-request
  code is never sent to a Mac Mini or other signing/release host.

## Immutable artifacts

Each successful build uploads these 30-day artifacts:

- `macos-release-<40-character-commit-sha>`
- `capture-helper-tcc-evidence-<40-character-commit-sha>` (allowlisted,
  content-free structured signature facts and strictly validated
  non-interactive probe evidence whose outcome, last phase, callback presence,
  termination kind, and exit status must form one valid probe contract; raw
  path-bearing `codesign` output is never uploaded; pair with the macOS artifact
  for the manual #407 TCC matrix)

Release binaries retain the Tauri bundle-type marker (Cargo release stripping
is disabled) so the updater can identify the packaged macOS application.
After final signing and notarization, the release job launches the exact packaged
capture worker, completes its production-v8 hello handshake, sends a bounded
AUHAL start request, and requires the worker's stream-open phase. This catches
sandbox or entitlement failures that static plist and signature checks cannot.

The macOS release artifact contains `provenance.json` with the exact commit SHA,
workflow run ID, updater names, sizes, and SHA-256 hashes. The separate TCC
evidence artifact contains only its allowlisted signature, entitlement, probe,
and manifest evidence; its validator enforces the cross-field probe contract
described above, and it carries no updater metadata. Promotion accepts exactly
one unexpired macOS artifact from a successful `Release Build` on `main` for the
exact source commit. Automatic promotion also requires a successful `push`
event, the version-bump commit prefix, and matching semver values in
`tauri.conf.json`, `Cargo.toml`, `Cargo.lock`, `package.json`, and
`package-lock.json`. The newest dated CHANGELOG section must match that same
version. Any tag, run, filename, version, changelog, hash, or updater-signature
mismatch fails before publication.

The modern updater manifest is generated from the downloaded `.sig` files.
After release-asset upload, the workflow downloads the remote `.sig` assets and
compares them byte-for-byte, uploads the manifests, downloads them again, and
checks that `latest-v2.json` contains those exact signatures before publishing.
It also reads the source-controlled `.github/updater-policy.json`: `null` keeps
the release optional, while a stable minimum version at or below the target
release is emitted as `min_version` on the modern channel. Invalid or future
minimum versions fail promotion.

After publication, the release is not done until the previous public build
passes the real OTA canary on the trusted Mac mini. There is one bootstrap
exception: the first public build containing canary support must be physically
installed once in the dedicated canary location because its previous public
client cannot write a canary result. Mandatory gating starts with the next
release. For all later releases, run the documented manual
gate (the mini is tailnet-only, so it is not run on a GitHub-hosted runner):

```bash
python3 scripts/murmur_canary_fleet.py --tag vX.Y.Z
```

The command must exit zero and report a complete result-file pass before the
release is announced. A failed or timed-out canary blocks release follow-up;
the runner terminates the canary process group before collecting bounded
stderr, and both the remote runner and Fleet wrapper must return nonzero. Fix
the updater and publish a patch release as needed. The app writes the documented
result schema through the nested Tauri command payload
`{ request: { action, result } }`; without `MURMUR_UPDATER_CANARY`, the command
returns an inert state and an ordinary user launch cannot enter canary mode.

Set that file before the version-bump commit:

```json
{ "min_version": null }
```

Use a quoted stable version such as `"0.24.0"` only when every installed
version below that threshold must update or quit. Never set `min_version` to
force an update until the canary from the currently shipped public version has
passed: a forced update combined with a broken checker is the worst failure
mode. Policy changes receive the same code review and trusted-main provenance
as the release itself.

Immediately before publication, the workflow requires the draft release body
to match the remotely downloaded updater manifest notes after applying the same
line-ending normalization and outer-whitespace trim to both. All remaining
content must match exactly; the check is not fuzzy. Once published,
updater assets are immutable; do not edit the release body independently. Ship
a patch release when corrected notes must appear both on GitHub and in Murmur.

## Non-publishing rehearsal

This is the supported way to measure a cold or warm build without creating a
tag or GitHub Release:

```bash
gh workflow run release-build.yml \
  --repo georgenijo/murmur-app \
  --ref main \
  -f prime_caches=true

# After the Release Build succeeds, use its exact head SHA and run ID.
gh workflow run release.yml \
  --repo georgenijo/murmur-app \
  --ref main \
  -f source_sha=<40-character-main-sha> \
  -f artifact_run_id=<release-build-run-id>
```

The second workflow downloads and validates the immutable artifacts but has no
manual input that can authorize publication. Its summary explicitly confirms
that no tag, draft, release asset, updater manifest, or published release was
created.

Run the build rehearsal once to prime caches and a second time to measure the
warm path. Record the `release-macos` and overall workflow durations, the Rust
cache summary, and repository cache usage. The release target is macOS and total
wall time <= 5 minutes.

## Cold fallback

If the automatic build for a version-bump commit fails, no tag or release is
created. Do not push a tag.
Correct the infrastructure problem and rerun the original workflow at the same
commit (`gh run rerun <run-id> --failed`). A rerun preserves the trusted push
event and exact source SHA. If `main` still points to the version-bump commit, a
manual `Release Build` dispatch is also supported; leave `prime_caches=false`
for a restore-only recovery build unless the cache itself is intentionally
being repaired.

If artifacts expired or `main` has advanced, rerun the original version-bump
workflow rather than building arbitrary PR or tag code with signing secrets.
Promotion remains blocked until a successful trusted push build and the
SHA-named macOS artifact exists for the version-bump commit.

## Release authorization and recovery

`prompts/PROMPT_RELEASE.md` requires explicit confirmation before pushing the
version-bump commit. `scripts/release_version.py prepare X.Y.Z` synchronizes the
five version surfaces and cuts `[Unreleased]` into the dated release section;
its `check` command enforces the same contract locally and during promotion.
That push is the release action: after its exact trusted build succeeds,
`Release` validates the run, synchronized versions, and CHANGELOG, downloads and
verifies the immutable artifacts, creates `vX.Y.Z`, prepares the release,
verifies remote updater integrity, and publishes.

Manual `Release Build` dispatches remain non-publishing rehearsals, even when
they succeed. The tag trigger remains an operator recovery path for an automatic
promotion failure; it applies the same commit, build, version, artifact, and
signature gates. Re-running promotion for an already-published tag at the same
commit exits successfully without replacing the release, while a tag that points
to a different commit fails closed.
