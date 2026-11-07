#!/usr/bin/env python3
"""Validate Murmur's CI, trusted release-build, cache, and promotion policy."""

from pathlib import Path
import hashlib
import re
from typing import Optional


ROOT = Path(__file__).resolve().parents[1]
CI_WORKFLOW = ROOT / ".github/workflows/ci.yml"
RELEASE_BUILD_WORKFLOW = ROOT / ".github/workflows/release-build.yml"
RELEASE_REHEARSAL_WORKFLOW = ROOT / ".github/workflows/release-rehearsal.yml"
RELEASE_WORKFLOW = ROOT / ".github/workflows/release.yml"
RELEASE_START_WORKFLOW = ROOT / ".github/workflows/release-start.yml"
OTA_CANARY_WORKFLOW = ROOT / ".github/workflows/ota-canary.yml"
CARGO_TOML = ROOT / "app/Cargo.toml"

CI_GUARD = (
    '"${{ github.event_name != \'push\' || '
    "!startsWith(github.event.head_commit.message, 'chore: bump version') }}\""
)
CI_PASS_GUARD = (
    '"${{ always() && (github.event_name != \'push\' || '
    "!startsWith(github.event.head_commit.message, 'chore: bump version')) }}\""
)
RELEASE_BUILD_GUARD = (
    '"${{ github.event_name == \'workflow_dispatch\' || '
    "startsWith(github.event.head_commit.message, 'chore: bump version') }}\""
)
CI_BUILD_JOBS = (
    "typecheck",
    "visual-linux",
    "visual-regression",
    "rust-lint",
    "rust-macos",
    "gpui-macos",
)
CI_RUST_TEST_STEPS = (
    "Run tests",
    "Run capture worker unit tests",
    "Run deterministic dictation evaluation",
)
SEMVER = re.compile(
    r"^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$"
)


def job_block(workflow: str, job: str) -> str:
    match = re.search(
        rf"^  {re.escape(job)}:\n(?P<body>.*?)(?=^  [A-Za-z0-9_-]+:\n|\Z)",
        workflow,
        re.MULTILINE | re.DOTALL,
    )
    if not match:
        raise AssertionError(f"missing job: {job}")
    return match.group("body")


def scalar(block: str, key: str) -> str:
    match = re.search(rf"^    {re.escape(key)}:\s*(.+)$", block, re.MULTILINE)
    if not match:
        raise AssertionError(f"missing {key!r} in job block")
    return match.group(1).strip()


def named_step_block(text: str, name: str, indent: int) -> str:
    marker = " " * indent + f"- name: {name}\n"
    start = text.find(marker)
    if start < 0:
        raise AssertionError(f"missing step: {name}")
    next_step = text.find("\n" + " " * indent + "- name:", start + len(marker))
    if next_step < 0:
        next_step = len(text)
    return text[start:next_step]


def mapping_blocks(text: str, key: str, indent: int):
    """Include inline values and nested block values at the exact YAML scope."""
    lines = text.splitlines(keepends=True)
    for start, line in enumerate(lines):
        if not re.match(rf"^{' ' * indent}(?:{re.escape(key)}|'{re.escape(key)}'|\"{re.escape(key)}\"):", line):
            continue
        end = start + 1
        while end < len(lines):
            following = lines[end]
            if following.strip() and not following.lstrip().startswith("#"):
                if len(following) - len(following.lstrip()) <= indent:
                    break
            end += 1
        yield "".join(lines[start:end])


def references_secrets(text: str) -> bool:
    # Include whole-context uses inside functions, not only property/index access.
    # Quoted strings may contain }}; they do not terminate an Actions expression.
    return any(re.search(r"\bsecrets\b", expression, re.IGNORECASE)
               for expression in re.findall(r"\$\{\{((?:'(?:[^']|'')*'|[^'}]|}(?!}))*?)\}\}", text, re.DOTALL)) or bool(
        re.search(r"\bsecrets\s*:\s*['\"]?inherit\b", text, re.IGNORECASE)
    )


# Deliberate updates require reviewing the entire job, including shell content.
GPUI_JOB_SHA256 = {
    "gpui-build": "4b2015c205c3e33914403fbd2226759f659e450ece5a3f21a13c5a3c70daf6e4",
    "gpui-signer": "eb7ac7e93ae582b2da6df56dbfb205d9053ff8323e3cafe62d42a81b7678760a",
    "gpui-sign": "17727ea273f3d5e5922c31d571fe8d1a7110c43bf8e311101b831750d0841888",
}


