#!/usr/bin/env python3
"""Validate Murmur's CI, trusted release-build, cache, and promotion policy."""

from pathlib import Path
import re
from typing import Optional


ROOT = Path(__file__).resolve().parents[1]
CI_WORKFLOW = ROOT / ".github/workflows/ci.yml"
RELEASE_BUILD_WORKFLOW = ROOT / ".github/workflows/release-build.yml"
RELEASE_WORKFLOW = ROOT / ".github/workflows/release.yml"
LINUX_SETUP_ACTION = ROOT / ".github/actions/setup-linux-build/action.yml"

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
TRUSTED_MAIN_CACHE_DEFAULT = (
    '"${{ github.event_name == \'push\' && github.ref == \'refs/heads/main\' }}\"'
)


def job_block(workflow: str, job: str) -> str:
    match = re.search(
        rf"^  {re.escape(job)}:\n(?P<body>(?:^(?:    .*|\s*)\n?)*)",
        workflow,
        re.MULTILINE,
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


def validate_ci(ci: str) -> int:
    assert "push:\n    branches: [main]" in ci
    assert "\n  pull_request:" in ci
    assert scalar(job_block(ci, "changes"), "if") == CI_GUARD
    for job in ("typecheck", "rust-macos", "linux"):
        assert scalar(job_block(ci, job), "needs") == "changes"
    assert scalar(job_block(ci, "ci-pass"), "needs") == (
        "[changes, typecheck, rust-macos, linux]"
    )
    assert scalar(job_block(ci, "ci-pass"), "if") == CI_PASS_GUARD
    assert "scripts/validate_workflow_policy.py" in ci
    assert "scripts/release_artifacts.py" in ci
    assert "tests/test_release_artifacts.py" in ci
    assert "tests/test_workflow_policy.py" in ci

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
    assert scalar(job_block(workflow, "context"), "if") == RELEASE_BUILD_GUARD
    for job in ("typecheck", "release-macos", "release-linux"):
        assert scalar(job_block(workflow, job), "needs") == "context"

    # Native builds and frontend verification share only `context`, so all three
    # enter the queue concurrently instead of serializing behind typecheck.
    assert "needs: [typecheck]" not in workflow
    assert "macos-release-${{ needs.context.outputs.source-sha }}" in workflow
    assert "linux-release-${{ needs.context.outputs.source-sha }}" in workflow
    assert "shared-key: macos-release-v1" in workflow
    assert "shared-key: linux-cuda-release-v1" in workflow
    assert workflow.count("${{ needs.context.outputs.cache-write == 'true' }}") >= 3
    assert "AppImage must not contain the runner-local NVIDIA driver stub" in workflow

    cases = (
        ("push", "chore: bump version to 0.17.0", True),
        ("push", "feat: normal merge", False),
        ("pull_request", "chore: bump version to 0.17.0", False),
        ("workflow_dispatch", None, True),
    )
    for event_name, message, expected in cases:
        assert should_run_release_build(event_name, message) is expected
    return len(cases)


def validate_linux_cache_policy(action: str) -> None:
    assert action.count(TRUSTED_MAIN_CACHE_DEFAULT) == 2
    assert "cuda-minimal-${{ runner.os }}-${{ runner.arch }}-${{ inputs.cuda-version }}-v1" in action
    assert 'sub-packages: \'["nvcc", "cudart-dev"]\'' in action
    assert 'non-cuda-sub-packages: \'["libcublas-dev"]\'' in action
    assert 'STUB_DIR="$RUNNER_TEMP/murmur-cuda-driver-stub"' in action
    assert "CUDA_DRIVER_STUB_DIR=$STUB_DIR" in action
    assert "LINUXDEPLOY_EXCLUDED_LIBRARIES=libcuda.so.1" in action

    prepare = named_step_block(action, "Prepare CUDA cache restore path", 4)
    assert 'sudo mkdir -p "/usr/local/cuda-${CUDA_MM}"' in prepare
    assert 'sudo chown -R "$(id -u):$(id -g)"' in prepare

    restore = named_step_block(action, "Restore CUDA toolkit cache", 4)
    save = named_step_block(action, "Save CUDA toolkit cache", 4)
    assert "path: /usr/local/cuda-${{ env.CUDA_MM }}" in restore
    assert "path: /usr/local/cuda-${{ env.CUDA_MM }}" in save
    for forbidden in (
        "/usr/local/cuda\n",
        "/usr/lib/x86_64-linux-gnu/libcuda",
        "/usr/lib/x86_64-linux-gnu/libnvidia",
        "/etc/ld.so.conf.d",
    ):
        assert forbidden not in restore
        assert forbidden not in save
    assert "$RUNNER_TEMP" not in restore
    assert "$RUNNER_TEMP" not in save
    assert (
        "if: steps.cuda-cache.outputs.cache-hit != 'true' && "
        "inputs.cuda-cache-save-if == 'true'"
    ) in save

    verify = named_step_block(action, "Verify CUDA install", 4)
    assert "nvcc --version" not in verify  # absolute versioned nvcc path is required
    assert '"$NVCC" --version' in verify
    assert "cache-hit=${{ steps.cuda-cache.outputs.cache-hit }}" in verify
    assert "release ${CUDA_MM}" in verify

    configure = named_step_block(action, "Configure CUDA environment", 4)
    assert "LD_LIBRARY_PATH=$STUB_DIR:/usr/local/cuda/lib64:" in configure


def validate_promotion_policy(workflow: str) -> int:
    assert "tags:\n      - 'v*'" in workflow
    assert "\n  workflow_dispatch:" in workflow
    assert "self-hosted" not in workflow
    assert "actions/cache" not in workflow
    assert "swatinem/rust-cache" not in workflow
    assert scalar(job_block(workflow, "promote"), "needs") == "resolve"
    assert "head_branch == \"main\"" in workflow
    assert ".head_sha == $sha" in workflow
    assert ".event == \"push\"" in workflow
    assert "release-build.yml" in workflow
    assert "expired == false" in workflow
    assert "scripts/release_artifacts.py validate" in workflow

    publish_steps = (
        "Create draft release",
        "Upload signed release assets",
        "Verify uploaded updater signatures",
        "Generate updater channel manifests from verified signatures",
        "Upload and verify updater manifests",
        "Publish release",
    )
    for name in publish_steps:
        block = named_step_block(workflow, name, 6)
        assert "if: needs.resolve.outputs.publish == 'true'" in block
    rehearsal = named_step_block(
        workflow, "Report non-publishing promotion rehearsal", 6
    )
    assert "if: needs.resolve.outputs.publish != 'true'" in rehearsal
    return len(publish_steps)


def main() -> None:
    ci = CI_WORKFLOW.read_text()
    release_build = RELEASE_BUILD_WORKFLOW.read_text()
    release = RELEASE_WORKFLOW.read_text()
    linux_action = LINUX_SETUP_ACTION.read_text()

    ci_cases = validate_ci(ci)
    release_build_cases = validate_release_build(release_build)
    validate_linux_cache_policy(linux_action)
    publication_steps = validate_promotion_policy(release)

    print(
        "workflow policy validation passed "
        f"({ci_cases} CI cases; {release_build_cases} release-build cases; "
        f"{publication_steps} publication gates; trusted cache ownership intact)"
    )


if __name__ == "__main__":
    main()
