from __future__ import annotations

import json
from pathlib import Path
import tempfile
import unittest

from scripts.finalize_macos_bundle import require_exact_macos_executables
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
        modern_path, legacy_path = write_updater_manifests(
            validated_path,
            "v1.2.3",
            "owner/repo",
            "https://example.invalid/bridge.app.tar.gz",
            "bridge-signature",
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
        main.write_bytes(b"main")
        helper.write_bytes(b"helper")

        require_exact_macos_executables(app, main, helper)

        for unexpected in ("mock_llm_helper", "murmur-eval"):
            with self.subTest(unexpected=unexpected):
                extra = executable_dir / unexpected
                extra.write_bytes(b"developer tool")
                with self.assertRaisesRegex(
                    SystemExit, "app bundle executables differ"
                ):
                    require_exact_macos_executables(app, main, helper)
                extra.unlink()

    def test_macos_bundle_rejects_missing_production_executable(self) -> None:
        app = self.root / "Murmur.app"
        executable_dir = app / "Contents" / "MacOS"
        executable_dir.mkdir(parents=True)
        main = executable_dir / "ui"
        helper = executable_dir / "murmur-llm-sidecar"
        main.write_bytes(b"main")

        with self.assertRaisesRegex(SystemExit, "app bundle executables differ"):
            require_exact_macos_executables(app, main, helper)


if __name__ == "__main__":
    unittest.main()