# The GPUI lane inherits workflow-level settings (env, defaults.run.shell) and
# consumes context-job outputs (source-sha), and YAML can append root keys after
# any job. Rather than chase each channel, pin the whole file: every edit to
# release-build.yml must deliberately update this digest after review. It is
# checked last so semantic failures report their specific cause first.
RELEASE_BUILD_SHA256 = "8211bfcaded192c7c7e0e4c5372bf15427afd37f52060e0cad6053914179be52"


def validate_gpui_isolation(workflow: str) -> None:
    for env in mapping_blocks(workflow, "env", 0):
        assert not references_secrets(env), "GPUI compilation inherits workflow secrets"
    guard = "github.event_name == 'workflow_dispatch' && needs.context.outputs.bundle-target == 'gpui'"
    jobs = {name: job_block(workflow, name) for name in ("gpui-build", "gpui-signer", "gpui-sign")}
    gpui_names = re.findall(r"^  (gpui-[A-Za-z0-9_-]+):\n", workflow, re.MULTILINE)
    assert len(gpui_names) == len(jobs) and set(gpui_names) == set(jobs), "unexpected GPUI job"
    for name, job in jobs.items():
        assert hashlib.sha256(job.encode()).hexdigest() == GPUI_JOB_SHA256[name], f"{name} job content must match its reviewed SHA-256 pin"
        assert scalar(job, "if") == guard
        assert scalar(job, "runs-on") == "macos-latest", "GPUI requires fresh hosted runners"
        for env in mapping_blocks(job, "env", 4):
            assert not references_secrets(env), f"{name} must not inherit job secrets"
        permissions = list(mapping_blocks(job, "permissions", 4))
        expected_permissions = "    permissions:\n      contents: read\n" + ("      actions: read\n" if name == "gpui-sign" else "")
        assert permissions == [expected_permissions], f"{name} must have minimal permissions"
        assert "persist-credentials: false" in job
        assert "ref: ${{ needs.context.outputs.source-sha }}" in job
        assert "save-if:" not in job and "actions/cache" not in job and "rust-cache" not in job
        for forbidden in ("macos-release-", "contents: write", "actions: write", "id-token: write",
                          "gh release", "git tag", "tauri-action"):
            assert forbidden not in job, f"{name} cannot publish or share promotion state"

    for name, phase, artifact in (("gpui-build", "build", "build"), ("gpui-signer", "build-signer", "signer")):
        job = jobs[name]
        assert scalar(job, "needs") == "context"
        assert not references_secrets(job), f"{name} must be entirely credential-free"
        assert "security create-keychain" not in job
        assert f"python3 scripts/release_gpui.py {phase}\n" in job
        assert "sha256: ${{ steps.payload.outputs.sha256 }}" in job
        assert "artifact-id: ${{ steps.artifact.outputs.artifact-id }}" in job
        assert f"python3 scripts/gpui_artifacts.py pack {artifact}" in job
        assert 'echo "sha256=$SHA256" >> "$GITHUB_OUTPUT"' in job
        assert f"name: gpui-{artifact}-${{{{ needs.context.outputs.source-sha }}}}" in job
        assert f"path: ${{{{ runner.temp }}}}/gpui-{artifact}.tar" in job

    helpers = named_step_block(jobs["gpui-build"], "Build capture helpers", 6)
    assert "python3 scripts/build_capture_helpers.py --release" in helpers

    sign = jobs["gpui-sign"]
    assert scalar(sign, "needs") == "[context, gpui-build, gpui-signer]"
    checkout = named_step_block(sign, "Check out trusted signing scripts", 6)
    for marker in ("uses: actions/checkout@v4", "persist-credentials: false",
                   "ref: ${{ needs.context.outputs.source-sha }}"):
        assert marker in checkout
    assert sign.count("actions/checkout@") == 1
    verify = named_step_block(sign, "Verify artifact hashes and extract allowlisted binaries", 6)
    assert "if:" not in verify and "continue-on-error:" not in verify
    readonly = named_step_block(sign, "Make trusted checkout read-only", 6)
    assert 'chmod -R a-w "$GITHUB_WORKSPACE"' in readonly
    assert "actions/download-artifact" not in sign
    download = named_step_block(sign, "Download raw isolated artifact ZIPs", 6)
    assert not references_secrets(download), "downloads must have no signing secrets"
    assert "GH_TOKEN: ${{ github.token }}" in download
    assert sign.index(checkout) < sign.index(readonly) < sign.index(download) < sign.index(verify)
    for kind, variable in (("build", "BUILD"), ("signer", "SIGNER")):
        for marker in (f"{variable}_ARTIFACT_ID: ${{{{ needs.gpui-{kind}.outputs.artifact-id }}}}",
                       f'gh api "repos/$GITHUB_REPOSITORY/actions/artifacts/${variable}_ARTIFACT_ID/zip" > "$RUNNER_TEMP/gpui-{kind}.zip"'):
            assert marker in download
        for marker in (f"{variable}_SHA256: ${{{{ needs.gpui-{kind}.outputs.sha256 }}}}",
                       f"python3 scripts/gpui_artifacts.py verify-zip {kind}",
                       f'--archive "$RUNNER_TEMP/gpui-{kind}.zip"',
                       f'--sha256 "${variable}_SHA256" --output "$RUNNER_TEMP/gpui-{kind}"'):
            assert marker in verify
        assert sign.index(download) < sign.index(verify)
    assert sign.index(checkout) < sign.index(verify) < sign.index("- name: Import Developer ID certificate")
    command = named_step_block(sign, "Sign, notarize and record GPUI rehearsal", 6)
    assert sign.index(verify) < sign.index("- name: Import Developer ID certificate") < sign.index(command)
    for marker in ("python3 scripts/release_gpui.py sign",
                   '--binary "$RUNNER_TEMP/gpui-build/ui"',
                   '--helpers-dir "$RUNNER_TEMP/gpui-build"',
                   '--native-signer "$RUNNER_TEMP/gpui-signer/murmur-updater-signer"'):
        assert marker in command
    assert "name: gpui-rehearsal-${{ needs.context.outputs.source-sha }}" in sign
    cleanup = named_step_block(sign, "Delete signing keychain", 6)
    assert "if: always()" in cleanup
    assert "security delete-keychain" in cleanup


