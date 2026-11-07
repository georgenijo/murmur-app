#!/usr/bin/env python3
"""Secretless Phase 2–3 identity/update checks. Never signs, launches or publishes.

The checked-in identity snapshot freezes the Tauri inputs at the host switch.
Team IDs below are placeholders; actual installed signatures/TCC require the
signed acceptance checklist. Flip readiness consumes a downloaded JSON inventory
as data and never downloads, modifies or removes a release itself.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import plistlib

from bundle_gpui import ROOT, TAURI, bundle_info
from finalize_macos_bundle import HELPERS

IDENTITY = ROOT / "scripts/gpui-identity-policy.json"
UPDATE_POLICY = ROOT / ".github/gpui-migration-policy.json"
PLISTS = (
    "macos/Info.plist", "entitlements.plist", "capture-agent.entitlements.plist",
    "capture-helper.entitlements.plist", "capture-worker.entitlements.plist",
    "capture-agent-info.plist", "sidecars/capture/Info.plist", "sidecars/capture/WorkerInfo.plist",
    "macos/com.localdictation.capture-agent.plist",
)


def identity_inputs(tauri: Path = TAURI) -> dict:
    config = json.loads((tauri / "tauri.conf.json").read_text())
    mac = config["bundle"]["macOS"]
    return {
        "host_identifier": config["identifier"],
        "helpers": dict(HELPERS),
        "external_bin": json.loads((tauri / "tauri.macos.conf.json").read_text())["bundle"]["externalBin"],
        "info_plist": mac["infoPlist"],
        "entitlements": mac["entitlements"],
        "plists": {name: plistlib.loads((tauri / name).read_bytes()) for name in PLISTS},
    }


def validate_identity(tauri: Path = TAURI, expected: dict | None = None) -> None:
    expected = expected if expected is not None else json.loads(IDENTITY.read_text())
    if identity_inputs(tauri) != expected:
        raise ValueError("Tauri/GPUI identity inputs drifted from the Phase 2 contract")
    info = bundle_info(tauri)
    if info["CFBundleIdentifier"] != expected["host_identifier"]:
        raise ValueError("GPUI host identifier differs from Tauri")
    for key, value in expected["plists"]["macos/Info.plist"].items():
        if info.get(key) != value:
            raise ValueError(f"GPUI permission description differs: {key}")
    for name, template in zip(HELPERS, ("capture-agent-info.plist", "sidecars/capture/Info.plist", "sidecars/capture/WorkerInfo.plist")):
        helper = expected["plists"][template]
        if helper["CFBundleIdentifier"] != HELPERS[name] or helper["CFBundleExecutable"] != name:
            raise ValueError("helper identifier/template mismatch")
        if any(helper[k] != "__MURMUR_VERSION__" for k in ("CFBundleVersion", "CFBundleShortVersionString")):
            raise ValueError("helper version must use the shared placeholder")


def validate_update_policy(policy: dict | None = None, updater: dict | None = None) -> None:
    policy = policy if policy is not None else json.loads(UPDATE_POLICY.read_text())
    updater = updater if updater is not None else json.loads((ROOT / ".github/updater-policy.json").read_text())
    if policy.get("schema") != 1 or policy.get("until_phase") != 4:
        raise ValueError("GPUI additive/downgrade policy must remain through Phase 3")
    if updater != {"min_version": None} or policy.get("min_version", "missing") is not None:
        raise ValueError("host flip must keep min_version null")
    if policy.get("retain_last_tauri_dmg") is not True:
        raise ValueError("last Tauri DMG must remain downloadable")


def validate_flip(manifest: dict, releases: list[dict], notes: str, policy: dict) -> str:
    """Fail closed until the flip explicitly names a published downgrade target.

    Missing min_version in the generated v2 manifest means null to the updater.
    The repository configuration still requires an explicit JSON null.
    """
    validate_update_policy(policy)
    if manifest.get("min_version") is not None:
        raise ValueError("flip manifest must not force an update")
    target = policy.get("last_tauri_release")
    if not isinstance(target, str) or not target or target not in notes:
        raise ValueError("flip notes must name the last Tauri release")
    matches = [r for r in releases if r.get("tag_name") == target]
    if len(matches) != 1 or matches[0].get("draft") is not False or matches[0].get("prerelease") is not False:
        raise ValueError("last Tauri release must remain published and stable")
    prefix = f"https://github.com/georgenijo/murmur-app/releases/download/{target}/"
    assets = [a for a in matches[0].get("assets", [])
              if isinstance(a.get("name"), str) and a["name"].endswith("_aarch64.dmg")
              and a.get("state") == "uploaded" and isinstance(a.get("size"), int) and a["size"] > 0
              and a.get("browser_download_url") == prefix + a["name"]]
    if len(assets) != 1:
        raise ValueError("last Tauri arm64 DMG must remain downloadable")
    return assets[0]["browser_download_url"]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path)
    parser.add_argument("--releases", type=Path)
    parser.add_argument("--notes", type=Path)
    args = parser.parse_args()
    validate_identity()
    validate_update_policy()
    if any((args.manifest, args.releases, args.notes)):
        if not all((args.manifest, args.releases, args.notes)):
            parser.error("flip readiness requires --manifest, --releases and --notes")
        validate_flip(json.loads(args.manifest.read_text()), json.loads(args.releases.read_text()),
                      args.notes.read_text(), json.loads(UPDATE_POLICY.read_text()))
    print("GPUI identity and update policy passed (signed acceptance still required)")


if __name__ == "__main__":
    main()
