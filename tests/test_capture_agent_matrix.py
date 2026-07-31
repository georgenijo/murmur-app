from copy import deepcopy
import unittest

from scripts.capture_agent_matrix import MatrixError, validate_matrix


class CaptureAgentMatrixTests(unittest.TestCase):
    def setUp(self) -> None:
        sha = "a" * 40
        service = lambda status: {
            "schema_version": 1,
            "outcome": "service_status",
            "service_status": status,
            "audio_content_retained": False,
        }
        status = lambda outcome, agent, instance, generation, worker, count=0: {
            "schema_version": 1,
            "outcome": outcome,
            "agent_pid": agent,
            "agent_instance": instance,
            "generation": generation,
            "worker_pid": worker,
            "synthetic_canary_count": count,
            "audio_content_retained": False,
        }
        synthetic = {
            "synthetic_fixture": "seq-v1",
            "synthetic_digest":
                "9fda676f94adbf56e31e91462c702dcda9fcf989eece435876a28778782abfd3",
            "synthetic_first_sequence": 0,
            "synthetic_last_sequence": 63,
            "synthetic_complete": True,
        }
        probe = lambda termination, is_synthetic: {
            "schema_version": 1,
            "outcome": "ok",
            "generation": 1,
            "agent_pid": 100,
            "agent_instance": "one",
            "worker_pid": 101,
            "synthetic_canary_count": 64 if is_synthetic else 2,
            "first_callback_ms": 1,
            "worker_termination": termination,
            "stop_elapsed_ms": 20 if termination == "cooperative" else 260,
            "worker_exited": True,
            "process_group_empty": True,
            "exit_signal": 9 if termination == "hard_kill" else 0,
            "audio_content_retained": False,
            **(synthetic if is_synthetic else {}),
        }
        recovery = lambda agent, instance, generation, worker, is_synthetic: {
            "schema_version": 1,
            "outcome": "recovery_acked",
            "generation": generation,
            "agent_pid": agent,
            "agent_instance": instance,
            "worker_pid": worker,
            "synthetic_canary_count": 64 if is_synthetic else 2,
            "first_callback_ms": 1,
            "worker_termination": "cooperative",
            "stop_elapsed_ms": 20,
            "recovery_ttl_ms": 29_000,
            "agent_survived": True,
            "worker_exited": True,
            "process_group_empty": True,
            "exit_signal": 0,
            "audio_content_retained": False,
            "claim_id": "claim",
            "recovered": True,
            "exact_once": True,
            **(synthetic if is_synthetic else {}),
        }
        envelope = lambda payload, code=0: {"exit_code": code, "payload": payload}
        self.matrix = {
            "schema_version": 1,
            "source_sha": sha,
            "signed_bundle_artifact": f"macos-release-{sha}",
            "artifact_provenance": {
                "commit_sha": sha,
                "workflow_run_id": 123,
                "capture_agent_sha256": "b" * 64,
                "capture_worker_sha256": "c" * 64,
                "capture_agent_identifier": "com.localdictation.capture-agent",
                "capture_worker_identifier": "com.localdictation.capture-worker",
                "team_id": "P2U3P8B923",
            },
            "previous_signed_bundle_artifact": f"macos-release-{'f' * 40}",
            "previous_artifact_provenance": {
                "commit_sha": "f" * 40,
                "workflow_run_id": 122,
                "capture_agent_sha256": "d" * 64,
                "capture_worker_sha256": "e" * 64,
                "capture_agent_identifier": "com.localdictation.capture-agent",
                "capture_worker_identifier": "com.localdictation.capture-worker",
                "team_id": "P2U3P8B923",
            },
            "observations": {
                "notarized": True,
                "stapled": True,
                "gatekeeper_accepted": True,
                "quarantine_applied": True,
                "launchservices_opened": True,
                "main_pid_before_revocation": 500,
                "main_pid_after_revocation": 501,
                "permission_transition": ["granted", "denied", "granted"],
                "revocation_trigger": "system_settings_microphone_toggle",
                "background_activity_labels": ["Murmur"],
                "microphone_identity_labels": ["Murmur"],
                "microphone_prompt_count": 1,
                "additional_microphone_prompt_observed": False,
                "previous_source_sha": "f" * 40,
                "previous_capture_agent_sha256": "d" * 64,
                "previous_capture_worker_sha256": "e" * 64,
                "installed_capture_agent_sha256": "b" * 64,
                "installed_capture_worker_sha256": "c" * 64,
                "residual_agent_processes": 0,
                "residual_worker_processes": 0,
            },
            "records": {
                "service_initial": envelope(service("not_registered")),
                "service_register": envelope(service("enabled")),
                "synthetic_cooperative": envelope(probe("cooperative", True)),
                "synthetic_hard_kill": envelope(probe("hard_kill", True)),
                "continuity_active": envelope(
                    status("active", 100, "one", 2, 102, 64)
                ),
                "continuity_recovery": envelope(
                    {
                        **recovery(100, "one", 2, 102, True),
                        "ack_replay_verified": True,
                    }
                ),
                "continuity_second_claim": envelope(
                    {
                        "schema_version": 1,
                        "outcome": "already_acked",
                        "generation": 2,
                        "recovered": False,
                        "exact_once": True,
                        "audio_content_retained": False,
                    }
                ),
                "continuity_expired": envelope(
                    {
                        "schema_version": 1,
                        "outcome": "expired",
                        "generation": 2,
                        "recovered": False,
                        "audio_content_retained": False,
                    }
                ),
                "refresh_before": envelope(status("idle", 100, "one", 2, 0)),
                "service_refresh": envelope(service("enabled")),
                "refresh_after": envelope(status("idle", 200, "two", 0, 0)),
                "microphone_active": envelope(
                    status("active", 200, "two", 1, 201, 2)
                ),
                "microphone_recovery": envelope(
                    recovery(200, "two", 1, 201, False)
                ),
                "microphone_denied": envelope(
                    {
                        "schema_version": 1,
                        "outcome": "permission_denied",
                        "audio_content_retained": False,
                    },
                    2,
                ),
                "denied_status": envelope(status("idle", 200, "two", 1, 0)),
                "microphone_restored": envelope(probe("cooperative", False)),
                "update_before": envelope(status("idle", 200, "two", 2, 0)),
                "update_after": envelope(status("idle", 300, "three", 0, 0)),
                "service_unregister": envelope(service("not_registered")),
            },
        }

    def test_complete_matrix_is_valid(self) -> None:
        self.assertEqual(validate_matrix(self.matrix), self.matrix)

    def test_cross_record_identity_change_is_rejected(self) -> None:
        mutated = deepcopy(self.matrix)
        mutated["records"]["continuity_recovery"]["payload"]["worker_pid"] = 999
        with self.assertRaises(MatrixError):
            validate_matrix(mutated)

    def test_unconfirmed_group_and_incomplete_sequence_are_rejected(self) -> None:
        for step, key, value in (
            ("synthetic_hard_kill", "process_group_empty", False),
            ("synthetic_cooperative", "synthetic_last_sequence", 62),
            ("synthetic_hard_kill", "exit_signal", 0),
            ("continuity_recovery", "exit_signal", 9),
        ):
            mutated = deepcopy(self.matrix)
            mutated["records"][step]["payload"][key] = value
            with self.subTest(step=step, key=key):
                with self.assertRaises(MatrixError):
                    validate_matrix(mutated)

    def test_denied_preflight_must_not_advance_generation(self) -> None:
        mutated = deepcopy(self.matrix)
        mutated["records"]["denied_status"]["payload"]["generation"] = 2
        with self.assertRaises(MatrixError):
            validate_matrix(mutated)

    def test_ack_replay_and_prompt_evidence_are_required(self) -> None:
        for path, value in (
            (
                ("records", "continuity_recovery", "payload", "ack_replay_verified"),
                False,
            ),
            (("observations", "additional_microphone_prompt_observed"), True),
            (("observations", "microphone_prompt_count"), 2),
        ):
            mutated = deepcopy(self.matrix)
            target = mutated
            for key in path[:-1]:
                target = target[key]
            target[path[-1]] = value
            with self.subTest(path=path):
                with self.assertRaises(MatrixError):
                    validate_matrix(mutated)

    def test_previous_runtime_hashes_must_match_previous_provenance(self) -> None:
        mutated = deepcopy(self.matrix)
        mutated["observations"]["previous_capture_agent_sha256"] = "9" * 64
        with self.assertRaises(MatrixError):
            validate_matrix(mutated)