def should_run_ci(event_name: str, head_commit_message: Optional[str]) -> bool:
    return event_name != "push" or not (head_commit_message or "").startswith(
        "chore: bump version"
    )


def should_run_release_build(
    event_name: str, head_commit_message: Optional[str]
) -> bool:
    return event_name == "workflow_dispatch" or (
        event_name == "push"
        and (head_commit_message or "").startswith("chore: bump version")
    )


def should_auto_promote(
    *,
    event_name: str,
    workflow_name: str,
    workflow_path: str,
    conclusion: str,
    head_branch: str,
    source_event: str,
    head_commit_message: Optional[str],
) -> bool:
    return (
        event_name == "workflow_run"
        and workflow_name == "Release Build"
        and workflow_path.split("@", 1)[0] == ".github/workflows/release-build.yml"
        and conclusion == "success"
        and head_branch == "main"
        and source_event == "push"
        and (head_commit_message or "").startswith("chore: bump version")
    )


def release_tag_for_versions(
    tauri_version: str,
    cargo_version: str,
    lock_version: str,
    package_version: str,
    package_lock_version: str,
    changelog_version: str,
) -> str:
    if not SEMVER.fullmatch(tauri_version):
        raise AssertionError(f"invalid release version: {tauri_version}")
    if any(
        version != tauri_version
        for version in (
            cargo_version,
            lock_version,
            package_version,
            package_lock_version,
            changelog_version,
        )
    ):
        raise AssertionError("release version surfaces differ")
    return f"v{tauri_version}"


def tag_action(existing_sha: Optional[str], source_sha: str) -> str:
    if existing_sha is None:
        return "create"
    if existing_sha != source_sha:
        raise AssertionError("existing tag points to a different commit")
    return "reuse"


