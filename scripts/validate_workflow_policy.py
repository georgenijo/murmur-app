#!/usr/bin/env python3
"""Validate Murmur's CI, trusted release-build, cache, and promotion policy."""

from pathlib import Path
import re
from typing import Optional


ROOT = Path(__file__).resolve().parents[1]
CI_WORKFLOW = ROOT / ".github/workflows/ci.yml"
RELEASE_BUILD_WORKFLOW = ROOT / ".github/workflows/release-build.yml"
RELEASE_REHEARSAL_WORKFLOW = ROOT / ".github/workflows/release-rehearsal.yml"
RELEASE_WORKFLOW = ROOT / ".github/workflows/release.yml"
CARGO_TOML = ROOT / "app/src-tauri/Cargo.toml"

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
    assert "push:\n    branches: [main]" in ci
    assert "\n  pull_request:" in ci
    changes = job_block(ci, "changes")
    rust_macos = job_block(ci, "rust-macos")
    dependency_audit = job_block(ci, "dependency-audit")
    ci_pass = job_block(ci, "ci-pass")
    assert scalar(changes, "if") == CI_GUARD
    for job in ("typecheck", "visual-regression", "rust-macos"):
        assert scalar(job_block(ci, job), "needs") == "changes"
    assert scalar(dependency_audit, "if") == CI_GUARD
    assert scalar(ci_pass, "needs") == (
        "[changes, typecheck, visual-regression, rust-macos]"
    )
    assert scalar(ci_pass, "if") == CI_PASS_GUARD
    ci_pass_step = named_step_block(ci_pass, "Check CI result", 6)
    assert "${{ needs.visual-regression.result }}" in ci_pass_step
    rust_filter = re.search(
        r"^            rust:\n(?P<paths>(?:^              - .+\n)+)",
        changes,
        re.MULTILINE,
    )
    assert rust_filter, "missing rust paths filter"
    assert "'rustfmt.toml'" in rust_filter.group("paths")
    assert "scripts/validate_workflow_policy.py" in ci
    assert "'scripts/validate_reference_docs.py'" in ci
    assert "python3 scripts/validate_reference_docs.py" in ci
    assert "'docs/reference/**'" in ci
    assert "scripts/release_artifacts.py" in ci
    assert "scripts/capture_agent_matrix.py" in ci
    assert "'scripts/release_version.py'" in ci
    assert "tests/test_release_artifacts.py" in ci
    assert ci.count("tests/test_reference_docs.py") >= 2
    assert "tests/test_release_version.py" in ci
    assert "tests/test_workflow_policy.py" in ci
    assert "tests/test_capture_agent_matrix.py" in ci
    assert ci.count("tests/test_event_store.py") >= 2
    capture_build = named_step_block(
        rust_macos, "Build capture isolation helpers and stub local-LLM externalBin", 6
    )
    rust_install = named_step_block(rust_macos, "Install Rust", 6)
    assert "uses: dtolnay/rust-toolchain@1.96.0" in rust_install
    assert "components: clippy, rustfmt" in rust_install
    rust_format = named_step_block(rust_macos, "Check Rust formatting", 6)
    assert "cargo fmt --all -- --check" in rust_format
    rust_check = named_step_block(rust_macos, "Compile check", 6)
    assert (
        "cargo check --workspace --exclude murmur-llm-sidecar --all-targets"
        in rust_check
    )
    rust_lint = named_step_block(rust_macos, "Lint Rust", 6)
    assert (
        "cargo clippy --workspace --exclude murmur-llm-sidecar --all-targets -- -D warnings"
        in rust_lint
    )
    assert "cargo clippy" not in capture_build
    macos_lib_tests = named_step_block(rust_macos, "Run tests", 6)
    assert (
        "cargo test --workspace --exclude murmur-llm-sidecar --lib -- --test-threads=1"
        in macos_lib_tests
    )
    assert "swiftc -warnings-as-errors" in capture_build
    assert "sidecars/capture-agent/main.swift" in capture_build
    assert "cargo build -p murmur-capture-helper" in capture_build
    assert "MURMUR_CAPTURE_ROLE=worker" in capture_build
    assert "CARGO_TARGET_DIR=target/capture-worker-build" in capture_build
    assert "target/capture-worker-build/debug/murmur-capture-helper" in capture_build
    assert "binaries/murmur-capture-worker-aarch64-apple-darwin" in capture_build
    capture_tests = named_step_block(rust_macos, "Run capture worker unit tests", 6)
    assert "cargo test -p murmur-capture-helper" in capture_tests
    rust_audit = named_step_block(
        dependency_audit, "Audit Rust dependencies (advisory)", 6
    )
    assert "continue-on-error: true" in rust_audit
    assert "cargo install cargo-audit --locked --version 0.22.2" in rust_audit
    assert "cargo audit" in rust_audit
    npm_audit = named_step_block(
        dependency_audit, "Audit npm dependencies (advisory)", 6
    )
    assert "continue-on-error: true" in npm_audit
    assert "npm audit --audit-level=high" in npm_audit
    llm_target = "binaries/murmur-llm-sidecar-aarch64-apple-darwin"
    assert capture_build.count(f": > {llm_target}") == 1
    assert capture_build.count(f"chmod +x {llm_target}") == 1

    cases = (
        ("push", "chore: bump version to 0.17.0", False),
        ("push", "chore: bump version", False),
        ("push", "feat: add a normal feature", True),
        ("pull_request", "chore: bump version to 0.17.0", True),
        ("pull_request", None, True),
    )
    for event_name, message, expected in cases:
        assert should_run_ci(event_name, message) is expected
    return len(cases)


