#!/usr/bin/env python3
"""Local Fleet control-node poller. Reads public main; dispatches private CI."""
from __future__ import annotations

import argparse
import fcntl
import json
import os
from pathlib import Path
import re
import subprocess

CONTROLLER = "georgenijo/murmur-native-ci"
SOURCE = "https://github.com/georgenijo/murmur-app.git"


def capture_path(path: str) -> bool:
    return (
        path.startswith("app/src-tauri/sidecars/capture/")
        or path.startswith("app/src-tauri/crates/capture-helper-protocol/")
        or bool(re.fullmatch(r"app/src-tauri/src/audio[^/]*\.rs", path))
        or path in {
            "app/src-tauri/Cargo.lock", "app/src-tauri/Cargo.toml",
            "scripts/smoke_test_capture_first_pcm.py",
            "scripts/smoke_test_capture_worker.py", "tests/test_capture_first_pcm.py",
        }
    )


def run(*args: str, cwd: Path, input: str | None = None) -> str:
    return subprocess.run(args, cwd=cwd, input=input, text=True, capture_output=True,
                          check=True, timeout=60).stdout


def poll(state_dir: Path, gh: str = "gh") -> dict[str, object]:
    state_dir.mkdir(parents=True, exist_ok=True)
    with (state_dir / "poll.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        mirror = state_dir / "source.git"
        if not mirror.exists():
            run("git", "init", "--bare", str(mirror), cwd=state_dir)
        run("git", "fetch", "--quiet", "--no-tags", SOURCE,
            "+refs/heads/main:refs/heads/main", cwd=mirror)
        current = run("git", "rev-parse", "refs/heads/main", cwd=mirror).strip()
        if not re.fullmatch(r"[0-9a-f]{40}", current):
            raise ValueError("main did not resolve to an immutable SHA")
        state_file = state_dir / "state.json"
        prior = json.loads(state_file.read_text())["last_seen_sha"] if state_file.exists() else None
        if prior == current:
            return {"dispatched": False, "changed": False, "source_sha": current}
        relevant = True
        if prior is not None:
            if not isinstance(prior, str) or not re.fullmatch(r"[0-9a-f]{40}", prior):
                raise ValueError("poll state contains an invalid SHA")
            changes = run("git", "diff", "--no-renames", "--name-only", "-z", prior, current, cwd=mirror)
            relevant = any(capture_path(path) for path in changes.split("\0"))
        dispatched = False
        if relevant:
            runs = json.loads(run(gh, "run", "list", "--repo", CONTROLLER,
                                  "--workflow", "capture-smoke.yml", "--limit", "100",
                                  "--json", "displayTitle", cwd=state_dir))
            if not any(r.get("displayTitle") == f"Capture smoke {current}" for r in runs):
                run(gh, "workflow", "run", "capture-smoke.yml", "--repo", CONTROLLER,
                    "--ref", "main", "--json", cwd=state_dir,
                    input=json.dumps({"source_sha": current, "source_ref": "main"}))
                dispatched = True
        temporary = state_dir / "state.json.tmp"
        temporary.write_text(json.dumps({"last_seen_sha": current}) + "\n")
        os.replace(temporary, state_file)
        return {"dispatched": dispatched, "capture_paths_changed": relevant, "source_sha": current}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--state-dir", required=True, type=Path)
    parser.add_argument("--gh", default="gh")
    args = parser.parse_args()
    try:
        print(json.dumps(poll(args.state_dir.resolve(), args.gh), sort_keys=True))
    except BlockingIOError:
        print(json.dumps({"dispatched": False, "already_running": True}))
    except (OSError, ValueError, KeyError, subprocess.SubprocessError):
        # A failed dispatch does not advance state. Never print API responses,
        # credentials, stderr, or arbitrary repository-controlled text.
        print(json.dumps({"dispatched": False, "error": "native CI poll failed"}))
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
