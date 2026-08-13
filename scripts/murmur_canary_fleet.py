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
PENDING = "pending"
RESULT_FIELDS = ("schemaVersion", "status", "checkedVersion", "offeredVersion", "forced", "dryRun", "stages", "error")
STATUSES = {"pending", "passed", "failed", "dry-run"}
STAGE_VALUES = {PASS, PENDING, "failed"}


class CanaryError(RuntimeError):
    """A bounded, user-actionable canary failure."""


def output(command: list[str], *, cwd: Path | None = None) -> str:
    return subprocess.run(command, cwd=cwd, check=True, capture_output=True, text=True).stdout.strip()


def _validate_result_shape(result: Any) -> None:
    """Validate the app-written result schema and successful OTA contract.

    This function intentionally has no macOS or Fleet dependencies so unit tests
    can exercise the gate's acceptance logic.
    """
    if not isinstance(result, dict):
        raise CanaryError("canary result is not a JSON object")
    missing_fields = [field for field in RESULT_FIELDS if field not in result]
    if missing_fields:
        raise CanaryError("canary result is missing fields: " + ", ".join(missing_fields))
    if type(result["schemaVersion"]) is not int:
        raise CanaryError("schemaVersion must be an integer")
    if result["schemaVersion"] != SCHEMA_VERSION:
        raise CanaryError(f"unsupported canary schemaVersion: {result['schemaVersion']!r}")
    if not isinstance(result["status"], str) or result["status"] not in STATUSES:
        raise CanaryError(f"canary status is invalid: {result.get('status')!r}")
    if not isinstance(result["checkedVersion"], str):
        raise CanaryError("checkedVersion must be a string")
    if result["offeredVersion"] is not None and not isinstance(result["offeredVersion"], str):
        raise CanaryError("offeredVersion must be a string or null")
    if not isinstance(result["forced"], bool):
        raise CanaryError("forced must be a boolean")
    if not isinstance(result["dryRun"], bool):
        raise CanaryError("dryRun must be a boolean")
    if result["error"] is not None and not isinstance(result["error"], str):
        raise CanaryError("error must be a string or null")
    stages = result["stages"]
    if not isinstance(stages, dict):
        raise CanaryError("canary result has no stages object")
    if set(stages) != set(STAGES):
        raise CanaryError("canary stages must contain exactly: " + ", ".join(STAGES))
    invalid = [stage for stage, value in stages.items() if not isinstance(value, str) or value not in STAGE_VALUES]
    if invalid:
        raise CanaryError("canary stages have invalid values: " + ", ".join(invalid))


def evaluate_canary_result(result: Any, expected_previous_version: str, expected_version: str) -> None:
    _validate_result_shape(result)
    if result["status"] != "passed":
        raise CanaryError(f"canary status is {result.get('status')!r}: {result.get('error') or 'no error supplied'}")
    if result["dryRun"]:
        raise CanaryError("dry-run result cannot satisfy the OTA gate")
    if result.get("offeredVersion") != expected_version:
        raise CanaryError(
            f"canary offeredVersion {result.get('offeredVersion')!r} does not match release {expected_version}"
        )
    if result["checkedVersion"] != expected_previous_version:
        raise CanaryError(
            f"canary checkedVersion {result['checkedVersion']!r} does not match previous version {expected_previous_version}"
        )
    if result.get("error") is not None:
        raise CanaryError(f"canary reported an error: {result['error']}")
    missing = [stage for stage in STAGES if result["stages"].get(stage) != PASS]
    if missing:
        raise CanaryError("canary stages did not pass: " + ", ".join(missing))


