from __future__ import annotations

import json
from pathlib import Path
import plistlib
import tempfile
import unittest

from scripts.capture_helper_evidence import (
    ALLOWED_PROBE_OUTCOMES,
    CAPTURE_ENTITLEMENTS,
    EvidenceError,
    structured_signature_evidence,
    validate_probe_evidence,
)
from scripts.finalize_macos_bundle import HELPERS, require_exact_macos_executables
from scripts.release_artifacts import (
    ArtifactError,
    create_provenance,
    validate_release,
    write_updater_manifests,
)


SHA = "1" * 40
OTHER_SHA = "2" * 40
RUN_ID = 123456


class ReleaseArtifactTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tempdir = tempfile.TemporaryDirectory()
        self.root = Path(self.tempdir.name)
        self.artifacts = self.root / "artifacts"
        macos = self.artifacts / "macos"
        linux = self.artifacts / "linux"
        macos.mkdir(parents=True)
        linux.mkdir(parents=True)

        (macos / "Murmur.dmg").write_bytes(b"dmg")
        (macos / "Murmur.app.tar.gz").write_bytes(b"mac updater")
        (macos / "Murmur.app.tar.gz.sig").write_text("mac-signature\n")
        (linux / "Murmur.deb").write_bytes(b"deb")
        (linux / "Murmur.AppImage").write_bytes(b"linux updater")
        (linux / "Murmur.AppImage.sig").write_text("linux-signature\n")

        create_provenance("macos", "darwin-aarch64", macos, SHA, RUN_ID)
        create_provenance("linux", "linux-x86_64", linux, SHA, RUN_ID)

    def tearDown(self) -> None:
        self.tempdir.cleanup()

    def test_valid_artifacts_and_manifest_signatures_match_sig_assets(self) -> None:
        validated_path = self.root / "validated.json"
        validate_release(self.artifacts, SHA, RUN_ID, validated_path)
        release_notes_path = self.root / "release-notes.md"
        release_notes_path.write_text(
            "## New Features\n\n- Added post-update release notes.\n",
            encoding="utf-8",
        )
        modern_path, legacy_path = write_updater_manifests(
            validated_path,
            "v1.2.3",
            "owner/repo",
            "https://example.invalid/bridge.app.tar.gz",
            "bridge-signature",
            release_notes_path,
            self.root / "manifests",
        )

        modern = json.loads(modern_path.read_text())
        legacy = json.loads(legacy_path.read_text())
        self.assertEqual(
            modern["platforms"]["darwin-aarch64"]["signature"], "mac-signature"
        )
        self.assertEqual(
            modern["platforms"]["linux-x86_64"]["signature"], "linux-signature"
        )
        self.assertEqual(
            legacy["platforms"]["darwin-aarch64"]["signature"],
            "bridge-signature",
        )
        self.assertEqual(
            legacy["platforms"]["linux-x86_64"]["signature"], "linux-signature"
        )
        self.assertEqual(
            modern["notes"],
            "## New Features\n\n- Added post-update release notes.",
        )

    def test_manifest_generation_requires_nonempty_release_notes(self) -> None:
        validated_path = self.root / "validated.json"
        validate_release(self.artifacts, SHA, RUN_ID, validated_path)
        release_notes_path = self.root / "release-notes.md"
        release_notes_path.write_text(" \n", encoding="utf-8")

        with self.assertRaisesRegex(ArtifactError, "must not be empty"):
            write_updater_manifests(
                validated_path,
                "v1.2.3",
                "owner/repo",
                "https://example.invalid/bridge.app.tar.gz",
                "bridge-signature",
                release_notes_path,
                self.root / "manifests",
            )

    def test_commit_sha_mismatch_fails_closed(self) -> None:
        with self.assertRaisesRegex(ArtifactError, "commit_sha mismatch"):
            validate_release(self.artifacts, OTHER_SHA, RUN_ID)

    def test_workflow_run_mismatch_fails_closed(self) -> None:
        with self.assertRaisesRegex(ArtifactError, "workflow_run_id mismatch"):
            validate_release(self.artifacts, SHA, RUN_ID + 1)

    def test_signature_tampering_fails_closed(self) -> None:
        signature = self.artifacts / "linux" / "Murmur.AppImage.sig"
        signature.write_text("xxxxx-signature\n")
        with self.assertRaisesRegex(ArtifactError, "SHA-256 mismatch"):
            validate_release(self.artifacts, SHA, RUN_ID)

    def test_missing_updater_signature_fails_closed(self) -> None:
        (self.artifacts / "macos" / "Murmur.app.tar.gz.sig").unlink()
        with self.assertRaisesRegex(ArtifactError, "artifact names differ"):
            validate_release(self.artifacts, SHA, RUN_ID)

    HELPER = {
        "sha256": "a" * 64,
        "architecture": "arm64",
        "designated_requirement": (
            'identifier "com.localdictation.local-llm-sidecar" and anchor apple generic '
            'and certificate leaf[subject.OU] = "ABCDE12345"'
        ),
        "team_id": "ABCDE12345",
        "entitlement_sha256": "b" * 64,
    }
    CAPTURE_HELPER = {
        **HELPER,
        "designated_requirement": (
            'identifier "com.localdictation.capture-helper" and anchor apple generic '
            'and certificate leaf[subject.OU] = "ABCDE12345"'
        ),
    }
    CAPTURE_PROBE = {
        "schema_version": 1,
        "outcome": "ok",
        "last_phase": "stopping",
        "helper_pid": 123,
        "first_callback_ms": 8,
        "elapsed_ms": 5000,
        "termination": "cooperative",
        "exit_code": 0,
        "exit_signal": None,
        "process_group_empty": True,
        "audio_content_retained": False,
    }

    def _rerecord_macos_with_helper(self, helper: dict) -> None:
        macos = self.artifacts / "macos"
        (macos / "provenance.json").unlink(missing_ok=True)
        create_provenance("macos", "darwin-aarch64", macos, SHA, RUN_ID, helper=helper)

    def test_helper_provenance_recorded_and_validated(self) -> None:
        self._rerecord_macos_with_helper(self.HELPER)
        result = validate_release(self.artifacts, SHA, RUN_ID, require_macos_helper=True)
        self.assertEqual(result["platforms"]["macos"]["helper"], self.HELPER)

    def test_helper_unquoted_team_id_requirement_is_valid(self) -> None:
        dr = (
            'identifier "com.localdictation.local-llm-sidecar" and anchor apple generic '
            'and certificate leaf[subject.OU] = ABCDE12345'
        )
        helper = {**self.HELPER, "designated_requirement": dr}
        self._rerecord_macos_with_helper(helper)
        result = validate_release(self.artifacts, SHA, RUN_ID, require_macos_helper=True)
        self.assertEqual(result["platforms"]["macos"]["helper"], helper)

    def test_require_macos_helper_fails_without_block(self) -> None:
        with self.assertRaisesRegex(ArtifactError, "missing the required local-LLM helper"):
            validate_release(self.artifacts, SHA, RUN_ID, require_macos_helper=True)

    def test_capture_helper_provenance_is_required_and_validated(self) -> None:
        macos = self.artifacts / "macos"
        (macos / "provenance.json").unlink()
        create_provenance(
            "macos",
            "darwin-aarch64",
            macos,
            SHA,
            RUN_ID,
            helper=self.HELPER,
            capture_helper=self.CAPTURE_HELPER,
        )
        result = validate_release(
            self.artifacts,
            SHA,
            RUN_ID,
            require_macos_helper=True,
            require_macos_capture_helper=True,
        )
        self.assertEqual(
            result["platforms"]["macos"]["capture_helper"], self.CAPTURE_HELPER
        )

    def test_require_capture_helper_fails_without_block(self) -> None:
        with self.assertRaisesRegex(ArtifactError, "required capture helper"):
            validate_release(
                self.artifacts,
                SHA,
                RUN_ID,
                require_macos_capture_helper=True,
            )

    def test_helper_wrong_architecture_fails_closed(self) -> None:
        with self.assertRaisesRegex(ArtifactError, "architecture must be arm64"):
            self._rerecord_macos_with_helper({**self.HELPER, "architecture": "x86_64"})

    def test_helper_bad_entitlement_digest_fails_closed(self) -> None:
        with self.assertRaisesRegex(ArtifactError, "entitlement_sha256"):
            self._rerecord_macos_with_helper({**self.HELPER, "entitlement_sha256": "short"})

    def test_helper_provenance_rejected_for_linux(self) -> None:
        linux = self.artifacts / "linux"
        (linux / "provenance.json").unlink()
        with self.assertRaisesRegex(ArtifactError, "only recorded for macos"):
            create_provenance("linux", "linux-x86_64", linux, SHA, RUN_ID, helper=self.HELPER)

    def test_helper_bad_team_id_fails_closed(self) -> None:
        with self.assertRaisesRegex(ArtifactError, "team_id must be a 10-character"):
            self._rerecord_macos_with_helper({**self.HELPER, "team_id": "abcde12345"})
        with self.assertRaisesRegex(ArtifactError, "team_id must be a 10-character"):
            self._rerecord_macos_with_helper({**self.HELPER, "team_id": "SHORT"})

    def test_helper_adhoc_cdhash_designated_requirement_rejected(self) -> None:
        with self.assertRaisesRegex(ArtifactError, "designated_requirement must pin"):
            self._rerecord_macos_with_helper(
                {**self.HELPER, "designated_requirement": 'cdhash H"deadbeefcafe"'}
            )

    def test_helper_designated_requirement_wrong_team_rejected(self) -> None:
        dr = (
            'identifier "com.localdictation.local-llm-sidecar" and anchor apple generic '
            'and certificate leaf[subject.OU] = "ZZZZZ99999"'
        )
        with self.assertRaisesRegex(ArtifactError, "designated_requirement must pin"):
            self._rerecord_macos_with_helper({**self.HELPER, "designated_requirement": dr})

    def test_helper_designated_requirement_team_prefix_rejected(self) -> None:
        dr = (
            'identifier "com.localdictation.local-llm-sidecar" and anchor apple generic '
            'and certificate leaf[subject.OU] = ABCDE12345EXTRA'
        )
        with self.assertRaisesRegex(ArtifactError, "designated_requirement must pin"):
            self._rerecord_macos_with_helper({**self.HELPER, "designated_requirement": dr})

    def test_helper_designated_requirement_wrong_operator_rejected(self) -> None:
        dr = (
            'identifier "com.localdictation.local-llm-sidecar" and anchor apple generic '
            'and certificate leaf[subject.OU] != ABCDE12345'
        )
        with self.assertRaisesRegex(ArtifactError, "designated_requirement must pin"):
            self._rerecord_macos_with_helper({**self.HELPER, "designated_requirement": dr})

    def test_helper_designated_requirement_or_branch_rejected(self) -> None:
        requirements = (
            'identifier "com.localdictation.local-llm-sidecar" or anchor apple generic '
            'and certificate leaf[subject.OU] = ABCDE12345',
            'identifier "com.localdictation.local-llm-sidecar" and anchor apple generic '
            'and certificate leaf[subject.OU] = ABCDE12345 or cdhash H"deadbeefcafe"',
        )
        for dr in requirements:
            with self.subTest(dr=dr):
                with self.assertRaisesRegex(ArtifactError, "designated_requirement must pin"):
                    self._rerecord_macos_with_helper(
                        {**self.HELPER, "designated_requirement": dr}
                    )

    def test_helper_designated_requirement_clause_prefix_decoys_rejected(self) -> None:
        requirements = (
            'notidentifier "com.localdictation.local-llm-sidecar" '
            'and anchor apple generic '
            'and certificate leaf[subject.OU] = ABCDE12345',
            'identifier "com.localdictation.local-llm-sidecar" '
            'and xanchor apple generic '
            'and certificate leaf[subject.OU] = ABCDE12345',
            'identifier "com.localdictation.local-llm-sidecar" '
            'and anchor apple generic '
            'and not certificate leaf[subject.OU] = ABCDE12345',
        )
        for dr in requirements:
            with self.subTest(dr=dr):
                with self.assertRaisesRegex(ArtifactError, "designated_requirement must pin"):
                    self._rerecord_macos_with_helper(
                        {**self.HELPER, "designated_requirement": dr}
                    )

    def test_validate_rejects_helper_block_on_linux(self) -> None:
        # A helper block must never appear on a non-macos platform, even if a
        # provenance file is hand-edited to smuggle one in.
        linux = self.artifacts / "linux"
        payload = json.loads((linux / "provenance.json").read_text())
        payload["helper"] = self.HELPER
        (linux / "provenance.json").write_text(json.dumps(payload))
        with self.assertRaisesRegex(ArtifactError, "must not carry a helper block"):
            validate_release(self.artifacts, SHA, RUN_ID)

    def test_macos_bundle_requires_exact_production_executables(self) -> None:
        app = self.root / "Murmur.app"
        executable_dir = app / "Contents" / "MacOS"
        executable_dir.mkdir(parents=True)
        main = executable_dir / "ui"
        helper = executable_dir / "murmur-llm-sidecar"
        capture_helper = executable_dir / "murmur-capture-helper"
        main.write_bytes(b"main")
        helper.write_bytes(b"helper")
        capture_helper.write_bytes(b"capture helper")

        require_exact_macos_executables(app, main, [capture_helper, helper])

        for unexpected in ("mock_llm_helper", "murmur-eval"):
            with self.subTest(unexpected=unexpected):
                extra = executable_dir / unexpected
                extra.write_bytes(b"developer tool")
                with self.assertRaisesRegex(
                    SystemExit, "app bundle executables differ"
                ):
                    require_exact_macos_executables(
                        app, main, [capture_helper, helper]
                    )
                extra.unlink()

    def test_macos_bundle_rejects_missing_production_executable(self) -> None:
        app = self.root / "Murmur.app"
        executable_dir = app / "Contents" / "MacOS"
        executable_dir.mkdir(parents=True)
        main = executable_dir / "ui"
        helper = executable_dir / "murmur-llm-sidecar"
        capture_helper = executable_dir / "murmur-capture-helper"
        main.write_bytes(b"main")
        helper.write_bytes(b"helper")

        with self.assertRaisesRegex(SystemExit, "app bundle executables differ"):
            require_exact_macos_executables(app, main, [capture_helper, helper])

    def test_capture_helper_identity_and_entitlements_are_exact(self) -> None:
        self.assertEqual(
            HELPERS["murmur-capture-helper"],
            "com.localdictation.capture-helper",
        )
        entitlement_path = (
            Path(__file__).parents[1]
            / "app/src-tauri/capture-helper.entitlements.plist"
        )
        with entitlement_path.open("rb") as handle:
            self.assertEqual(
                plistlib.load(handle),
                {
                    "com.apple.security.app-sandbox": True,
                    "com.apple.security.device.audio-input": True,
                    "com.apple.security.device.microphone": True,
                },
            )

    def test_capture_probe_requires_complete_confirmed_allowlisted_evidence(self) -> None:
        self.assertEqual(
            validate_probe_evidence(self.CAPTURE_PROBE, 0), self.CAPTURE_PROBE
        )
        for outcome in ALLOWED_PROBE_OUTCOMES:
            with self.subTest(allowed_outcome=outcome):
                validate_probe_evidence(
                    {**self.CAPTURE_PROBE, "outcome": outcome},
                    0 if outcome == "ok" else 2,
                )
        for outcome in (
            "signature_invalid",
            "spawn_failed",
            "protocol",
            "busy",
            "handshake_timeout",
            "invalid_message",
            "internal",
        ):
            with self.subTest(outcome=outcome):
                with self.assertRaisesRegex(EvidenceError, "not allowed"):
                    validate_probe_evidence(
                        {**self.CAPTURE_PROBE, "outcome": outcome}, 2
                    )
        self.assertNotIn("protocol", ALLOWED_PROBE_OUTCOMES)

        invalid_cases = (
            ({key: value for key, value in self.CAPTURE_PROBE.items() if key != "exit_signal"}, 0),
            ({**self.CAPTURE_PROBE, "process_group_empty": False}, 0),
            (
                {
                    **self.CAPTURE_PROBE,
                    "termination": "hard_kill",
                    "exit_code": None,
                    "exit_signal": None,
                },
                0,
            ),
            ({**self.CAPTURE_PROBE, "audio_content_retained": True}, 0),
            (self.CAPTURE_PROBE, 2),
        )
        for payload, probe_exit in invalid_cases:
            with self.subTest(payload=payload, probe_exit=probe_exit):
                with self.assertRaises(EvidenceError):
                    validate_probe_evidence(payload, probe_exit)

    def test_signature_evidence_allowlists_fields_and_drops_runner_paths(self) -> None:
        team_id = "ABCDE12345"
        runner_path = "/Users/runner/work/private/repo/murmur-capture-helper"
        details = (
            f"Executable={runner_path}\n"
            "Identifier=com.localdictation.capture-helper\n"
            f"TeamIdentifier={team_id}\n"
            "flags=0x10000(runtime)\n"
        )
        requirement = (
            f"Executable={runner_path}\n"
            'designated => identifier "com.localdictation.capture-helper" '
            "and anchor apple generic "
            f'and certificate leaf[subject.OU] = "{team_id}"'
        )
        evidence = structured_signature_evidence(
            details, requirement, CAPTURE_ENTITLEMENTS, "arm64"
        )
        serialized = json.dumps(evidence)
        self.assertNotIn(runner_path, serialized)
        self.assertEqual(
            set(evidence),
            {
                "schema_version",
                "identifier",
                "team_id",
                "architecture",
                "hardened_runtime",
                "designated_requirement",
                "entitlement_sha256",
                "entitlement_keys",
            },
        )


if __name__ == "__main__":
    unittest.main()
