from __future__ import annotations

import json
from pathlib import Path
import plistlib
import tempfile
import tomllib
import unittest

from scripts.capture_helper_evidence import (
    ALLOWED_PROBE_OUTCOMES,
    CAPTURE_AGENT_ENTITLEMENTS,
    CAPTURE_AGENT_IDENTIFIER,
    CAPTURE_WORKER_ENTITLEMENTS,
    CAPTURE_WORKER_IDENTIFIER,
    CAPTURE_ENTITLEMENTS,
    EvidenceError,
    PROBE_OUTCOME_CONTRACTS,
    extract_entitlements_plist,
    structured_signature_evidence,
    validate_probe_evidence,
)
from scripts.finalize_macos_bundle import (
    HELPERS,
    decode_otool_info_plist,
    require_exact_macos_executables,
)
from scripts.release_artifacts import (
    ArtifactError,
    create_provenance,
    validate_release,
    verify_release_notes_match,
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

    def test_publish_check_normalizes_crlf_and_outer_whitespace_symmetrically(self) -> None:
        manifest_path = self.root / "latest-v2.json"
        manifest_path.write_text(
            json.dumps(
                {
                    "notes": (
                        "## New Features\n\n"
                        "- Added post-update release notes."
                    )
                }
            ),
            encoding="utf-8",
        )
        draft_path = self.root / "draft-release-notes.md"
        draft_path.write_bytes(
            b" \r\n## New Features\r\n\r\n"
            b"- Added post-update release notes.   \r\n\t"
        )

        verify_release_notes_match(manifest_path, draft_path)

    def test_publish_check_rejects_release_note_content_changes(self) -> None:
        manifest_path = self.root / "latest-v2.json"
        manifest_path.write_text(
            json.dumps({"notes": "## Bug Fixes\n\n- Fixed the release gate."}),
            encoding="utf-8",
        )
        draft_path = self.root / "draft-release-notes.md"
        draft_path.write_text(
            "## Bug Fixes\n\n- Edited after manifest generation.\n",
            encoding="utf-8",
        )

        with self.assertRaisesRegex(
            ArtifactError, "draft release notes differ from the updater manifest"
        ):
            verify_release_notes_match(manifest_path, draft_path)

    def test_manifest_generation_includes_valid_minimum_version_on_modern_channel(self) -> None:
        validated_path = self.root / "validated.json"
        validate_release(self.artifacts, SHA, RUN_ID, validated_path)
        release_notes_path = self.root / "release-notes.md"
        release_notes_path.write_text("Required reliability update.\n")

        modern_path, legacy_path = write_updater_manifests(
            validated_path,
            "v1.2.3",
            "owner/repo",
            "https://example.invalid/bridge.app.tar.gz",
            "bridge-signature",
            release_notes_path,
            self.root / "manifests",
            min_version="1.1.0",
        )

        self.assertEqual(json.loads(modern_path.read_text())["min_version"], "1.1.0")
        self.assertNotIn("min_version", json.loads(legacy_path.read_text()))

    def test_manifest_generation_omits_absent_minimum_version(self) -> None:
        validated_path = self.root / "validated.json"
        validate_release(self.artifacts, SHA, RUN_ID, validated_path)
        release_notes_path = self.root / "release-notes.md"
        release_notes_path.write_text("Optional update.\n")

        modern_path, _ = write_updater_manifests(
            validated_path,
            "v1.2.3",
            "owner/repo",
            "https://example.invalid/bridge.app.tar.gz",
            "bridge-signature",
            release_notes_path,
            self.root / "manifests",
        )

        self.assertNotIn("min_version", json.loads(modern_path.read_text()))

    def test_manifest_generation_rejects_invalid_or_future_minimum_version(self) -> None:
        validated_path = self.root / "validated.json"
        validate_release(self.artifacts, SHA, RUN_ID, validated_path)
        release_notes_path = self.root / "release-notes.md"
        release_notes_path.write_text("Policy validation.\n")
        common = (
            validated_path,
            "v1.2.3",
            "owner/repo",
            "https://example.invalid/bridge.app.tar.gz",
            "bridge-signature",
            release_notes_path,
            self.root / "manifests",
        )

        for invalid in ("", "v1.0.0", "1.2", "1.2.3-beta"):
            with self.subTest(invalid=invalid):
                with self.assertRaisesRegex(ArtifactError, "stable major.minor.patch"):
                    write_updater_manifests(*common, min_version=invalid)

        with self.assertRaisesRegex(ArtifactError, "newer than release"):
            write_updater_manifests(*common, min_version="1.2.4")

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
    CAPTURE_AGENT = {
        **HELPER,
        "designated_requirement": (
            'identifier "com.localdictation.capture-agent" and anchor apple generic '
            'and certificate leaf[subject.OU] = "ABCDE12345"'
        ),
    }
    CAPTURE_WORKER = {
        **HELPER,
        "designated_requirement": (
            'identifier "com.localdictation.capture-worker" and anchor apple generic '
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

    def test_capture_agent_provenance_is_required_and_validated(self) -> None:
        macos = self.artifacts / "macos"
        (macos / "provenance.json").unlink()
        create_provenance(
            "macos",
            "darwin-aarch64",
            macos,
            SHA,
            RUN_ID,
            capture_agent=self.CAPTURE_AGENT,
        )
        result = validate_release(
            self.artifacts,
            SHA,
            RUN_ID,
            require_macos_capture_agent=True,
        )
        self.assertEqual(
            result["platforms"]["macos"]["capture_agent"], self.CAPTURE_AGENT
        )

    def test_require_capture_agent_fails_without_block(self) -> None:
        with self.assertRaisesRegex(ArtifactError, "required capture agent"):
            validate_release(
                self.artifacts,
                SHA,
                RUN_ID,
                require_macos_capture_agent=True,
            )

    def test_capture_worker_provenance_is_required_and_validated(self) -> None:
        macos = self.artifacts / "macos"
        (macos / "provenance.json").unlink()
        create_provenance(
            "macos",
            "darwin-aarch64",
            macos,
            SHA,
            RUN_ID,
            capture_worker=self.CAPTURE_WORKER,
        )
        result = validate_release(
            self.artifacts,
            SHA,
            RUN_ID,
            require_macos_capture_worker=True,
        )
        self.assertEqual(
            result["platforms"]["macos"]["capture_worker"], self.CAPTURE_WORKER
        )

    def test_require_capture_worker_fails_without_block(self) -> None:
        with self.assertRaisesRegex(ArtifactError, "required capture worker"):
            validate_release(
                self.artifacts,
                SHA,
                RUN_ID,
                require_macos_capture_worker=True,
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
        capture_agent = executable_dir / "murmur-capture-agent"
        capture_helper = executable_dir / "murmur-capture-helper"
        capture_worker = executable_dir / "murmur-capture-worker"
        main.write_bytes(b"main")
        helper.write_bytes(b"helper")
        capture_agent.write_bytes(b"capture agent")
        capture_helper.write_bytes(b"capture helper")
        capture_worker.write_bytes(b"capture worker")

        require_exact_macos_executables(
            app, main, [capture_agent, capture_helper, capture_worker, helper]
        )

        for unexpected in ("mock_llm_helper", "murmur-eval"):
            with self.subTest(unexpected=unexpected):
                extra = executable_dir / unexpected
                extra.write_bytes(b"developer tool")
                with self.assertRaisesRegex(
                    SystemExit, "app bundle executables differ"
                ):
                    require_exact_macos_executables(
                        app, main, [capture_agent, capture_helper, capture_worker, helper]
                    )
                extra.unlink()

    def test_macos_bundle_rejects_missing_production_executable(self) -> None:
        app = self.root / "Murmur.app"
        executable_dir = app / "Contents" / "MacOS"
        executable_dir.mkdir(parents=True)
        main = executable_dir / "ui"
        helper = executable_dir / "murmur-llm-sidecar"
        capture_agent = executable_dir / "murmur-capture-agent"
        capture_helper = executable_dir / "murmur-capture-helper"
        capture_worker = executable_dir / "murmur-capture-worker"
        main.write_bytes(b"main")
        helper.write_bytes(b"helper")
        capture_agent.write_bytes(b"capture agent")
        capture_worker.write_bytes(b"capture worker")

        with self.assertRaisesRegex(SystemExit, "app bundle executables differ"):
            require_exact_macos_executables(
                app, main, [capture_agent, capture_helper, capture_worker, helper]
            )

    def test_embedded_info_plist_accepts_otool_short_final_word(self) -> None:
        payload = plistlib.dumps({"CFBundleIdentifier": "com.localdictation.test"})
        # XML permits trailing whitespace. Make the final `otool -X` group
        # deliberately shorter than four bytes, matching valid Mach-O sections
        # whose embedded plist length is not word-aligned.
        while len(payload) % 4 == 0:
            payload += b"\n"
        words = [
            chunk[::-1].hex()
            for start in range(0, len(payload), 4)
            for chunk in (payload[start : start + 4],)
        ]
        self.assertLess(len(words[-1]), 8)
        output = "0000000100000000\t" + " ".join(words) + "\n"

        self.assertEqual(
            decode_otool_info_plist(output),
            {"CFBundleIdentifier": "com.localdictation.test"},
        )

    def test_embedded_info_plist_rejects_odd_length_otool_word(self) -> None:
        with self.assertRaisesRegex(SystemExit, "malformed words"):
            decode_otool_info_plist("0000000100000000\t123\n")

    def test_capture_agent_identity_entitlements_and_launchd_plist_are_exact(
        self,
    ) -> None:
        self.assertEqual(
            HELPERS["murmur-capture-agent"],
            "com.localdictation.capture-agent",
        )
        self.assertEqual(
            HELPERS["murmur-capture-worker"],
            CAPTURE_WORKER_IDENTIFIER,
        )
        self.assertEqual(CAPTURE_WORKER_IDENTIFIER, "com.localdictation.capture-worker")
        root = Path(__file__).parents[1]
        with (root / "app/src-tauri/capture-agent.entitlements.plist").open(
            "rb"
        ) as handle:
            self.assertEqual(
                plistlib.load(handle),
                {
                    "com.apple.security.app-sandbox": True,
                    "com.apple.security.device.audio-input": True,
                    "com.apple.security.device.microphone": True,
                    "com.apple.security.temporary-exception.mach-lookup.global-name": [
                        "com.localdictation.capture-agent.xpc"
                    ],
                    "com.apple.security.temporary-exception.mach-register.global-name": [
                        "com.localdictation.capture-agent.xpc"
                    ],
                },
            )

        agent_source = (
            root / "app/src-tauri/sidecars/capture-agent/main.swift"
        ).read_text(encoding="utf-8")
        for forbidden_capture_api in (
            "AVFoundation",
            "AVAudioEngine",
            "AudioUnit",
            "AudioQueue",
            "CoreAudio",
            "CPAL",
        ):
            self.assertNotIn(forbidden_capture_api, agent_source)
        self.assertIn('"murmur-capture-worker"', agent_source)
        self.assertIn("SecCodeCopyPath", agent_source)
        self.assertIn("currentExecutableURL()", agent_source)
        self.assertNotIn("CommandLine.arguments[0]", agent_source)
        with (root / "app/src-tauri/capture-worker.entitlements.plist").open(
            "rb"
        ) as handle:
            self.assertEqual(plistlib.load(handle), CAPTURE_WORKER_ENTITLEMENTS)
        with (
            root
            / "app/src-tauri/macos/com.localdictation.capture-agent.plist"
        ).open("rb") as handle:
            self.assertEqual(
                plistlib.load(handle),
                {
                    "Label": "com.localdictation.capture-agent",
                    "BundleProgram": "Contents/MacOS/murmur-capture-agent",
                    "MachServices": {
                        "com.localdictation.capture-agent.xpc": True
                    },
                    "ProcessType": "Interactive",
                    "ThrottleInterval": 10,
                },
            )
        with (root / "app/src-tauri/capture-agent-info.plist").open(
            "rb"
        ) as handle:
            self.assertEqual(
                plistlib.load(handle),
                {
                    "CFBundleExecutable": "murmur-capture-agent",
                    "CFBundleIdentifier": "com.localdictation.capture-agent",
                    "CFBundleName": "Murmur Capture Agent",
                    "CFBundlePackageType": "APPL",
                    "CFBundleShortVersionString": "__MURMUR_VERSION__",
                    "CFBundleVersion": "__MURMUR_VERSION__",
                },
            )
        build_script = (root / "scripts/build_local_llm_sidecar.py").read_text()
        self.assertIn('"capture-agent-info.plist"', build_script)
        self.assertIn('"__info_plist"', build_script)

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

    def test_capture_executable_info_templates_are_role_specific_and_versioned(self) -> None:
        capture_helper = (
            Path(__file__).parents[1] / "app/src-tauri/sidecars/capture"
        )
        with (capture_helper / "Info.plist").open("rb") as handle:
            helper_info = plistlib.load(handle)
        with (capture_helper / "WorkerInfo.plist").open("rb") as handle:
            worker_info = plistlib.load(handle)

        for info in (helper_info, worker_info):
            self.assertEqual(info["CFBundleShortVersionString"], "__MURMUR_VERSION__")
            self.assertEqual(info["CFBundleVersion"], "__MURMUR_VERSION__")
            self.assertIn("NSMicrophoneUsageDescription", info)
        self.assertEqual(
            helper_info["CFBundleIdentifier"],
            "com.localdictation.capture-helper",
        )
        self.assertEqual(
            worker_info["CFBundleIdentifier"],
            "com.localdictation.capture-worker",
        )
        self.assertEqual(helper_info["CFBundleExecutable"], "murmur-capture-helper")
        self.assertEqual(worker_info["CFBundleExecutable"], "murmur-capture-worker")
        build_script = (capture_helper / "build.rs").read_text()
        self.assertIn("MURMUR_CAPTURE_ROLE", build_script)
        self.assertIn("MURMUR_APP_VERSION", build_script)

    def test_capture_probe_requires_complete_confirmed_allowlisted_evidence(self) -> None:
        self.assertEqual(
            validate_probe_evidence(self.CAPTURE_PROBE, 0), self.CAPTURE_PROBE
        )
        self.assertEqual(set(PROBE_OUTCOME_CONTRACTS), set(ALLOWED_PROBE_OUTCOMES))
        for outcome, contracts in PROBE_OUTCOME_CONTRACTS.items():
            for phase, callback_seen, termination, exit_code, exit_signal in contracts:
                with self.subTest(
                    allowed_outcome=outcome,
                    phase=phase,
                    callback_seen=callback_seen,
                ):
                    payload = {
                        **self.CAPTURE_PROBE,
                        "outcome": outcome,
                        "last_phase": phase,
                        "first_callback_ms": 8 if callback_seen else None,
                        "termination": termination,
                        "exit_code": exit_code,
                        "exit_signal": exit_signal,
                    }
                    self.assertEqual(
                        validate_probe_evidence(
                            payload, 0 if outcome == "ok" else 2
                        ),
                        payload,
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
            ({**self.CAPTURE_PROBE, "schema_version": True}, 0),
            ({**self.CAPTURE_PROBE, "outcome": True}, 0),
            ({**self.CAPTURE_PROBE, "last_phase": True}, 0),
            ({**self.CAPTURE_PROBE, "termination": True}, 0),
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
            (self.CAPTURE_PROBE, False),
        )
        for payload, probe_exit in invalid_cases:
            with self.subTest(payload=payload, probe_exit=probe_exit):
                with self.assertRaises(EvidenceError):
                    validate_probe_evidence(payload, probe_exit)

    def test_capture_probe_rejects_cross_field_contract_mutations(self) -> None:
        for outcome, contracts in PROBE_OUTCOME_CONTRACTS.items():
            for phase, callback_seen, termination, exit_code, exit_signal in contracts:
                baseline = {
                    **self.CAPTURE_PROBE,
                    "outcome": outcome,
                    "last_phase": phase,
                    "first_callback_ms": 8 if callback_seen else None,
                    "termination": termination,
                    "exit_code": exit_code,
                    "exit_signal": exit_signal,
                }
                mutations = (
                    {**baseline, "last_phase": None},
                    {
                        **baseline,
                        "first_callback_ms": None if callback_seen else 8,
                    },
                    {
                        **baseline,
                        "termination": "exited"
                        if termination == "cooperative"
                        else "cooperative",
                    },
                    {
                        **baseline,
                        "termination": "hard_kill",
                        "exit_code": None,
                        "exit_signal": 9,
                    },
                    {**baseline, "exit_code": 77},
                )
                for mutation in mutations:
                    with self.subTest(outcome=outcome, mutation=mutation):
                        with self.assertRaises(EvidenceError):
                            validate_probe_evidence(
                                mutation, 0 if outcome == "ok" else 2
                            )

    def test_capture_probe_rejects_reported_adversarial_combinations(self) -> None:
        contradictions = (
            (
                {
                    **self.CAPTURE_PROBE,
                    "last_phase": None,
                    "first_callback_ms": None,
                    "termination": "hard_kill",
                    "exit_code": None,
                    "exit_signal": 9,
                },
                0,
            ),
            (
                {
                    **self.CAPTURE_PROBE,
                    "outcome": "no_first_callback",
                    "first_callback_ms": 8,
                },
                2,
            ),
            (
                {
                    **self.CAPTURE_PROBE,
                    "outcome": "permission_denied",
                    "last_phase": "active",
                    "termination": "exited",
                },
                2,
            ),
            (
                {
                    **self.CAPTURE_PROBE,
                    "outcome": "callback_stalled",
                    "last_phase": None,
                    "first_callback_ms": None,
                    "termination": "exited",
                    "exit_code": 77,
                },
                2,
            ),
        )
        for payload, probe_exit in contradictions:
            with self.subTest(payload=payload):
                with self.assertRaisesRegex(EvidenceError, "contract"):
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
            "and certificate 1[field.1.2.840.113635.100.6.2.6] /* exists */ "
            "and certificate leaf[field.1.2.840.113635.100.6.1.13] /* exists */ "
            f'and certificate leaf[subject.OU] = "{team_id}"'
        )
        evidence = structured_signature_evidence(
            details, requirement, CAPTURE_ENTITLEMENTS, "arm64"
        )
        serialized = json.dumps(evidence)
        self.assertNotIn(runner_path, serialized)
        self.assertNotIn("anchor apple generic", serialized)
        self.assertEqual(
            set(evidence),
            {
                "schema_version",
                "identifier",
                "team_id",
                "architecture",
                "hardened_runtime",
                "designated_requirement_profile",
                "designated_requirement_sha256",
                "entitlement_sha256",
                "entitlement_keys",
            },
        )

    def test_capture_agent_signature_evidence_uses_exact_identity_and_policy(
        self,
    ) -> None:
        team_id = "ABCDE12345"
        details = (
            f"Identifier={CAPTURE_AGENT_IDENTIFIER}\n"
            f"TeamIdentifier={team_id}\n"
            "flags=0x10000(runtime)\n"
        )
        requirement = (
            f'designated => identifier "{CAPTURE_AGENT_IDENTIFIER}" '
            "and anchor apple generic "
            "and certificate 1[field.1.2.840.113635.100.6.2.6] /* exists */ "
            "and certificate leaf[field.1.2.840.113635.100.6.1.13] /* exists */ "
            f'and certificate leaf[subject.OU] = "{team_id}"'
        )
        evidence = structured_signature_evidence(
            details,
            requirement,
            CAPTURE_AGENT_ENTITLEMENTS,
            "arm64",
            expected_identifier=CAPTURE_AGENT_IDENTIFIER,
            expected_entitlements=CAPTURE_AGENT_ENTITLEMENTS,
            label="capture-agent",
        )
        self.assertEqual(evidence["identifier"], CAPTURE_AGENT_IDENTIFIER)
        self.assertEqual(
            evidence["entitlement_keys"], sorted(CAPTURE_AGENT_ENTITLEMENTS)
        )

    def test_signature_evidence_rejects_extra_path_bearing_requirement_clause(
        self,
    ) -> None:
        team_id = "ABCDE12345"
        runner_path = "/Users/runner/work/private/repo"
        details = (
            "Identifier=com.localdictation.capture-helper\n"
            f"TeamIdentifier={team_id}\n"
            "flags=0x10000(runtime)\n"
        )
        requirement = (
            'designated => identifier "com.localdictation.capture-helper" '
            "and anchor apple generic "
            "and certificate 1[field.1.2.840.113635.100.6.2.6] /* exists */ "
            "and certificate leaf[field.1.2.840.113635.100.6.1.13] /* exists */ "
            f'and certificate leaf[subject.OU] = "{team_id}" '
            f'and info["trace"] = "{runner_path}"'
        )
        with self.assertRaisesRegex(EvidenceError, "exact canonical"):
            structured_signature_evidence(
                details, requirement, CAPTURE_ENTITLEMENTS, "arm64"
            )

    def test_entitlements_extraction_bounds_realistic_codesign_noise(self) -> None:
        runner_path = b"/Users/runner/work/private/repo/murmur-capture-helper"
        xml = plistlib.dumps(CAPTURE_ENTITLEMENTS, sort_keys=True)
        payload = (
            b"Executable="
            + runner_path
            + b"\nwarning: codesign diagnostic preamble\n"
            + xml
            + b"\nExecutable="
            + runner_path
            + b"\nwarning: codesign trailing diagnostic\n"
        )
        entitlements = extract_entitlements_plist(payload)
        self.assertEqual(entitlements, CAPTURE_ENTITLEMENTS)
        canonical_output = plistlib.dumps(entitlements, sort_keys=True)
        self.assertNotIn(runner_path, canonical_output)
        self.assertNotIn(b"codesign", canonical_output)

    def test_entitlements_extraction_rejects_missing_or_incomplete_xml(self) -> None:
        xml = plistlib.dumps(CAPTURE_ENTITLEMENTS, sort_keys=True)
        cases = (
            b"Executable=/Users/runner/work/private/repo/helper\n",
            xml[: xml.index(b"</plist>")],
            xml[xml.index(b"<plist") :],
        )
        for payload in cases:
            with self.subTest(payload=payload):
                with self.assertRaises(EvidenceError):
                    extract_entitlements_plist(payload)

    def test_entitlements_extraction_rejects_malformed_xml(self) -> None:
        xml = plistlib.dumps(CAPTURE_ENTITLEMENTS, sort_keys=True)
        malformed = xml.replace(b"<true/>", b"<true>", 1)
        with self.assertRaisesRegex(EvidenceError, "malformed"):
            extract_entitlements_plist(malformed)

    def test_entitlements_extraction_rejects_ambiguous_xml(self) -> None:
        xml = plistlib.dumps(CAPTURE_ENTITLEMENTS, sort_keys=True)
        cases = (
            xml + b"\n" + xml,
            xml + b"\n<?xml trailing diagnostic",
            xml + b"\n</plist>",
        )
        for payload in cases:
            with self.subTest(payload=payload):
                with self.assertRaisesRegex(EvidenceError, "ambiguous"):
                    extract_entitlements_plist(payload)


if __name__ == "__main__":
    unittest.main()
