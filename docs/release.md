# Release build and promotion

Murmur builds signed release artifacts once on trusted `main`, then promotes
those exact artifacts when a version tag is pushed. A tag does not compile the
application and cannot save Cargo or CUDA caches.

## Trust and cache policy

- `Release Build` runs automatically only for a `main` push whose commit starts
  with `chore: bump version`, or by an explicit `workflow_dispatch` on `main`.
- Frontend validation, macOS build/sign/notarization, and Linux CUDA packaging
  run concurrently. A workflow run is successful only when all three pass.
- Cargo and CUDA cache writes are authorized only for trusted `main` pushes or
  a manually dispatched cache-prime rehearsal. Pull requests restore default-
  branch caches but never save CUDA or release-profile Cargo caches.
- No release workflow uses a self-hosted runner. In particular, pull-request
  code is never sent to a Mac Mini or other signing/release host.
- CUDA caching contains only `/usr/local/cuda-12.8`. The transient
  `/usr/local/cuda` symlink and loader configuration are recreated after every
  restore. A claimed hit with a missing or wrong-version `nvcc` fails instead
  of silently reinstalling.

## Immutable artifacts

Each successful build uploads these 30-day artifacts:

- `macos-release-<40-character-commit-sha>`
- `linux-release-<40-character-commit-sha>`

Each artifact contains `provenance.json` with the exact commit SHA, workflow
run ID, platform/updater names, sizes, and SHA-256 hashes. Promotion accepts one
unexpired macOS artifact and one unexpired Linux artifact from a successful
`Release Build` on `main` for the exact tag commit. Any tag, run, filename,
hash, or updater-signature mismatch fails before a draft release is created.

The modern updater manifest is generated from the downloaded `.sig` files.
After release-asset upload, the workflow downloads the remote `.sig` assets and
compares them byte-for-byte, uploads the manifests, downloads them again, and
checks that `latest-v2.json` contains those exact signatures before publishing.

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
warm path. Record the `release-macos`, `release-linux`, and overall workflow
durations, the CUDA/Rust cache summaries, and repository cache usage. The
release targets are macOS <= 5 minutes, Linux <= 9 minutes, and total wall time
<= 9 minutes.

## Cold fallback

If the automatic build for a version-bump commit fails, do not push a tag.
Correct the infrastructure problem and rerun the original workflow at the same
commit (`gh run rerun <run-id> --failed`). A rerun preserves the trusted push
event and exact source SHA. If `main` still points to the version-bump commit, a
manual `Release Build` dispatch is also supported; leave `prime_caches=false`
for a restore-only recovery build unless the cache itself is intentionally
being repaired.

If artifacts expired or `main` has advanced, rerun the original version-bump
workflow rather than building arbitrary PR or tag code with signing secrets.
Promotion remains blocked until a successful build and both SHA-named artifacts
exist for the tag commit.

## Final release gate

`prompts/PROMPT_RELEASE.md` requires a separate explicit confirmation after the
trusted build succeeds and before creating or pushing `vX.Y.Z`. Pushing that
tag is the real release action: it validates the tag SHA, promotes the stored
artifacts, creates a draft, verifies remote updater integrity, and publishes.
