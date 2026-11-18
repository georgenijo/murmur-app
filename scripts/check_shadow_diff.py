#!/usr/bin/env python3
"""Fail if the shadow capture changed a live-path statement (D-S2c-8 a).

The dictation shadow is **insertion only** in the live path: against `main`, a live-path
file's diff adds lines and removes none. No existing statement is removed, moved or
reordered, so the default build's behaviour (and a feature build's with the shadow
disarmed) is the code `main` already reviewed. `git diff --numstat` reports the removed
lines of each file; this check requires 0 for every file in `LIVE_FILES`, except what
`SHADOW_DIFF_ALLOW` names (a path, the most lines it may remove, and why).

    python3 scripts/check_shadow_diff.py [--base origin/main]

A file's additions are checked by the Rust tests (`shadow_off_by_default` reads every
`cfg` attribute): this check is the other half, that nothing was taken out.
"""

from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TAURI = "app/src-tauri"

# Files whose statements run on the live dictation path. A shadow change to any of them
# may only insert lines.
LIVE_FILES = [
    f"{TAURI}/src/commands/recording.rs",
    f"{TAURI}/src/commands/recording/cancel_ports.rs",
    f"{TAURI}/src/commands/recording/lifecycle_ports.rs",
    f"{TAURI}/src/commands/recording/process_audio_ports.rs",
    f"{TAURI}/src/commands/recording/ready_ports.rs",
    f"{TAURI}/src/commands/recording/start_ports.rs",
    f"{TAURI}/src/commands/recording/stop_ports.rs",
    f"{TAURI}/src/commands/dictation_preview.rs",
    f"{TAURI}/src/dictation_executor/adapter.rs",
    f"{TAURI}/src/dictation_executor/executor.rs",
    f"{TAURI}/src/audio_lifecycle.rs",
    f"{TAURI}/src/core_history.rs",
    f"{TAURI}/src/keyboard.rs",
    f"{TAURI}/src/state.rs",
    f"{TAURI}/src/lib.rs",
    f"{TAURI}/crates/dictation-core/src/decision.rs",
]

# A live-path file that may lose lines: `path -> (most removed lines, why)`. Empty by
# design; an entry needs a reason a reviewer can weigh.
SHADOW_DIFF_ALLOW: dict[str, tuple[int, str]] = {}


def numstat(root: Path, base: str) -> dict[str, tuple[int, int]]:
    """`path -> (added, removed)` of the branch against its merge base with `base`."""
    result = subprocess.run(
        ["git", "diff", "--numstat", f"{base}...HEAD"],
        cwd=root,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise RuntimeError(f"git diff --numstat {base}...HEAD failed: {result.stderr.strip()}")
    stats: dict[str, tuple[int, int]] = {}
    for line in result.stdout.splitlines():
        added, removed, path = line.split("\t", 2)
        # A binary file reports `-`: it is never a live-path source file.
        stats[path] = (
            int(added) if added.isdigit() else 0,
            int(removed) if removed.isdigit() else 0,
        )
    return stats


def violations(
    stats: dict[str, tuple[int, int]],
    live: list[str] = LIVE_FILES,
    allow: dict[str, tuple[int, str]] | None = None,
) -> list[str]:
    allow = SHADOW_DIFF_ALLOW if allow is None else allow
    found = []
    for path in live:
        added, removed = stats.get(path, (0, 0))
        permitted = allow.get(path, (0, ""))[0]
        if removed > permitted:
            found.append(
                f"{path}: removes {removed} lines (+{added}/-{removed}); the shadow may only "
                f"insert in a live-path file ({permitted} allowed)"
            )
    for path, (limit, reason) in allow.items():
        if not reason.strip():
            found.append(f"SHADOW_DIFF_ALLOW[{path}] gives no reason")
        if path not in live:
            found.append(f"SHADOW_DIFF_ALLOW[{path}] names a file that is not a live-path file")
    return found


def main(root: Path = ROOT, base: str = "origin/main") -> int:
    try:
        stats = numstat(root, base)
    except RuntimeError as error:
        print(error, file=sys.stderr)
        return 2
    problems = violations(stats)
    for path in LIVE_FILES:
        added, removed = stats.get(path, (0, 0))
        if added or removed:
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
