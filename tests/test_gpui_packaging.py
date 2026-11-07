"""Packaging contracts, parity failures, and the publication boundary."""
import hashlib
import io
import json
from pathlib import Path
import plistlib
import shutil
import stat
import sys
import tarfile
import tempfile
import tomllib
import unittest
import warnings
import zipfile
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
from bundle_gpui import TAURI, bundle, bundle_info, compare_bundles
from finalize_macos_bundle import HELPERS, require_exact_macos_executables
import gpui_artifacts
from gpui_artifacts import BUILD_FILES, SIGNER_FILES, pack, sha256, verify, verify_zip
from release_bundle_target import validate_target
from validate_workflow_policy import (GPUI_JOB_SHA256, RELEASE_BUILD_SHA256, job_block, references_secrets,
                                      validate_release_build, validate_promotion_policy)
from notarize_and_repackage_macos import native_sign, tauri_sign
from rehearse_gpui_bundle import check_links, isolated_launch


class BundleTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
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

    def test_only_production_entry_is_packaged(self):
        manifest = tomllib.loads((ROOT / "app/gpui/Cargo.toml").read_text())
        self.assertFalse(manifest["package"]["autobins"])
        self.assertEqual(manifest["package"]["default-run"], "ui")
        entry = next(target for target in manifest["bin"] if target["name"] == "ui")
        self.assertEqual(entry["path"], "src/main.rs")
        self.assertEqual(entry["required-features"], ["production-host"])
        self.assertFalse((ROOT / "app/gpui/src/bin/ui.rs").exists())
        self.assertEqual(BUILD_FILES[0], bundle_info()["CFBundleExecutable"])

    def test_rehearsal_launch_isolates_identity_and_home_and_terminates_exact_child(self):
        original = (self.app / "Contents/Info.plist").read_bytes()
        observed = {}

        def launch(command, **kwargs):
            executable = Path(command[0])
            info = plistlib.loads((executable.parent.parent / "Info.plist").read_bytes())
            self.assertEqual(info["CFBundleIdentifier"], "com.localdictation.gpui-packaging-qa")
            self.assertNotEqual(executable, self.app / "Contents/MacOS/ui")
            home = Path(kwargs["env"]["HOME"])
            self.assertTrue(home.is_dir())
            self.assertEqual(kwargs["env"]["CFFIXED_USER_HOME"], str(home))
            observed["home"] = home
            return child

        from unittest.mock import Mock
        child = Mock()
        child.poll.return_value = None
        with patch("rehearse_gpui_bundle.finalize") as finalize, \
             patch("rehearse_gpui_bundle.subprocess.Popen", side_effect=launch), \
             patch("rehearse_gpui_bundle.time.sleep"):
            result = isolated_launch(self.app, self.root, {"HOME": "/synthetic/original"})
        self.assertIn("production entry", result)
        self.assertEqual(finalize.call_args.args[1], "-")
        child.terminate.assert_called_once_with()
        child.wait.assert_called_once_with(timeout=10)
        child.kill.assert_not_called()
        self.assertFalse(observed["home"].exists())
        self.assertEqual((self.app / "Contents/Info.plist").read_bytes(), original)

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

    def test_signed_lane_uses_downloaded_helpers_instead_of_checkout_binaries(self):
        helpers = self.root / "downloaded-helpers"
        helpers.mkdir()
        for name in HELPERS:
            (helpers / f"{name}-aarch64-apple-darwin").write_bytes(b"downloaded " + name.encode())
        shutil.rmtree(self.tauri / "binaries")
        app = bundle(self.binary, self.root / "downloaded.app", self.tauri, helpers_dir=helpers)
        for name in HELPERS:
            self.assertEqual((app / "Contents/MacOS" / name).read_bytes(), b"downloaded " + name.encode())

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
             patch("sys.argv", ["release_gpui.py", "sign", "--binary", str(self.binary), "--helpers-dir", str(self.tauri / "binaries"), "--native-signer", str(self.root / "signer/murmur-updater-signer"), "--output", str(output)]), \
             patch("release_gpui.bundle", side_effect=lambda binary, app, **kwargs: bundle(binary, app, self.tauri, **kwargs)), \
             patch("release_gpui.check_links"), patch("release_gpui.finalize") as finalize, \
             patch("release_gpui.subprocess.run", side_effect=process) as commands, \
             patch("release_gpui.run", side_effect=capture), \
             patch("release_gpui.signature_details", return_value="TeamIdentifier=ABCDEFGHIJ\n"), \
             patch("release_gpui.entitlements", return_value={}):
            release_gpui.main()
        finalize.assert_called_once_with(output.resolve() / "bundle/Murmur.app", "synthetic identity", "ABCDEFGHIJ")
        self.assertFalse(any(call.args[0][0] == "cargo" for call in commands.call_args_list),
                         "the credentialed sign phase must not compile")
        notarize = next(call.args[0] for call in commands.call_args_list
                        if "scripts/notarize_and_repackage_macos.py" in call.args[0])
        self.assertIn("--native-signer", notarize)
        self.assertEqual(notarize[-1], str(self.root / "signer/murmur-updater-signer"))
        provenance = json.loads((output / "artifacts/provenance.json").read_text())
        self.assertEqual(provenance["bundle_target"], "gpui")
        self.assertIs(provenance["publication"], False)
        self.assertEqual(provenance["commit_sha"], "a" * 40)
        self.assertEqual(provenance["workflow_run_id"], 42)
        for name in ("capture_agent", "capture_helper", "capture_worker"):
            self.assertEqual(provenance[name]["team_id"], "ABCDEFGHIJ")
            self.assertEqual(provenance[name]["architecture"], "arm64")


class ArtifactTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.inputs = self.root / "inputs"
        self.inputs.mkdir()
        for name in (*BUILD_FILES, *SIGNER_FILES):
            (self.inputs / name).write_bytes(b"synthetic " + name.encode())

    def wrap_zip(self, kind, payload, members=None):
        archive = self.root / "artifact.zip"
        with warnings.catch_warnings(), zipfile.ZipFile(archive, "w") as output:
            warnings.simplefilter("ignore", UserWarning)  # Deliberate duplicate members.
            for name in members if members is not None else [f"gpui-{kind}.tar"]:
                output.writestr(name, payload)
        return archive

    def test_raw_zip_roundtrip_and_hash_validation_for_both_builders(self):
        for kind, names in (("build", BUILD_FILES), ("signer", SIGNER_FILES)):
            archive = self.root / f"{kind}.tar"
            digest = pack(kind, archive, self.inputs / names[0], self.inputs)
            raw = self.wrap_zip(kind, archive.read_bytes())
            with self.subTest(kind=kind):
                output = self.root / kind
                verify_zip(kind, raw, digest, output)
                self.assertEqual({p.name for p in output.iterdir()}, set(names))
                for expected in ("", "0" * 64, digest.upper()):
                    with self.assertRaisesRegex(ValueError, "sha256 mismatch"):
                        verify_zip(kind, raw, expected, self.root / "rejected")
                self.assertFalse((self.root / "rejected").exists())

    def test_raw_zip_rejects_traversal_extra_duplicate_absolute_and_link_members(self):
        trusted = self.root / "murmur-app/murmur-app/scripts/notarize_and_repackage_macos.py"
        trusted.parent.mkdir(parents=True)
        trusted.write_bytes(b"trusted script")
        for kind, names in (("build", BUILD_FILES), ("signer", SIGNER_FILES)):
            tar = self.root / f"{kind}.tar"
            digest = pack(kind, tar, self.inputs / names[0], self.inputs)
            expected = f"gpui-{kind}.tar"
            link = zipfile.ZipInfo(expected)
            link.create_system = 3
            link.external_attr = (stat.S_IFLNK | 0o777) << 16
            mutations = ([expected, "nested/../../../murmur-app/murmur-app/scripts/notarize_and_repackage_macos.py"],
                         [expected, "extra"], [expected, expected], ["/" + expected],
                         ["../" + expected], ["nested/../" + expected], [link], [])
            for members in mutations:
                with self.subTest(kind=kind, members=members):
                    raw = self.wrap_zip(kind, tar.read_bytes(), members)
                    with self.assertRaisesRegex(ValueError, "ZIP must contain exactly"):
                        verify_zip(kind, raw, digest, self.root / "output")
                    self.assertFalse((self.root / "output").exists())
                    self.assertEqual(trusted.read_bytes(), b"trusted script")
                    # The payload's valid hash must not make the outer ZIP acceptable.
                    self.assertEqual(sha256(tar), digest)

    def test_raw_zip_rejects_malicious_inner_tars_for_both_builders(self):
        for kind, names in (("build", BUILD_FILES), ("signer", SIGNER_FILES)):
            for mutation in ("traversal", "absolute", "extra", "duplicate", "symlink", "hardlink", "device", "fifo"):
                payload = io.BytesIO()
                with tarfile.open(fileobj=payload, mode="w") as output:
                    for name in names:
                        member = tarfile.TarInfo(name)
                        if name == names[0]:
                            if mutation == "traversal":
                                member.name = "nested/../../" + name
                            elif mutation == "absolute":
                                member.name = "/" + name
                            elif mutation in ("symlink", "hardlink", "device", "fifo"):
                                member.type = {"symlink": tarfile.SYMTYPE, "hardlink": tarfile.LNKTYPE,
                                               "device": tarfile.CHRTYPE, "fifo": tarfile.FIFOTYPE}[mutation]
                                member.linkname = "../trusted-script"
                        output.addfile(member)
                    if mutation in ("extra", "duplicate"):
                        output.addfile(tarfile.TarInfo("extra" if mutation == "extra" else names[0]))
                data = payload.getvalue()
                raw = self.wrap_zip(kind, data)
                with self.subTest(kind=kind, mutation=mutation), self.assertRaisesRegex(ValueError, "allowlisted regular binaries"):
                    verify_zip(kind, raw, hashlib.sha256(data).hexdigest(), self.root / "output")
                self.assertFalse((self.root / "output").exists())

    def test_oversized_archives_are_rejected_before_full_decompression(self):
        # A small, highly compressible payload must not be inflated past the cap.
        data = b"\0" * (64 * 1024)
        raw = self.wrap_zip("signer", data)
        with patch.object(gpui_artifacts, "MAX_PAYLOAD_BYTES", 1024), \
             self.assertRaisesRegex(ValueError, "size limit"):
            verify_zip("signer", raw, hashlib.sha256(data).hexdigest(), self.root / "output")
        with patch.object(gpui_artifacts, "MAX_ZIP_BYTES", 16), \
             self.assertRaisesRegex(ValueError, "size limit"):
            verify_zip("signer", raw, hashlib.sha256(data).hexdigest(), self.root / "output")
        self.assertFalse((self.root / "output").exists())

    def test_separate_archives_transfer_only_expected_payload_and_restore_executable_modes(self):
        for kind, names in (("build", BUILD_FILES), ("signer", SIGNER_FILES)):
            with self.subTest(kind=kind):
                archive = self.root / f"{kind}.tar"
                digest = pack(kind, archive, self.inputs / names[0], self.inputs)
                self.assertEqual(digest, hashlib.sha256(archive.read_bytes()).hexdigest())
                output = self.root / kind
                verify(kind, archive, digest, output)
                self.assertEqual({p.name for p in output.iterdir()}, set(names))
                for name in names:
                    self.assertEqual((output / name).read_bytes(), (self.inputs / name).read_bytes())
                    self.assertEqual((output / name).stat().st_mode & 0o777, 0o755)

    def test_changed_or_missing_job_output_hash_is_rejected_before_extracting(self):
        archive = self.root / "build.tar"
        digest = pack("build", archive, self.inputs / "ui", self.inputs)
        for expected in ("", "0" * 64, digest.upper()):
            with self.subTest(expected=expected), self.assertRaisesRegex(ValueError, "sha256 mismatch"):
                verify("build", archive, expected, self.root / "output")
        archive.write_bytes(archive.read_bytes() + b"tampered")
        with self.assertRaisesRegex(ValueError, "sha256 mismatch"):
            verify("build", archive, digest, self.root / "output")
        self.assertFalse((self.root / "output").exists())

    def test_app_archive_cannot_supply_signer_scripts_links_or_traversal(self):
        mutations = (("scripts/notarize_and_repackage_macos.py", tarfile.REGTYPE),
                     ("murmur-updater-signer", tarfile.REGTYPE),
                     ("../ui", tarfile.REGTYPE), ("/ui", tarfile.REGTYPE),
                     ("ui", tarfile.SYMTYPE), ("ui", tarfile.LNKTYPE),
                     ("ui", tarfile.DIRTYPE), ("ui", tarfile.REGTYPE))
        for index, (name, kind) in enumerate(mutations):
            with self.subTest(name=name, kind=kind):
                archive = self.root / f"mutated-{index}.tar"
                with tarfile.open(archive, "w") as output:
                    for allowed in BUILD_FILES:
                        member = tarfile.TarInfo(allowed)
                        member.size = 1
                        output.addfile(member, io.BytesIO(b"x"))
                    extra = tarfile.TarInfo(name)
                    extra.type = kind
                    if kind in (tarfile.SYMTYPE, tarfile.LNKTYPE):
                        extra.linkname = "../trusted-script"
                    output.addfile(extra)
                with self.assertRaisesRegex(ValueError, "allowlisted regular binaries"):
                    verify("build", archive, sha256(archive), self.root / "output")
                self.assertFalse((self.root / "output").exists())

    def test_link_replacing_expected_member_and_missing_member_are_rejected(self):
        for kind in (tarfile.SYMTYPE, tarfile.LNKTYPE, None):
            archive = self.root / f"replace-{kind}.tar"
            with tarfile.open(archive, "w") as output:
                for name in BUILD_FILES:
                    if name == "ui" and kind is None:
                        continue
                    member = tarfile.TarInfo(name)
                    if name == "ui":
                        member.type = kind
                        member.linkname = "../trusted-script"
                    output.addfile(member)
            with self.subTest(kind=kind), self.assertRaisesRegex(ValueError, "allowlisted regular binaries"):
                verify("build", archive, sha256(archive), self.root / "output")

    def test_build_artifact_is_not_a_signer_artifact(self):
        archive = self.root / "app.tar"
        digest = pack("build", archive, self.inputs / "ui", self.inputs)
        with self.assertRaisesRegex(ValueError, "allowlisted regular binaries"):
            verify("signer", archive, digest, self.root / "output")


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
        self.assertEqual([call.args[0] for call in process.call_args_list],
                         [["cargo", "build", "--locked", "--release", "--features", "production-host", "--bin", "ui"]])
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

    def test_signer_build_uses_only_its_independent_locked_tree(self):
        import release_gpui
        with patch.dict("os.environ", {}, clear=True), patch("release_gpui.subprocess.run") as process:
            release_gpui.build(Path("/synthetic/signer-target"), signer_only=True)
        process.assert_called_once()
        self.assertEqual(process.call_args.args[0], ["cargo", "build", "--locked", "--release",
                         "--manifest-path", str(ROOT / "scripts/updater-signer/Cargo.toml")])
        self.assertEqual(process.call_args.kwargs["env"]["CARGO_TARGET_DIR"], "/synthetic/signer-target")
        with patch.dict("os.environ", {"APPLE_PASSWORD": "synthetic"}, clear=True), \
             patch("release_gpui.subprocess.run") as process:
            with self.assertRaisesRegex(ValueError, "without credentials"):
                release_gpui.build(Path("/synthetic/signer-target"), signer_only=True)
            process.assert_not_called()

    def test_inherited_workflow_and_job_secrets_are_rejected(self):
        workflow = (ROOT / ".github/workflows/release-build.yml").read_text()
        mutations = [workflow.replace("\njobs:\n", "\nenv:\n  APPLE_PASSWORD: ${{ secrets.APPLE_PASSWORD }}\n\njobs:\n"),
                     workflow.replace("\njobs:\n", "\nenv: { APPLE_PASSWORD: '${{ secrets.APPLE_PASSWORD }}' }\n\njobs:\n")]
        for job in ("gpui-build", "gpui-signer", "gpui-sign"):
            mutations.append(workflow.replace(f"  {job}:\n", f"  {job}:\n    env:\n      APPLE_PASSWORD: ${{{{ secrets.APPLE_PASSWORD }}}}\n"))
        for mutated in mutations:
            with self.subTest(mutation=mutated[-120:]):
                self.assertNotEqual(workflow, mutated)
                with self.assertRaises(AssertionError):
                    validate_release_build(mutated)

    def test_all_secret_context_forms_are_rejected_even_after_deliberately_repinning_jobs(self):
        workflow = (ROOT / ".github/workflows/release-build.yml").read_text()
        expressions = ("${{ secrets.APPLE_PASSWORD }}", "${{ secrets['APPLE_PASSWORD'] }}",
                       "${{ secrets }}", "${{ toJSON(secrets) }}", "${{ format('{0}', toJSON(secrets)) }}",
                       "${{ format('}}{0}', toJSON(secrets)) }}")
        for expression in expressions:
            self.assertTrue(references_secrets(expression), expression)
            scopes = [workflow.replace("\njobs:\n", f"\nenv: {{ PAYLOAD: '{expression}' }}\n\njobs:\n")]
            for name in GPUI_JOB_SHA256:
                scopes.append(workflow.replace(f"  {name}:\n", f"  {name}:\n    env: {{ PAYLOAD: '{expression}' }}\n"))
            for name in ("gpui-build", "gpui-signer"):
                block = job_block(workflow, name)
                scopes.append(workflow.replace(block, block.replace("    steps:\n", f"    environment: {expression}\n    steps:\n")))
            for mutated in scopes:
                pins = {name: hashlib.sha256(job_block(mutated, name).encode()).hexdigest() for name in GPUI_JOB_SHA256}
                with self.subTest(expression=expression), patch.dict(GPUI_JOB_SHA256, pins), self.assertRaisesRegex(AssertionError, "secrets|credential-free"):
                    validate_release_build(mutated)
        for name in ("gpui-build", "gpui-signer", "gpui-sign"):
            mutated = workflow.replace(f"  {name}:\n", f"  {name}:\n    secrets: inherit\n")
            with self.subTest(job=name), self.assertRaises(AssertionError):
                validate_release_build(mutated)
        self.assertTrue(references_secrets("secrets: inherit"))
        self.assertFalse(references_secrets("${{ github.token }}"))

    def test_all_gpui_job_bytes_are_pinned(self):
        workflow = (ROOT / ".github/workflows/release-build.yml").read_text()
        expected = {
            "gpui-build": "6c771424f82f3fc52d7841c04692186def2a0e9e7a59a6f5ed1539b044dceb52",
            "gpui-signer": "eb7ac7e93ae582b2da6df56dbfb205d9053ff8323e3cafe62d42a81b7678760a",
            "gpui-sign": "17727ea273f3d5e5922c31d571fe8d1a7110c43bf8e311101b831750d0841888",
        }
        self.assertEqual(GPUI_JOB_SHA256, expected)
        for name, digest in expected.items():
            block = job_block(workflow, name)
            self.assertEqual(hashlib.sha256(block.encode()).hexdigest(), digest)
            with self.subTest(job=name), self.assertRaisesRegex(AssertionError, "SHA-256 pin"):
                validate_release_build(workflow.replace(block, block + "    # Unreviewed edit\n"))

    def test_whole_release_workflow_is_pinned(self):
        workflow = (ROOT / ".github/workflows/release-build.yml").read_text()
        self.assertEqual(hashlib.sha256(workflow.encode()).hexdigest(), RELEASE_BUILD_SHA256)
        shell = ("defaults:\n  run:\n    shell: bash --noprofile --norc -c 'source \"$1\"; "
                 "if [ -f \"$RUNNER_TEMP/gpui-build/ui\" ]; then \"$RUNNER_TEMP/gpui-build/ui\"; fi' -- {0}\n\n")
        escaped = 'env:\n  PAYLOAD: "${{ toJSON(\\u0073ecrets) }}"\n\n'
        trailing_job = "  later:\n    runs-on: ubuntu-latest\n    steps:\n      - run: true\n\n"
        mutations = [workflow.replace("\njobs:\n", "\n" + addition + "jobs:\n", 1)
                     for addition in (shell, escaped, "env:\n  FLAG: harmless\n\n")]
        # Root keys appended after a terminating job sit outside every job pin.
        mutations += [workflow.rstrip("\n") + "\n\n" + trailing_job + addition for addition in (shell, escaped)]
        mutations.append(workflow.replace("source-sha: ${{ steps.context.outputs.source_sha }}",
                                          "source-sha: refs/heads/unreviewed"))
        for mutated in mutations:
            self.assertNotEqual(mutated, workflow)
            with self.subTest(mutated=mutated[-200:]), self.assertRaisesRegex(AssertionError, "reviewed SHA-256 pin"):
                validate_release_build(mutated)

    def test_every_compilation_step_and_builder_job_are_secretless(self):
        workflow = (ROOT / ".github/workflows/release-build.yml").read_text()
        for name in ("Build capture helpers", "Build GPUI rehearsal binaries without credentials",
                     "Build independent updater signer without credentials"):
            mutated = workflow.replace(f"      - name: {name}\n",
                f"      - name: {name}\n        env:\n          APPLE_PASSWORD: ${{{{ secrets.APPLE_PASSWORD }}}}\n")
            with self.subTest(step=name), self.assertRaises(AssertionError):
                validate_release_build(mutated)
        for job in ("gpui-build", "gpui-signer"):
            block = job_block(workflow, job)
            for extra in ("    environment: ${{ secrets.BUILD_ENVIRONMENT }}\n",
                          "    env: { APPLE_ID: '${{ secrets.APPLE_ID }}' }\n"):
                mutated = workflow.replace(block, extra + block)
                with self.subTest(job=job, extra=extra), self.assertRaises(AssertionError):
                    validate_release_build(mutated)

    def test_signing_job_rejects_compilation_in_any_step_or_appended_to_sign_command(self):
        workflow = (ROOT / ".github/workflows/release-build.yml").read_text()
        sign = job_block(workflow, "gpui-sign")
        compilers = ("cargo build --locked --release --bin ui", "npm run build", "npx vite build",
                     'cd app/gpui && c"ar"go build', "python3 -m pip install malicious", "brew install malicious",
                     "meson setup build", "cmake --build build", "swift build", "swiftc main.swift",
                     "python3 scripts/build_capture_helpers.py --release",
                     "python3 scripts/release_gpui.py build --target-dir /tmp/target")
        for command in compilers:
            for modified in (sign.replace('--output "$RUNNER_TEMP/gpui-release"',
                                          '--output "$RUNNER_TEMP/gpui-release" && ' + command),
                             sign.replace("    steps:\n", "    steps:\n      - name: Hidden compilation\n        run: " + command + "\n")):
                with self.subTest(command=command), self.assertRaisesRegex(AssertionError, "SHA-256 pin"):
                    validate_release_build(workflow.replace(sign, modified))

    def test_isolation_hash_checkout_and_nonpublication_gates_cannot_be_removed(self):
        workflow = (ROOT / ".github/workflows/release-build.yml").read_text()
        sign = job_block(workflow, "gpui-sign")
        for marker in ("needs: [context, gpui-build, gpui-signer]", "Check out trusted signing scripts",
                       "ref: ${{ needs.context.outputs.source-sha }}", "persist-credentials: false",
                       "Verify artifact hashes and extract allowlisted binaries",
                       "needs.gpui-build.outputs.sha256", "needs.gpui-signer.outputs.sha256",
                       '--sha256 "$BUILD_SHA256"', '--sha256 "$SIGNER_SHA256"',
                       "python3 scripts/gpui_artifacts.py verify-zip build",
                       "python3 scripts/gpui_artifacts.py verify-zip signer",
                       '"$RUNNER_TEMP/gpui-build.zip"',
                       '"$RUNNER_TEMP/gpui-signer.zip"',
                       '--native-signer "$RUNNER_TEMP/gpui-signer/murmur-updater-signer"'):
            with self.subTest(marker=marker), self.assertRaises(AssertionError):
                validate_release_build(workflow.replace(sign, sign.replace(marker, "removed")))
        for job in ("gpui-build", "gpui-signer", "gpui-sign"):
            block = job_block(workflow, job)
            for old, new in (("persist-credentials: false", "persist-credentials: true"),
                             ("runs-on: macos-latest", "runs-on: ubuntu-latest"),
                             ("    steps:\n", "    permissions:\n      contents: write\n    steps:\n"),
                             ("name: gpui-", "name: macos-release-")):
                with self.subTest(job=job, old=old), self.assertRaises(AssertionError):
                    validate_release_build(workflow.replace(block, block.replace(old, new)))
        promotion = (ROOT / ".github/workflows/release.yml").read_text()
        for artifact in ("gpui-build-", "gpui-signer-", "gpui-rehearsal-"):
            with self.subTest(artifact=artifact), self.assertRaises(AssertionError):
                validate_promotion_policy(promotion.replace("macos-release-", artifact))

    def test_signed_script_rejects_non_manual_or_non_main_before_work(self):
        import release_gpui
        for phase in ("build", "build-signer", "sign"):
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
