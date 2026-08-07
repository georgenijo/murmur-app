#!/usr/bin/env python3
"""Prepare and validate Murmur's synchronized release version surfaces."""

from __future__ import annotations

import argparse
from datetime import date
import json
from pathlib import Path
import re
import subprocess
import sys
from typing import Callable


ROOT = Path(__file__).resolve().parents[1]
SEMVER = re.compile(
    r"^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$"
)
PATHS = {
    "tauri": Path("app/src-tauri/tauri.conf.json"),
    "cargo": Path("app/src-tauri/Cargo.toml"),
    "lock": Path("app/src-tauri/Cargo.lock"),
    "package": Path("app/package.json"),
    "package_lock": Path("app/package-lock.json"),
    "changelog": Path("CHANGELOG.md"),
}


class ReleaseVersionError(ValueError):
    """A release version surface is missing, malformed, or inconsistent."""


def _read(root: Path, relative: Path, git_ref: str | None = None) -> str:
    if git_ref is None:
        return (root / relative).read_text()
    try:
        return subprocess.check_output(
            ["git", "-C", str(root), "show", f"{git_ref}:{relative.as_posix()}"],
            text=True,
            stderr=subprocess.PIPE,
        )
    except subprocess.CalledProcessError as error:
        detail = error.stderr.strip() or str(error)
        raise ReleaseVersionError(
            f"could not read {relative} at {git_ref}: {detail}"
        ) from error


def _cargo_package_version(text: str, path: Path) -> str:
    package = re.search(r"^\[package\]\s*(.*?)(?=^\[|\Z)", text, re.M | re.S)
    if package is None:
        raise ReleaseVersionError(f"{path}: missing [package] block")
    version = re.search(r'^version\s*=\s*"([^"]+)"', package.group(1), re.M)
    if version is None:
        raise ReleaseVersionError(f"{path}: missing package version")
    return version.group(1)


def _cargo_lock_version(text: str, path: Path) -> str:
    for block in re.split(r"^\[\[package\]\]\s*$", text, flags=re.M):
        if re.search(r'^name\s*=\s*"ui"\s*$', block, re.M):
            version = re.search(r'^version\s*=\s*"([^"]+)"', block, re.M)
            if version is None:
                break
            return version.group(1)
    raise ReleaseVersionError(f"{path}: missing ui package version")


def _json(text: str, path: Path) -> dict:
    try:
        value = json.loads(text)
    except json.JSONDecodeError as error:
        raise ReleaseVersionError(f"{path}: invalid JSON: {error}") from error
    if not isinstance(value, dict):
        raise ReleaseVersionError(f"{path}: expected a JSON object")
    return value


def _package_lock_version(value: dict, path: Path) -> str:
    root_package = value.get("packages", {}).get("", {})
    top = value.get("version")
    nested = root_package.get("version")
    if not isinstance(top, str) or not isinstance(nested, str):
        raise ReleaseVersionError(f"{path}: missing root package versions")
    if top != nested:
        raise ReleaseVersionError(
            f"{path}: top-level version {top!r} differs from root package {nested!r}"
        )
    return top


def _latest_changelog_version(text: str, path: Path) -> str:
    headings = list(
        re.finditer(
            r"^## \[([^\]]+)\](?: - (\d{4}-\d{2}-\d{2}))?\s*$",
            text,
            re.M,
        )
    )
    if not headings or headings[0].group(1) != "Unreleased":
        raise ReleaseVersionError(f"{path}: missing top-level [Unreleased] section")
    if len(headings) < 2 or headings[1].group(2) is None:
        raise ReleaseVersionError(
            f"{path}: [Unreleased] must be followed by a dated release section"
        )
    try:
        date.fromisoformat(headings[1].group(2))
    except ValueError as error:
        raise ReleaseVersionError(
            f"{path}: invalid release date {headings[1].group(2)!r}"
        ) from error
    return headings[1].group(1)


def release_versions(
    root: Path = ROOT, git_ref: str | None = None
) -> dict[str, str]:
    tauri_path = PATHS["tauri"]
    cargo_path = PATHS["cargo"]
    lock_path = PATHS["lock"]
    package_path = PATHS["package"]
    package_lock_path = PATHS["package_lock"]
    changelog_path = PATHS["changelog"]

    tauri = _json(_read(root, tauri_path, git_ref), tauri_path)
    package = _json(_read(root, package_path, git_ref), package_path)
    package_lock = _json(
        _read(root, package_lock_path, git_ref), package_lock_path
    )
    versions = {
        "tauri.conf.json": tauri.get("version"),
        "Cargo.toml": _cargo_package_version(
            _read(root, cargo_path, git_ref), cargo_path
        ),
        "Cargo.lock": _cargo_lock_version(
            _read(root, lock_path, git_ref), lock_path
        ),
        "package.json": package.get("version"),
        "package-lock.json": _package_lock_version(
            package_lock, package_lock_path
        ),
        "CHANGELOG.md": _latest_changelog_version(
            _read(root, changelog_path, git_ref), changelog_path
        ),
    }
    for surface, version in versions.items():
        if not isinstance(version, str):
            raise ReleaseVersionError(f"{surface}: missing version")
    return versions


