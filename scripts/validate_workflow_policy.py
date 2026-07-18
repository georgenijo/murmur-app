#!/usr/bin/env python3
"""Validate the CI release-bump guard and release publication dependencies."""

from pathlib import Path
import re
from typing import Optional


ROOT = Path(__file__).resolve().parents[1]
CI_WORKFLOW = ROOT / ".github/workflows/ci.yml"
RELEASE_WORKFLOW = ROOT / ".github/workflows/release.yml"

CI_GUARD = (
    '"${{ github.event_name != \'push\' || '
    "!startsWith(github.event.head_commit.message, 'chore: bump version') }}\""
)
CI_PASS_GUARD = (
    '"${{ always() && (github.event_name != \'push\' || '
    "!startsWith(github.event.head_commit.message, 'chore: bump version')) }}\""
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


def should_run_ci(event_name: str, head_commit_message: Optional[str]) -> bool:
    return event_name != "push" or not (head_commit_message or "").startswith(
        "chore: bump version"
    )


def main() -> None:
    ci = CI_WORKFLOW.read_text()
    release = RELEASE_WORKFLOW.read_text()

    assert "push:\n    branches: [main]" in ci
    assert "\n  pull_request:" in ci
    assert scalar(job_block(ci, "changes"), "if") == CI_GUARD
    for job in ("typecheck", "rust-macos", "linux"):
        assert scalar(job_block(ci, job), "needs") == "changes"
    assert scalar(job_block(ci, "ci-pass"), "needs") == (
        "[changes, typecheck, rust-macos, linux]"
    )
    assert scalar(job_block(ci, "ci-pass"), "if") == CI_PASS_GUARD

    cases = (
        ("push", "chore: bump version to 0.17.0", False),
        ("push", "chore: bump version", False),
        ("push", "feat: add a normal feature", True),
        ("push", "fix: chore: bump version text elsewhere", True),
        ("pull_request", "chore: bump version to 0.17.0", True),
        ("pull_request", None, True),
    )
    for event_name, message, expected in cases:
        actual = should_run_ci(event_name, message)
        assert actual is expected, (
            f"unexpected CI decision for event={event_name!r}, message={message!r}: "
            f"expected {expected}, got {actual}"
        )

    assert "tags:\n      - 'v*'" in release
    assert scalar(job_block(release, "release-macos"), "needs") == "[typecheck]"
    assert scalar(job_block(release, "release-linux"), "needs") == "[typecheck]"
    assert scalar(job_block(release, "publish"), "needs") == (
        "[release-macos, release-linux]"
    )

    print(
        f"workflow policy validation passed ({len(cases)} CI cases; "
        "release publication gating intact)"
    )


if __name__ == "__main__":
    main()
