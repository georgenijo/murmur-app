#!/usr/bin/env python3
"""Collect and validate content-free capture-helper release evidence."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import plistlib
import re
import subprocess
from typing import Any
from xml.parsers.expat import ExpatError


CAPTURE_HELPER_IDENTIFIER = "com.localdictation.capture-helper"
CAPTURE_AGENT_IDENTIFIER = "com.localdictation.capture-agent"
CAPTURE_WORKER_IDENTIFIER = "com.localdictation.capture-worker"
CAPTURE_ENTITLEMENTS = {
    "com.apple.security.app-sandbox": True,
    "com.apple.security.device.audio-input": True,
    "com.apple.security.device.microphone": True,
}
CAPTURE_AGENT_ENTITLEMENTS = {
    "com.apple.security.app-sandbox": True,
    "com.apple.security.device.audio-input": True,
    "com.apple.security.device.microphone": True,
    "com.apple.security.temporary-exception.mach-lookup.global-name": [
        "com.localdictation.capture-agent.xpc"
    ],
    "com.apple.security.temporary-exception.mach-register.global-name": [
        "com.localdictation.capture-agent.xpc"
    ],
}
CAPTURE_WORKER_ENTITLEMENTS = {
    "com.apple.security.app-sandbox": True,
    "com.apple.security.device.audio-input": True,
    "com.apple.security.device.microphone": True,
}
PROBE_FIELDS = {
    "schema_version",
    "outcome",
    "last_phase",
    "helper_pid",
    "first_callback_ms",
    "elapsed_ms",
    "termination",
    "exit_code",
    "exit_signal",
    "process_group_empty",
    "audio_content_retained",
}
ProbeContract = tuple[str, bool, str, int | None, int | None]
PROBE_OUTCOME_CONTRACTS: dict[str, frozenset[ProbeContract]] = {
    # Successful observation always reaches the first callback and then the
    # helper's cooperative stopping phase.
    "ok": frozenset({("stopping", True, "cooperative", 0, None)}),
    # A deliberately shortened observation may cancel before the helper's
    # three-second callback-stall deadline.
    "no_first_callback": frozenset(
        {("stopping", False, "cooperative", 0, None)}
    ),
    # Hardware/TCC failures are emitted after their named phase and the helper
    # returns Ok after recording the failure frame.
    "permission_denied": frozenset(
        {
            ("enumeration", False, "exited", 0, None),
            ("stream_open", False, "exited", 0, None),
        }
    ),
    "no_input_device": frozenset(
        {("enumeration", False, "exited", 0, None)}
    ),
    "enumeration_failed": frozenset(
        {("enumeration", False, "exited", 0, None)}
    ),
    "configuration_failed": frozenset(
        {("stream_open", False, "exited", 0, None)}
    ),
    "stream_open_failed": frozenset(
        {("stream_open", False, "exited", 0, None)}
    ),
    "stream_start_failed": frozenset(
        {("stream_open", False, "exited", 0, None)}
    ),
    # Runtime failures may happen before the first callback or after active
    # capture. Callback presence must agree with the last recorded phase.
    "stream_error": frozenset(
        {
            ("awaiting_first_callback", False, "exited", 0, None),
            ("active", True, "exited", 0, None),
        }
    ),
    "callback_stalled": frozenset(
        {
            ("awaiting_first_callback", False, "exited", 0, None),
            ("active", True, "exited", 0, None),
        }
    ),
}
ALLOWED_PROBE_OUTCOMES = frozenset(PROBE_OUTCOME_CONTRACTS)
ALLOWED_PHASES = {
    None,
    "enumeration",
    "stream_open",
    "awaiting_first_callback",
    "active",
    "stopping",
}
ALLOWED_TERMINATIONS = {"cooperative", "exited", "hard_kill"}
SHA_RE = re.compile(r"^[0-9a-f]{40}$")
TEAM_ID_RE = re.compile(r"^[A-Z0-9]{10}$")


class EvidenceError(ValueError):
    pass


def _nonnegative_int(value: object, label: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise EvidenceError(f"{label} must be a non-negative integer")
    return value


def _designated_requirement_profile(
    requirement: str, identifier: str, team_id: str, label: str
) -> tuple[str, str]:
    """Validate a complete known-safe requirement and return normalized facts."""

    normalized = " ".join(requirement.split())
    identity = (
        rf'identifier\s+"{re.escape(identifier)}"\s+and\s+'
        r"anchor\s+apple\s+generic"
    )
    team = (
        r"certificate\s+leaf\[subject\.OU\]\s*=\s*"
        rf'(?:{re.escape(team_id)}|"{re.escape(team_id)}")'
    )
    profiles = {
        "developer_id_application": re.compile(
            rf"{identity}\s+and\s+"
            r"certificate\s+1\[field\.1\.2\.840\.113635\.100\.6\.2\.6\]"
            r"(?:\s+/\*\s*exists\s*\*/)?\s+and\s+"
            r"certificate\s+leaf\[field\.1\.2\.840\.113635\.100\.6\.1\.13\]"
            r"(?:\s+/\*\s*exists\s*\*/)?\s+and\s+"
            rf"{team}"
        )
    }
    for profile, pattern in profiles.items():
        if pattern.fullmatch(normalized):
            return profile, normalized
    raise EvidenceError(
        f"{label} designated requirement is not an exact canonical profile"
    )


def validate_probe_evidence(payload: object, probe_exit: int) -> dict[str, Any]:
    if not isinstance(payload, dict) or set(payload) != PROBE_FIELDS:
        raise EvidenceError("capture-helper probe must use the complete exact schema")
    if type(payload["schema_version"]) is not int or payload["schema_version"] != 1:
        raise EvidenceError("capture-helper evidence schema mismatch")
    outcome = payload["outcome"]
    if not isinstance(outcome, str) or outcome not in ALLOWED_PROBE_OUTCOMES:
        raise EvidenceError(f"capture-helper outcome is not allowed: {outcome!r}")
    last_phase = payload["last_phase"]
    if not (last_phase is None or isinstance(last_phase, str)):
        raise EvidenceError("capture-helper phase must be a string or null")
    if last_phase not in ALLOWED_PHASES:
        raise EvidenceError("capture-helper phase is invalid")
    helper_pid = _nonnegative_int(payload["helper_pid"], "helper_pid")
    if helper_pid == 0:
        raise EvidenceError("helper_pid must identify a spawned helper")
    first_callback = payload["first_callback_ms"]
    if first_callback is not None:
        _nonnegative_int(first_callback, "first_callback_ms")
    _nonnegative_int(payload["elapsed_ms"], "elapsed_ms")
    termination = payload["termination"]
    if not isinstance(termination, str) or termination not in ALLOWED_TERMINATIONS:
        raise EvidenceError("capture-helper termination is not confirmed")
    exit_code = payload["exit_code"]
    exit_signal = payload["exit_signal"]
    if exit_code is not None:
        _nonnegative_int(exit_code, "exit_code")
    if exit_signal is not None:
        _nonnegative_int(exit_signal, "exit_signal")
    if (exit_code is None) == (exit_signal is None):
        raise EvidenceError("confirmed termination needs exactly one exit code or signal")
    if termination == "cooperative" and (exit_code != 0 or exit_signal is not None):
        raise EvidenceError("cooperative termination must be a clean exit")
    if termination == "hard_kill" and (exit_code is not None or exit_signal != 9):
        raise EvidenceError("hard-kill termination must be confirmed SIGKILL")
    if payload["process_group_empty"] is not True:
        raise EvidenceError("capture-helper process group must be confirmed empty")
    if payload["audio_content_retained"] is not False:
        raise EvidenceError("capture-helper evidence must prove no retained audio")
    observed_contract: ProbeContract = (
        last_phase,
        first_callback is not None,
        termination,
        exit_code,
        exit_signal,
    )
    if observed_contract not in PROBE_OUTCOME_CONTRACTS[outcome]:
        raise EvidenceError(
            "capture-helper outcome contradicts phase, callback, or exit contract"
        )
    if type(probe_exit) is not int:
        raise EvidenceError("probe exit must be an integer")
    expected_exit = 0 if outcome == "ok" else 2
    if probe_exit != expected_exit:
        raise EvidenceError(
            f"probe exit {probe_exit} does not match outcome {outcome!r}"
        )
    return dict(payload)


def structured_signature_evidence(
    details: str,
    designated_requirement_output: str,
    entitlements: dict[str, object],
    architecture: str,
    *,
    expected_identifier: str = CAPTURE_HELPER_IDENTIFIER,
    expected_entitlements: dict[str, object] = CAPTURE_ENTITLEMENTS,
    label: str = "capture-helper",
) -> dict[str, Any]:
    identifier_match = re.search(r"^Identifier=(.+)$", details, re.MULTILINE)
    team_match = re.search(r"^TeamIdentifier=(.+)$", details, re.MULTILINE)
    identifier = identifier_match.group(1).strip() if identifier_match else ""
    team_id = team_match.group(1).strip() if team_match else ""
    if identifier != expected_identifier:
        raise EvidenceError(f"{label} signature identifier mismatch")
    if not TEAM_ID_RE.fullmatch(team_id):
        raise EvidenceError(f"{label} Team ID is invalid")
    hardened_runtime = "(runtime)" in details or "Runtime Version=" in details
    if not hardened_runtime:
        raise EvidenceError(f"{label} hardened runtime is missing")
    if architecture.strip() != "arm64":
        raise EvidenceError(f"{label} architecture must be exactly arm64")
    if entitlements != expected_entitlements:
        raise EvidenceError(f"{label} entitlements differ from the exact policy")

    requirement_match = re.search(
        r"^#*\s*designated\s*=>\s*(.+)$",
        designated_requirement_output,
        re.MULTILINE,
    )
    requirement = requirement_match.group(1).strip() if requirement_match else ""
    requirement_profile, normalized_requirement = _designated_requirement_profile(
        requirement, expected_identifier, team_id, label
    )

    entitlement_bytes = plistlib.dumps(entitlements, sort_keys=True)
    return {
        "schema_version": 1,
        "identifier": identifier,
        "team_id": team_id,
        "architecture": "arm64",
        "hardened_runtime": True,
        "designated_requirement_profile": requirement_profile,
        "designated_requirement_sha256": hashlib.sha256(
            normalized_requirement.encode("utf-8")
        ).hexdigest(),
        "entitlement_sha256": hashlib.sha256(entitlement_bytes).hexdigest(),
        "entitlement_keys": sorted(entitlements),
    }


def _run(command: list[str]) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(command, check=True, capture_output=True)


def extract_entitlements_plist(payload: bytes) -> dict[str, object]:
    """Extract exactly one complete XML plist from mixed codesign output."""

    xml_starts = [match.start() for match in re.finditer(br"<\?xml\b", payload)]
    plist_starts = [
        match.start() for match in re.finditer(br"<plist(?:\s|>)", payload)
    ]
    plist_ends = [
        match.end() for match in re.finditer(br"</plist\s*>", payload)
    ]
    if not xml_starts and not plist_starts and not plist_ends:
        raise EvidenceError("codesign returned no capture-helper entitlements")
    if len(xml_starts) != 1 or len(plist_starts) != 1 or len(plist_ends) != 1:
        raise EvidenceError(
            "codesign returned ambiguous or incomplete capture-helper entitlements"
        )
    xml_start = xml_starts[0]
    plist_start = plist_starts[0]
    plist_end = plist_ends[0]
    if not xml_start < plist_start < plist_end:
        raise EvidenceError("codesign returned malformed capture-helper entitlements")

    document = payload[xml_start:plist_end]
    try:
        entitlements = plistlib.loads(document)
    except (ExpatError, plistlib.InvalidFileException, TypeError, ValueError) as error:
        raise EvidenceError(
            "codesign returned malformed capture-helper entitlements"
        ) from error
    if not isinstance(entitlements, dict):
        raise EvidenceError("capture-helper entitlements plist must contain a dictionary")
    return entitlements


def collect_signature(
    helper: Path,
    signature_output: Path,
    entitlements_output: Path,
    kind: str = "capture-helper",
) -> None:
    policies = {
        "capture-helper": (CAPTURE_HELPER_IDENTIFIER, CAPTURE_ENTITLEMENTS),
        "capture-agent": (CAPTURE_AGENT_IDENTIFIER, CAPTURE_AGENT_ENTITLEMENTS),
        "capture-worker": (CAPTURE_WORKER_IDENTIFIER, CAPTURE_WORKER_ENTITLEMENTS),
    }
    identifier, expected_entitlements = policies[kind]
    details_result = _run(["codesign", "-d", "--verbose=4", str(helper)])
    details = (details_result.stdout + details_result.stderr).decode(
        "utf-8", errors="strict"
    )
    requirement_result = _run(["codesign", "-d", "-r-", str(helper)])
    requirement = (requirement_result.stdout + requirement_result.stderr).decode(
        "utf-8", errors="strict"
    )
    entitlement_result = _run(
        ["codesign", "-d", "--entitlements", ":-", "--xml", str(helper)]
    )
    entitlement_payload = entitlement_result.stdout + entitlement_result.stderr
    entitlements = extract_entitlements_plist(entitlement_payload)
    architecture = _run(["lipo", "-archs", str(helper)]).stdout.decode().strip()
    evidence = structured_signature_evidence(
        details,
        requirement,
        entitlements,
        architecture,
        expected_identifier=identifier,
        expected_entitlements=expected_entitlements,
        label=kind,
    )
    signature_output.write_text(
        json.dumps(evidence, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    entitlements_output.write_bytes(plistlib.dumps(entitlements, sort_keys=True))


def validate_probe_file(
    probe: Path,
    probe_exit: int,
    source_sha: str,
    signed_bundle_artifact: str,
    manifest_output: Path,
) -> None:
    if not SHA_RE.fullmatch(source_sha):
        raise EvidenceError("source SHA must be an exact lowercase commit SHA")
    expected_artifact = f"macos-release-{source_sha}"
    if signed_bundle_artifact != expected_artifact:
        raise EvidenceError("signed bundle artifact must be bound to the source SHA")
    payload = validate_probe_evidence(
        json.loads(probe.read_text(encoding="utf-8")), probe_exit
    )
    manifest = {
        "schema_version": 1,
        "source_sha": source_sha,
        "signed_bundle_artifact": signed_bundle_artifact,
        "interactive_tcc_matrix": "required_on_downloaded_notarized_bundle",
        "probe_outcome": payload["outcome"],
        "confirmed_termination": True,
        "process_group_empty": True,
        "audio_content_retained": False,
    }
    manifest_output.write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)
    collect = subparsers.add_parser("collect-signature")
    collect.add_argument("--helper", type=Path, required=True)
    collect.add_argument(
        "--kind",
        choices=("capture-helper", "capture-agent", "capture-worker"),
        default="capture-helper",
    )
    collect.add_argument("--signature-output", type=Path, required=True)
    collect.add_argument("--entitlements-output", type=Path, required=True)
    validate = subparsers.add_parser("validate-probe")
    validate.add_argument("--probe", type=Path, required=True)
    validate.add_argument("--probe-exit", type=int, required=True)
    validate.add_argument("--source-sha", required=True)
    validate.add_argument("--signed-bundle-artifact", required=True)
    validate.add_argument("--manifest-output", type=Path, required=True)
    args = parser.parse_args()

    if args.command == "collect-signature":
        collect_signature(
            args.helper,
            args.signature_output,
            args.entitlements_output,
            args.kind,
        )
    else:
        validate_probe_file(
            args.probe,
            args.probe_exit,
            args.source_sha,
            args.signed_bundle_artifact,
            args.manifest_output,
        )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (EvidenceError, json.JSONDecodeError, OSError, subprocess.CalledProcessError) as error:
        raise SystemExit(f"capture-helper evidence rejected: {error}") from None