def validate_release_build(workflow: str) -> int:
    assert "push:\n    branches: [main]" in workflow
    assert "\n  workflow_dispatch:" in workflow
    assert "pull_request" not in workflow
    assert "self-hosted" not in workflow
    assert "contents: write" not in workflow
    assert "tests/test_release_version.py" in workflow
    assert scalar(job_block(workflow, "context"), "if") == RELEASE_BUILD_GUARD
    for job in ("typecheck", "release-macos"):
        assert scalar(job_block(workflow, job), "needs") == "context"

    # The native build and frontend verification share only `context`, so both
    # enter the queue concurrently instead of serializing behind typecheck.
    assert "needs: [typecheck]" not in workflow
    assert "macos-release-${{ needs.context.outputs.source-sha }}" in workflow
    assert "capture-helper-tcc-evidence-${{ needs.context.outputs.source-sha }}" in workflow
    assert (
        "--capture-helper-entitlements app/src-tauri/capture-helper.entitlements.plist"
        in workflow
    )
    assert (
        "--capture-agent-entitlements app/src-tauri/capture-agent.entitlements.plist"
        in workflow
    )
    assert "--capture-agent-info-plist app/src-tauri/capture-agent-info.plist" in workflow
    assert (
        "--capture-helper-info-plist app/src-tauri/sidecars/capture/Info.plist"
        in workflow
    )
    assert (
        "--capture-worker-info-plist app/src-tauri/sidecars/capture/WorkerInfo.plist"
        in workflow
    )
    assert (
        "--capture-worker-entitlements app/src-tauri/capture-worker.entitlements.plist"
        in workflow
    )
    worker_smoke = named_step_block(
        workflow, "Smoke test signed capture worker protocol", 6
    )
    assert "scripts/smoke_test_capture_worker.py" in worker_smoke
    assert '--worker "$CAPTURE_WORKER"' in worker_smoke
    assert workflow.index("Finalize, notarize, and repackage") < workflow.index(
        "Smoke test signed capture worker protocol"
    )
    assert workflow.index("Smoke test signed capture worker protocol") < workflow.index(
        "Smoke test signed application"
    )
    assert (
        "--capture-agent-launchd-plist "
        "app/src-tauri/macos/com.localdictation.capture-agent.plist"
        in workflow
    )
    assert '--capture-helper-sha256 "$CAPTURE_HELPER_SHA"' in workflow
    assert '--capture-agent-sha256 "$CAPTURE_AGENT_SHA"' in workflow
    assert '--capture-worker-sha256 "$CAPTURE_WORKER_SHA"' in workflow
    capture_evidence = named_step_block(
        workflow, "Record capture-helper signing and non-interactive probe evidence", 6
    )
    assert "scripts/capture_helper_evidence.py collect-signature" in capture_evidence
    assert "--kind capture-agent" in capture_evidence
    assert "--kind capture-worker" in capture_evidence
    assert "scripts/capture_helper_evidence.py validate-probe" in capture_evidence
    assert '--signed-bundle-artifact "macos-release-$SOURCE_SHA"' in capture_evidence
    assert "signature.json" in capture_evidence
    assert "codesign.txt" not in capture_evidence
    assert "designated-requirement.txt" not in capture_evidence
    assert "shared-key: macos-release-v1" in workflow
    assert "Print :CFBundleExecutable" in workflow
    assert '$(ls "$APP/Contents/MacOS/" | head -1)' not in workflow
    assert workflow.count("${{ needs.context.outputs.cache-write == 'true' }}") == 1

    cases = (
        ("push", "chore: bump version to 0.17.0", True),
        ("push", "feat: normal merge", False),
        ("pull_request", "chore: bump version to 0.17.0", False),
        ("workflow_dispatch", None, True),
    )
    for event_name, message, expected in cases:
        assert should_run_release_build(event_name, message) is expected
    return len(cases)