def check_release(
    expected: str | None = None,
    *,
    root: Path = ROOT,
    git_ref: str | None = None,
) -> str:
    versions = release_versions(root, git_ref)
    expected = expected or versions["tauri.conf.json"]
    if not SEMVER.fullmatch(expected):
        raise ReleaseVersionError(f"invalid release version: {expected}")
    mismatches = {
        surface: version
        for surface, version in versions.items()
        if version != expected
    }
    if mismatches:
        detail = ", ".join(
            f"{surface}={version}" for surface, version in mismatches.items()
        )
        raise ReleaseVersionError(
            f"release versions differ from expected {expected}: {detail}"
        )
    return expected


def _replace_package_version(text: str, version: str, path: Path) -> str:
    package = re.search(r"^\[package\]\s*(.*?)(?=^\[|\Z)", text, re.M | re.S)
    if package is None:
        raise ReleaseVersionError(f"{path}: missing [package] block")
    block = package.group(0)
    replaced, count = re.subn(
        r'(^version\s*=\s*")[^"]+(")',
        rf"\g<1>{version}\g<2>",
        block,
        count=1,
        flags=re.M,
    )
    if count != 1:
        raise ReleaseVersionError(f"{path}: missing package version")
    return text[: package.start()] + replaced + text[package.end() :]


def _replace_lock_version(text: str, version: str, path: Path) -> str:
    blocks = list(re.finditer(r"^\[\[package\]\]\s*$", text, re.M))
    for index, marker in enumerate(blocks):
        end = blocks[index + 1].start() if index + 1 < len(blocks) else len(text)
        block = text[marker.start() : end]
        if not re.search(r'^name\s*=\s*"ui"\s*$', block, re.M):
            continue
        replaced, count = re.subn(
            r'(^version\s*=\s*")[^"]+(")',
            rf"\g<1>{version}\g<2>",
            block,
            count=1,
            flags=re.M,
        )
        if count != 1:
            break
        return text[: marker.start()] + replaced + text[end:]
    raise ReleaseVersionError(f"{path}: missing ui package version")


def _cut_changelog(text: str, version: str, release_date: str, path: Path) -> str:
    _latest_changelog_version(text, path)
    if re.search(rf"^## \[{re.escape(version)}\](?:\s|$)", text, re.M):
        raise ReleaseVersionError(f"{path}: release section {version} already exists")
    unreleased = re.search(
        r"^## \[Unreleased\]\s*\n(?P<body>.*?)(?=^## \[)",
        text,
        re.M | re.S,
    )
    if unreleased is None:
        raise ReleaseVersionError(f"{path}: missing [Unreleased] section")
    if not unreleased.group("body").strip():
        raise ReleaseVersionError(f"{path}: [Unreleased] has no release notes")
    return (
        text[: unreleased.start()]
        + "## [Unreleased]\n\n"
        + f"## [{version}] - {release_date}\n\n"
        + unreleased.group("body").lstrip("\n")
        + text[unreleased.end() :]
    )


def prepare_release(
    version: str,
    release_date: str,
    *,
    root: Path = ROOT,
) -> None:
    if not SEMVER.fullmatch(version):
        raise ReleaseVersionError(f"invalid release version: {version}")
    try:
        date.fromisoformat(release_date)
    except ValueError as error:
        raise ReleaseVersionError(f"invalid release date: {release_date}") from error

    originals = {name: _read(root, path) for name, path in PATHS.items()}
    tauri = _json(originals["tauri"], PATHS["tauri"])
    package = _json(originals["package"], PATHS["package"])
    package_lock = _json(originals["package_lock"], PATHS["package_lock"])

    tauri["version"] = version
    package["version"] = version
    package_lock["version"] = version
    root_package = package_lock.get("packages", {}).get("")
    if not isinstance(root_package, dict):
        raise ReleaseVersionError(
            f"{PATHS['package_lock']}: missing root package metadata"
        )
    root_package["version"] = version

    updated = {
        "tauri": json.dumps(tauri, indent=2) + "\n",
        "cargo": _replace_package_version(
            originals["cargo"], version, PATHS["cargo"]
        ),
        "lock": _replace_lock_version(originals["lock"], version, PATHS["lock"]),
        "package": json.dumps(package, indent=2) + "\n",
        "package_lock": json.dumps(package_lock, indent=2) + "\n",
        "changelog": _cut_changelog(
            originals["changelog"], version, release_date, PATHS["changelog"]
        ),
    }
    for name, content in updated.items():
        (root / PATHS[name]).write_text(content)
    check_release(version, root=root)


def _run(action: Callable[[], str | None]) -> int:
    try:
        result = action()
    except (OSError, ReleaseVersionError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 1
    if result is not None:
        print(result)
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    prepare = subparsers.add_parser(
        "prepare", help="bump every version surface and cut [Unreleased]"
    )
    prepare.add_argument("version")
    prepare.add_argument("--date", default=date.today().isoformat())
    check = subparsers.add_parser(
        "check", help="verify every version surface at the worktree or a git ref"
    )
    check.add_argument("version", nargs="?")
    check.add_argument("--git-ref")
    args = parser.parse_args()

    if args.command == "prepare":
        return _run(lambda: prepare_release(args.version, args.date))
    return _run(lambda: check_release(args.version, git_ref=args.git_ref))


if __name__ == "__main__":
    raise SystemExit(main())
