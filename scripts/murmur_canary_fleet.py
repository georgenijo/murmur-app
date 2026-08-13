#!/usr/bin/env python3
"""Run Murmur's post-release OTA canary on the trusted Mac mini.

The normal invocation is a small Fleet wrapper.  ``--remote`` is used by that
wrapper on the mini itself, where the previous public bundle is installed in a
dedicated directory, launched with ``MURMUR_UPDATER_CANARY``, and evaluated.
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import plistlib
import shlex
import shutil
import signal
import subprocess
import sys
import tarfile
import tempfile
import time
from typing import Any


DEFAULT_NODE = "mac-mini"
DEFAULT_REPO = "/Users/george-mac-mini/Documents/code/murmur-app"
DEFAULT_ROOT = "/Users/george-mac-mini/Library/Application Support/Murmur OTA Canary"
DEFAULT_TIMEOUT_SECONDS = 900
BUNDLE_EXECUTABLE = "ui"
SCHEMA_VERSION = 1
STAGES = ("discover", "policy", "download", "signatureVerify", "install", "relaunch")
PASS = "passed"


class CanaryError(RuntimeError):
    """A bounded, user-actionable canary failure."""


def output(command: list[str], *, cwd: Path | None = None) -> str:
    return subprocess.run(command, cwd=cwd, check=True, capture_output=True, text=True).stdout.strip()


def evaluate_canary_result(result: Any, expected_version: str) -> None:
    """Validate the app-written result schema and successful OTA contract.

    This function intentionally has no macOS or Fleet dependencies so Linux CI
    can exercise the gate's acceptance logic.
    """
    if not isinstance(result, dict):
        raise CanaryError("canary result is not a JSON object")
    if result.get("schemaVersion") != SCHEMA_VERSION:
        raise CanaryError(f"unsupported canary schemaVersion: {result.get('schemaVersion')!r}")
    if result.get("status") != "passed":
        raise CanaryError(f"canary status is {result.get('status')!r}: {result.get('error') or 'no error supplied'}")
    if result.get("offeredVersion") != expected_version:
        raise CanaryError(
            f"canary offeredVersion {result.get('offeredVersion')!r} does not match release {expected_version}"
        )
    if not isinstance(result.get("checkedVersion"), str) or result["checkedVersion"] == expected_version:
        raise CanaryError("canary did not prove an update from a previous version")
    if result.get("error") is not None:
        raise CanaryError(f"canary reported an error: {result['error']}")
    stages = result.get("stages")
    if not isinstance(stages, dict):
        raise CanaryError("canary result has no stages object")
    missing = [stage for stage in STAGES if stages.get(stage) != PASS]
    if missing:
        raise CanaryError("canary stages did not pass: " + ", ".join(missing))


def release_versions(repository: str) -> list[str]:
    payload = json.loads(output(["gh", "api", f"repos/{repository}/releases?per_page=100"]))
    return [item["tag_name"] for item in payload if not item.get("draft") and not item.get("prerelease")]


def previous_release(repository: str, tag: str) -> str:
    versions = release_versions(repository)
    try:
        index = versions.index(tag)
    except ValueError as error:
        raise CanaryError(f"release tag {tag} is not a published release") from error
    if index + 1 >= len(versions):
        raise CanaryError(f"no previous public release found before {tag}")
    return versions[index + 1]


def release_asset(repository: str, tag: str) -> str:
    payload = json.loads(output(["gh", "api", f"repos/{repository}/releases/tags/{tag}"]))
    for asset in payload.get("assets", []):
        if asset.get("name") == "Murmur.app.tar.gz":
            return asset["browser_download_url"]
    raise CanaryError(f"{tag} has no signed Murmur.app.tar.gz asset")


def install_previous_bundle(
    repository: str, tag: str, root: Path, *, dry_run: bool = False
) -> tuple[Path, str]:
    app_path = root / "Murmur Canary.app"
    executable = app_path / "Contents" / "MacOS" / BUNDLE_EXECUTABLE
    if executable.exists():
        try:
            with (app_path / "Contents" / "Info.plist").open("rb") as info:
                installed_version = plistlib.load(info).get("CFBundleShortVersionString")
        except (OSError, plistlib.InvalidFileException, ValueError):
            installed_version = None
        if installed_version == tag.removeprefix("v"):
            return app_path, executable.as_posix()
        if not dry_run:
            shutil.rmtree(app_path)

    root.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="murmur-canary-") as temporary:
        archive = Path(temporary) / "Murmur.app.tar.gz"
        subprocess.run(["curl", "--fail", "--silent", "--show-error", "--location", release_asset(repository, tag), "--output", str(archive)], check=True)
        with tarfile.open(archive, "r:gz") as bundle:
            destination = Path(temporary).resolve()
            members = bundle.getmembers()
            for member in members:
                target = (destination / member.name).resolve()
                if target != destination and destination not in target.parents:
                    raise CanaryError("release archive contains an unsafe path")
            bundle.extractall(destination)
        extracted = Path(temporary) / "Murmur.app"
        if not (extracted / "Contents" / "MacOS" / BUNDLE_EXECUTABLE).exists():
            raise CanaryError("downloaded release did not contain Murmur.app")
        if dry_run:
            return app_path, executable.as_posix()
        if app_path.exists():
            shutil.rmtree(app_path)
        shutil.move(str(extracted), str(app_path))
    return app_path, executable.as_posix()


def run_remote(args: argparse.Namespace) -> int:
    tag = args.tag
    expected_version = tag.removeprefix("v")
    root = Path(args.root).expanduser()
    previous = previous_release(args.repository, tag)
    app_path, executable = install_previous_bundle(
        args.repository, previous, root, dry_run=args.dry_run
    )
    result_path = root / "result.json"
    if result_path.exists():
        result_path.unlink()
    command = [executable]
    print(f"Canary source: {previous} ({app_path})", flush=True)
    if args.dry_run:
        print("Dry run: bundle prepared; launch/install skipped.", flush=True)
        print("MURMUR_UPDATER_CANARY=" + str(result_path), shlex.join(command), flush=True)
        return 0

    environment = os.environ.copy()
    environment["MURMUR_UPDATER_CANARY"] = str(result_path)
    process = subprocess.Popen(command, env=environment, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, start_new_session=True)
    deadline = time.monotonic() + args.timeout
    while time.monotonic() < deadline:
        if result_path.exists():
            try:
                result = json.loads(result_path.read_text(encoding="utf-8"))
            except (OSError, json.JSONDecodeError):
                result = None
            if isinstance(result, dict) and result.get("status") in ("passed", "failed"):
                try:
                    evaluate_canary_result(result, expected_version)
                except CanaryError as error:
                    if process.poll() is None:
                        os.killpg(process.pid, signal.SIGTERM)
                    print(f"murmur-canary-fleet: {error}", file=sys.stderr)
                    return 1
                print(json.dumps(result, sort_keys=True), flush=True)
                return 0
        if process.poll() is not None and not result_path.exists():
            break
        time.sleep(2)
    if process.poll() is None:
        os.killpg(process.pid, signal.SIGTERM)
    stderr = process.communicate(timeout=5)[1].decode(errors="replace")
    print(f"murmur-canary-fleet: timed out waiting for {result_path}; {stderr[-1000:]}", file=sys.stderr)
    return 1


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", required=True, help="published release tag, for example v0.31.4")
    parser.add_argument("--node", default=DEFAULT_NODE)
    parser.add_argument("--repository", default="georgenijo/murmur-app")
    parser.add_argument("--repo", default=DEFAULT_REPO, help=argparse.SUPPRESS)
    parser.add_argument("--root", default=DEFAULT_ROOT, help=argparse.SUPPRESS)
    parser.add_argument("--timeout", type=int, default=DEFAULT_TIMEOUT_SECONDS)
    parser.add_argument("--dry-run", action="store_true", help="prepare the previous bundle but stop before launch/install")
    parser.add_argument("--remote", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.remote:
        return run_remote(args)

    remote = [
        "python3", f"{args.repo}/scripts/murmur_canary_fleet.py", "--remote",
        "--tag", args.tag, "--repository", args.repository, "--root", args.root,
        "--timeout", str(args.timeout),
    ]
    if args.dry_run:
        remote.append("--dry-run")
    command = ["fleet", "exec", "--timeout", str(args.timeout), args.node, "--", shlex.join(remote)]
    print("Running OTA canary on Fleet node", args.node, flush=True)
    return subprocess.run(command, check=False).returncode


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (CanaryError, OSError, subprocess.CalledProcessError) as error:
        print(f"murmur-canary-fleet: {error}", file=sys.stderr)
        raise SystemExit(1)
