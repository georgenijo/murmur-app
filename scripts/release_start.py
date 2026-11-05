#!/usr/bin/env python3
"""Prepare a release from green, current main and push its version bump.

Only the six release metadata files are committed. The push starts the existing
trusted Release Build / Release workflows; no signing or publishing is duplicated.
"""

from __future__ import annotations

import argparse
from datetime import date
import json
import os
from pathlib import Path
import re
import subprocess
import sys

try:
    from scripts.release_version import PATHS, ReleaseVersionError, check_release, prepare_release
except ModuleNotFoundError:
    from release_version import PATHS, ReleaseVersionError, check_release, prepare_release


ROOT = Path(__file__).resolve().parents[1]


class ScheduledSkip(Exception):
    def __init__(self, level: str, message: str):
        self.level = level
        super().__init__(message)


def skip_or_fail(scheduled: bool, level: str, message: str) -> None:
    if scheduled:
        raise ScheduledSkip(level, message)
    raise ReleaseVersionError(message)


def output(command: list[str], root: Path) -> str:
    return subprocess.check_output(command, cwd=root, text=True).strip()


def api(repository: str, path: str, root: Path, *arguments: str):
    return json.loads(output(["gh", "api", f"repos/{repository}/{path}", *arguments], root))


def next_version(version: str, bump: str) -> str:
    if not re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", version):
        raise ReleaseVersionError("the release button supports stable versions only")
    major, minor, patch = map(int, version.split("."))
    if bump == "minor":
        return f"{major}.{minor + 1}.0"
    if bump == "patch":
        return f"{major}.{minor}.{patch + 1}"
    raise ReleaseVersionError("bump must be patch or minor")


def require_green_ci(checks: list[dict], sha: str) -> None:
    # filter=latest selects the latest run for each check name. Require GitHub's
    # CI sentinel on the exact source, never an unrelated provider's status.
    candidates = [check for check in checks if check.get("name") == "ci-pass"
                  and check.get("app", {}).get("slug") == "github-actions"
                  and check.get("head_sha") == sha]
    if not candidates or any(check.get("conclusion") != "success" for check in candidates):
        raise ReleaseVersionError("main must have a successful ci-pass check before releasing")


def ensure_notes(root: Path, notes: str) -> None:
    path = root / "CHANGELOG.md"
    text = path.read_text()
    match = re.search(r"^## \[Unreleased\]\s*\n(?P<body>.*?)(?=^## \[)", text, re.M | re.S)
    if match is None:
        raise ReleaseVersionError("CHANGELOG must have an Unreleased section")
    if match.group("body").strip():
        return
    if not notes.strip():
        raise ReleaseVersionError("GitHub did not generate release notes")
    path.write_text(text[:match.start()] + "## [Unreleased]\n\n" + notes.strip() + "\n\n" + text[match.end():])


def start_release(bump: str, expected_sha: str, repository: str, *, root: Path = ROOT, scheduled: bool = False) -> tuple[str, str]:
    if scheduled and bump != "patch":
        raise ReleaseVersionError("scheduled releases must use patch")
    if not re.fullmatch(r"[0-9a-f]{40}", expected_sha):
        raise ReleaseVersionError("expected SHA must be a full commit SHA")
    if output(["git", "status", "--porcelain"], root):
        raise ReleaseVersionError("release checkout must be clean")
    subprocess.run(["git", "fetch", "origin", "main", "--tags"], cwd=root, check=True)
    head = output(["git", "rev-parse", "HEAD"], root)
    main = output(["git", "rev-parse", "origin/main"], root)
    if head != expected_sha or main != expected_sha:
        raise ReleaseVersionError("main moved; start a new Release Murmur run (do not rerun the old request)")

    current = check_release(root=root)
    version = next_version(current, bump)
    latest = api(repository, "releases/latest", root)
    if latest.get("tag_name") != f"v{current}":
        skip_or_fail(scheduled, "warning", "current version must already be the latest public release; finish or recover it first")
    previous_sha = output(["git", "rev-list", "-n", "1", f"v{current}"], root)
    subprocess.run(["git", "merge-base", "--is-ancestor", previous_sha, head], cwd=root, check=True)
    if previous_sha == head:
        skip_or_fail(scheduled, "notice", "there are no new commits to release" if not scheduled else "nothing to release")
    checks = api(repository, f"commits/{head}/check-runs?check_name=ci-pass&filter=latest&per_page=100", root)
    try:
        require_green_ci(checks["check_runs"], head)
    except ReleaseVersionError as error:
        skip_or_fail(scheduled, "warning", str(error))

    # Preserve curated notes. Generate them from merged PRs when none were added.
    changelog = (root / "CHANGELOG.md").read_text()
    unreleased = re.search(r"^## \[Unreleased\]\s*\n(?P<body>.*?)(?=^## \[)", changelog, re.M | re.S)
    if unreleased is None or not unreleased.group("body").strip():
        generated = api(repository, "releases/generate-notes", root, "--method", "POST",
                        "-f", f"tag_name=v{version}", "-f", f"target_commitish={head}",
                        "-f", f"previous_tag_name=v{current}")
        ensure_notes(root, generated.get("body", ""))
    prepare_release(version, date.today().isoformat(), root=root)
    check_release(version, root=root)
    # Keep updater policy unchanged: the button never makes an update mandatory.
    subprocess.run(["git", "add", "--", *(str(path) for path in PATHS.values())], cwd=root, check=True)
    subprocess.run(["git", "-c", "user.name=github-actions[bot]", "-c",
                    "user.email=41898282+github-actions[bot]@users.noreply.github.com",
                    "commit", "-m", f"chore: bump version to {version}"], cwd=root, check=True)
    sha = output(["git", "rev-parse", "HEAD"], root)
    # A normal fast-forward push fails if another change landed. Never force,
    # rebase, or silently release a different tree than the one checked above.
    subprocess.run(["git", "push", "origin", "HEAD:refs/heads/main"], cwd=root, check=True)
    return version, sha


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bump", choices=("patch", "minor"), required=True)
    parser.add_argument("--expected-sha", required=True)
    parser.add_argument("--scheduled", action="store_true")
    args = parser.parse_args()
    repository = os.environ["GITHUB_REPOSITORY"]
    try:
        version, sha = start_release(args.bump, args.expected_sha, repository, scheduled=args.scheduled)
    except ScheduledSkip as skip:
        message = f"Scheduled release skipped: {skip}"
        print(f"::{skip.level}::{message}")
        if path := os.environ.get("GITHUB_STEP_SUMMARY"):
            with open(path, "a") as stream:
                stream.write(f"{message}\n")
        return 0
    except (OSError, KeyError, ReleaseVersionError, subprocess.CalledProcessError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 1
    summary = (
        f"### Release v{version} started\n\n"
        f"Version bump: `{sha}`. Build, publication, and OTA verification continue in these workflows:\n\n"
        f"- [Release Build](https://github.com/{repository}/actions/workflows/release-build.yml)\n"
        f"- [Release and OTA result](https://github.com/{repository}/actions/workflows/release.yml)\n\n"
        "The release is complete when **Release**, including **OTA canary**, is green.\n"
    )
    print(summary)
    if path := os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(path, "a") as stream:
            stream.write(summary)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