def validate_ci(ci: str) -> int:
    assert "push:\n    branches: [main]" in ci and "\n  pull_request:" in ci
    assert "setup-node" not in ci and "npm " not in ci
    assert ci.count("save-if: ${{ github.ref == 'refs/heads/main' }}") == ci.count("uses: swatinem/rust-cache@v2")
    sentinel = job_block(ci, "ci-pass")
    for job in ("changes", "policy", *CI_BUILD_JOBS):
        job_block(ci, job)
        assert job in scalar(sentinel, "needs"), f"ci-pass must require {job}"
    assert scalar(sentinel, "if") == "always()"
    assert "job['result'] != 'success'" in sentinel, "skipped native jobs must not pass"
    policy = job_block(ci, "policy")
    for script in ("check_core_dependencies.py", "check_native_only.py", "validate_capture_boundary.py", "validate_workflow_policy.py", "validate_reference_docs.py", "validate_markdown_links.py"):
        assert f"python3 scripts/{script}" in policy
    assert "pip install -r requirements-test.txt" in policy
    lint = job_block(ci, "rust-lint")
    assert "dtolnay/rust-toolchain@1.98.1" in lint
    assert "cargo clippy --locked" in lint and "--all-targets --features gallery,production-host -- -D warnings" in lint
    assert "cargo fmt --manifest-path app/Cargo.toml" in lint and "-- --check" in lint
    assert "cargo clippy --locked --manifest-path app/sidecars/capture/Cargo.toml --all-targets -- -D warnings" in lint
    native = job_block(ci, "rust-macos")
    assert "-p murmur-services -p murmur-core -p murmur-webkit -p murmur-capture-helper-protocol -- --test-threads=1" in native
    assert "cargo test --locked --manifest-path app/sidecars/capture/Cargo.toml" in native
    assert "--example murmur-eval -- deterministic" in native
    assert "scripts/test_gpui.py --features production-host" in job_block(ci, "gpui-macos")
    for job in ("visual-linux", "visual-regression"):
        assert "scripts/check_gpui_goldens.py" in job_block(ci, job)
    audit = job_block(ci, "dependency-audit")
    assert "cargo-audit@0.22.2" in audit and "continue-on-error: true" in audit
    assert "cargo audit --file app/Cargo.lock" in audit
    assert "cargo audit --file app/sidecars/capture/Cargo.lock" in audit
    return len(CI_BUILD_JOBS)


def validate_release_build(workflow: str) -> int:
    assert "\n  workflow_dispatch:" in workflow and "pull_request" not in workflow
    assert "self-hosted" not in workflow and "contents: write" not in workflow
    assert "setup-node" not in workflow
    assert "configure_ci_rust_library.py" not in workflow
    assert "group: release-build-${{ github.event_name }}-${{ github.ref }}-${{ github.sha }}" in workflow
    context = job_block(workflow, "context")
    assert scalar(context, "if") == "github.event_name == 'workflow_dispatch'"
    assert "options: [gpui]\n        default: gpui" in workflow
    assert '[ "$GITHUB_REF" != "refs/heads/main" ]' in context
    assert "git merge-base --is-ancestor" in context
    assert 'TARGET=$(python3 scripts/release_bundle_target.py) || exit 1' in context
    validate_gpui_isolation(workflow)
    assert hashlib.sha256(workflow.encode()).hexdigest() == RELEASE_BUILD_SHA256, "release workflow must match its reviewed SHA-256 pin"
    return 3


def validate_release_rehearsal(workflow: str) -> int:
    assert "\n  workflow_dispatch:" in workflow and "pull_request" not in workflow
    assert "self-hosted" not in workflow and not references_secrets(workflow)
    assert "contents: write" not in workflow
    context = job_block(workflow, "context")
    assert '[ "$GITHUB_REF" != "refs/heads/main" ]' in context
    assert 'git rev-parse "$SOURCE_SHA^{commit}"' in context
    native = job_block(workflow, "rehearse-macos")
    assert "ref: ${{ needs.context.outputs.source-sha }}" in native
    assert "scripts/rehearse_gpui_bundle.py" in native
    assert "setup-node" not in workflow and "npm " not in workflow
    return 1


def validate_release_profile(cargo_toml: str) -> None:
    profile = cargo_toml.split("[profile.release]", 1)[1].split("\n[", 1)[0]
    assert re.search(r'^strip\s*=\s*"symbols"\s*$', profile, re.MULTILINE)


def validate_release_start(workflow: str) -> None:
    assert "name: Release Murmur" in workflow
    assert "\n  workflow_dispatch:" in workflow
    assert "options: [patch, minor]" in workflow
    assert "group: release-start" in workflow
    assert "cancel-in-progress: false" in workflow
    for marker in (
        '[ "$GITHUB_REF" != "refs/heads/main" ]',
        '[ -z "$RELEASE_TOKEN" ]',
        "token: ${{ secrets.RELEASE_TOKEN }}",
        'python3 scripts/release_start.py "${args[@]}"',
        'args=(--bump "$BUMP" --expected-sha "$GITHUB_SHA")',
        "timeout-minutes: 5",
    ):
        assert marker in workflow
    assert "self-hosted" not in workflow


