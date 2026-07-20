#!/usr/bin/env python3
"""Build Murmur's macOS-arm64 local-LLM sidecar for Tauri externalBin."""

from __future__ import annotations

import argparse
import os
from pathlib import Path
import platform
import shutil
import stat
import subprocess
import sys


ROOT = Path(__file__).resolve().parents[1]
TAURI_ROOT = ROOT / "app" / "src-tauri"
SIDECAR_NAME = "murmur-llm-sidecar"
TARGET = "aarch64-apple-darwin"


def run(command: list[str], *, cwd: Path = ROOT) -> subprocess.CompletedProcess[str]:
    return subprocess.run(command, cwd=cwd, text=True, check=True, capture_output=True)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--release", action="store_true")
    parser.add_argument("--print-output", action="store_true")
    args = parser.parse_args()

    if sys.platform != "darwin" or platform.machine() != "arm64":
        print("local-LLM sidecar: unsupported platform; typed host stub will be used")
        return 0

    command = ["cargo", "build", "-p", SIDECAR_NAME]
    profile = "debug"
    if args.release:
        command.append("--release")
        profile = "release"

    env = os.environ.copy()
    env.update(
        {
            "LLAMA_BUILD_SHARED_LIBS": "OFF",
            "MACOSX_DEPLOYMENT_TARGET": "14.0",
            "CMAKE_OSX_DEPLOYMENT_TARGET": "14.0",
        }
    )
    subprocess.run(command, cwd=TAURI_ROOT, env=env, check=True)

    built = TAURI_ROOT / "target" / profile / SIDECAR_NAME
    if not built.is_file():
        raise SystemExit(f"sidecar build did not produce {built}")

    destination = TAURI_ROOT / "binaries" / f"{SIDECAR_NAME}-{TARGET}"
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(built, destination)
    destination.chmod(destination.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)

    archs = run(["lipo", "-archs", str(destination)]).stdout.strip().split()
    if archs != ["arm64"]:
        raise SystemExit(f"sidecar architecture must be exactly arm64, found {archs}")

    dependencies = run(["otool", "-L", str(destination)]).stdout.lower()
    forbidden = ("libcurl", "libssl", "libcrypto")
    present = [name for name in forbidden if name in dependencies]
    if present:
        raise SystemExit(f"sidecar links forbidden networking dependencies: {present}")

    if args.print_output:
        print(destination)
    else:
        print(f"built {destination.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
