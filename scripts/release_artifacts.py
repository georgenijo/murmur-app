#!/usr/bin/env python3
"""Create and verify immutable Murmur release artifact provenance."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
from typing import Any


SCHEMA_VERSION = 1
SHA_RE = re.compile(r"^[0-9a-f]{40}$")
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
HELPER_FIELDS = (
    "sha256",
    "architecture",
    "designated_requirement",
    "team_id",
    "entitlement_sha256",
)
PLATFORM_SUFFIXES = {
    "macos": (".dmg", ".app.tar.gz", ".app.tar.gz.sig"),
    "linux": (".deb", ".AppImage", ".AppImage.sig"),
}
UPDATER_SUFFIX = {
    "macos": ".app.tar.gz",
    "linux": ".AppImage",
}


class ArtifactError(ValueError):
    """Raised when release artifacts fail closed validation."""


def _require_sha(value: str, label: str = "commit SHA") -> str:
    if not SHA_RE.fullmatch(value):
        raise ArtifactError(f"{label} must be a full lowercase 40-character SHA")
    return value


def _require_run_id(value: str | int) -> int:
    try:
        run_id = int(value)
    except (TypeError, ValueError) as exc:
        raise ArtifactError("workflow run ID must be an integer") from exc
    if run_id <= 0:
        raise ArtifactError("workflow run ID must be positive")
    return run_id


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _files(root: Path) -> list[Path]:
    if not root.is_dir():
        raise ArtifactError(f"artifact directory does not exist: {root}")
    return sorted(
        path for path in root.iterdir() if path.is_file() and path.name != "provenance.json"
    )


def _one_with_suffix(files: list[Path], suffix: str) -> Path:
    matches = [path for path in files if path.name.endswith(suffix)]
    if len(matches) != 1:
        raise ArtifactError(
            f"expected exactly one *{suffix} artifact, found {len(matches)}"
        )
    return matches[0]


def _signature_text(path: Path) -> str:
    value = path.read_text(encoding="utf-8").strip()
    if not value:
        raise ArtifactError(f"updater signature is empty: {path.name}")
    if "\n" in value or "\r" in value:
        raise ArtifactError(f"updater signature must be a single line: {path.name}")
    return value


def _require_helper(helper: dict[str, Any]) -> dict[str, Any]:
    """Validate the shape of the local-LLM helper provenance block.

    The signed-local-LLM ADR requires provenance to additionally record the
    helper hash, architecture, designated requirement, Team ID, and entitlement
    digest. This checks internal consistency; the workflow that records it proves
    those values against the finalized bundle.
    """
    if not isinstance(helper, dict):
        raise ArtifactError("helper provenance must be an object")
    missing = [field for field in HELPER_FIELDS if not str(helper.get(field, "")).strip()]
    if missing:
        raise ArtifactError(f"helper provenance is missing fields: {missing}")
    if not SHA256_RE.fullmatch(str(helper["sha256"])):
        raise ArtifactError("helper sha256 must be a 64-character hex digest")
    if not SHA256_RE.fullmatch(str(helper["entitlement_sha256"])):
        raise ArtifactError("helper entitlement_sha256 must be a 64-character hex digest")
    if str(helper["architecture"]) != "arm64":
        raise ArtifactError(
            f"helper architecture must be arm64, got {helper['architecture']!r}"
        )
    return {field: str(helper[field]) for field in HELPER_FIELDS}


def create_provenance(
    platform: str,
    platform_key: str,
    root: Path,
    commit_sha: str,
    run_id: str | int,
    helper: dict[str, Any] | None = None,
) -> dict[str, Any]:
    if platform not in PLATFORM_SUFFIXES:
        raise ArtifactError(f"unsupported platform: {platform}")
    if not platform_key:
        raise ArtifactError("updater platform key must not be empty")
    commit_sha = _require_sha(commit_sha)
    run_id = _require_run_id(run_id)
    files = _files(root)

    expected = {
        _one_with_suffix(files, suffix).name for suffix in PLATFORM_SUFFIXES[platform]
    }
    actual = {path.name for path in files}
    if actual != expected:
        extras = sorted(actual - expected)
        raise ArtifactError(f"unexpected files in {platform} artifact set: {extras}")

    updater = _one_with_suffix(files, UPDATER_SUFFIX[platform])
    signature = root / f"{updater.name}.sig"
    if signature not in files:
        raise ArtifactError(f"missing updater signature: {signature.name}")
    _signature_text(signature)

    payload: dict[str, Any] = {
        "schema_version": SCHEMA_VERSION,
        "commit_sha": commit_sha,
        "workflow_run_id": run_id,
        "platform": platform,
        "platform_key": platform_key,
        "updater_bundle": updater.name,
        "updater_signature": signature.name,
        "assets": [
            {
                "name": path.name,
                "size": path.stat().st_size,
                "sha256": sha256_file(path),
            }
            for path in files
        ],
    }
    if helper is not None:
        if platform != "macos":
            raise ArtifactError("helper provenance is only recorded for macos")
        payload["helper"] = _require_helper(helper)
    (root / "provenance.json").write_text(
        json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    return payload


def validate_platform(
    platform: str,
    root: Path,
    expected_sha: str,
    expected_run_id: str | int,
    require_helper: bool = False,
) -> dict[str, Any]:
    expected_sha = _require_sha(expected_sha, "expected commit SHA")
    expected_run_id = _require_run_id(expected_run_id)
    provenance_path = root / "provenance.json"
    if not provenance_path.is_file():
        raise ArtifactError(f"missing provenance: {provenance_path}")
    try:
        payload = json.loads(provenance_path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        raise ArtifactError(f"invalid provenance JSON: {provenance_path}") from exc

    expected_fields = {
        "schema_version": SCHEMA_VERSION,
        "commit_sha": expected_sha,
        "workflow_run_id": expected_run_id,
        "platform": platform,
    }
    for field, expected in expected_fields.items():
        if payload.get(field) != expected:
            raise ArtifactError(
                f"{platform} provenance {field} mismatch: "
                f"expected {expected!r}, got {payload.get(field)!r}"
            )
    if not payload.get("platform_key"):
        raise ArtifactError(f"{platform} provenance has an empty platform key")

    files = _files(root)
    declared_assets = payload.get("assets")
    if not isinstance(declared_assets, list):
        raise ArtifactError(f"{platform} provenance assets must be a list")
    declared_names = [entry.get("name") for entry in declared_assets]
    actual_names = [path.name for path in files]
    if declared_names != actual_names:
        raise ArtifactError(
            f"{platform} artifact names differ from signed provenance: "
            f"declared={declared_names!r}, actual={actual_names!r}"
        )

    for entry, path in zip(declared_assets, files):
        if entry.get("size") != path.stat().st_size:
            raise ArtifactError(f"artifact size mismatch: {path.name}")
        if entry.get("sha256") != sha256_file(path):
            raise ArtifactError(f"artifact SHA-256 mismatch: {path.name}")

    updater_name = payload.get("updater_bundle")
    signature_name = payload.get("updater_signature")
    if signature_name != f"{updater_name}.sig":
        raise ArtifactError(f"{platform} updater/signature filenames do not match")
    updater = root / str(updater_name)
    signature = root / str(signature_name)
    if updater not in files or signature not in files:
        raise ArtifactError(f"{platform} updater files are absent from the artifact set")

    helper = payload.get("helper")
    if helper is not None:
        payload["helper"] = _require_helper(helper)
    elif require_helper:
        raise ArtifactError(
            f"{platform} provenance is missing the required local-LLM helper block"
        )

    payload["signature"] = _signature_text(signature)
    return payload


def validate_release(
    artifacts_root: Path,
    expected_sha: str,
    expected_run_id: str | int,
    output: Path | None = None,
    require_macos_helper: bool = False,
) -> dict[str, Any]:
    platforms = {
        platform: validate_platform(
            platform,
            artifacts_root / platform,
            expected_sha,
            expected_run_id,
            require_helper=(require_macos_helper and platform == "macos"),
        )
        for platform in ("macos", "linux")
    }
    names: list[str] = []
    for payload in platforms.values():
        names.extend(entry["name"] for entry in payload["assets"])
    if len(names) != len(set(names)):
        raise ArtifactError("release artifacts contain duplicate asset basenames")

    result = {
        "schema_version": SCHEMA_VERSION,
        "commit_sha": _require_sha(expected_sha),
        "workflow_run_id": _require_run_id(expected_run_id),
        "platforms": platforms,
    }
    if output is not None:
        output.write_text(
            json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )
    return result


def write_updater_manifests(
    validated_path: Path,
    tag: str,
    repository: str,
    bridge_url: str,
    bridge_signature: str,
    output_dir: Path,
) -> tuple[Path, Path]:
    if not re.fullmatch(r"v\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?", tag):
        raise ArtifactError(f"invalid release tag: {tag}")
    if not re.fullmatch(r"[^/\s]+/[^/\s]+", repository):
        raise ArtifactError(f"invalid repository: {repository}")
    bridge_signature = bridge_signature.strip()
    if not bridge_url.startswith("https://") or not bridge_signature:
        raise ArtifactError("bridge updater URL and signature are required")

    validated = json.loads(validated_path.read_text(encoding="utf-8"))
    macos = validated["platforms"]["macos"]
    linux = validated["platforms"]["linux"]
    version = tag[1:]
    base_url = f"https://github.com/{repository}/releases/download/{tag}"
    pub_date = "${PUB_DATE}"

    modern = {
        "version": version,
        "pub_date": pub_date,
        "platforms": {
            macos["platform_key"]: {
                "url": f"{base_url}/{macos['updater_bundle']}",
                "signature": macos["signature"],
            },
            linux["platform_key"]: {
                "url": f"{base_url}/{linux['updater_bundle']}",
                "signature": linux["signature"],
            },
        },
        "notes": f"See release notes at https://github.com/{repository}/releases/tag/{tag}",
    }
    legacy = {
        "version": version,
        "pub_date": pub_date,
        "platforms": {
            macos["platform_key"]: {
                "url": bridge_url,
                "signature": bridge_signature,
            },
            linux["platform_key"]: {
                "url": f"{base_url}/{linux['updater_bundle']}",
                "signature": linux["signature"],
            },
        },
        "notes": (
            "Compatibility bridge for existing macOS installs. "
            "Murmur will offer the current release after relaunch."
        ),
    }

    output_dir.mkdir(parents=True, exist_ok=True)
    modern_path = output_dir / "latest-v2.json"
    legacy_path = output_dir / "latest.json"
    modern_path.write_text(json.dumps(modern, indent=2) + "\n", encoding="utf-8")
    legacy_path.write_text(json.dumps(legacy, indent=2) + "\n", encoding="utf-8")
    return modern_path, legacy_path


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    record = subparsers.add_parser("record")
    record.add_argument("--platform", choices=sorted(PLATFORM_SUFFIXES), required=True)
    record.add_argument("--platform-key", required=True)
    record.add_argument("--artifacts", type=Path, required=True)
    record.add_argument("--commit-sha", required=True)
    record.add_argument("--run-id", required=True)
    record.add_argument("--helper-sha256")
    record.add_argument("--helper-arch")
    record.add_argument("--helper-designated-requirement")
    record.add_argument("--helper-team-id")
    record.add_argument("--helper-entitlement-sha256")

    validate = subparsers.add_parser("validate")
    validate.add_argument("--artifacts", type=Path, required=True)
    validate.add_argument("--expected-sha", required=True)
    validate.add_argument("--expected-run-id", required=True)
    validate.add_argument("--output", type=Path, required=True)
    validate.add_argument(
        "--require-macos-helper",
        action="store_true",
        help="fail unless the macOS provenance records the local-LLM helper block",
    )

    manifests = subparsers.add_parser("manifests")
    manifests.add_argument("--validated", type=Path, required=True)
    manifests.add_argument("--tag", required=True)
    manifests.add_argument("--repository", required=True)
    manifests.add_argument("--bridge-url", required=True)
    manifests.add_argument("--bridge-signature", required=True)
    manifests.add_argument("--output-dir", type=Path, required=True)
    return parser


def _helper_from_args(args: argparse.Namespace) -> dict[str, Any] | None:
    provided = {
        "sha256": args.helper_sha256,
        "architecture": args.helper_arch,
        "designated_requirement": args.helper_designated_requirement,
        "team_id": args.helper_team_id,
        "entitlement_sha256": args.helper_entitlement_sha256,
    }
    supplied = {key: value for key, value in provided.items() if value}
    if not supplied:
        return None
    if len(supplied) != len(provided):
        missing = sorted(set(provided) - set(supplied))
        raise ArtifactError(f"incomplete helper provenance arguments: missing {missing}")
    return supplied


def main() -> None:
    args = _parser().parse_args()
    try:
        if args.command == "record":
            helper = _helper_from_args(args)
            payload = create_provenance(
                args.platform,
                args.platform_key,
                args.artifacts,
                args.commit_sha,
                args.run_id,
                helper=helper,
            )
            print(
                f"recorded {args.platform} provenance for {payload['commit_sha']} "
                f"(run {payload['workflow_run_id']})"
            )
        elif args.command == "validate":
            payload = validate_release(
                args.artifacts,
                args.expected_sha,
                args.expected_run_id,
                args.output,
                require_macos_helper=args.require_macos_helper,
            )
            print(
                f"validated immutable release artifacts for {payload['commit_sha']} "
                f"(run {payload['workflow_run_id']})"
            )
        else:
            modern, legacy = write_updater_manifests(
                args.validated,
                args.tag,
                args.repository,
                args.bridge_url,
                args.bridge_signature,
                args.output_dir,
            )
            print(f"wrote updater manifests: {modern}, {legacy}")
    except ArtifactError as exc:
        raise SystemExit(f"ERROR: {exc}") from exc


if __name__ == "__main__":
    main()
