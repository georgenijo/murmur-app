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
TEAM_ID_RE = re.compile(r"^[A-Z0-9]{10}$")
LLM_HELPER_IDENTIFIER = "com.localdictation.local-llm-sidecar"
CAPTURE_AGENT_IDENTIFIER = "com.localdictation.capture-agent"
CAPTURE_WORKER_IDENTIFIER = "com.localdictation.capture-worker"
CAPTURE_HELPER_IDENTIFIER = "com.localdictation.capture-helper"
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


def _require_helper(
    helper: dict[str, Any], identifier: str, label: str
) -> dict[str, Any]:
    """Validate the shape of one signed helper provenance block.

    The signed-local-LLM ADR requires provenance to additionally record the
    helper hash, architecture, designated requirement, Team ID, and entitlement
    digest. This checks internal consistency; the workflow that records it proves
    those values against the finalized bundle.
    """
    if not isinstance(helper, dict):
        raise ArtifactError(f"{label} provenance must be an object")
    missing = [field for field in HELPER_FIELDS if not str(helper.get(field, "")).strip()]
    if missing:
        raise ArtifactError(f"{label} provenance is missing fields: {missing}")
    if not SHA256_RE.fullmatch(str(helper["sha256"])):
        raise ArtifactError(f"{label} sha256 must be a 64-character hex digest")
    if not SHA256_RE.fullmatch(str(helper["entitlement_sha256"])):
        raise ArtifactError(
            f"{label} entitlement_sha256 must be a 64-character hex digest"
        )
    if str(helper["architecture"]) != "arm64":
        raise ArtifactError(
            f"{label} architecture must be arm64, got {helper['architecture']!r}"
        )

    team_id = str(helper["team_id"])
    if not TEAM_ID_RE.fullmatch(team_id):
        raise ArtifactError(
            f"{label} team_id must be a 10-character Apple Team ID, got {team_id!r}"
        )

    # The designated requirement must pin the fixed helper identifier to a real
    # Apple-anchored Developer ID certificate for this exact Team ID. This rejects
    # bare-cdhash ad-hoc requirements, which carry no identity or anchor.
    dr = str(helper["designated_requirement"])
    clause_start = r"(?:^|\band\s+)"
    clause_end = r"(?=\s*(?:and\b|$))"
    identifier_clause = re.compile(
        rf'{clause_start}identifier\s+"{re.escape(identifier)}"{clause_end}'
    )
    anchor_clause = re.compile(
        rf"{clause_start}anchor\s+apple\s+generic{clause_end}"
    )
    team_clause = re.compile(
        rf"{clause_start}certificate\s+leaf\[subject\.OU\]\s*=\s*"
        rf"(?:\"{re.escape(team_id)}\"|{re.escape(team_id)})"
        rf"{clause_end}"
    )
    has_unsafe_alternative = (
        re.search(r"(?:^|\W)or(?:\W|$)", dr) is not None
        or re.search(r"(?:^|\W)cdhash(?:\W|$)", dr) is not None
    )
    if (
        has_unsafe_alternative
        or identifier_clause.search(dr) is None
        or anchor_clause.search(dr) is None
        or team_clause.search(dr) is None
    ):
        raise ArtifactError(
            f"{label} designated_requirement must pin the fixed identifier, an Apple "
            f"anchor, and subject.OU = {team_id!r}; got {dr!r}"
        )
    return {field: str(helper[field]) for field in HELPER_FIELDS}


