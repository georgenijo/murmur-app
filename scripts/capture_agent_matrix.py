#!/usr/bin/env python3
"""Validate issue #407's content-free signed LaunchAgent runtime matrix."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
from typing import Any


SHA_RE = re.compile(r"^[0-9a-f]{40}$")
SYNTHETIC_DIGEST = (
    "9fda676f94adbf56e31e91462c702dcda9fcf989eece435876a28778782abfd3"
)
REQUIRED_STEPS = (
    "service_initial",
    "service_register",
    "synthetic_cooperative",
    "synthetic_hard_kill",
    "continuity_active",
    "continuity_recovery",
    "continuity_second_claim",
    "continuity_expired",
    "refresh_before",
    "service_refresh",
    "refresh_after",
    "microphone_active",
    "microphone_recovery",
    "microphone_denied",
    "denied_status",
    "microphone_restored",
    "update_before",
    "update_after",
    "service_unregister",
)


class MatrixError(ValueError):
    pass


def _exact(value: object, keys: set[str], label: str) -> dict[str, Any]:
    if not isinstance(value, dict) or set(value) != keys:
        raise MatrixError(f"{label} must use the complete exact schema")
    return value


def _uint(value: object, label: str, *, positive: bool = False) -> int:
    if type(value) is not int or value < (1 if positive else 0):
        raise MatrixError(f"{label} must be a valid unsigned integer")
    return value


def _common(payload: dict[str, Any], outcome: str) -> None:
    if payload["schema_version"] != 1 or payload["outcome"] != outcome:
        raise MatrixError(f"{outcome} schema/outcome mismatch")
    if payload["audio_content_retained"] is not False:
        raise MatrixError("runtime evidence must retain no audio content")


def _synthetic(payload: dict[str, Any]) -> None:
    if (
        payload["synthetic_fixture"] != "seq-v1"
        or payload["synthetic_digest"] != SYNTHETIC_DIGEST
        or payload["synthetic_first_sequence"] != 0
        or payload["synthetic_last_sequence"] != 63
        or payload["synthetic_canary_count"] != 64
        or payload["synthetic_complete"] is not True
    ):
        raise MatrixError("synthetic sequence evidence is incomplete or contradictory")


def validate_service(payload: object, expected_status: str) -> dict[str, Any]:
    value = _exact(
        payload,
        {
            "schema_version",
            "outcome",
            "service_status",
            "audio_content_retained",
        },
        "service evidence",
    )
    _common(value, "service_status")
    if value["service_status"] != expected_status:
        raise MatrixError(
            f"service status must be {expected_status!r}, got {value['service_status']!r}"
        )
    return value


def validate_initial_service(envelope: object) -> dict[str, Any]:
    record = _exact(envelope, {"exit_code", "payload"}, "service_initial")
    if type(record["exit_code"]) is not int:
        raise MatrixError("initial service exit_code must be an integer")
    value = _exact(
        record["payload"],
        {
            "schema_version",
            "outcome",
            "service_status",
            "audio_content_retained",
        },
        "initial service evidence",
    )
    status = value["service_status"]
    if status == "not_registered":
        if record["exit_code"] != 0:
            raise MatrixError("not_registered initial service status must exit 0")
        _common(value, "service_status")
    elif status == "not_found":
        if record["exit_code"] != 2:
            raise MatrixError("not_found initial service status must exit 2")
        _common(value, "service_error")
    else:
        raise MatrixError(
            "initial service status must be 'not_registered' or macOS 26 'not_found'"
        )
    return value


def validate_status(payload: object, expected_outcome: str) -> dict[str, Any]:
    value = _exact(
        payload,
        {
            "schema_version",
            "outcome",
            "agent_pid",
            "agent_instance",
            "generation",
            "worker_pid",
            "synthetic_canary_count",
            "audio_content_retained",
        },
        "agent status",
    )
    _common(value, expected_outcome)
    _uint(value["agent_pid"], "agent_pid", positive=True)
    _uint(value["generation"], "generation")
    _uint(value["worker_pid"], "worker_pid")
    _uint(value["synthetic_canary_count"], "synthetic_canary_count")
    if not isinstance(value["agent_instance"], str) or not value["agent_instance"]:
        raise MatrixError("agent_instance must be non-empty")
    if expected_outcome == "active" and value["worker_pid"] == 0:
        raise MatrixError("active status must identify the worker")
    if expected_outcome == "idle" and value["worker_pid"] != 0:
        raise MatrixError("idle status must not identify a worker")
    return value


def validate_probe(
    payload: object,
    termination: str,
    *,
    synthetic: bool,
) -> dict[str, Any]:
    keys = {
        "schema_version",
        "outcome",
        "generation",
        "agent_pid",
        "agent_instance",
        "worker_pid",
        "synthetic_canary_count",
        "first_callback_ms",
        "worker_termination",
        "stop_elapsed_ms",
        "worker_exited",
        "process_group_empty",
        "exit_signal",
        "audio_content_retained",
    }
    if synthetic:
        keys |= {
            "synthetic_fixture",
            "synthetic_digest",
            "synthetic_first_sequence",
            "synthetic_last_sequence",
            "synthetic_complete",
        }
    value = _exact(payload, keys, "probe evidence")
    _common(value, "ok")
    for key in ("generation", "agent_pid", "worker_pid"):
        _uint(value[key], key, positive=True)
    _uint(value["first_callback_ms"], "first_callback_ms")
    _uint(value["stop_elapsed_ms"], "stop_elapsed_ms")
    if value["worker_termination"] != termination:
        raise MatrixError(f"worker termination must be {termination!r}")
    if value["worker_exited"] is not True or value["process_group_empty"] is not True:
        raise MatrixError("probe must prove worker exit and an empty process group")
    expected_signal = 9 if termination == "hard_kill" else 0
    if value["exit_signal"] != expected_signal:
        raise MatrixError("worker exit signal contradicts its termination")
    if synthetic:
        _synthetic(value)
    return value


def validate_recovery(
    payload: object,
    *,
    synthetic: bool,
    ack_replay: bool = False,
) -> dict[str, Any]:
    keys = {
        "schema_version",
        "outcome",
        "generation",
        "agent_pid",
        "agent_instance",
        "worker_pid",
        "synthetic_canary_count",
        "first_callback_ms",
        "worker_termination",
        "stop_elapsed_ms",
        "recovery_ttl_ms",
        "agent_survived",
        "worker_exited",
        "process_group_empty",
        "exit_signal",
        "audio_content_retained",
        "claim_id",
        "recovered",
        "exact_once",
    }
    if synthetic:
        keys |= {
            "synthetic_fixture",
            "synthetic_digest",
            "synthetic_first_sequence",
            "synthetic_last_sequence",
            "synthetic_complete",
        }
    if ack_replay:
        keys.add("ack_replay_verified")
    value = _exact(payload, keys, "recovery evidence")
    _common(value, "recovery_acked")
    for key in ("generation", "agent_pid", "worker_pid"):
        _uint(value[key], key, positive=True)
    _uint(value["first_callback_ms"], "first_callback_ms")
    _uint(value["stop_elapsed_ms"], "stop_elapsed_ms")
    _uint(value["recovery_ttl_ms"], "recovery_ttl_ms")
    termination = value["worker_termination"]
    signal = value["exit_signal"]
    if (
        termination not in {"cooperative", "exited", "hard_kill"}
        or (termination == "hard_kill" and signal != 9)
        or (termination != "hard_kill" and signal != 0)
    ):
        raise MatrixError("recovery termination contradicts its exit signal")
    if (
        value["agent_survived"] is not True
        or value["worker_exited"] is not True
        or value["process_group_empty"] is not True
        or value["recovered"] is not True
        or value["exact_once"] is not True
    ):
        raise MatrixError("recovery lifecycle proof is incomplete")
    if not isinstance(value["claim_id"], str) or not value["claim_id"]:
        raise MatrixError("recovery claim ID must be non-empty")
    if ack_replay and value["ack_replay_verified"] is not True:
        raise MatrixError("same-peer acknowledgement replay was not verified")
    if synthetic:
        _synthetic(value)
    return value


def validate_matrix(payload: object) -> dict[str, Any]:
    matrix = _exact(
        payload,
        {
            "schema_version",
            "source_sha",
            "signed_bundle_artifact",
            "artifact_provenance",
            "previous_signed_bundle_artifact",
            "previous_artifact_provenance",
            "observations",
            "records",
        },
        "capture-agent matrix",
    )
    if matrix["schema_version"] != 1:
        raise MatrixError("capture-agent matrix schema mismatch")
    source_sha = matrix["source_sha"]
    if not isinstance(source_sha, str) or not SHA_RE.fullmatch(source_sha):
        raise MatrixError("source_sha must be a lowercase 40-character SHA")
    if matrix["signed_bundle_artifact"] != f"macos-release-{source_sha}":
        raise MatrixError("signed bundle artifact must bind to source_sha")
    provenance = _exact(
        matrix["artifact_provenance"],
        {
            "commit_sha",
            "workflow_run_id",
            "capture_agent_sha256",
            "capture_worker_sha256",
            "capture_agent_identifier",
            "capture_worker_identifier",
            "team_id",
        },
        "artifact provenance",
    )
    if provenance["commit_sha"] != source_sha:
        raise MatrixError("artifact provenance commit does not match source_sha")
    _uint(provenance["workflow_run_id"], "workflow_run_id", positive=True)
    for key in ("capture_agent_sha256", "capture_worker_sha256"):
        if (
            not isinstance(provenance[key], str)
            or re.fullmatch(r"[0-9a-f]{64}", provenance[key]) is None
        ):
            raise MatrixError(f"{key} must be a lowercase SHA-256")
    if provenance["capture_agent_identifier"] != "com.localdictation.capture-agent":
        raise MatrixError("artifact provenance capture-agent identifier mismatch")
    if provenance["capture_worker_identifier"] != "com.localdictation.capture-worker":
        raise MatrixError("artifact provenance capture-worker identifier mismatch")
    if (
        not isinstance(provenance["team_id"], str)
        or re.fullmatch(r"[A-Z0-9]{10}", provenance["team_id"]) is None
    ):
        raise MatrixError("artifact provenance Team ID is invalid")
    previous_provenance = _exact(
        matrix["previous_artifact_provenance"],
        {
            "commit_sha",
            "workflow_run_id",
            "capture_agent_sha256",
            "capture_worker_sha256",
            "capture_agent_identifier",
            "capture_worker_identifier",
            "team_id",
        },
        "previous artifact provenance",
    )
    previous_sha = previous_provenance["commit_sha"]
    if (
        not isinstance(previous_sha, str)
        or not SHA_RE.fullmatch(previous_sha)
        or previous_sha == source_sha
    ):
        raise MatrixError("previous artifact must identify a distinct source commit")
    if matrix["previous_signed_bundle_artifact"] != f"macos-release-{previous_sha}":
        raise MatrixError("previous signed bundle artifact must bind to its commit")
    previous_run_id = _uint(
        previous_provenance["workflow_run_id"],
        "previous workflow_run_id",
        positive=True,
    )
    if previous_run_id == provenance["workflow_run_id"]:
        raise MatrixError("previous artifact must come from a distinct workflow run")
    for key in ("capture_agent_sha256", "capture_worker_sha256"):
        if (
            not isinstance(previous_provenance[key], str)
            or re.fullmatch(r"[0-9a-f]{64}", previous_provenance[key]) is None
        ):
            raise MatrixError(f"previous {key} must be a lowercase SHA-256")
    for key in ("capture_agent_identifier", "capture_worker_identifier", "team_id"):
        if previous_provenance[key] != provenance[key]:
            raise MatrixError(f"previous artifact {key} does not match current provenance")

    observations = _exact(
        matrix["observations"],
        {
            "notarized",
            "stapled",
            "gatekeeper_accepted",
            "quarantine_applied",
            "launchservices_opened",
            "main_pid_before_revocation",
            "main_pid_after_revocation",
            "permission_transition",
            "revocation_trigger",
            "background_activity_labels",
            "microphone_identity_labels",
            "microphone_prompt_count",
            "additional_microphone_prompt_observed",
            "previous_source_sha",
            "previous_capture_agent_sha256",
            "previous_capture_worker_sha256",
            "installed_capture_agent_sha256",
            "installed_capture_worker_sha256",
            "residual_agent_processes",
            "residual_worker_processes",
        },
        "runtime observations",
    )
    for key in (
        "notarized",
        "stapled",
        "gatekeeper_accepted",
        "quarantine_applied",
        "launchservices_opened",
    ):
        if observations[key] is not True:
            raise MatrixError(f"{key} must be independently confirmed")
    before_pid = _uint(
        observations["main_pid_before_revocation"],
        "main_pid_before_revocation",
        positive=True,
    )
    after_pid = _uint(
        observations["main_pid_after_revocation"],
        "main_pid_after_revocation",
        positive=True,
    )
    if before_pid == after_pid:
        raise MatrixError("permission revocation did not replace the main process")
    if observations["permission_transition"] != ["granted", "denied", "granted"]:
        raise MatrixError("permission transition must prove revoke and restoration")
    if observations["revocation_trigger"] != "system_settings_microphone_toggle":
        raise MatrixError("revocation must be triggered through System Settings")
    if observations["background_activity_labels"] != ["Murmur"]:
        raise MatrixError("Background Activity must show one Murmur identity")
    if observations["microphone_identity_labels"] != ["Murmur"]:
        raise MatrixError("Microphone privacy UI must show one Murmur identity")
    prompt_count = _uint(
        observations["microphone_prompt_count"],
        "microphone_prompt_count",
    )
    if prompt_count > 1 or observations["additional_microphone_prompt_observed"] is not False:
        raise MatrixError("capture agent introduced an additional microphone prompt")
    if observations["previous_source_sha"] != previous_sha:
        raise MatrixError("previous runtime source does not match artifact provenance")
    for key in (
        "previous_capture_agent_sha256",
        "previous_capture_worker_sha256",
        "installed_capture_agent_sha256",
        "installed_capture_worker_sha256",
    ):
        if (
            not isinstance(observations[key], str)
            or re.fullmatch(r"[0-9a-f]{64}", observations[key]) is None
        ):
            raise MatrixError(f"{key} must be a lowercase SHA-256")
    if (
        observations["previous_capture_agent_sha256"]
        != previous_provenance["capture_agent_sha256"]
        or observations["previous_capture_worker_sha256"]
        != previous_provenance["capture_worker_sha256"]
    ):
        raise MatrixError("previous installed hashes do not match artifact provenance")
    if (
        observations["installed_capture_agent_sha256"]
        != provenance["capture_agent_sha256"]
        or observations["installed_capture_worker_sha256"]
        != provenance["capture_worker_sha256"]
    ):
        raise MatrixError("installed helper hashes do not match artifact provenance")
    if (
        observations["previous_capture_agent_sha256"]
        == observations["installed_capture_agent_sha256"]
        or observations["previous_capture_worker_sha256"]
        == observations["installed_capture_worker_sha256"]
    ):
        raise MatrixError("signed update did not replace both capture executables")
    if observations["residual_agent_processes"] != 0:
        raise MatrixError("agent process remains after unregistration")
    if observations["residual_worker_processes"] != 0:
        raise MatrixError("worker process remains after unregistration")
    records = _exact(matrix["records"], set(REQUIRED_STEPS), "matrix records")

    def record(name: str, exit_code: int) -> dict[str, Any]:
        envelope = _exact(records[name], {"exit_code", "payload"}, name)
        if envelope["exit_code"] != exit_code:
            raise MatrixError(f"{name} exit code mismatch")
        return envelope["payload"]

    validate_initial_service(records["service_initial"])
    validate_service(record("service_register", 0), "enabled")
    validate_probe(
        record("synthetic_cooperative", 0),
        "cooperative",
        synthetic=True,
    )
    validate_probe(
        record("synthetic_hard_kill", 0),
        "hard_kill",
        synthetic=True,
    )

    continuity_active = validate_status(record("continuity_active", 0), "active")
    continuity_recovery = validate_recovery(
        record("continuity_recovery", 0),
        synthetic=True,
        ack_replay=True,
    )
    for key in ("agent_pid", "agent_instance", "generation", "worker_pid"):
        if continuity_active[key] != continuity_recovery[key]:
            raise MatrixError(f"continuity recovery changed {key}")
    second_claim = _exact(
        record("continuity_second_claim", 0),
        {
            "schema_version",
            "outcome",
            "generation",
            "recovered",
            "exact_once",
            "audio_content_retained",
        },
        "second recovery claim",
    )
    _common(second_claim, "already_acked")
    if (
        second_claim["generation"] != continuity_recovery["generation"]
        or second_claim["recovered"] is not False
        or second_claim["exact_once"] is not True
    ):
        raise MatrixError("second recovery claim exposed or adopted recovery content")
    expired = _exact(
        record("continuity_expired", 0),
        {
            "schema_version",
            "outcome",
            "generation",
            "recovered",
            "audio_content_retained",
        },
        "expired recovery claim",
    )
    _common(expired, "expired")
    if (
        expired["generation"] != continuity_recovery["generation"]
        or expired["recovered"] is not False
    ):
        raise MatrixError("expired recovery tombstone contradicts its generation")

    refresh_before = validate_status(record("refresh_before", 0), "idle")
    validate_service(record("service_refresh", 0), "enabled")
    refresh_after = validate_status(record("refresh_after", 0), "idle")
    if refresh_before["agent_pid"] == refresh_after["agent_pid"]:
        raise MatrixError("service refresh did not replace the agent process")
    if refresh_before["agent_instance"] == refresh_after["agent_instance"]:
        raise MatrixError("service refresh did not replace the agent instance")

    microphone_active = validate_status(record("microphone_active", 0), "active")
    microphone_recovery = validate_recovery(
        record("microphone_recovery", 0),
        synthetic=False,
    )
    for key in ("agent_pid", "agent_instance", "generation", "worker_pid"):
        if microphone_active[key] != microphone_recovery[key]:
            raise MatrixError(f"microphone recovery changed {key}")
    denied = _exact(
        record("microphone_denied", 2),
        {"schema_version", "outcome", "audio_content_retained"},
        "denied microphone probe",
    )
    _common(denied, "permission_denied")
    denied_status = validate_status(record("denied_status", 0), "idle")
    for key in ("agent_pid", "agent_instance", "generation"):
        if denied_status[key] != microphone_recovery[key]:
            raise MatrixError(f"denied preflight replaced surviving agent {key}")
    validate_probe(
        record("microphone_restored", 0),
        "cooperative",
        synthetic=False,
    )
    update_before = validate_status(record("update_before", 0), "idle")
    update_after = validate_status(record("update_after", 0), "idle")
    if update_before["agent_pid"] == update_after["agent_pid"]:
        raise MatrixError("signed update did not replace the agent process")
    if update_before["agent_instance"] == update_after["agent_instance"]:
        raise MatrixError("signed update did not replace the agent instance")
    validate_service(record("service_unregister", 0), "not_registered")
    return dict(matrix)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("matrix", type=Path)
    args = parser.parse_args()
    try:
        payload = json.loads(args.matrix.read_text(encoding="utf-8"))
        validated = validate_matrix(payload)
    except (OSError, json.JSONDecodeError, MatrixError) as exc:
        raise SystemExit(f"capture-agent matrix invalid: {exc}") from exc
    print(json.dumps(validated, sort_keys=True, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
