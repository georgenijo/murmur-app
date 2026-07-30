#!/usr/bin/env python3
"""Build Murmur's macOS-arm64 bundled helpers for Tauri externalBin.

The historical filename remains the developer prerequisite, but it now builds
both exact production helpers: the local-LLM sidecar and the Phase-0 capture
helper. A Tauri bundle must never silently omit either signed executable.
"""

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
CAPTURE_HELPER_NAME = "murmur-capture-helper"
TARGET = "aarch64-apple-darwin"


def run(command: list[str], *, cwd: Path = ROOT) -> subprocess.CompletedProcess[str]:
    return subprocess.run(command, cwd=cwd, text=True, check=True, capture_output=True)

def publish_binary(name: str, profile: str) -> Path:
    built = TAURI_ROOT / "target" / profile / name
    if not built.is_file():
        raise SystemExit(f"helper build did not produce {built}")

    destination = TAURI_ROOT / "binaries" / f"{name}-{TARGET}"
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(built, destination)
    destination.chmod(destination.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)

    archs = run(["lipo", "-archs", str(destination)]).stdout.strip().split()
    if archs != ["arm64"]:
        raise SystemExit(f"{name} architecture must be exactly arm64, found {archs}")

    dependencies = run(["otool", "-L", str(destination)]).stdout.lower()
    forbidden = ("libcurl", "libssl", "libcrypto")
    present = [library for library in forbidden if library in dependencies]
    if present:
        raise SystemExit(f"{name} links forbidden networking dependencies: {present}")
    return destination


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

    capture_command = ["cargo", "build", "-p", CAPTURE_HELPER_NAME]
    if args.release:
        capture_command.append("--release")
    subprocess.run(capture_command, cwd=TAURI_ROOT, env=env, check=True)

    llm_destination = publish_binary(SIDECAR_NAME, profile)
    capture_destination = publish_binary(CAPTURE_HELPER_NAME, profile)
    if args.print_output:
        print(llm_destination)
        print(capture_destination)
    else:
        print(f"built {llm_destination.relative_to(ROOT)}")
        print(f"built {capture_destination.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
