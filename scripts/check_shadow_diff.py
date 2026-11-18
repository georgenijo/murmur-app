#!/usr/bin/env python3
"""Fail if the shadow capture changed a production statement (D-S2c-8 a, D-S2c-9 f).

The dictation shadow is **insertion only** in production code: against `main`, a production
file's diff adds lines and removes none. No existing statement is removed, moved or
reordered, so the default build's behaviour (and a feature build's with the shadow
disarmed) is the code `main` already reviewed. `git diff --numstat --no-renames` reports
the removed lines of each file, a renamed file as the deletion of the old path plus the
addition of the new one; this check requires 0 removed lines for **every changed production
file** (any `.rs` under `app/src-tauri/src` or `app/src-tauri/crates/*/src`), except

* test files: a name ending in `_tests.rs` or `tests.rs`, a file that declares
  `#![cfg(test)]`, a file under a `tests/` directory, and the named `TEST_ONLY_FILES`;
* the named shadow-only modules (`SHADOW_ONLY`): code that exists only for the shadow;
* what `SHADOW_DIFF_ALLOW` names (a path, the most lines it may remove, and why).

    python3 scripts/check_shadow_diff.py [--base origin/main]

A file's additions are checked by the Rust tests (`shadow_off_by_default` reads every
`cfg` attribute): this check is the other half, that nothing was taken out.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TAURI = "app/src-tauri"

# Code that exists only for the shadow (a prefix names a directory, otherwise a file).
SHADOW_ONLY = [
    f"{TAURI}/src/dictation_shadow/",
    f"{TAURI}/src/dictation_executor/shadow.rs",
    f"{TAURI}/src/commands/recording/shadow_commands.rs",
]

# Test code the name and the file's own attributes do not say is test code: a harness
# included only under `#[cfg(test)]`.
TEST_ONLY_FILES = [
    f"{TAURI}/src/commands/recording_golden_harness.rs",
]

# A production file that may lose lines: `path -> (most removed lines, why)`. Empty by
# design; an entry needs a reason a reviewer can weigh.
SHADOW_DIFF_ALLOW: dict[str, tuple[int, str]] = {}

_PRODUCTION = re.compile(rf"^{re.escape(TAURI)}/(src|crates/[^/]+/src)/.+\.rs$")


def is_production_path(path: str) -> bool:
    """A Rust source file of the app or of one of its crates."""
    return bool(_PRODUCTION.match(path))


def is_test_file(path: str, text: str) -> bool:
    name = path.rsplit("/", 1)[-1]
    return (
        name.endswith("_tests.rs")
        or name == "tests.rs"
        or "/tests/" in path
        or path in TEST_ONLY_FILES
        or re.search(r"^#!\[cfg\(test\)\]", text, re.M) is not None
    )


def is_shadow_only(path: str) -> bool:
    return any(path == entry or (entry.endswith("/") and path.startswith(entry)) for entry in SHADOW_ONLY)


def numstat(root: Path, base: str) -> dict[str, tuple[int, int]]:
    """`path -> (added, removed)` of the branch against its merge base with `base`.

    Renames are not followed: a renamed file is the deletion of its old path and the
    addition of its new one, so a rename cannot hide a removal.
    """
    result = subprocess.run(
        ["git", "diff", "--numstat", "--no-renames", f"{base}...HEAD"],
        cwd=root,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise RuntimeError(f"git diff --numstat {base}...HEAD failed: {result.stderr.strip()}")
    stats: dict[str, tuple[int, int]] = {}
    for line in result.stdout.splitlines():
        added, removed, path = line.split("\t", 2)
        # A binary file reports `-`: it is never a source file.
        stats[path] = (
            int(added) if added.isdigit() else 0,
            int(removed) if removed.isdigit() else 0,
        )
    return stats


def show(root: Path, ref: str, path: str) -> str:
    """The file's text at `ref` (empty if it does not exist there)."""
    result = subprocess.run(
        ["git", "show", f"{ref}:{path}"], cwd=root, capture_output=True, text=True
    )
    return result.stdout if result.returncode == 0 else ""


def guarded_paths(stats: dict[str, tuple[int, int]], root: Path, base: str) -> list[str]:
    """The changed production files the shadow may only insert into."""
    guarded = []
    for path in sorted(stats):
        if not is_production_path(path) or is_shadow_only(path):
            continue
        # A file is a test file by what it is on either side of the change: a deletion
        # shows the base's text, an addition the branch's.
        text = show(root, "HEAD", path) or show(root, base, path)
        if is_test_file(path, text):
            continue
        guarded.append(path)
    return guarded


def violations(
    stats: dict[str, tuple[int, int]],
    guarded: list[str] | None = None,
    allow: dict[str, tuple[int, str]] | None = None,
) -> list[str]:
    allow = SHADOW_DIFF_ALLOW if allow is None else allow
    if guarded is None:
        guarded = [
            path for path in stats if is_production_path(path) and not is_shadow_only(path)
        ]
    found = []
    for path in guarded:
        added, removed = stats.get(path, (0, 0))
        permitted = allow.get(path, (0, ""))[0]
        if removed > permitted:
            found.append(
                f"{path}: removes {removed} lines (+{added}/-{removed}); the shadow may only "
                f"insert in a production file ({permitted} allowed)"
            )
    for path, (limit, reason) in allow.items():
        if not reason.strip():
            found.append(f"SHADOW_DIFF_ALLOW[{path}] gives no reason")
        if not is_production_path(path):
            found.append(f"SHADOW_DIFF_ALLOW[{path}] names a file that is not a production file")
    return found


def main(root: Path = ROOT, base: str = "origin/main") -> int:
    try:
        stats = numstat(root, base)
    except RuntimeError as error:
        print(error, file=sys.stderr)
        return 2
    guarded = guarded_paths(stats, root, base)
    problems = violations(stats, guarded)
    for path in guarded:
        added, removed = stats[path]
        print(f"{added}\t{removed}\t{path}")
    if problems:
        for problem in problems:
            print(f"error: {problem}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", default="origin/main")
    arguments = parser.parse_args()
    sys.exit(main(ROOT, arguments.base))