def create_provenance(
    platform: str,
    platform_key: str,
    root: Path,
    commit_sha: str,
    run_id: str | int,
    helper: dict[str, Any] | None = None,
    capture_agent: dict[str, Any] | None = None,
    capture_worker: dict[str, Any] | None = None,
    capture_helper: dict[str, Any] | None = None,
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
        payload["helper"] = _require_helper(
            helper, LLM_HELPER_IDENTIFIER, "local-LLM helper"
        )
    if capture_helper is not None:
        if platform != "macos":
            raise ArtifactError(
                "capture helper provenance is only recorded for macos"
            )
        payload["capture_helper"] = _require_helper(
            capture_helper, CAPTURE_HELPER_IDENTIFIER, "capture helper"
        )
    if capture_agent is not None:
        if platform != "macos":
            raise ArtifactError(
                "capture agent provenance is only recorded for macos"
            )
        payload["capture_agent"] = _require_helper(
            capture_agent, CAPTURE_AGENT_IDENTIFIER, "capture agent"
        )
    if capture_worker is not None:
        if platform != "macos":
            raise ArtifactError(
                "capture worker provenance is only recorded for macos"
            )
        payload["capture_worker"] = _require_helper(
            capture_worker, CAPTURE_WORKER_IDENTIFIER, "capture worker"
        )
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
    require_capture_agent: bool = False,
    require_capture_worker: bool = False,
    require_capture_helper: bool = False,
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
        if platform != "macos":
            raise ArtifactError(f"{platform} provenance must not carry a helper block")
        payload["helper"] = _require_helper(
            helper, LLM_HELPER_IDENTIFIER, "local-LLM helper"
        )
    elif require_helper:
        raise ArtifactError(
            f"{platform} provenance is missing the required local-LLM helper block"
        )

    capture_helper = payload.get("capture_helper")
    if capture_helper is not None:
        if platform != "macos":
            raise ArtifactError(
                f"{platform} provenance must not carry a capture_helper block"
            )
        payload["capture_helper"] = _require_helper(
            capture_helper, CAPTURE_HELPER_IDENTIFIER, "capture helper"
        )
    elif require_capture_helper:
        raise ArtifactError(
            f"{platform} provenance is missing the required capture helper block"
        )

    capture_agent = payload.get("capture_agent")
    if capture_agent is not None:
        if platform != "macos":
            raise ArtifactError(
                f"{platform} provenance must not carry a capture_agent block"
            )
        payload["capture_agent"] = _require_helper(
            capture_agent, CAPTURE_AGENT_IDENTIFIER, "capture agent"
        )
    elif require_capture_agent:
        raise ArtifactError(
            f"{platform} provenance is missing the required capture agent block"
        )
    capture_worker = payload.get("capture_worker")
    if capture_worker is not None:
        if platform != "macos":
            raise ArtifactError(
                f"{platform} provenance must not carry a capture_worker block"
            )
        payload["capture_worker"] = _require_helper(
            capture_worker, CAPTURE_WORKER_IDENTIFIER, "capture worker"
        )
    elif require_capture_worker:
        raise ArtifactError(
            f"{platform} provenance is missing the required capture worker block"
        )

    payload["signature"] = _signature_text(signature)
    return payload


def validate_release(
    artifacts_root: Path,
    expected_sha: str,
    expected_run_id: str | int,
    output: Path | None = None,
    require_macos_helper: bool = False,
    require_macos_capture_agent: bool = False,
    require_macos_capture_worker: bool = False,
    require_macos_capture_helper: bool = False,
) -> dict[str, Any]:
    platforms = {
        platform: validate_platform(
            platform,
            artifacts_root / platform,
            expected_sha,
            expected_run_id,
            require_helper=(require_macos_helper and platform == "macos"),
            require_capture_agent=(
                require_macos_capture_agent and platform == "macos"
            ),
            require_capture_worker=(
                require_macos_capture_worker and platform == "macos"
            ),
            require_capture_helper=(
                require_macos_capture_helper and platform == "macos"
            ),
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


def _normalize_release_notes(value: str) -> str:
    return value.replace("\r\n", "\n").replace("\r", "\n").strip()


def write_updater_manifests(
    validated_path: Path,
    tag: str,
    repository: str,
    bridge_url: str,
    bridge_signature: str,
    release_notes_path: Path,
    output_dir: Path,
    min_version: str | None = None,
) -> tuple[Path, Path]:
    if not re.fullmatch(r"v\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?", tag):
        raise ArtifactError(f"invalid release tag: {tag}")
    if not re.fullmatch(r"[^/\s]+/[^/\s]+", repository):
        raise ArtifactError(f"invalid repository: {repository}")
    bridge_signature = bridge_signature.strip()
    if not bridge_url.startswith("https://") or not bridge_signature:
        raise ArtifactError("bridge updater URL and signature are required")
    if not release_notes_path.is_file():
        raise ArtifactError("release notes file is required")
    release_notes = _normalize_release_notes(
        release_notes_path.read_text(encoding="utf-8")
    )
    if not release_notes:
        raise ArtifactError("release notes must not be empty")

    validated = json.loads(validated_path.read_text(encoding="utf-8"))
    macos = validated["platforms"]["macos"]
    linux = validated["platforms"]["linux"]
    version = tag[1:]
    if min_version is not None:
        min_version = min_version.strip()
        if not re.fullmatch(r"\d+\.\d+\.\d+", min_version):
            raise ArtifactError(
                "min_version must be a stable major.minor.patch version"
            )
        release_core = re.match(r"^(\d+)\.(\d+)\.(\d+)", version)
        assert release_core is not None
        release_parts = tuple(int(part) for part in release_core.groups())
        min_parts = tuple(int(part) for part in min_version.split("."))
        if min_parts > release_parts:
            raise ArtifactError(
                f"min_version {min_version} is newer than release {version}"
            )
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
        "notes": release_notes,
    }
    if min_version is not None:
        modern["min_version"] = min_version
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


def verify_release_notes_match(
    manifest_path: Path, release_notes_path: Path
) -> None:
    if not manifest_path.is_file():
        raise ArtifactError("updater manifest is required")
    if not release_notes_path.is_file():
        raise ArtifactError("release notes file is required")
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        raise ArtifactError("updater manifest is not valid JSON") from exc
    if not isinstance(manifest, dict) or not isinstance(manifest.get("notes"), str):
        raise ArtifactError("updater manifest notes must be a string")

    manifest_notes = _normalize_release_notes(manifest["notes"])
    release_notes = _normalize_release_notes(
        release_notes_path.read_text(encoding="utf-8")
    )
    if not manifest_notes or not release_notes:
        raise ArtifactError("release notes must not be empty")
    if manifest_notes != release_notes:
        raise ArtifactError("draft release notes differ from the updater manifest")


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
    record.add_argument("--capture-helper-sha256")
    record.add_argument("--capture-helper-arch")
    record.add_argument("--capture-helper-designated-requirement")
    record.add_argument("--capture-helper-team-id")
    record.add_argument("--capture-helper-entitlement-sha256")
    record.add_argument("--capture-agent-sha256")
    record.add_argument("--capture-agent-arch")
    record.add_argument("--capture-agent-designated-requirement")
    record.add_argument("--capture-agent-team-id")
    record.add_argument("--capture-agent-entitlement-sha256")
    record.add_argument("--capture-worker-sha256")
    record.add_argument("--capture-worker-arch")
    record.add_argument("--capture-worker-designated-requirement")
    record.add_argument("--capture-worker-team-id")
    record.add_argument("--capture-worker-entitlement-sha256")

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
    validate.add_argument(
        "--require-macos-capture-agent",
        action="store_true",
        help="fail unless macOS provenance records the capture agent block",
    )
    validate.add_argument(
        "--require-macos-capture-worker",
        action="store_true",
        help="fail unless macOS provenance records the capture worker block",
    )
    validate.add_argument(
        "--require-macos-capture-helper",
        action="store_true",
        help="fail unless macOS provenance records the capture helper block",
    )

    manifests = subparsers.add_parser("manifests")
    manifests.add_argument("--validated", type=Path, required=True)
    manifests.add_argument("--tag", required=True)
    manifests.add_argument("--repository", required=True)
    manifests.add_argument("--bridge-url", required=True)
    manifests.add_argument("--bridge-signature", required=True)
    manifests.add_argument("--release-notes", type=Path, required=True)
    manifests.add_argument("--min-version")
    manifests.add_argument("--output-dir", type=Path, required=True)

    verify_notes = subparsers.add_parser("verify-notes")
    verify_notes.add_argument("--manifest", type=Path, required=True)
    verify_notes.add_argument("--release-notes", type=Path, required=True)
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


def _capture_helper_from_args(args: argparse.Namespace) -> dict[str, Any] | None:
    provided = {
        "sha256": args.capture_helper_sha256,
        "architecture": args.capture_helper_arch,
        "designated_requirement": args.capture_helper_designated_requirement,
        "team_id": args.capture_helper_team_id,
        "entitlement_sha256": args.capture_helper_entitlement_sha256,
    }
    supplied = {key: value for key, value in provided.items() if value}
    if not supplied:
        return None
    if len(supplied) != len(provided):
        missing = sorted(set(provided) - set(supplied))
        raise ArtifactError(
            f"incomplete capture helper provenance arguments: missing {missing}"
        )
    return supplied


def _capture_agent_from_args(args: argparse.Namespace) -> dict[str, Any] | None:
    provided = {
        "sha256": args.capture_agent_sha256,
        "architecture": args.capture_agent_arch,
        "designated_requirement": args.capture_agent_designated_requirement,
        "team_id": args.capture_agent_team_id,
        "entitlement_sha256": args.capture_agent_entitlement_sha256,
    }
    supplied = {key: value for key, value in provided.items() if value}
    if not supplied:
        return None
    if len(supplied) != len(provided):
        missing = sorted(set(provided) - set(supplied))
        raise ArtifactError(
            f"incomplete capture agent provenance arguments: missing {missing}"
        )
    return supplied


def _capture_worker_from_args(args: argparse.Namespace) -> dict[str, Any] | None:
    provided = {
        "sha256": args.capture_worker_sha256,
        "architecture": args.capture_worker_arch,
        "designated_requirement": args.capture_worker_designated_requirement,
        "team_id": args.capture_worker_team_id,
        "entitlement_sha256": args.capture_worker_entitlement_sha256,
    }
    supplied = {key: value for key, value in provided.items() if value}
    if not supplied:
        return None
    if len(supplied) != len(provided):
        missing = sorted(set(provided) - set(supplied))
        raise ArtifactError(
            f"incomplete capture worker provenance arguments: missing {missing}"
        )
    return supplied


def main() -> None:
    args = _parser().parse_args()
    try:
        if args.command == "record":
            helper = _helper_from_args(args)
            capture_agent = _capture_agent_from_args(args)
            capture_worker = _capture_worker_from_args(args)
            capture_helper = _capture_helper_from_args(args)
            payload = create_provenance(
                args.platform,
                args.platform_key,
                args.artifacts,
                args.commit_sha,
                args.run_id,
                helper=helper,
                capture_agent=capture_agent,
                capture_worker=capture_worker,
                capture_helper=capture_helper,
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
                require_macos_capture_agent=args.require_macos_capture_agent,
                require_macos_capture_worker=args.require_macos_capture_worker,
                require_macos_capture_helper=args.require_macos_capture_helper,
            )
            print(
                f"validated immutable release artifacts for {payload['commit_sha']} "
                f"(run {payload['workflow_run_id']})"
            )
        elif args.command == "manifests":
            modern, legacy = write_updater_manifests(
                args.validated,
                args.tag,
                args.repository,
                args.bridge_url,
                args.bridge_signature,
                args.release_notes,
                args.output_dir,
                args.min_version,
            )
            print(f"wrote updater manifests: {modern}, {legacy}")
        else:
            verify_release_notes_match(args.manifest, args.release_notes)
            print("verified draft release notes match the updater manifest")
    except ArtifactError as exc:
        raise SystemExit(f"ERROR: {exc}") from exc


if __name__ == "__main__":
    main()
