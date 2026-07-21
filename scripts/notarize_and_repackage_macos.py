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
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile


def run(command: list[str], *, cwd: Path | None = None, env: dict | None = None) -> str:
    result = subprocess.run(
        command, cwd=cwd, env=env, text=True, check=True, capture_output=True
    )
    return result.stdout


def notarize(archive: Path, apple_id: str, password: str, team_id: str) -> None:
    output = run(
        [
            "xcrun",
            "notarytool",
            "submit",
            str(archive),
            "--apple-id",
            apple_id,
            "--password",
            password,
            "--team-id",
            team_id,
            "--wait",
            "--output-format",
            "json",
        ]
    )
    status = json.loads(output).get("status")
    if status != "Accepted":
        raise SystemExit(f"notarization was not accepted: status={status!r} ({output})")


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


def tauri_sign(tarball: Path, app_dir: Path, key: str, key_password: str) -> Path:
    run(
        [
            "npx",
            "--no-install",
            "tauri",
            "signer",
            "sign",
            "--private-key",
            key,
            "--password",
            key_password,
            str(tarball),
        ],
        cwd=app_dir,
    )
    signature = Path(f"{tarball}.sig")
    if not signature.is_file():
        raise SystemExit(f"updater signature was not produced: {signature}")
    return signature


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--app", type=Path, required=True)
    parser.add_argument("--app-dir", type=Path, required=True, help="Tauri project dir")
    parser.add_argument("--identity", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--arch", default="aarch64")
    parser.add_argument("--dmg-out", type=Path, required=True)
    parser.add_argument("--apple-id", required=True)
    parser.add_argument("--apple-password", required=True)
    parser.add_argument("--team-id", required=True)
    parser.add_argument("--tauri-signing-key", required=True)
    parser.add_argument("--tauri-signing-password", default="")
    args = parser.parse_args()

    app = args.app.resolve()
    if not app.is_dir():
        raise SystemExit(f"app bundle not found: {app}")

    # 1. Notarize and staple the finalized app.
    with tempfile.TemporaryDirectory(prefix="murmur-notarize-") as tmp:
        app_zip = Path(tmp) / f"{app.stem}.zip"
        run(["ditto", "-c", "-k", "--keepParent", str(app), str(app_zip)])
        notarize(app_zip, args.apple_id, args.apple_password, args.team_id)
    staple(app)

    # 2. DMG built from the stapled app, then signed, notarized, and stapled.
    dmg = build_dmg(app, args.version, args.arch, args.dmg_out.resolve())
    codesign_dmg(dmg, args.identity)
    notarize(dmg, args.apple_id, args.apple_password, args.team_id)
    staple(dmg)

    # 3. Updater archive + Ed25519 signature from the stapled app.
    tarball = build_updater_archive(app)
    signature = tauri_sign(
        tarball, args.app_dir.resolve(), args.tauri_signing_key, args.tauri_signing_password
    )

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