def validate_ota_canary(workflow: str) -> None:
    assert "\n  workflow_call:" in workflow
    assert "\n  workflow_dispatch:" in workflow
    assert "\n  pull_request:" in workflow
    assert scalar(job_block(workflow, "canary"), "runs-on") == "macos-latest"
    assert "contents: read" in workflow
    assert "secrets." not in workflow
    assert "contents: write" not in workflow
    assert "self-hosted" not in workflow
    assert "--remote --isolated" in workflow
    assert '--root "$RUNNER_TEMP/murmur-ota-canary"' in workflow
    assert "ref: ${{ inputs.source_sha || github.sha }}" in workflow
    assert "persist-credentials: false" in workflow
    assert "--dry-run" not in workflow
    assert "timeout-minutes: 20" in workflow


def validate_promotion_policy(workflow: str) -> int:
    assert "tags:\n      - 'v*'" in workflow
    assert "workflow_run:\n    workflows: [Release Build]\n    types: [completed]" in workflow
    assert "\n  workflow_dispatch:" in workflow
    assert "gpui-" not in workflow, "promotion must never consume any GPUI artifact"
    assert "self-hosted" not in workflow
    canary = job_block(workflow, "ota-canary")
    assert "needs: [resolve, promote]" in canary
    assert "needs.resolve.outputs.publish == 'true'" in canary
    assert "needs.promote.result == 'success'" in canary
    assert "needs.resolve.outputs.already-published == 'true'" in canary
    assert "uses: ./.github/workflows/ota-canary.yml" in canary
    assert "source_sha: ${{ needs.resolve.outputs.source-sha }}" in canary
    assert "tag: ${{ needs.resolve.outputs.tag }}" in canary
    assert "actions/cache" not in workflow
    assert "swatinem/rust-cache" not in workflow
    assert scalar(job_block(workflow, "promote"), "needs") == "resolve"
    assert scalar(job_block(workflow, "promote"), "if") == (
        "needs.resolve.outputs.eligible == 'true' && "
        "needs.resolve.outputs.already-published != 'true'"
    )
    assert "github.event.workflow_run.head_sha || github.sha" in workflow
    for gate in (
        'WORKFLOW_RUN_NAME: ${{ github.event.workflow_run.name }}',
        'WORKFLOW_RUN_PATH: ${{ github.event.workflow_run.path }}',
        'WORKFLOW_RUN_BRANCH: ${{ github.event.workflow_run.head_branch }}',
        'WORKFLOW_RUN_EVENT: ${{ github.event.workflow_run.event }}',
        'WORKFLOW_RUN_CONCLUSION: ${{ github.event.workflow_run.conclusion }}',
        '[ "$WORKFLOW_RUN_NAME" != "Release Build" ]',
        '[ "$WORKFLOW_RUN_CONCLUSION" != "success" ]',
        '[ "$WORKFLOW_RUN_BRANCH" != "main" ]',
        '[ "$WORKFLOW_RUN_EVENT" != "push" ]',
        '[[ "$SUBJECT" != "chore: bump version"* ]]',
    ):
        assert gate in workflow
    assert "head_branch == \"main\"" in workflow
    assert ".head_sha == $sha" in workflow
    assert ".event == \"push\"" in workflow
    assert 'split("@")[0]) == ".github/workflows/release-build.yml"' in workflow
    assert "expired == false" in workflow
    assert "scripts/release_artifacts.py validate" in workflow
    assert 'for NAME in "macos-release-${SOURCE_SHA}"; do' in workflow
    assert "scripts/release_version.py check" in workflow
    assert '--git-ref "$SOURCE_SHA"' in workflow
    assert "--require-macos-capture-helper" in workflow
    assert "--require-macos-capture-agent" in workflow
    assert "--require-macos-capture-worker" in workflow
    assert "release versions or CHANGELOG differ" in workflow
    assert "already_published=true" in workflow
    assert "contains unexpected asset" in workflow
    assert 'gh release view "$TAG"' in workflow
    assert "--json body --jq .body > release-notes.md" in workflow
    assert "--release-notes release-notes.md" in workflow
    assert ".github/updater-policy.json" in workflow
    assert "updater policy must contain exactly one null or string min_version" in workflow
    assert '--min-version "$MIN_VERSION"' in workflow
    assert "published updater policy differs from the trusted source policy" in workflow
    assert workflow.index("scripts/release_artifacts.py validate") < workflow.index(
        "Create automatic release tag"
    )

    publish_steps = (
        "Create automatic release tag",
        "Create or reuse draft release",
        "Upload signed release assets",
        "Verify uploaded updater signatures",
        "Generate updater channel manifests from verified signatures",
        "Upload and verify updater manifests",
        "Verify release metadata matches updater manifest",
        "Publish release",
        "Promote highest published release to latest",
    )
    for name in publish_steps:
        block = named_step_block(workflow, name, 6)
        assert "if: needs.resolve.outputs.publish == 'true'" in block
    assert workflow.index("Verify release metadata matches updater manifest") < workflow.index(
        "Publish release"
    )
    # Concurrent version builds can finish out of order, so publication never
    # lets GitHub pick "latest"; the promotion step converges on the highest.
    publish = named_step_block(workflow, "Publish release", 6)
    assert "--draft=false --latest=false" in publish
    promote = named_step_block(workflow, "Promote highest published release to latest", 6)
    assert "scripts/release_artifacts.py highest-release" in promote
    assert "--exclude-drafts --exclude-pre-releases" in promote
    assert workflow.index("Publish release") < workflow.index(
        "Promote highest published release to latest"
    )
    metadata_check = named_step_block(
        workflow, "Verify release metadata matches updater manifest", 6
    )
    assert "scripts/release_artifacts.py verify-notes" in metadata_check
    assert "--json body --jq .body > draft-release-notes.md" in metadata_check
    assert "--manifest remote-manifests/latest-v2.json" in metadata_check
    assert "--release-notes draft-release-notes.md" in metadata_check
    assert "RELEASE_NOTES=$(" not in metadata_check
    assert ".notes == $notes" not in metadata_check
    rehearsal = named_step_block(
        workflow, "Report non-publishing promotion rehearsal", 6
    )
    assert "if: needs.resolve.outputs.publish != 'true'" in rehearsal
    trusted = dict(
        event_name="workflow_run",
        workflow_name="Release Build",
        workflow_path=".github/workflows/release-build.yml",
        conclusion="success",
        head_branch="main",
        source_event="push",
        head_commit_message="chore: bump version to 0.18.0",
    )
    auto_cases = (
        trusted,
        {**trusted, "source_event": "workflow_dispatch"},
        {**trusted, "head_branch": "feature"},
        {**trusted, "conclusion": "failure"},
        {**trusted, "head_commit_message": "fix: ordinary main commit"},
    )
    expected = (True, False, False, False, False)
    for case, result in zip(auto_cases, expected):
        assert should_auto_promote(**case) is result
    assert release_tag_for_versions(*(["0.18.0"] * 6)) == "v0.18.0"
    assert tag_action(None, "a" * 40) == "create"
    assert tag_action("a" * 40, "a" * 40) == "reuse"
    return len(publish_steps) + len(auto_cases)


def main() -> None:
    from gpui_migration_policy import validate_identity, validate_update_policy
    validate_identity()
    validate_update_policy()
    ci = CI_WORKFLOW.read_text()
    release_build = RELEASE_BUILD_WORKFLOW.read_text()
    release_rehearsal = RELEASE_REHEARSAL_WORKFLOW.read_text()
    release = RELEASE_WORKFLOW.read_text()
    cargo_toml = CARGO_TOML.read_text()

    ci_cases = validate_ci(ci)
    release_build_cases = validate_release_build(release_build)
    rehearsal_jobs = validate_release_rehearsal(release_rehearsal)
    validate_release_profile(cargo_toml)
    validate_release_start(RELEASE_START_WORKFLOW.read_text())
    validate_ota_canary(OTA_CANARY_WORKFLOW.read_text())
    publication_steps = validate_promotion_policy(release)

    print(
        "workflow policy validation passed "
        f"({ci_cases} CI cases; {release_build_cases} release-build cases; "
        f"{rehearsal_jobs} secretless rehearsal jobs; "
        f"{publication_steps} publication gates; trusted cache ownership intact)"
    )


if __name__ == "__main__":
    main()
