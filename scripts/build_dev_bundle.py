#!/usr/bin/env python3
"""Build and sign a local Murmur dev bundle with an optional stable identity.

macOS stores each Microphone and Accessibility grant with the app's designated
requirement. An ad-hoc signature's requirement is its code hash, which changes
on every rebuild, so each ad-hoc dev bundle is a new app to TCC. Signing with a
keychain identity gives every rebuild the same requirement, so grants survive.

Set ``MURMUR_DEV_SIGNING_IDENTITY`` to a code-signing identity name, a unique
part of one, or its SHA-1 hash (see ``security find-identity -v -p
codesigning``). When it is unset, ``-``, or not a valid identity in the
keychain, the bundle is ad-hoc signed as before. ``--require-identity`` turns
the missing-identity fallback into an error.

The bundle is built unsigned and signed by ``finalize_macos_bundle.py``, the
same finalizer the release uses, so every nested binary gets its own
entitlements and fixed identifier. This script never runs in CI or release
workflows and refuses the production bundle identifier.
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import json
import os
from pathlib import Path
import plistlib
import re
import subprocess
import sys
from typing import Callable, Mapping, Sequence


ROOT = Path(__file__).resolve().parents[1]
APP_DIR = ROOT / "app"
TAURI_ROOT = APP_DIR / "src-tauri"
DEFAULT_CONFIG = TAURI_ROOT / "tauri.dev.conf.json"
BASE_CONFIG = TAURI_ROOT / "tauri.conf.json"
PRODUCTION_IDENTIFIER = "com.localdictation"
IDENTITY_ENV = "MURMUR_DEV_SIGNING_IDENTITY"
AD_HOC = "-"
BUNDLED_EXECUTABLES = (
    "murmur-capture-agent",
    "murmur-capture-helper",
    "murmur-capture-worker",
)

_IDENTITY_LINE = re.compile(r'^\s*\d+\)\s+([0-9A-Fa-f]{40})\s+"(.+)"\s*$')
_SHA1 = re.compile(r"^[0-9A-Fa-f]{40}$")
_TEAM_ID = re.compile(r"^[A-Z0-9]{10}$")


class DevSigningError(Exception):
    """A dev-signing configuration problem the caller must fix."""


@dataclass(frozen=True)
class Identity:
    sha1: str
    name: str


@dataclass(frozen=True)
class Selection:
    """The identity passed to codesign, and why it was chosen."""

    codesign_identity: str
    label: str
    reason: str

    @property
    def ad_hoc(self) -> bool:
        return self.codesign_identity == AD_HOC


def parse_identities(find_identity_output: str) -> list[Identity]:
    """Parse ``security find-identity -v -p codesigning`` output."""
    identities = []
    for line in find_identity_output.splitlines():
        match = _IDENTITY_LINE.match(line)
        if match:
            identities.append(Identity(match.group(1).upper(), match.group(2)))
    return identities


def select_identity(
    requested: str | None,
    identities: Sequence[Identity] | None,
    *,
    require: bool = False,
) -> Selection:
    """Pick the codesign identity for a dev bundle.

    ``identities`` is ``None`` when the keychain could not be queried. Matching
    follows codesign: a full SHA-1 hash, an exact name, or a unique substring of
    a name. The selection always names the matched certificate by hash so that
    codesign cannot pick a different certificate with the same name.
    """
    value = (requested or "").strip()
    if not value or value == AD_HOC:
        if require:
            raise DevSigningError(
                f"{IDENTITY_ENV} is not set; --require-identity needs a signing identity"
            )
        return Selection(AD_HOC, "ad-hoc", f"{IDENTITY_ENV} is not set")

    if identities is None:
        matches: list[Identity] = []
        reason = "the keychain could not be queried"
    elif _SHA1.match(value):
        matches = [identity for identity in identities if identity.sha1 == value.upper()]
        reason = f"no valid code-signing identity has hash {value}"
    else:
        exact = [identity for identity in identities if identity.name == value]
        matches = exact or [identity for identity in identities if value in identity.name]
        reason = f"no valid code-signing identity matches {value!r}"

    unique = {identity.sha1: identity for identity in matches}
    if len(unique) > 1:
        choices = ", ".join(
            f"{identity.sha1} {identity.name!r}" for identity in unique.values()
        )
        raise DevSigningError(
            f"{IDENTITY_ENV}={value!r} matches several identities; "
            f"set it to one hash: {choices}"
        )
    if not unique:
        if require:
            raise DevSigningError(f"{reason}; --require-identity forbids ad-hoc fallback")
        return Selection(AD_HOC, "ad-hoc", f"{reason}; falling back to ad-hoc")
    identity = next(iter(unique.values()))
    return Selection(identity.sha1, f"{identity.name} ({identity.sha1})", f"{IDENTITY_ENV}")


def load_config(config: Path) -> dict[str, object]:
    with config.open(encoding="utf-8") as handle:
        payload = json.load(handle)
    if not isinstance(payload, dict):
        raise DevSigningError(f"{config} is not a JSON object")
    return payload


def bundle_settings(config: Path, base_config: Path = BASE_CONFIG) -> tuple[str, str]:
    """Return the (identifier, productName) the merged config builds."""
    base = load_config(base_config)
    overlay = load_config(config)
    identifier = str(overlay.get("identifier", base.get("identifier", "")))
    product_name = str(overlay.get("productName", base.get("productName", "")))
    if not identifier or identifier == PRODUCTION_IDENTIFIER:
        raise DevSigningError(
            f"{config.name} builds the production identifier {PRODUCTION_IDENTIFIER!r}; "
            "dev signing only signs dev and test bundles. "
            "Release builds use the release workflow."
        )
    if not product_name:
        raise DevSigningError(f"{config.name} has no productName")
    return identifier, product_name


def target_dir(env: Mapping[str, str]) -> Path:
    configured = env.get("CARGO_TARGET_DIR")
    if not configured:
        return TAURI_ROOT / "target"
    path = Path(configured)
    # Tauri runs cargo from app/src-tauri, so a relative target is relative to it.
    return path if path.is_absolute() else TAURI_ROOT / path


def bundle_path(product_name: str, *, debug: bool, env: Mapping[str, str]) -> Path:
    profile = "debug" if debug else "release"
    return target_dir(env) / profile / "bundle" / "macos" / f"{product_name}.app"


def helper_build_command(*, debug: bool) -> list[str]:
    command = [sys.executable, str(ROOT / "scripts" / "build_capture_helpers.py")]
    if not debug:
        command.append("--release")
    return command


def tauri_build_command(config: Path, *, debug: bool) -> list[str]:
    # --no-sign also ignores APPLE_SIGNING_IDENTITY in the environment, so a
    # stray release variable can never sign this bundle through Tauri.
    command = ["npx", "tauri", "build", "--no-sign", "--bundles", "app", "--config", str(config)]
    if debug:
        command.append("--debug")
    return command


def finalize_command(app: Path, identity: str) -> list[str]:
    return [
        sys.executable,
        str(ROOT / "scripts" / "finalize_macos_bundle.py"),
        "--app",
        str(app),
        "--identity",
        identity,
        "--main-entitlements",
        str(TAURI_ROOT / "entitlements.plist"),
        "--capture-agent-entitlements",
        str(TAURI_ROOT / "capture-agent.entitlements.plist"),
        "--capture-agent-info-plist",
        str(TAURI_ROOT / "capture-agent-info.plist"),
        "--capture-helper-info-plist",
        str(TAURI_ROOT / "sidecars" / "capture" / "Info.plist"),
        "--capture-worker-info-plist",
        str(TAURI_ROOT / "sidecars" / "capture" / "WorkerInfo.plist"),
        "--capture-agent-launchd-plist",
        str(TAURI_ROOT / "macos" / "com.localdictation.capture-agent.plist"),
        "--capture-helper-entitlements",
        str(TAURI_ROOT / "capture-helper.entitlements.plist"),
        "--capture-worker-entitlements",
        str(TAURI_ROOT / "capture-worker.entitlements.plist"),
    ]


def team_identifier(codesign_details: str) -> str | None:
    """Return the TeamIdentifier from ``codesign -dvv`` output, if any."""
    for line in codesign_details.splitlines():
        if line.startswith("TeamIdentifier="):
            value = line.split("=", 1)[1].strip()
            return value if _TEAM_ID.match(value) else None
    return None


def designated_requirement(codesign_requirements: str) -> str | None:
    for line in codesign_requirements.splitlines():
        if line.startswith("designated => "):
            return line.split("=> ", 1)[1].strip()
    return None


def team_warnings(selection: Selection, teams: Mapping[str, str | None], *, debug: bool) -> list[str]:
    """Explain signatures the app's bundled-helper validation will reject.

    Release-profile builds validate each helper before spawning it: the helper
    must chain to Apple and carry the main executable's Team ID. Ad-hoc and
    self-signed identities have no Team ID, so recording fails with
    ``signature_invalid``. Debug-profile builds skip that check.
    """
    if debug:
        return []
    values = set(teams.values())
    if None in values:
        kind = "ad-hoc" if selection.ad_hoc else "this identity (no Apple Team ID)"
        return [
            f"warning: {kind} signing cannot pass the bundled-helper Team ID check in a "
            "release-profile build; dictation fails with signature_invalid. Use an "
            "Apple Development identity, or --debug."
        ]
    if len(values) != 1:
        return [f"warning: bundled executables have different Team IDs: {dict(teams)!r}"]
    return []


Runner = Callable[..., subprocess.CompletedProcess[str]]


def query_identities(run: Runner) -> list[Identity] | None:
    try:
        result = run(
            ["security", "find-identity", "-v", "-p", "codesigning"],
            text=True,
            capture_output=True,
            check=False,
        )
    except OSError:
        return None
    if result.returncode != 0:
        return None
    return parse_identities(result.stdout)


def signature_text(run: Runner, arguments: list[str]) -> str:
    result = run(["codesign", *arguments], text=True, capture_output=True, check=True)
    return result.stdout + result.stderr


def main(
    argv: Sequence[str] | None = None,
    *,
    env: Mapping[str, str] | None = None,
    run: Runner = subprocess.run,
    platform: str = sys.platform,
) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--config",
        type=Path,
        default=DEFAULT_CONFIG,
        help="Tauri config overlay (default: app/src-tauri/tauri.dev.conf.json)",
    )
    parser.add_argument("--debug", action="store_true", help="build the debug profile")
    parser.add_argument(
        "--skip-helpers",
        action="store_true",
        help="reuse the helper binaries already in app/src-tauri/binaries",
    )
    parser.add_argument(
        "--require-identity",
        action="store_true",
        help=f"fail instead of falling back to ad-hoc when {IDENTITY_ENV} does not resolve",
    )
    parser.add_argument(
        "--sign-only",
        action="store_true",
        help="re-sign the existing bundle without building",
    )
    args = parser.parse_args(argv)
    env = os.environ if env is None else env

    try:
        config = args.config.resolve()
        _identifier, product_name = bundle_settings(config)
        if platform != "darwin":
            raise DevSigningError("dev bundles can only be built and signed on macOS")
        selection = select_identity(
            env.get(IDENTITY_ENV),
            query_identities(run),
            require=args.require_identity,
        )
    except DevSigningError as error:
        print(f"error: {error}", file=sys.stderr)
        return 2

    print(f"dev signing: {selection.label} ({selection.reason})")
    if selection.ad_hoc:
        print(
            "dev signing: ad-hoc bundles get a new signature on every rebuild, so macOS "
            "asks for Microphone and Accessibility again. See docs/DEVELOPMENT.md."
        )

    app = bundle_path(product_name, debug=args.debug, env=env)
    if not args.sign_only:
        if not args.skip_helpers:
            run(helper_build_command(debug=args.debug), cwd=ROOT, check=True)
        run(tauri_build_command(config, debug=args.debug), cwd=APP_DIR, check=True)
    if not app.is_dir():
        print(f"error: bundle not found at {app}", file=sys.stderr)
        return 1

    run(finalize_command(app, selection.codesign_identity), cwd=ROOT, check=True)

    macos = app / "Contents" / "MacOS"
    with (app / "Contents" / "Info.plist").open("rb") as handle:
        main_name = str(plistlib.load(handle)["CFBundleExecutable"])
    teams = {
        name: team_identifier(signature_text(run, ["-dvv", str(macos / name)]))
        for name in (main_name, *BUNDLED_EXECUTABLES)
    }
    for warning in team_warnings(selection, teams, debug=args.debug):
        print(warning, file=sys.stderr)
    requirement = designated_requirement(signature_text(run, ["-d", "-r-", str(app)]))
    print(f"team id: {teams[main_name] or 'none'}")
    print(f"designated requirement: {requirement or 'unknown'}")
    print(f"DEV_APP={app}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
