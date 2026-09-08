#!/usr/bin/env python3
"""Private owner-only CI entry point. No cross-repository write credential."""
from __future__ import annotations

import json
import os
from pathlib import Path
import re
import subprocess
import tempfile


def command(*args: str, cwd: Path, env=None, timeout=600) -> None:
    subprocess.run(args, cwd=cwd, env=env, check=True, timeout=timeout)


def validate_source(sha: str, ref: str) -> None:
    if not re.fullmatch(r"[0-9a-f]{40}", sha):
        raise ValueError("source_sha must be an immutable lowercase commit SHA")
    if ref != "main" and not re.fullmatch(r"issue/[0-9]+-[a-z0-9-]+", ref):
        raise ValueError("source_ref must be main or an owner-reviewed issue branch")


def main() -> None:
    sha = os.environ.get("SOURCE_SHA", "")
    ref = os.environ.get("SOURCE_REF", "")
    validate_source(sha, ref)
    with tempfile.TemporaryDirectory(prefix="murmur-capture-ci-") as directory:
        source = Path(directory)
        command("git", "init", "--quiet", cwd=source)
        command("git", "remote", "add", "origin", "https://github.com/georgenijo/murmur-app.git", cwd=source)
        command("git", "fetch", "--quiet", "--no-tags", "origin", f"refs/heads/{ref}", cwd=source)
        command("git", "merge-base", "--is-ancestor", sha, "FETCH_HEAD", cwd=source)
        command("git", "checkout", "--quiet", "--detach", sha, cwd=source)
        command("python3", "-m", "unittest", "tests/test_capture_worker_smoke.py",
                "tests/test_capture_first_pcm.py", cwd=source)
        tauri = source / "app/src-tauri"
        # Cache outside the checkout, but only this private owner-only job writes it.
        target = Path(os.environ["RUNNER_TEMP"]) / "murmur-capture-worker-build"
        empty = source / "empty-pkgconfig"
        empty.mkdir()
        env = os.environ.copy()
        env.pop("PKG_CONFIG_PATH", None)
        env.update(
            MURMUR_CAPTURE_ROLE="worker", CARGO_TARGET_DIR=str(target),
            MURMUR_APP_VERSION=str(json.loads((tauri / "tauri.conf.json").read_text())["version"]),
            PKG_CONFIG_LIBDIR=str(empty), MACOSX_DEPLOYMENT_TARGET="14.0",
            CMAKE_OSX_DEPLOYMENT_TARGET="14.0",
        )
        command("cargo", "build", "--locked", "-p", "murmur-capture-helper", "-j", "2", cwd=tauri, env=env)
        worker = target / "debug/murmur-capture-helper"
        print(json.dumps({"source_sha": sha, "source_ref": ref}), flush=True)
        command("python3", "scripts/smoke_test_capture_first_pcm.py", "--worker", str(worker),
                "--first-pcm-seconds", "2", "--timeout-seconds", "10", cwd=source, timeout=45)


if __name__ == "__main__":
    main()
