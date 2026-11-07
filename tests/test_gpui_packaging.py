"""Packaging contracts, parity failures, and the publication boundary."""
import hashlib
import json
from pathlib import Path
import plistlib
import shutil
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
from bundle_gpui import TAURI, bundle, bundle_info, compare_bundles
from finalize_macos_bundle import HELPERS, require_exact_macos_executables
from release_bundle_target import validate_target
from validate_workflow_policy import job_block, validate_release_build, validate_promotion_policy
from notarize_and_repackage_macos import native_sign, tauri_sign


class BundleTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.tauri = self.root / "tauri"
        self.tauri.mkdir()
        for name in ("tauri.conf.json", "macos/Info.plist", "macos/com.localdictation.capture-agent.plist", "icons/icon.icns"):
            target = self.tauri / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(TAURI / name, target)
        (self.tauri / "binaries").mkdir()
        for name in HELPERS:
            (self.tauri / "binaries" / f"{name}-aarch64-apple-darwin").write_bytes(b"synthetic helper")
        self.binary = self.root / "ui"
        self.binary.write_bytes(b"synthetic main")
        self.app = bundle(self.binary, self.root / "native.app", self.tauri)
        self.oracle = self.root / "oracle.app"
        shutil.copytree(self.app, self.oracle)

    def test_identity_version_and_permission_descriptions_share_tauri_sources(self):
        info = bundle_info(self.tauri)
        config = json.loads((self.tauri / "tauri.conf.json").read_text())
        self.assertEqual(info["CFBundleExecutable"], "ui")
        self.assertEqual(info["LSMinimumSystemVersion"], "14.0")
        self.assertEqual(info["CFBundleIdentifier"], config["identifier"])
        for key in ("CFBundleVersion", "CFBundleShortVersionString"):
            self.assertEqual(info[key], config["version"])
        permissions = plistlib.loads((self.tauri / "macos/Info.plist").read_bytes())
        self.assertEqual({k: info[k] for k in permissions}, permissions)
        config["version"] = "99.98.97"
        (self.tauri / "tauri.conf.json").write_text(json.dumps(config))
        self.assertEqual(bundle_info(self.tauri)["CFBundleVersion"], "99.98.97")

    def test_exact_parity_and_code_signature_seal_exception(self):
        seal = self.oracle / "Contents/_CodeSignature"
        seal.mkdir()
        (seal / "CodeResources").write_bytes(b"synthetic seal")
        evidence = compare_bundles(self.app, self.oracle)
        self.assertEqual(evidence["plist_diff"], [])
        self.assertEqual(evidence["layout_diff"], [])
        self.assertIn("Contents/Library/LaunchAgents/com.localdictation.capture-agent.plist", evidence["layout"])

    def test_parity_rejects_missing_added_or_changed_plist_keys(self):
        path = self.oracle / "Contents/Info.plist"
        initial = plistlib.loads(path.read_bytes())
        for mutation in ({**initial, "NSUnexpected": True},
                         {**initial, "LSMinimumSystemVersion": "15.0"},
                         {k: v for k, v in initial.items() if k != "NSMicrophoneUsageDescription"}):
            with self.subTest(mutation=mutation):
                path.write_bytes(plistlib.dumps(mutation))
                with self.assertRaisesRegex(ValueError, "Info.plist differs"):
                    compare_bundles(self.app, self.oracle)

    def test_parity_rejects_missing_helper_and_extra_layout(self):
        helper = self.oracle / "Contents/MacOS/murmur-capture-worker"
        helper.unlink()
        with self.assertRaisesRegex(ValueError, "layout differs"):
            compare_bundles(self.app, self.oracle)
        helper.write_bytes(b"helper")
        (self.oracle / "Contents/Resources/extra").write_bytes(b"extra")
        with self.assertRaisesRegex(ValueError, "layout differs"):
            compare_bundles(self.app, self.oracle)

    def test_parity_rejects_different_launch_agent_bytes(self):
        path = self.oracle / "Contents/Library/LaunchAgents/com.localdictation.capture-agent.plist"
        path.write_bytes(b"wrong agent")
        with self.assertRaisesRegex(ValueError, "resource differs"):
            compare_bundles(self.app, self.oracle)

    def test_allowlist_refuses_qa_main_even_when_plist_would_name_it(self):
        macos = self.app / "Contents/MacOS"
        (macos / "ui").rename(macos / "remaining-qa")
        with self.assertRaisesRegex(SystemExit, "not allowlisted"):
            require_exact_macos_executables(self.app, macos / "remaining-qa", [macos / name for name in HELPERS])

    def test_symlinked_main_refused(self):
        macos = self.app / "Contents/MacOS"
        (macos / "ui").unlink()
        (macos / "ui").symlink_to(self.binary)
        with self.assertRaisesRegex(SystemExit, "regular files"):
            require_exact_macos_executables(self.app, macos / "ui", [macos / name for name in HELPERS])

    def test_output_is_never_overwritten(self):
        with self.assertRaisesRegex(ValueError, "new .app"):
            bundle(self.binary, self.app, self.tauri)


