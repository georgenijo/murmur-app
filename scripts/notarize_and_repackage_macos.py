#!/usr/bin/env python3
"""Notarize a finalized Murmur .app and rebuild the DMG and updater archive.

The signed-local-LLM ADR requires the release pipeline to build the app with
``--no-sign``, sign nested code inside-out with per-binary entitlements via
``finalize_macos_bundle.py``, and only then notarize. Because Tauri's own signing
path cannot give the helper its stricter sandbox entitlements, the DMG and the
updater ``.app.tar.gz`` must be produced from the *finalized, notarized* app
rather than from Tauri's stock bundle output. This script performs exactly that
post-finalization repackaging so the release build and the non-publishing signing
rehearsal share one implementation:

  1. notarize the finalized ``.app`` and staple the ticket onto it,
  2. build a DMG from the stapled app, code-sign it, notarize it, and staple it,
  3. tar+gzip the stapled app into ``<AppName>.app.tar.gz`` and produce its
     Ed25519 signature with the Tauri updater signer.

It never weakens ``finalize_macos_bundle.py``; that finalizer must already have
run and verified the bundle before this script is invoked.

Secret handling: Apple and Tauri credentials are read from the environment, never
passed as command-line arguments (which would be visible in ``ps`` and could leak
into a raised traceback). The app-specific password is stored once into a
notarytool keychain profile (fed on stdin), and every ``notarytool submit`` then
references only the profile name. The Tauri updater signer reads its key and
password from ``TAURI_PRIVATE_KEY`` / ``TAURI_PRIVATE_KEY_PASSWORD`` in the child
environment. On failure, subprocess output is dropped and only the leading,
non-sensitive part of the argv is surfaced.

Required environment: ``APPLE_ID``, ``APPLE_PASSWORD``, ``APPLE_TEAM_ID``,
``TAURI_SIGNING_PRIVATE_KEY``. Optional: ``TAURI_SIGNING_PRIVATE_KEY_PASSWORD``
and ``SIGNING_KEYCHAIN`` (path to the keychain that holds the notary profile).
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile


NOTARY_PROFILE = "MurmurNotary"


def run(
    command: list[str],
    *,
    cwd: Path | None = None,
    env: dict | None = None,
    input: str | None = None,
) -> str:
    try:
        result = subprocess.run(
            command,
            cwd=cwd,
            env=env,
            input=input,
            text=True,
            check=True,
            capture_output=True,
        )
    except subprocess.CalledProcessError as exc:
        # Never surface captured output (may echo a secret) and never let the
        # chained CalledProcessError (which holds the full argv) reach the
        # traceback. Show only the leading, non-sensitive part of the command.
        raise SystemExit(
            f"command failed (exit {exc.returncode}): {' '.join(command[:3])}"
        ) from None
    return result.stdout


def store_notary_profile(apple_id: str, team_id: str, password: str, keychain: str) -> None:
    command = [
        "xcrun",
        "notarytool",
        "store-credentials",
        NOTARY_PROFILE,
        "--apple-id",
        apple_id,
        "--team-id",
        team_id,
    ]
    if keychain:
        command += ["--keychain", keychain]
    # The app-specific password is fed on stdin so it never appears in argv/ps.
    run(command, input=password + "\n")


def notarize(archive: Path, keychain: str) -> None:
    command = [
        "xcrun",
        "notarytool",
        "submit",
        str(archive),
        "--keychain-profile",
        NOTARY_PROFILE,
        "--wait",
        "--output-format",
        "json",
    ]
    if keychain:
        command += ["--keychain", keychain]
    output = run(command)
    status = json.loads(output).get("status")
    if status != "Accepted":
        raise SystemExit(f"notarization was not accepted: status={status!r}")


def staple(target: Path) -> None:
    run(["xcrun", "stapler", "staple", str(target)])


def codesign_dmg(dmg: Path, identity: str) -> None:
    run(["codesign", "--force", "--timestamp", "--sign", identity, str(dmg)])


def build_dmg(app: Path, version: str, arch: str, out_dir: Path) -> Path:
    out_dir.mkdir(parents=True, exist_ok=True)
    volume = app.stem  # "Murmur.app" -> "Murmur"
    dmg = out_dir / f"{volume}_{version}_{arch}.dmg"
    if dmg.exists():
        dmg.unlink()
    with tempfile.TemporaryDirectory(prefix="murmur-dmg-stage-") as staging:
        stage = Path(staging)
        shutil.copytree(app, stage / app.name, symlinks=True)
        os.symlink("/Applications", stage / "Applications")
        run(
            [
                "hdiutil",
                "create",
                "-volname",
                volume,
                "-srcfolder",
                str(stage),
                "-ov",
                "-format",
                "UDZO",
                str(dmg),
            ]
        )
    return dmg


def build_updater_archive(app: Path) -> Path:
    macos_dir = app.parent
    tarball = macos_dir / f"{app.name}.tar.gz"
    for stale in (tarball, Path(f"{tarball}.sig")):
        if stale.exists():
            stale.unlink()
    # COPYFILE_DISABLE avoids AppleDouble (._*) members so the archive matches a
    # portable tar; the Tauri updater only requires the .app at the archive root.
    env = os.environ.copy()
    env["COPYFILE_DISABLE"] = "1"
    run(
        ["tar", "-C", str(macos_dir), "-czf", str(tarball), app.name],
        env=env,
    )
    return tarball


def tauri_sign(tarball: Path, app_dir: Path) -> Path:
    # The signer subcommand reads TAURI_PRIVATE_KEY / TAURI_PRIVATE_KEY_PASSWORD
    # from the environment, so the key never appears in argv/ps. Map the release
    # build's TAURI_SIGNING_* names onto the signer's names.
    env = os.environ.copy()
    env["TAURI_PRIVATE_KEY"] = os.environ["TAURI_SIGNING_PRIVATE_KEY"]
    env["TAURI_PRIVATE_KEY_PASSWORD"] = os.environ.get(
        "TAURI_SIGNING_PRIVATE_KEY_PASSWORD", ""
    )
    run(
        ["npx", "--no-install", "tauri", "signer", "sign", str(tarball)],
        cwd=app_dir,
        env=env,
    )
    signature = Path(f"{tarball}.sig")
    if not signature.is_file():
        raise SystemExit(f"updater signature was not produced: {signature}")
    return signature


def _require_env(name: str) -> str:
    value = os.environ.get(name, "")
    if not value:
        raise SystemExit(f"missing required environment variable: {name}")
    return value


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--app", type=Path, required=True)
    parser.add_argument("--app-dir", type=Path, required=True, help="Tauri project dir")
    parser.add_argument("--identity", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--arch", default="aarch64")
    parser.add_argument("--dmg-out", type=Path, required=True)
    args = parser.parse_args()

    # Secrets come from the environment, never argv.
    apple_id = _require_env("APPLE_ID")
    apple_password = _require_env("APPLE_PASSWORD")
    team_id = _require_env("APPLE_TEAM_ID")
    _require_env("TAURI_SIGNING_PRIVATE_KEY")
    keychain = os.environ.get("SIGNING_KEYCHAIN", "")

    app = args.app.resolve()
    if not app.is_dir():
        raise SystemExit(f"app bundle not found: {app}")

    # Store the app-specific password once into a notarytool keychain profile so
    # the repeated submit calls carry only the profile name.
    store_notary_profile(apple_id, team_id, apple_password, keychain)

    # 1. Notarize and staple the finalized app.
    with tempfile.TemporaryDirectory(prefix="murmur-notarize-") as tmp:
        app_zip = Path(tmp) / f"{app.stem}.zip"
        run(["ditto", "-c", "-k", "--keepParent", str(app), str(app_zip)])
        notarize(app_zip, keychain)
    staple(app)

    # 2. DMG built from the stapled app, then signed, notarized, and stapled.
    dmg = build_dmg(app, args.version, args.arch, args.dmg_out.resolve())
    codesign_dmg(dmg, args.identity)
    notarize(dmg, keychain)
    staple(dmg)

    # 3. Updater archive + Ed25519 signature from the stapled app.
    tarball = build_updater_archive(app)
    signature = tauri_sign(tarball, args.app_dir.resolve())

    print(
        json.dumps(
            {
                "app": str(app),
                "dmg": str(dmg),
                "updater_archive": str(tarball),
                "updater_signature": str(signature),
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
