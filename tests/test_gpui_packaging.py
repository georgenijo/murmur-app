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
from rehearse_gpui_bundle import check_links


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

    def test_signed_orchestration_wires_notarization_signer_and_helper_provenance(self):
        # All credentials and artifacts here are synthetic. No Apple/signing
        # command executes, but real provenance validation checks the result.
        import release_gpui
        output = self.root / "signed"
        environment = {"GITHUB_EVENT_NAME": "workflow_dispatch", "GITHUB_REF": "refs/heads/main",
                       "GITHUB_SHA": "a" * 40, "GITHUB_RUN_ID": "42",
                       "APPLE_SIGNING_IDENTITY": "synthetic identity", "APPLE_TEAM_ID": "ABCDEFGHIJ"}

        def process(command, **kwargs):
            if "scripts/notarize_and_repackage_macos.py" in command:
                (output / "dmg").mkdir()
                (output / "dmg/Murmur_0.62.1_aarch64.dmg").write_bytes(b"synthetic dmg")
                (output / "bundle/Murmur.app.tar.gz").write_bytes(b"synthetic archive")
                (output / "bundle/Murmur.app.tar.gz.sig").write_text("synthetic-signature")

        def capture(command, **kwargs):
            if command[0] == "lipo":
                return "arm64"
            name = Path(command[-1]).name
            return f'designated => identifier "{HELPERS[name]}" and anchor apple generic and certificate leaf[subject.OU] = "ABCDEFGHIJ"'

        with patch.dict("os.environ", environment, clear=True), \
             patch("sys.argv", ["release_gpui.py", "sign", "--target-dir", str(self.root / "target"), "--output", str(output)]), \
             patch("release_gpui.bundle", side_effect=lambda binary, app: bundle(self.binary, app, self.tauri)), \
             patch("release_gpui.check_links"), patch("release_gpui.finalize") as finalize, \
             patch("release_gpui.subprocess.run", side_effect=process) as commands, \
             patch("release_gpui.run", side_effect=capture), \
             patch("release_gpui.signature_details", return_value="TeamIdentifier=ABCDEFGHIJ\n"), \
             patch("release_gpui.entitlements", return_value={}):
            release_gpui.main()
        finalize.assert_called_once_with(output / "bundle/Murmur.app", "synthetic identity", "ABCDEFGHIJ")
        self.assertFalse(any(call.args[0][0] == "cargo" for call in commands.call_args_list),
                         "the credentialed sign phase must not compile")
        notarize = next(call.args[0] for call in commands.call_args_list
                        if "scripts/notarize_and_repackage_macos.py" in call.args[0])
        self.assertIn("--native-signer", notarize)
        self.assertEqual(notarize[-1], str(self.root / "target/release/murmur-updater-signer"))
        provenance = json.loads((output / "artifacts/provenance.json").read_text())
        self.assertEqual(provenance["bundle_target"], "gpui")
        self.assertIs(provenance["publication"], False)
        self.assertEqual(provenance["commit_sha"], "a" * 40)
        self.assertEqual(provenance["workflow_run_id"], 42)
        for name in ("capture_agent", "capture_helper", "capture_worker"):
            self.assertEqual(provenance[name]["team_id"], "ABCDEFGHIJ")
            self.assertEqual(provenance[name]["architecture"], "arm64")


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

    def test_macho_contract_rejects_missing_rpath_strong_framework_or_newer_minimum(self):
        loads = """Load command 0
cmd LC_RPATH
path /usr/lib/swift (offset 12)
Load command 1
cmd LC_LOAD_WEAK_DYLIB
name /System/Library/Frameworks/FoundationModels.framework/Versions/A/FoundationModels
Load command 2
cmd LC_BUILD_VERSION
minos 14.0
"""
        with tempfile.TemporaryDirectory() as temp:
            sdk = Path(temp)
            (sdk / "System/Library/Frameworks/FoundationModels.framework").mkdir(parents=True)
            for content, error in ((loads, None),
                                   (loads.replace("/usr/lib/swift", "/wrong/rpath"), "rpath missing"),
                                   (loads.replace("LC_LOAD_WEAK_DYLIB", "LC_LOAD_DYLIB"), "weak-linked"),
                                   (loads.replace("minos 14.0", "minos 15.0"), "minimum")):
                with self.subTest(error=error), patch("rehearse_gpui_bundle.run", side_effect=[content, str(sdk)]):
                    if error:
                        with self.assertRaisesRegex(ValueError, error):
                            check_links(Path("/synthetic/ui"))
                    else:
                        self.assertEqual(check_links(Path("/synthetic/ui"))["foundation_models"], "weak")

    def test_build_phase_compiles_without_credentials(self):
        import release_gpui
        environment = {"GITHUB_EVENT_NAME": "workflow_dispatch", "GITHUB_REF": "refs/heads/main",
                       "PATH": "/usr/bin", "HOME": "/synthetic/home"}
        with patch.dict("os.environ", environment, clear=True), \
             patch("sys.argv", ["release_gpui.py", "build", "--target-dir", "/synthetic/target"]), \
             patch("release_gpui.subprocess.run") as process:
            release_gpui.main()
        self.assertEqual([call.args[0][0] for call in process.call_args_list], ["cargo", "cargo"])
        for call in process.call_args_list:
            env = call.kwargs["env"]
            self.assertEqual(env["CARGO_TARGET_DIR"], "/synthetic/target")
            self.assertEqual(env["PATH"], "/usr/bin")
            self.assertFalse([name for name in env if release_gpui.is_credential(name)])

    def test_build_phase_refuses_any_credential_before_compiling(self):
        # Synthetic values only; the names mirror the signing secrets the
        # workflow injects into the later sign step.
        import release_gpui
        for name in ("APPLE_SIGNING_IDENTITY", "APPLE_ID", "APPLE_PASSWORD", "APPLE_TEAM_ID",
                     "APPLE_CERTIFICATE", "APPLE_CERTIFICATE_PASSWORD", "TAURI_SIGNING_PRIVATE_KEY",
                     "TAURI_SIGNING_PRIVATE_KEY_PASSWORD", "SIGNING_KEYCHAIN", "GITHUB_TOKEN"):
            with self.subTest(name=name), \
                 patch.dict("os.environ", {"GITHUB_EVENT_NAME": "workflow_dispatch",
                                           "GITHUB_REF": "refs/heads/main", name: "synthetic"}, clear=True), \
                 patch("sys.argv", ["release_gpui.py", "build", "--target-dir", "/synthetic/target"]), \
                 patch("release_gpui.subprocess.run") as process:
                with self.assertRaisesRegex(ValueError, "without credentials"):
                    release_gpui.main()
                process.assert_not_called()

    def test_workflow_rejects_credentials_or_keychain_before_the_build(self):
        workflow = (ROOT / ".github/workflows/release-build.yml").read_text()
        validate_release_build(workflow)
        build = "      - name: Build GPUI rehearsal binaries without credentials\n"
        gpui = job_block(workflow, "gpui-macos")
        self.assertLess(gpui.index(build), gpui.index("      - name: Import Developer ID certificate\n"))
        with_secret = workflow.replace(
            build + "        run: >-\n",
            build + "        env:\n          APPLE_ID: ${{ secrets.APPLE_ID }}\n        run: >-\n")
        with self.assertRaises(AssertionError):
            validate_release_build(with_secret)

    def test_signed_script_rejects_non_manual_or_non_main_before_work(self):
        import release_gpui
        for phase in ("build", "sign"):
            for event, ref, error in (("push", "refs/heads/main", ValueError),
                                      ("workflow_dispatch", "refs/heads/gpui/18-packaging", SystemExit)):
                with self.subTest(phase=phase, event=event, ref=ref), \
                     patch.dict("os.environ", {"GITHUB_EVENT_NAME": event, "GITHUB_REF": ref}, clear=True), \
                     patch("sys.argv", ["release_gpui.py", phase, "--target-dir", "/synthetic/target", "--output", "/synthetic/output"]), \
                     patch("release_gpui.bundle") as bundle_mock, \
                     patch("release_gpui.subprocess.run") as process:
                    with self.assertRaises(error):
                        release_gpui.main()
                    bundle_mock.assert_not_called()
                    process.assert_not_called()


if __name__ == "__main__":
    unittest.main()