class WorkflowTests(unittest.TestCase):
    def test_target_event_matrix(self):
        for event in ("push", "pull_request", "workflow_run", "workflow_dispatch", "schedule"):
            with self.subTest(event=event):
                self.assertEqual(validate_target(event, "tauri"), "tauri")
                if event == "workflow_dispatch":
                    self.assertEqual(validate_target(event, "gpui"), "gpui")
                else:
                    with self.assertRaisesRegex(ValueError, "manual rehearsals"):
                        validate_target(event, "gpui")
        for invalid in ("", "GPUI", "unknown", "gpui\npublish"):
            with self.assertRaises(ValueError):
                validate_target("workflow_dispatch", invalid)

    def test_workflow_guards_fail_closed_if_removed(self):
        workflow = (ROOT / ".github/workflows/release-build.yml").read_text()
        validate_release_build(workflow)
        for guard in ("github.event_name == 'workflow_dispatch' && needs.context.outputs.bundle-target == 'gpui'",
                      'TARGET=$(python3 scripts/release_bundle_target.py) || exit 1',
                      "needs.context.outputs.bundle-target == 'tauri'"):
            with self.subTest(guard=guard), self.assertRaises(AssertionError):
                validate_release_build(workflow.replace(guard, "true"))
        promotion = (ROOT / ".github/workflows/release.yml").read_text()
        validate_promotion_policy(promotion)
        self.assertNotIn("gpui-rehearsal-", promotion)

    def test_tauri_release_job_is_unchanged_except_target_selection(self):
        workflow = (ROOT / ".github/workflows/release-build.yml").read_text()
        job = job_block(workflow, "release-macos")
        job = job.replace("    if: needs.context.outputs.bundle-target == 'tauri'\n", "")
        self.assertEqual(hashlib.sha256(job.rstrip().encode()).hexdigest(), "88721a915e7bb8f56b9562e77736fd05536374349ef1365846fdf82a6bc62cdb")

    def test_native_signer_is_opt_in_and_uses_public_config(self):
        with tempfile.TemporaryDirectory() as temp:
            archive = Path(temp) / "Murmur.app.tar.gz"
            Path(f"{archive}.sig").write_text("synthetic signature")
            with patch("notarize_and_repackage_macos.run") as run:
                native_sign(archive, ROOT / "app", Path("/synthetic/signer"))
                self.assertEqual(run.call_args.args[0], ["/synthetic/signer", str(archive)])
                self.assertEqual(run.call_args.kwargs["env"]["MURMUR_SIGNING_PUBLIC_KEY"],
                                 json.loads((TAURI / "tauri.conf.json").read_text())["plugins"]["updater"]["pubkey"])
            with patch.dict("os.environ", {"TAURI_SIGNING_PRIVATE_KEY": "synthetic"}), patch("notarize_and_repackage_macos.run") as run:
                tauri_sign(archive, ROOT / "app")
                self.assertEqual(run.call_args.args[0][:5], ["npx", "--no-install", "tauri", "signer", "sign"])


if __name__ == "__main__":
    unittest.main()