def validate_release_rehearsal(workflow: str) -> int:
    assert "name: Release Rehearsal" in workflow
    assert "\n  workflow_dispatch:" in workflow
    for forbidden_trigger in ("\n  push:", "\n  pull_request:", "\n  workflow_run:"):
        assert forbidden_trigger not in workflow
    assert "permissions:\n  contents: read" in workflow
    assert "tests/test_release_version.py" in workflow
    for forbidden in (
        "secrets.",
        "GITHUB_TOKEN",
        "contents: write",
        "actions: write",
        "id-token: write",
        "pull-requests: write",
        "gh release",
        "git tag",
        "tauri-action",
    ):
        assert forbidden not in workflow

    context = job_block(workflow, "context")
    assert "ref: main" in context
    assert 'if [ "$GITHUB_REF" != "refs/heads/main" ]' in workflow
    assert "source_sha must be an exact lowercase 40-character commit SHA" in workflow
    assert 'git fetch --no-tags --depth=1 origin "$SOURCE_SHA"' in workflow
    assert 'RESOLVED=$(git rev-parse "$SOURCE_SHA^{commit}")' in workflow
    assert 'echo "source_sha=$RESOLVED" >> "$GITHUB_OUTPUT"' in workflow
    assert 'echo "workflow_sha=$GITHUB_SHA" >> "$GITHUB_OUTPUT"' in workflow

    for job in ("typecheck", "rehearse-macos"):
        assert scalar(job_block(workflow, job), "needs") == "context"
    assert workflow.count("ref: ${{ needs.context.outputs.source-sha }}") == 2
    assert workflow.count("persist-credentials: false") == 3

    assert (
        "shared-key: macos-release-rehearsal-${{ needs.context.outputs.source-sha }}"
        in workflow
    )
    assert "macos-release-v1" not in workflow
    assert workflow.count("--no-sign") == 1
    assert '"proxy": "unsigned-release-build"' in workflow
    assert '"workflow_sha": os.environ["WORKFLOW_SHA"]' in workflow
    assert '"source_sha": os.environ["SOURCE_SHA"]' in workflow
    assert '"build_seconds": int(os.environ["BUILD_SECONDS"])' in workflow
    assert workflow.count("uses: actions/upload-artifact@v4") == 1
    return 2


def validate_release_profile(cargo_toml: str) -> None:
    profile = cargo_toml.split("[profile.release]", 1)[1]
    profile = profile.split("\n[", 1)[0]
    assert re.search(r"^strip\s*=\s*false\s*$", profile, re.MULTILINE)


def validate_promotion_policy(workflow: str) -> int:
    assert "tags:\n      - 'v*'" in workflow
    assert "workflow_run:\n    workflows: [Release Build]\n    types: [completed]" in workflow
    assert "\n  workflow_dispatch:" in workflow
    assert "self-hosted" not in workflow
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
    )
    for name in publish_steps:
        block = named_step_block(workflow, name, 6)
        assert "if: needs.resolve.outputs.publish == 'true'" in block
    assert workflow.index("Verify release metadata matches updater manifest") < workflow.index(
        "Publish release"
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
    ci = CI_WORKFLOW.read_text()
    release_build = RELEASE_BUILD_WORKFLOW.read_text()
    release_rehearsal = RELEASE_REHEARSAL_WORKFLOW.read_text()
    release = RELEASE_WORKFLOW.read_text()
    cargo_toml = CARGO_TOML.read_text()

    ci_cases = validate_ci(ci)
    release_build_cases = validate_release_build(release_build)
    rehearsal_jobs = validate_release_rehearsal(release_rehearsal)
    validate_release_profile(cargo_toml)
    publication_steps = validate_promotion_policy(release)

    print(
        "workflow policy validation passed "
        f"({ci_cases} CI cases; {release_build_cases} release-build cases; "
        f"{rehearsal_jobs} secretless rehearsal jobs; "
        f"{publication_steps} publication gates; trusted cache ownership intact)"
    )


if __name__ == "__main__":
    main()
