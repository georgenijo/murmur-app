from __future__ import annotations

import json
from pathlib import Path
import tempfile
import unittest

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
            'identifier "com.localdictation.local-llm-sidecar" and anchor apple generic'
        ),
        "team_id": "ABCDE12345",
        "entitlement_sha256": "b" * 64,
    }

    def _rerecord_macos_with_helper(self, helper: dict) -> None:
        macos = self.artifacts / "macos"
        (macos / "provenance.json").unlink()
        create_provenance("macos", "darwin-aarch64", macos, SHA, RUN_ID, helper=helper)

    def test_helper_provenance_recorded_and_validated(self) -> None:
        self._rerecord_macos_with_helper(self.HELPER)
        result = validate_release(self.artifacts, SHA, RUN_ID, require_macos_helper=True)
        self.assertEqual(result["platforms"]["macos"]["helper"], self.HELPER)

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


if __name__ == "__main__":
    unittest.main()