def evaluate_dry_run_result(result: Any, expected_previous_version: str, expected_version: str) -> None:
    _validate_result_shape(result)
    if result["status"] != "dry-run" or not result["dryRun"]:
        raise CanaryError("canary did not emit an identifiable dry-run result")
    if result["checkedVersion"] != expected_previous_version:
        raise CanaryError("dry-run checkedVersion does not match previous release")
    if result["offeredVersion"] != expected_version:
        raise CanaryError("dry-run offeredVersion does not match release")
    if result["error"] is not None:
        raise CanaryError(f"dry-run reported an error: {result['error']}")
    for stage in ("discover", "policy"):
        if result["stages"][stage] != PASS:
            raise CanaryError(f"dry-run {stage} stage did not pass")
    for stage in STAGES[2:]:
        if result["stages"][stage] != PENDING:
            raise CanaryError(f"dry-run {stage} stage must remain pending")


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
        shutil.rmtree(app_path)

    root.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="murmur-canary-") as temporary:
        archive = Path(temporary) / "Murmur.app.tar.gz"
        subprocess.run(["curl", "--fail", "--silent", "--show-error", "--location", release_asset(repository, tag), "--output", str(archive)], check=True)
        with tarfile.open(archive, "r:gz") as bundle:
            destination = Path(temporary).resolve()
            members = bundle.getmembers()
            roots = set()
            for member in members:
                if member.issym() or member.islnk() or member.isdev():
                    raise CanaryError("release archive contains a link or device entry")
                name = Path(member.name)
                if name.is_absolute() or not name.parts or name.parts[0] != "Murmur.app":
                    raise CanaryError("release archive must contain one Murmur.app root")
                roots.add(name.parts[0])
                target = (destination / member.name).resolve()
                if target != destination and destination not in target.parents:
                    raise CanaryError("release archive contains an unsafe path")
            if roots != {"Murmur.app"}:
                raise CanaryError("release archive must contain one Murmur.app root")
            bundle.extractall(destination)
        extracted = Path(temporary) / "Murmur.app"
        info = extracted / "Contents" / "Info.plist"
        try:
            with info.open("rb") as stream:
                extracted_version = plistlib.load(stream).get("CFBundleShortVersionString")
        except (OSError, plistlib.InvalidFileException, ValueError) as error:
            raise CanaryError("downloaded release has no valid Info.plist") from error
        if extracted_version != tag.removeprefix("v"):
            raise CanaryError(f"downloaded bundle version {extracted_version!r} does not match {tag}")
        if not (extracted / "Contents" / "MacOS" / BUNDLE_EXECUTABLE).is_file():
            raise CanaryError("downloaded release did not contain Murmur.app")
        if app_path.exists():
            shutil.rmtree(app_path)
        shutil.move(str(extracted), str(app_path))
    return app_path, executable.as_posix()


def run_remote(args: argparse.Namespace) -> int:
    tag = args.tag
    expected_version = tag.removeprefix("v")
    root = Path(args.root).expanduser()
    previous = previous_release(args.repository, tag)
    previous_version = previous.removeprefix("v")
    app_path, executable = install_previous_bundle(
        args.repository, previous, root, dry_run=args.dry_run
    )
    result_path = root / "result.json"
    if result_path.exists():
        result_path.unlink()
    command = [executable]
    print(f"Canary source: {previous} ({app_path})", flush=True)
    environment = os.environ.copy()
    environment["MURMUR_UPDATER_CANARY"] = str(result_path)
    if args.dry_run:
        environment["MURMUR_UPDATER_CANARY_DRY_RUN"] = "1"
    print(shlex.join([f"MURMUR_UPDATER_CANARY={result_path}", *(["MURMUR_UPDATER_CANARY_DRY_RUN=1"] if args.dry_run else []), *command]), flush=True)
    process = subprocess.Popen(command, env=environment, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, start_new_session=True)
    try:
        deadline = time.monotonic() + args.timeout
        while time.monotonic() < deadline:
            if result_path.exists():
                try:
                    result = json.loads(result_path.read_text(encoding="utf-8"))
                except (OSError, json.JSONDecodeError):
                    result = None
                if isinstance(result, dict) and result.get("status") in ("passed", "failed", "dry-run"):
                    if args.dry_run:
                        evaluate_dry_run_result(result, previous_version, expected_version)
                    else:
                        evaluate_canary_result(result, previous_version, expected_version)
                    print(json.dumps(result, sort_keys=True), flush=True)
                    return 0
            if process.poll() is not None and not result_path.exists():
                break
            time.sleep(2)
        stderr = process.communicate(timeout=5)[1].decode(errors="replace")
        raise CanaryError(f"timed out waiting for {result_path}; {stderr[-1000:]}")
    finally:
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait(timeout=5)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", required=True, help="published release tag, for example v0.31.4")
    parser.add_argument("--node", default=DEFAULT_NODE)
    parser.add_argument("--repository", default="georgenijo/murmur-app")
    parser.add_argument("--repo", default=DEFAULT_REPO, help=argparse.SUPPRESS)
    parser.add_argument("--root", default=DEFAULT_ROOT, help=argparse.SUPPRESS)
    parser.add_argument("--timeout", type=int, default=DEFAULT_TIMEOUT_SECONDS)
    parser.add_argument("--dry-run", action="store_true", help="launch discovery/policy only; never download or install the update")
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
    outer_timeout = args.timeout + 120
    command = ["fleet", "exec", "--timeout", str(outer_timeout), args.node, "--", shlex.join(remote)]
    print("Running OTA canary on Fleet node", args.node, flush=True)
    return subprocess.run(command, check=False).returncode


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (CanaryError, OSError, subprocess.CalledProcessError) as error:
        print(f"murmur-canary-fleet: {error}", file=sys.stderr)
        raise SystemExit(1)
