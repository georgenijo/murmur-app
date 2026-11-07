#!/usr/bin/env python3
"""Build the macOS capture helpers and recovery agent for the native app bundle."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import platform
import plistlib
import shutil
import stat
import subprocess
import sys


ROOT = Path(__file__).resolve().parents[1]
APP_ROOT = ROOT / "app"
TARGET_ROOT = Path(os.environ.get("CARGO_TARGET_DIR", APP_ROOT / "target")).resolve()
CAPTURE_HELPER_NAME = "murmur-capture-helper"
CAPTURE_AGENT_NAME = "murmur-capture-agent"
CAPTURE_WORKER_NAME = "murmur-capture-worker"
TARGET = "aarch64-apple-darwin"
ALLOWED_DYLIB_PREFIXES = (
    "/System/Library/",
    "/usr/lib/",
    "@executable_path/",
    "@loader_path/",
    "@rpath/",
)


def run(command: list[str], *, cwd: Path = ROOT) -> subprocess.CompletedProcess[str]:
    return subprocess.run(command, cwd=cwd, text=True, check=True, capture_output=True)


def unexpected_dynamic_dependencies(otool_output: str) -> list[str]:
    dependencies: list[str] = []
    for line in otool_output.splitlines()[1:]:
        value = line.strip().split(" (compatibility version", 1)[0]
        if not value or value.startswith(ALLOWED_DYLIB_PREFIXES):
            continue
        dependencies.append(value)
    return dependencies


def publish_binary(
    name: str,
    profile: str,
    source_name: str | None = None,
    built_path: Path | None = None,
) -> Path:
    built = built_path or TARGET_ROOT / profile / (source_name or name)
    if not built.is_file():
        raise SystemExit(f"helper build did not produce {built}")

    destination = APP_ROOT / "binaries" / f"{name}-{TARGET}"
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(built, destination)
    destination.chmod(destination.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)

    archs = run(["lipo", "-archs", str(destination)]).stdout.strip().split()
    if archs != ["arm64"]:
        raise SystemExit(f"{name} architecture must be exactly arm64, found {archs}")

    dependency_output = run(["otool", "-L", str(destination)]).stdout
    dependencies = dependency_output.lower()
    forbidden = ("libcurl", "libssl", "libcrypto")
    present = [library for library in forbidden if library in dependencies]
    if present:
        raise SystemExit(f"{name} links forbidden networking dependencies: {present}")
    unexpected = unexpected_dynamic_dependencies(dependency_output)
    if unexpected:
        raise SystemExit(f"{name} links non-system dynamic dependencies: {unexpected}")
    return destination


def build_capture_agent(profile: str, env: dict[str, str]) -> None:
    """Build the probe-only SMAppService launch agent with the system Swift toolchain."""
    output = TARGET_ROOT / profile / CAPTURE_AGENT_NAME
    output.parent.mkdir(parents=True, exist_ok=True)
    with (APP_ROOT / "macos/capture-agent-info.plist").open("rb") as handle:
        embedded_info_payload = plistlib.load(handle)
    with (APP_ROOT / "macos/bundle.json").open(encoding="utf-8") as handle:
        app_version = str(json.load(handle)["version"])
    for key in ("CFBundleShortVersionString", "CFBundleVersion"):
        embedded_info_payload[key] = app_version
    embedded_info = output.parent / "capture-agent-info.plist"
    with embedded_info.open("wb") as handle:
        plistlib.dump(embedded_info_payload, handle, sort_keys=True)
    optimization = "-O" if profile == "release" else "-Onone"
    subprocess.run(
        [
            "xcrun",
            "swiftc",
            optimization,
            "-parse-as-library",
            "-target",
            "arm64-apple-macos14.0",
            str(APP_ROOT / "sidecars" / "capture-agent" / "main.swift"),
            "-framework",
            "Foundation",
            "-framework",
            "Security",
            "-Xlinker",
            "-sectcreate",
            "-Xlinker",
            "__TEXT",
            "-Xlinker",
            "__info_plist",
            "-Xlinker",
            str(embedded_info),
            "-o",
            str(output),
        ],
        cwd=APP_ROOT,
        env=env,
        check=True,
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--release", action="store_true")
    parser.add_argument("--print-output", action="store_true")
    args = parser.parse_args()

    if sys.platform != "darwin" or platform.machine() != "arm64":
        print("capture helpers: unsupported platform")
        return 0

    profile = "release" if args.release else "debug"
    app_version = str(json.loads((APP_ROOT / "macos/bundle.json").read_text())["version"])
    env = {
        **os.environ,
        "CARGO_TARGET_DIR": str(TARGET_ROOT),
        "MACOSX_DEPLOYMENT_TARGET": "14.0",
        "CMAKE_OSX_DEPLOYMENT_TARGET": "14.0",
        "MURMUR_APP_VERSION": app_version,
    }
    capture_command = ["cargo", "build", "--manifest-path", str(APP_ROOT / "sidecars/capture/Cargo.toml"), "-p", CAPTURE_HELPER_NAME]
    if args.release:
        capture_command.append("--release")
    capture_env = env.copy()
    empty_pkg_config = TARGET_ROOT / "capture-empty-pkgconfig"
    empty_pkg_config.mkdir(parents=True, exist_ok=True)
    capture_env.pop("PKG_CONFIG_PATH", None)
    capture_env["PKG_CONFIG_LIBDIR"] = str(empty_pkg_config)
    subprocess.run(capture_command, cwd=APP_ROOT, env=capture_env, check=True)
    worker_target = TARGET_ROOT / "capture-worker-build"
    worker_env = capture_env.copy()
    worker_env.update(
        {
            "MURMUR_CAPTURE_ROLE": "worker",
            "CARGO_TARGET_DIR": str(worker_target),
        }
    )
    subprocess.run(capture_command, cwd=APP_ROOT, env=worker_env, check=True)
    build_capture_agent(profile, env)

    capture_destination = publish_binary(CAPTURE_HELPER_NAME, profile)
    capture_agent_destination = publish_binary(CAPTURE_AGENT_NAME, profile)
    capture_worker_destination = publish_binary(
        CAPTURE_WORKER_NAME,
        profile,
        built_path=worker_target / profile / CAPTURE_HELPER_NAME,
    )
    if args.print_output:
        print(capture_destination)
        print(capture_agent_destination)
        print(capture_worker_destination)
    else:
        print(f"built {capture_destination.relative_to(ROOT)}")
        print(f"built {capture_agent_destination.relative_to(ROOT)}")
        print(f"built {capture_worker_destination.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
